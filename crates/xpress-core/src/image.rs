//! Pure-Rust image optimisation and conversion.
//!
//! No external binaries are needed for raster images: PNG uses `imagequant`
//! (lossy quantisation) + `oxipng` (lossless squeeze), JPEG/GIF/WebP/BMP/TIFF go
//! through the `image` crate. HEIC/JXL conversion still shells out (no practical
//! pure-Rust encoder), which is why those remain optional.

use std::path::{Path, PathBuf};

use tempfile::TempDir;

use crate::compression::CompressionQuality;
use crate::filetype::{extension_lower, MediaKind};
use crate::result::{
    backup_file, copy_dates, file_name_lossy, file_size, file_stem_lossy, OptimisationResult,
    OptimiseError, OptimiseOptions,
};
use crate::tools::{self, Tool};

fn other<E: std::fmt::Display>(e: E) -> OptimiseError {
    OptimiseError::Other(e.to_string())
}

/// Optimise an image in place (or to `options.output`).
pub fn optimise(path: &Path, options: &OptimiseOptions) -> Result<OptimisationResult, OptimiseError> {
    if !path.is_file() {
        return Err(OptimiseError::NotFound(path.to_path_buf()));
    }
    let ext = extension_lower(path).ok_or_else(|| OptimiseError::Unsupported(path.to_path_buf()))?;
    let old_size = file_size(path);
    let cq = options.compression;

    let tmp = TempDir::new()?;
    let temp_out = tmp.path().join(file_name_lossy(path));

    match ext.as_str() {
        "png" => optimise_png(path, &temp_out, cq)?,
        "jpg" | "jpeg" => optimise_jpeg(path, &temp_out, cq)?,
        "gif" => reencode(path, &temp_out)?,
        "webp" | "bmp" | "tiff" | "tif" => reencode(path, &temp_out)?,
        _ => return Err(OptimiseError::Unsupported(path.to_path_buf())),
    }

    finalise(path, &temp_out, old_size, cq, options)
}

/// PNG: quantise to a palette (quality from the compression value) then run a
/// lossless oxipng pass.
fn optimise_png(src: &Path, out: &Path, cq: CompressionQuality) -> Result<(), OptimiseError> {
    let img = image::open(src).map_err(other)?.to_rgba8();
    let (w, h) = (img.width() as usize, img.height() as usize);
    let pixels: Vec<imagequant::RGBA> = img
        .pixels()
        .map(|p| imagequant::RGBA {
            r: p.0[0],
            g: p.0[1],
            b: p.0[2],
            a: p.0[3],
        })
        .collect();

    // Map the compression factor to a pngquant-like quality ceiling + speed.
    let qmax: u8 = cq
        .pngquant_quality()
        .rsplit('-')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(90);
    let qmin = qmax.saturating_sub(30);
    let speed = cq.pngquant_speed().clamp(1, 10);

    let png_bytes = (|| -> Result<Vec<u8>, OptimiseError> {
        let mut liq = imagequant::new();
        liq.set_speed(speed).map_err(other)?;
        liq.set_quality(qmin, qmax).map_err(other)?;
        let mut qimg = liq
            .new_image_borrowed(&pixels, w, h, 0.0)
            .map_err(other)?;
        let mut res = liq.quantize(&mut qimg).map_err(other)?;
        res.set_dithering_level(1.0).map_err(other)?;
        let (palette, indices) = res.remapped(&mut qimg).map_err(other)?;

        let mut buf = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut buf, w as u32, h as u32);
            enc.set_color(png::ColorType::Indexed);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_palette(palette.iter().flat_map(|c| [c.r, c.g, c.b]).collect::<Vec<u8>>());
            enc.set_trns(palette.iter().map(|c| c.a).collect::<Vec<u8>>());
            let mut writer = enc.write_header().map_err(other)?;
            writer.write_image_data(&indices).map_err(other)?;
        }
        Ok(buf)
    })();

    // If quantisation fails (e.g. quality too strict), fall back to a lossless
    // oxipng pass on the original.
    let raw = match png_bytes {
        Ok(b) => b,
        Err(_) => std::fs::read(src)?,
    };
    let opts = oxipng::Options::from_preset(2);
    let optimised = oxipng::optimize_from_memory(&raw, &opts).unwrap_or(raw);
    std::fs::write(out, optimised)?;
    Ok(())
}

/// JPEG: decode and re-encode at a quality derived from the compression value.
fn optimise_jpeg(src: &Path, out: &Path, cq: CompressionQuality) -> Result<(), OptimiseError> {
    let img = image::open(src).map_err(other)?;
    let q = cq.jpeg_max_quality().clamp(1, 100) as u8;
    let mut f = std::fs::File::create(out)?;
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut f, q);
    enc.encode_image(&img).map_err(other)?;
    Ok(())
}

/// Decode and re-encode in the same format (used for GIF/WebP/BMP/TIFF).
fn reencode(src: &Path, out: &Path) -> Result<(), OptimiseError> {
    let img = image::open(src).map_err(other)?;
    img.save(out).map_err(other)?;
    Ok(())
}

/// Move the temp output into place, honouring backup / dates / size guard.
fn finalise(
    src: &Path,
    temp_out: &Path,
    old_size: u64,
    cq: CompressionQuality,
    options: &OptimiseOptions,
) -> Result<OptimisationResult, OptimiseError> {
    let new_size = file_size(temp_out);

    if !options.allow_larger && (new_size == 0 || new_size >= old_size) {
        return Ok(OptimisationResult {
            kind: MediaKind::Image,
            source: src.to_path_buf(),
            output: src.to_path_buf(),
            backup: None,
            old_size,
            new_size: old_size,
            aggressive: cq.image_is_aggressive(),
        });
    }

    let backup = if options.backup && options.output.is_none() {
        Some(backup_file(src)?)
    } else {
        None
    };

    let dest = options.output.clone().unwrap_or_else(|| src.to_path_buf());
    std::fs::copy(temp_out, &dest)?;
    if options.preserve_dates {
        copy_dates(src, &dest);
    }

    Ok(OptimisationResult {
        kind: MediaKind::Image,
        source: src.to_path_buf(),
        output: dest,
        backup,
        old_size,
        new_size,
        aggressive: cq.image_is_aggressive(),
    })
}

// ---------------------------------------------------------------------------
// Adaptive
// ---------------------------------------------------------------------------

/// Cheap transparency check: reads the PNG IHDR colour-type byte (6 = RGBA,
/// 4 = gray+alpha). Non-PNG inputs conservatively report no alpha.
pub fn has_alpha(path: &Path) -> bool {
    let Some(ext) = extension_lower(path) else {
        return false;
    };
    if ext != "png" {
        return false;
    }
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut header = [0u8; 26];
    if f.read_exact(&mut header).is_err() {
        return false;
    }
    matches!(header[25], 4 | 6)
}

/// Adaptively optimise an image: try the in-format optimisation plus JPEG and
/// PNG candidates, keep the smallest. Skips the JPEG candidate for images with
/// an alpha channel (so transparency is never flattened).
pub fn optimise_adaptive(
    path: &Path,
    options: &OptimiseOptions,
) -> Result<OptimisationResult, OptimiseError> {
    if !path.is_file() {
        return Err(OptimiseError::NotFound(path.to_path_buf()));
    }
    let old_size = file_size(path);
    let tmp = TempDir::new()?;

    let src_ext = extension_lower(path).unwrap_or_else(|| "png".into());
    let c_same = tmp.path().join(format!("c0_same.{src_ext}"));
    let same_opts = OptimiseOptions {
        output: Some(c_same.clone()),
        backup: false,
        allow_larger: true,
        ..options.clone()
    };
    let mut candidates: Vec<(PathBuf, ImageFormat)> = Vec::new();
    if optimise(path, &same_opts).is_ok() {
        let f = ImageFormat::from_str(&src_ext).unwrap_or(ImageFormat::Png);
        candidates.push((c_same, f));
    }

    let formats: &[ImageFormat] = if has_alpha(path) {
        &[ImageFormat::Png]
    } else {
        &[ImageFormat::Jpeg, ImageFormat::Png]
    };
    for (i, fmt) in formats.iter().copied().enumerate() {
        let c = tmp.path().join(format!("c{}.{}", i + 1, fmt.extension()));
        let opts = OptimiseOptions {
            output: Some(c.clone()),
            backup: false,
            allow_larger: true,
            ..options.clone()
        };
        if convert(path, fmt, &opts).is_ok() {
            candidates.push((c, fmt));
        }
    }

    let (best_path, best_fmt) = candidates
        .into_iter()
        .filter(|(p, _)| file_size(p) > 0)
        .min_by_key(|(p, _)| file_size(p))
        .ok_or_else(|| OptimiseError::Other("adaptive: no candidate produced".into()))?;
    let new_size = file_size(&best_path);

    if !options.allow_larger && new_size >= old_size {
        return Ok(OptimisationResult {
            kind: MediaKind::Image,
            source: path.to_path_buf(),
            output: path.to_path_buf(),
            backup: None,
            old_size,
            new_size: old_size,
            aggressive: options.compression.image_is_aggressive(),
        });
    }

    let same_ext = extension_lower(path).as_deref() == Some(best_fmt.extension());
    let dest = options.output.clone().unwrap_or_else(|| {
        if same_ext {
            path.to_path_buf()
        } else {
            path.with_extension(best_fmt.extension())
        }
    });
    let backup = if options.backup && options.output.is_none() && same_ext {
        Some(backup_file(path)?)
    } else {
        None
    };
    std::fs::copy(&best_path, &dest)?;
    if options.preserve_dates {
        copy_dates(path, &dest);
    }

    Ok(OptimisationResult {
        kind: MediaKind::Image,
        source: path.to_path_buf(),
        output: dest,
        backup,
        old_size,
        new_size,
        aggressive: options.compression.image_is_aggressive(),
    })
}

// ---------------------------------------------------------------------------
// Conversion
// ---------------------------------------------------------------------------

/// Target formats for image conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Webp,
    Avif,
    Heic,
    Jxl,
    Png,
    Jpeg,
}

impl ImageFormat {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<ImageFormat> {
        match s.to_ascii_lowercase().as_str() {
            "webp" => Some(ImageFormat::Webp),
            "avif" => Some(ImageFormat::Avif),
            "heic" | "heif" => Some(ImageFormat::Heic),
            "jxl" => Some(ImageFormat::Jxl),
            "png" => Some(ImageFormat::Png),
            "jpeg" | "jpg" => Some(ImageFormat::Jpeg),
            _ => None,
        }
    }

    pub fn extension(&self) -> &'static str {
        match self {
            ImageFormat::Webp => "webp",
            ImageFormat::Avif => "avif",
            ImageFormat::Heic => "heic",
            ImageFormat::Jxl => "jxl",
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
        }
    }
}

/// Convert an image to another format.
///
/// PNG/JPEG/WebP are handled natively in Rust; HEIC/JXL still require the
/// external `heif-enc`/`cjxl` tools.
pub fn convert(
    path: &Path,
    format: ImageFormat,
    options: &OptimiseOptions,
) -> Result<OptimisationResult, OptimiseError> {
    if !path.is_file() {
        return Err(OptimiseError::NotFound(path.to_path_buf()));
    }
    let old_size = file_size(path);
    let cq = options.compression;

    let tmp = TempDir::new()?;
    let temp_out = tmp
        .path()
        .join(format!("{}.{}", file_stem_lossy(path), format.extension()));

    match format {
        ImageFormat::Png => optimise_png(path, &temp_out, cq)?,
        ImageFormat::Jpeg => optimise_jpeg(path, &temp_out, cq)?,
        ImageFormat::Webp => {
            // image crate writes lossless WebP.
            let img = image::open(path).map_err(other)?;
            img.save(&temp_out).map_err(other)?;
        }
        ImageFormat::Avif => {
            let img = image::open(path).map_err(other)?;
            img.save(&temp_out).map_err(other)?; // image crate AVIF encoder
        }
        ImageFormat::Heic => {
            let q = cq.conversion_quality().to_string();
            tools::run_with_retries(
                Tool::HeifEnc,
                ["-q", &q, "-o", &temp_out.display().to_string(), &path.display().to_string()],
                2,
            )?;
        }
        ImageFormat::Jxl => {
            tools::run_with_retries(
                Tool::Cjxl,
                [
                    "-q",
                    &cq.jxl_quality().to_string(),
                    "-e",
                    &cq.jxl_effort().to_string(),
                    &path.display().to_string(),
                    &temp_out.display().to_string(),
                ],
                2,
            )?;
        }
    }

    let new_size = file_size(&temp_out);
    let dest = options
        .output
        .clone()
        .unwrap_or_else(|| path.with_extension(format.extension()));
    std::fs::copy(&temp_out, &dest)?;
    if options.preserve_dates {
        copy_dates(path, &dest);
    }

    Ok(OptimisationResult {
        kind: MediaKind::Image,
        source: path.to_path_buf(),
        output: dest,
        backup: None,
        old_size,
        new_size,
        aggressive: cq.image_is_aggressive(),
    })
}

// ---------------------------------------------------------------------------
// Resize / crop (pure Rust, used by scale.rs and crop.rs)
// ---------------------------------------------------------------------------

/// Load an image, run `f` to transform it, and save to `out` (format from ext).
pub fn transform(
    src: &Path,
    out: &Path,
    f: impl FnOnce(image::DynamicImage) -> image::DynamicImage,
) -> Result<(), OptimiseError> {
    let img = image::open(src).map_err(other)?;
    f(img).save(out).map_err(other)?;
    Ok(())
}

/// Scale an image by `factor` (0.0–1.0), writing to `out`.
pub fn scale_image(src: &Path, out: &Path, factor: f64) -> Result<(), OptimiseError> {
    let img = image::open(src).map_err(other)?;
    let w = ((img.width() as f64 * factor).round() as u32).max(1);
    let h = ((img.height() as f64 * factor).round() as u32).max(1);
    img.resize_exact(w, h, image::imageops::FilterType::Lanczos3)
        .save(out)
        .map_err(other)?;
    Ok(())
}

/// Resize an image to exactly `w`x`h` (keeps aspect only if caller computed it).
pub fn resize_to(src: &Path, out: &Path, w: u32, h: u32) -> Result<(), OptimiseError> {
    let img = image::open(src).map_err(other)?;
    img.resize_exact(w.max(1), h.max(1), image::imageops::FilterType::Lanczos3)
        .save(out)
        .map_err(other)?;
    Ok(())
}

/// Scale to cover `w`x`h` then centre-crop to exactly `w`x`h`.
pub fn cover_crop(src: &Path, out: &Path, w: u32, h: u32) -> Result<(), OptimiseError> {
    let img = image::open(src).map_err(other)?;
    let (iw, ih) = (img.width().max(1), img.height().max(1));
    let scale = (w as f64 / iw as f64).max(h as f64 / ih as f64);
    let sw = ((iw as f64 * scale).ceil() as u32).max(w);
    let sh = ((ih as f64 * scale).ceil() as u32).max(h);
    let scaled = img.resize_exact(sw, sh, image::imageops::FilterType::Lanczos3);
    let x = (sw - w) / 2;
    let y = (sh - h) / 2;
    image::imageops::crop_imm(&scaled, x, y, w, h)
        .to_image()
        .save(out)
        .map_err(other)?;
    Ok(())
}

/// Crop an image to a pixel rect then optionally resize, writing to `out`.
pub fn crop_image_px(
    src: &Path,
    out: &Path,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    resize_to: Option<(u32, u32)>,
) -> Result<(), OptimiseError> {
    let mut img = image::open(src).map_err(other)?;
    let cropped = img.crop(x, y, w, h);
    let final_img = match resize_to {
        Some((rw, rh)) => cropped.resize_exact(rw, rh, image::imageops::FilterType::Lanczos3),
        None => cropped,
    };
    final_img.save(out).map_err(other)?;
    Ok(())
}
