//! Perceptual quality targets: instead of guessing a compression factor, find
//! the smallest encode that still *looks* as good as asked.
//!
//! Candidates are scored with SSIMULACRA2 (0–100; ~90 is visually lossless at
//! 1:1, ~80 high, ~70 medium, ~50 low) against the decoded original, and the
//! compression factor is binary-searched for the strongest compression that
//! still meets the target. Images only (JPEG, PNG, WebP outputs).

use std::path::{Path, PathBuf};

use image::DynamicImage;
use tempfile::TempDir;

use crate::compression::CompressionQuality;
use crate::filetype::{classify, MediaKind};
use crate::image::{self as ximage, ImageFormat};
use crate::result::{
    file_size, finish, unchanged, OptimisationResult, OptimiseError, OptimiseOptions, Placement,
};

/// Parse a quality target: a named level or a SSIMULACRA2 score (1–100).
pub fn parse_target(s: &str) -> Result<f64, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "visually-lossless" | "lossless" | "max" => Ok(90.0),
        "high" => Ok(80.0),
        "medium" => Ok(70.0),
        "low" => Ok(50.0),
        other => match other.parse::<f64>() {
            Ok(v) if (1.0..=100.0).contains(&v) => Ok(v),
            _ => Err(format!(
                "bad quality '{s}': use visually-lossless, high, medium, low or a score 1-100"
            )),
        },
    }
}

fn to_ssim_rgb(img: &image::RgbImage) -> Option<ssimulacra2::Rgb> {
    let data = img
        .pixels()
        .map(|p| p.0.map(|c| c as f32 / 255.0))
        .collect();
    ssimulacra2::Rgb::new(
        data,
        img.width() as usize,
        img.height() as usize,
        ssimulacra2::TransferCharacteristic::SRGB,
        ssimulacra2::ColorPrimaries::BT709,
    )
    .ok()
}

fn over(img: &DynamicImage, bg: u8) -> image::RgbImage {
    let rgba = img.to_rgba8();
    image::RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let p = rgba.get_pixel(x, y).0;
        let a = p[3] as u32;
        image::Rgb(
            [p[0], p[1], p[2]].map(|c| ((c as u32 * a + bg as u32 * (255 - a) + 127) / 255) as u8),
        )
    })
}

/// SSIMULACRA2 score of `candidate` against `reference` (same dimensions).
/// Transparent images are judged over both black and white; the worse counts.
pub fn score(reference: &DynamicImage, candidate: &DynamicImage) -> Option<f64> {
    if (reference.width(), reference.height()) != (candidate.width(), candidate.height()) {
        return None;
    }
    let one = |a: image::RgbImage, b: image::RgbImage| {
        ssimulacra2::compute_frame_ssimulacra2(to_ssim_rgb(&a)?, to_ssim_rgb(&b)?).ok()
    };
    if reference.color().has_alpha() || candidate.color().has_alpha() {
        let black = one(over(reference, 0), over(candidate, 0))?;
        let white = one(over(reference, 255), over(candidate, 255))?;
        Some(black.min(white))
    } else {
        one(reference.to_rgb8(), candidate.to_rgb8())
    }
}

/// Binary-search the compression factor for the strongest compression whose
/// output scores at least `target`. `encode(factor, out)` writes a candidate.
/// Returns the chosen candidate file and its score, or `None` if even the
/// gentlest setting misses the target.
fn search(
    reference: &DynamicImage,
    target: f64,
    tmp: &Path,
    ext: &str,
    encode: impl Fn(i32, &Path) -> Result<(), OptimiseError>,
) -> Result<Option<(PathBuf, f64)>, OptimiseError> {
    let probe = |factor: i32| -> Result<Option<(PathBuf, f64)>, OptimiseError> {
        let out = tmp.join(format!("q{factor}.{ext}"));
        encode(factor, &out)?;
        let decoded = ximage::open_oriented(&out)?;
        Ok(score(reference, &decoded)
            .filter(|s| *s >= target)
            .map(|s| (out, s)))
    };
    let (mut lo, mut hi) = (5, 100);
    let Some(mut best) = probe(lo)? else {
        return Ok(None);
    };
    if let Some(hit) = probe(hi)? {
        return Ok(Some(hit));
    }
    // Invariant: `lo` meets the target, `hi` doesn't.
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        match probe(mid)? {
            Some(hit) => {
                lo = mid;
                best = hit;
            }
            None => hi = mid,
        }
    }
    Ok(Some(best))
}

fn check_image(path: &Path) -> Result<(), OptimiseError> {
    if !path.is_file() {
        return Err(OptimiseError::NotFound(path.to_path_buf()));
    }
    if classify(path) != Some(MediaKind::Image) {
        return Err(OptimiseError::Other(format!(
            "quality targets are supported for images only: {}",
            path.display()
        )));
    }
    Ok(())
}

fn staged(base: &OptimiseOptions, factor: i32, out: &Path) -> OptimiseOptions {
    OptimiseOptions {
        compression: CompressionQuality::factor(factor),
        output: Some(out.to_path_buf()),
        backup: false,
        allow_larger: true,
        preserve_dates: false,
        use_cache: false,
        ..base.clone()
    }
}

/// Optimise an image in place (or to `base.output`) to the smallest file that
/// still scores at least `target` SSIMULACRA2. Never grows the file.
pub fn optimise_to_quality(
    path: &Path,
    target: f64,
    base: &OptimiseOptions,
) -> Result<OptimisationResult, OptimiseError> {
    check_image(path)?;
    let old_size = file_size(path);
    let reference = ximage::open_oriented(path)?;
    let tmp = TempDir::new()?;
    let ext = crate::filetype::extension_lower(path).unwrap_or_else(|| "png".into());
    let found = search(&reference, target, tmp.path(), &ext, |factor, out| {
        ximage::optimise(path, &staged(base, factor, out)).map(|_| ())
    })?;
    let Some((best, s)) = found else {
        return Ok(unchanged(MediaKind::Image, path, old_size, false));
    };
    let mut r = finish(
        MediaKind::Image,
        path,
        &best,
        path.to_path_buf(),
        old_size,
        false,
        base,
        Placement {
            size_guard: true,
            backup: true,
            replace_source: false,
        },
    )?;
    if r.output != *path || r.improved() {
        r.score = Some(s);
    }
    Ok(r)
}

/// Convert an image to `format` at the smallest size that still scores at
/// least `target` SSIMULACRA2 against the original. Written alongside the
/// source unless `base.output` is set.
pub fn convert_to_quality(
    path: &Path,
    format: ImageFormat,
    target: f64,
    base: &OptimiseOptions,
) -> Result<OptimisationResult, OptimiseError> {
    check_image(path)?;
    if matches!(
        format,
        ImageFormat::Avif | ImageFormat::Heic | ImageFormat::Jxl
    ) {
        return Err(OptimiseError::Other(format!(
            "quality targets support jpeg, png and webp output, not {}",
            format.extension()
        )));
    }
    let old_size = file_size(path);
    let reference = ximage::open_oriented(path)?;
    let tmp = TempDir::new()?;
    let found = search(
        &reference,
        target,
        tmp.path(),
        format.extension(),
        |factor, out| ximage::convert(path, format, &staged(base, factor, out)).map(|_| ()),
    )?;
    let (best, s) = match found {
        Some(hit) => hit,
        // Lossy WebP always subsamples chroma, which some content can't
        // survive at high targets; lossless WebP always meets the target.
        None if format == ImageFormat::Webp => {
            let out = tmp.path().join("lossless.webp");
            ximage::write_lossless_webp(path, &out, ximage::Strip::from_options(base))?;
            if file_size(&out) > old_size {
                return Err(OptimiseError::Other(format!(
                    "webp only reaches quality {target} losslessly, which is larger than the \
                     source; try a lower --quality"
                )));
            }
            (out, 100.0)
        }
        None => {
            return Err(OptimiseError::Other(format!(
                "{} can't reach quality {target} even at the best setting",
                format.extension()
            )))
        }
    };
    let default_dest = path.with_extension(format.extension());
    let overwrites_source = default_dest == path;
    let mut r = finish(
        MediaKind::Image,
        path,
        &best,
        default_dest,
        old_size,
        false,
        base,
        Placement {
            size_guard: false,
            backup: overwrites_source,
            replace_source: false,
        },
    )?;
    r.score = Some(s);
    Ok(r)
}
