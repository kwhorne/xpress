//! Pure-Rust image optimisation and conversion.
//!
//! No external binaries are needed for raster images: PNG uses `quantette` /
//! `exoquant` (lossy palette quantisation) + `oxipng` (lossless squeeze), JPEG/GIF/BMP/TIFF/AVIF go
//! through the `image` crate and lossy WebP through `libwebp`. HEIC/HEIF (the
//! iPhone photo format) is handled on macOS by the built-in `sips` tool — no
//! install — so Apple photos convert both ways out of the box. JXL still uses the
//! optional `cjxl` tool.
//!
//! Every decode applies the EXIF orientation to the pixels (and resets the tag),
//! and carries the ICC colour profile and EXIF block through to the output, so
//! photos stay upright and keep their colours. Animated GIF/WebP/APNG are never
//! flattened to a single frame.

use std::io::BufWriter;
use std::path::{Path, PathBuf};

use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageEncoder};
use tempfile::TempDir;

use crate::compression::CompressionQuality;
use crate::filetype::{extension_lower, MediaKind};
use crate::result::{
    file_name_lossy, file_size, file_stem_lossy, finish, OptimisationResult, OptimiseError,
    OptimiseOptions, Placement,
};
use crate::tools::{self, Tool};

fn other<E: std::fmt::Display>(e: E) -> OptimiseError {
    OptimiseError::Other(e.to_string())
}

// ---------------------------------------------------------------------------
// Decoding with metadata
// ---------------------------------------------------------------------------

/// Metadata carried from the source image to the re-encoded output.
#[derive(Debug, Clone, Default)]
pub struct Meta {
    /// ICC colour profile (kept even when stripping metadata: it is colour-critical).
    pub icc: Option<Vec<u8>>,
    /// Raw EXIF (TIFF) block, with the orientation tag already reset to "normal".
    pub exif: Option<Vec<u8>>,
    /// XMP packet (edit history, ratings, captions, …).
    pub xmp: Option<Vec<u8>>,
}

/// Whether `path` is an animated image (multi-frame GIF, animated WebP or APNG).
/// Decoding such a file with [`image::open`] silently keeps only the first frame.
pub fn is_animated(path: &Path) -> bool {
    use image::AnimationDecoder;
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let reader = std::io::BufReader::new(file);
    match extension_lower(path).as_deref() {
        Some("gif") => image::codecs::gif::GifDecoder::new(reader)
            .map(|d| d.into_frames().take(2).count() > 1)
            .unwrap_or(false),
        Some("webp") => image::codecs::webp::WebPDecoder::new(reader)
            .map(|d| d.has_animation())
            .unwrap_or(false),
        Some("png") | Some("apng") => image::codecs::png::PngDecoder::new(reader)
            .and_then(|d| d.is_apng())
            .unwrap_or(false),
        _ => false,
    }
}

fn open_decoder(path: &Path) -> Result<impl ImageDecoder, OptimiseError> {
    image::ImageReader::open(path)?
        .with_guessed_format()?
        .into_decoder()
        .map_err(other)
}

/// Decode a still image upright (EXIF orientation applied to the pixels) along
/// with its ICC profile and EXIF block. Refuses animated images rather than
/// silently dropping every frame but the first.
pub fn load(path: &Path) -> Result<(DynamicImage, Meta), OptimiseError> {
    if is_animated(path) {
        return Err(OptimiseError::Other(format!(
            "{} is animated; this operation would keep only the first frame",
            path.display()
        )));
    }
    let mut decoder = open_decoder(path)?;
    let icc = decoder.icc_profile().ok().flatten();
    let mut exif = decoder.exif_metadata().ok().flatten();
    let xmp = decoder.xmp_metadata().ok().flatten();
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder).map_err(other)?;
    img.apply_orientation(orientation);
    if let Some(e) = exif.as_mut() {
        let _ = Orientation::remove_from_exif_chunk(e);
    }
    Ok((img, Meta { icc, exif, xmp }))
}

/// Decode a still image upright, without its metadata (e.g. for previews).
pub fn open_oriented(path: &Path) -> Result<DynamicImage, OptimiseError> {
    load(path).map(|(img, _)| img)
}

/// Write raw, tightly packed RGBA8 pixels (e.g. a clipboard image) as a PNG.
pub fn save_rgba_png(
    rgba: &[u8],
    width: u32,
    height: u32,
    out: &Path,
) -> Result<(), OptimiseError> {
    let img = image::RgbaImage::from_raw(width, height, rgba.to_vec())
        .ok_or_else(|| other("RGBA buffer does not match the image dimensions"))?;
    img.save_with_format(out, image::ImageFormat::Png)
        .map_err(other)
}

/// Pixel dimensions as displayed, i.e. with the EXIF orientation applied.
pub fn oriented_dimensions(path: &Path) -> Option<(u32, u32)> {
    let size = imagesize::size(path).ok()?;
    let (w, h) = (size.width as u32, size.height as u32);
    let rotated = open_decoder(path)
        .ok()
        .and_then(|mut d| d.orientation().ok())
        .is_some_and(|o| {
            matches!(
                o,
                Orientation::Rotate90
                    | Orientation::Rotate270
                    | Orientation::Rotate90FlipH
                    | Orientation::Rotate270FlipH
            )
        });
    Some(if rotated { (h, w) } else { (w, h) })
}

fn apply_meta(enc: &mut impl ImageEncoder, meta: &Meta) {
    if let Some(icc) = &meta.icc {
        let _ = enc.set_icc_profile(icc.clone());
    }
    if let Some(exif) = &meta.exif {
        let _ = enc.set_exif_metadata(exif.clone());
    }
}

/// JPEG has no alpha channel: composite transparent images onto white instead
/// of letting transparent pixels turn into whatever colour sits underneath.
fn flatten_alpha(img: &DynamicImage) -> DynamicImage {
    if !img.color().has_alpha() {
        return img.clone();
    }
    let rgba = img.to_rgba8();
    let mut rgb = image::RgbImage::new(rgba.width(), rgba.height());
    for (dst, src) in rgb.pixels_mut().zip(rgba.pixels()) {
        let a = src.0[3] as u32;
        for c in 0..3 {
            dst.0[c] = ((src.0[c] as u32 * a + 255 * (255 - a) + 127) / 255) as u8;
        }
    }
    DynamicImage::ImageRgb8(rgb)
}

fn write_jpeg(
    img: &DynamicImage,
    out: &Path,
    quality: u8,
    meta: &Meta,
) -> Result<(), OptimiseError> {
    std::fs::write(out, encode_jpeg(img, quality, meta, true)?)?;
    Ok(())
}

/// Encode a JPEG with mozjpeg (trellis quantisation, optimised Huffman
/// tables; `progressive` scans for image files — baseline where a consumer may
/// not support progressive, e.g. PDF). Keeps EXIF, ICC and XMP. Transparent
/// images are flattened onto white; grayscale stays one component.
pub(crate) fn encode_jpeg(
    img: &DynamicImage,
    quality: u8,
    meta: &Meta,
    progressive: bool,
) -> Result<Vec<u8>, OptimiseError> {
    let img = flatten_alpha(img);
    let (w, h) = (img.width() as usize, img.height() as usize);
    let (color_space, pixels) = if img.color().channel_count() == 1 {
        (
            mozjpeg::ColorSpace::JCS_GRAYSCALE,
            img.to_luma8().into_raw(),
        )
    } else {
        (mozjpeg::ColorSpace::JCS_RGB, img.to_rgb8().into_raw())
    };
    // libjpeg reports errors by unwinding; keep them from escaping as panics.
    let encoded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> std::io::Result<Vec<u8>> {
            let mut c = mozjpeg::Compress::new(color_space);
            c.set_size(w, h);
            c.set_quality(quality.clamp(1, 100) as f32);
            // 4:2:0 chroma subsampling caps how faithful colour detail can be;
            // at high quality settings keep full-resolution chroma (4:4:4).
            // (4:2:2 in between was measured: bigger files at equal
            // SSIMULACRA2, no help for saturated edges.)
            if quality >= 90 && color_space == mozjpeg::ColorSpace::JCS_RGB {
                c.set_chroma_sampling_pixel_sizes((1, 1), (1, 1));
            }
            if progressive {
                c.set_progressive_mode();
                c.set_optimize_scans(true);
            }
            c.set_optimize_coding(true);
            let mut started = c.start_compress(Vec::new())?;
            const MAX_SEGMENT: usize = 65533;
            if let Some(exif) = &meta.exif {
                let seg = [&b"Exif\0\0"[..], exif].concat();
                if seg.len() <= MAX_SEGMENT {
                    started.write_marker(mozjpeg::Marker::APP(1), &seg);
                }
            }
            if let Some(icc) = &meta.icc {
                // Not `write_icc_profile`: mozjpeg-rs numbers the chunks from 0,
                // but the ICC spec (and strict decoders) require 1-based
                // sequence numbers — the profile would be silently dropped.
                const ICC_DATA_PER_SEGMENT: usize = MAX_SEGMENT - 14;
                let chunks: Vec<&[u8]> = icc.chunks(ICC_DATA_PER_SEGMENT).collect();
                if chunks.len() <= 255 {
                    for (i, chunk) in chunks.iter().enumerate() {
                        let mut seg = b"ICC_PROFILE\0".to_vec();
                        seg.extend([i as u8 + 1, chunks.len() as u8]);
                        seg.extend_from_slice(chunk);
                        started.write_marker(mozjpeg::Marker::APP(2), &seg);
                    }
                }
            }
            if let Some(xmp) = &meta.xmp {
                let seg = [&b"http://ns.adobe.com/xap/1.0/\0"[..], xmp].concat();
                if seg.len() <= MAX_SEGMENT {
                    started.write_marker(mozjpeg::Marker::APP(1), &seg);
                }
            }
            started.write_scanlines(&pixels)?;
            started.finish()
        },
    ))
    .map_err(|_| other("jpeg encoding failed"))?
    .map_err(other)?;
    Ok(encoded)
}

/// Insert an XMP packet as an uncompressed `iTXt` chunk right after `IHDR`.
fn png_with_xmp(png: Vec<u8>, xmp: &[u8]) -> Vec<u8> {
    const IHDR_END: usize = 8 + 4 + 4 + 13 + 4;
    if png.len() < IHDR_END || &png[12..16] != b"IHDR" {
        return png;
    }
    let mut chunk = b"iTXt".to_vec();
    chunk.extend_from_slice(b"XML:com.adobe.xmp\0");
    chunk.extend_from_slice(&[0, 0, 0, 0]); // uncompressed, method, empty lang + keyword
    chunk.extend_from_slice(xmp);
    let data_len = (chunk.len() - 4) as u32;
    let crc = crc32fast::hash(&chunk);
    let mut out = Vec::with_capacity(png.len() + chunk.len() + 8);
    out.extend_from_slice(&png[..IHDR_END]);
    out.extend_from_slice(&data_len.to_be_bytes());
    out.extend_from_slice(&chunk);
    out.extend_from_slice(&crc.to_be_bytes());
    out.extend_from_slice(&png[IHDR_END..]);
    out
}

/// Save an intermediate (resized/cropped) image in the format given by `out`'s
/// extension, keeping the metadata. JPEG intermediates use a high quality since
/// they are optimised again afterwards.
fn save_with_meta(img: &DynamicImage, out: &Path, meta: &Meta) -> Result<(), OptimiseError> {
    match extension_lower(out).as_deref() {
        Some("jpg") | Some("jpeg") => write_jpeg(img, out, 95, meta),
        Some("png") => {
            let buf = encode_png(img, meta)?;
            std::fs::write(out, buf)?;
            Ok(())
        }
        Some("webp") => {
            let mut buf = Vec::new();
            img.write_with_encoder(image::codecs::webp::WebPEncoder::new_lossless(&mut buf))
                .map_err(other)?;
            let buf = webp_with_meta(
                &buf,
                meta,
                img.width(),
                img.height(),
                img.color().has_alpha(),
            )
            .unwrap_or(buf);
            std::fs::write(out, buf)?;
            Ok(())
        }
        _ => img.save(out).map_err(other),
    }
}

/// Lossless PNG with ICC, EXIF and XMP.
fn encode_png(img: &DynamicImage, meta: &Meta) -> Result<Vec<u8>, OptimiseError> {
    let mut buf = Vec::new();
    let mut enc = image::codecs::png::PngEncoder::new(&mut buf);
    apply_meta(&mut enc, meta);
    img.write_with_encoder(enc).map_err(other)?;
    Ok(match &meta.xmp {
        Some(xmp) => png_with_xmp(buf, xmp),
        None => buf,
    })
}

/// Write `src` as lossless WebP (with its metadata, per the `strip` policy).
pub(crate) fn write_lossless_webp(
    src: &Path,
    out: &Path,
    strip: Strip,
) -> Result<(), OptimiseError> {
    let (img, meta) = load(src)?;
    save_with_meta(&img, out, &meta_for(meta, strip))
}

/// Encode lossy WebP via libwebp at `quality` (0–100), embedding the ICC profile.
fn write_webp(
    img: &DynamicImage,
    out: &Path,
    quality: f32,
    meta: &Meta,
) -> Result<(), OptimiseError> {
    let (w, h) = (img.width(), img.height());
    let alpha = img.color().has_alpha();
    // Sharp RGB->YUV conversion keeps coloured edges (text, UI, saturated
    // detail) crisp through WebP's 4:2:0 chroma subsampling.
    let mut config = webp::WebPConfig::new().map_err(|_| other("webp config"))?;
    config.quality = quality.clamp(0.0, 100.0);
    config.alpha_compression = 1;
    config.use_sharp_yuv = 1;
    let encoded = if alpha {
        let rgba = img.to_rgba8();
        webp::Encoder::from_rgba(&rgba, w, h).encode_advanced(&config)
    } else {
        let rgb = img.to_rgb8();
        webp::Encoder::from_rgb(&rgb, w, h).encode_advanced(&config)
    }
    .map_err(|e| other(format!("webp encoding failed: {e:?}")))?
    .to_vec();
    if encoded.is_empty() {
        return Err(other("webp encoding failed"));
    }
    let bytes = webp_with_meta(&encoded, meta, w, h, alpha).unwrap_or(encoded);
    std::fs::write(out, bytes)?;
    Ok(())
}

/// Rewrite a WebP file's metadata: drop any existing `ICCP`/`EXIF`/`XMP `
/// chunks and add the ones in `meta`, with a `VP8X` header whose flags match.
/// Returns `None` (keep the input) when it isn't a WebP we understand.
fn webp_with_meta(webp: &[u8], meta: &Meta, w: u32, h: u32, alpha: bool) -> Option<Vec<u8>> {
    if webp.len() < 12 || &webp[0..4] != b"RIFF" || &webp[8..12] != b"WEBP" {
        return None;
    }
    let mut vp8x: Option<Vec<u8>> = None;
    let mut rest: Vec<u8> = Vec::new();
    let mut has_alph = false;
    let mut pos = 12;
    while pos + 8 <= webp.len() {
        let fourcc = &webp[pos..pos + 4];
        let size = u32::from_le_bytes(webp[pos + 4..pos + 8].try_into().ok()?) as usize;
        let end = pos + 8 + size;
        let padded = end + (size & 1);
        if end > webp.len() {
            return None;
        }
        match fourcc {
            b"VP8X" => vp8x = Some(webp[pos + 8..end].to_vec()),
            b"ICCP" | b"EXIF" | b"XMP " => {}
            _ => {
                has_alph |= fourcc == b"ALPH";
                rest.extend_from_slice(&webp[pos..padded.min(webp.len())]);
            }
        }
        pos = padded;
    }
    if meta.icc.is_none() && meta.exif.is_none() && meta.xmp.is_none() && vp8x.is_none() {
        return None;
    }

    const ICC_FLAG: u8 = 0x20;
    const ALPHA_FLAG: u8 = 0x10;
    const EXIF_FLAG: u8 = 0x08;
    const XMP_FLAG: u8 = 0x04;
    let mut header = vp8x.unwrap_or_else(|| {
        let mut v = vec![0u8; 10];
        v[4..7].copy_from_slice(&(w - 1).to_le_bytes()[..3]);
        v[7..10].copy_from_slice(&(h - 1).to_le_bytes()[..3]);
        v
    });
    header[0] &= !(ICC_FLAG | EXIF_FLAG | XMP_FLAG);
    for (present, flag) in [
        (meta.icc.is_some(), ICC_FLAG),
        (meta.exif.is_some(), EXIF_FLAG),
        (meta.xmp.is_some(), XMP_FLAG),
        (alpha || has_alph, ALPHA_FLAG),
    ] {
        if present {
            header[0] |= flag;
        }
    }

    fn push_chunk(body: &mut Vec<u8>, fourcc: &[u8; 4], data: &[u8]) {
        body.extend_from_slice(fourcc);
        body.extend_from_slice(&(data.len() as u32).to_le_bytes());
        body.extend_from_slice(data);
        if data.len() % 2 == 1 {
            body.push(0);
        }
    }
    let mut body = b"WEBP".to_vec();
    push_chunk(&mut body, b"VP8X", &header);
    if let Some(icc) = &meta.icc {
        push_chunk(&mut body, b"ICCP", icc);
    }
    body.extend_from_slice(&rest);
    if let Some(exif) = &meta.exif {
        push_chunk(&mut body, b"EXIF", exif);
    }
    if let Some(xmp) = &meta.xmp {
        push_chunk(&mut body, b"XMP ", xmp);
    }

    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    Some(out)
}

/// AVIF via the pure-Rust `ravif` encoder. `quality` is 1–100. The encoder
/// can't embed an ICC profile (the file is tagged sRGB), so pixels in another
/// colour space (e.g. Display P3) are converted to sRGB first.
fn write_avif(
    img: &DynamicImage,
    out: &Path,
    quality: u8,
    meta: &Meta,
) -> Result<(), OptimiseError> {
    let f = BufWriter::new(std::fs::File::create(out)?);
    let enc = image::codecs::avif::AvifEncoder::new_with_speed_quality(f, 6, quality);
    let converted = meta
        .icc
        .as_deref()
        .and_then(|icc| convert_to_srgb(img, icc));
    let img = converted.as_ref().unwrap_or(img);
    let img = if img.color().has_alpha() {
        DynamicImage::ImageRgba8(img.to_rgba8())
    } else {
        DynamicImage::ImageRgb8(img.to_rgb8())
    };
    img.write_with_encoder(enc).map_err(other)
}

/// Convert an image's pixels from the colour space described by `icc` to sRGB
/// (8-bit, perceptual intent). `None` if the profile is unusable, already sRGB,
/// or not an RGB profile.
pub fn convert_to_srgb(img: &DynamicImage, icc: &[u8]) -> Option<DynamicImage> {
    let input = qcms::Profile::new_from_slice(icc, false)?;
    if input.is_sRGB() {
        return None;
    }
    let mut output = qcms::Profile::new_sRGB();
    output.precache_output_transform();
    if img.color().has_alpha() {
        let mut rgba = img.to_rgba8();
        let xfm = qcms::Transform::new(
            &input,
            &output,
            qcms::DataType::RGBA8,
            qcms::Intent::default(),
        )?;
        xfm.apply(&mut rgba);
        Some(DynamicImage::ImageRgba8(rgba))
    } else {
        let mut rgb = img.to_rgb8();
        let xfm = qcms::Transform::new(
            &input,
            &output,
            qcms::DataType::RGB8,
            qcms::Intent::default(),
        )?;
        xfm.apply(&mut rgb);
        Some(DynamicImage::ImageRgb8(rgb))
    }
}

/// Which metadata to drop when re-encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strip {
    Keep,
    /// Only where it was taken (GPS / XMP location).
    Location,
    /// EXIF and XMP (the colour profile always stays).
    All,
}

impl Strip {
    pub fn from_options(options: &OptimiseOptions) -> Strip {
        if options.strip_metadata {
            Strip::All
        } else if options.strip_location {
            Strip::Location
        } else {
            Strip::Keep
        }
    }
}

/// Apply the strip policy (ICC always stays: it's colour-critical).
fn meta_for(meta: Meta, strip: Strip) -> Meta {
    match strip {
        Strip::Keep => meta,
        Strip::All => Meta {
            icc: meta.icc,
            exif: None,
            xmp: None,
        },
        Strip::Location => Meta {
            icc: meta.icc,
            exif: meta.exif.map(|mut e| {
                crate::privacy::strip_gps(&mut e);
                e
            }),
            xmp: meta.xmp.map(|x| crate::privacy::strip_xmp_location(&x)),
        },
    }
}

/// Optimise an image in place (or to `options.output`).
pub fn optimise(
    path: &Path,
    options: &OptimiseOptions,
) -> Result<OptimisationResult, OptimiseError> {
    if !path.is_file() {
        return Err(OptimiseError::NotFound(path.to_path_buf()));
    }
    let ext =
        extension_lower(path).ok_or_else(|| OptimiseError::Unsupported(path.to_path_buf()))?;
    let old_size = file_size(path);
    let cq = options.compression;

    let tmp = TempDir::new()?;
    let temp_out = tmp.path().join(file_name_lossy(path));

    let strip = Strip::from_options(options);
    match ext.as_str() {
        _ if is_animated(path) => optimise_animated(path, &ext, &temp_out, cq)?,
        "png" => optimise_png(path, &temp_out, cq, strip)?,
        "jpg" | "jpeg" => optimise_jpeg(path, &temp_out, cq, strip)?,
        "webp" => {
            let (img, meta) = load(path)?;
            write_webp(
                &img,
                &temp_out,
                cq.conversion_quality() as f32,
                &meta_for(meta, strip),
            )?
        }
        "gif" | "bmp" | "tiff" | "tif" => reencode(path, &temp_out, strip)?,
        "heic" | "heif" => {
            #[cfg(target_os = "macos")]
            {
                heic_encode(path, &temp_out, cq)?;
            }
            #[cfg(not(target_os = "macos"))]
            {
                return Err(OptimiseError::Unsupported(path.to_path_buf()));
            }
        }
        _ => return Err(OptimiseError::Unsupported(path.to_path_buf())),
    }

    finalise(path, &temp_out, old_size, cq, options)
}

/// PNG: quantise to a palette sized from the compression value, then run a
/// lossless oxipng pass. If the palette would cost too much quality (PSNR below
/// the compression value's floor) the image is kept lossless instead.
fn optimise_png(
    src: &Path,
    out: &Path,
    cq: CompressionQuality,
    strip: Strip,
) -> Result<(), OptimiseError> {
    let (img, meta) = load(src)?;
    let meta = meta_for(meta, strip);
    let rgba = img.to_rgba8();

    let paletted = quantise(&img, &rgba, cq.png_palette_size())
        .filter(|(palette, indices)| psnr(&rgba, palette, indices) >= cq.png_min_psnr())
        .map(|(palette, indices)| write_indexed_png(&rgba, &palette, &indices, &meta))
        .transpose()?;
    let raw = match paletted {
        Some(bytes) => bytes,
        None => encode_png(&img, &meta)?,
    };
    let opts = oxipng::Options::from_preset(2);
    let optimised = oxipng::optimize_from_memory(&raw, &opts).unwrap_or(raw);
    std::fs::write(out, optimised)?;
    Ok(())
}

/// Reduce an image to at most `colors` palette entries, returning an RGBA
/// palette and one index per pixel.
///
/// Opaque images use `quantette` (k-means in Oklab, Floyd–Steinberg dithered).
/// Images with transparency use `exoquant`, which understands alpha; they are
/// left undithered, since dithering soft alpha edges bloats the file.
fn quantise(
    img: &DynamicImage,
    rgba: &image::RgbaImage,
    colors: u16,
) -> Option<(Vec<[u8; 4]>, Vec<u8>)> {
    if rgba.pixels().all(|p| p.0[3] == 255) {
        let rgb = img.to_rgb8();
        let input = quantette::ImageRef::try_from(&rgb).ok()?;
        let indexed = quantette::Pipeline::new()
            .palette_size(quantette::PaletteSize::try_from_u16(colors)?)
            .quantize_method(quantette::QuantizeMethod::kmeans())
            .ditherer(quantette::dither::FloydSteinberg::new())
            .input_image(input)
            .output_srgb8_indexed_image();
        let (palette, indices) = indexed.into_parts();
        let palette = palette
            .iter()
            .map(|c| [c.red, c.green, c.blue, 255])
            .collect();
        Some((palette, indices))
    } else {
        let pixels: Vec<exoquant::Color> = rgba
            .pixels()
            .map(|p| exoquant::Color::new(p.0[0], p.0[1], p.0[2], p.0[3]))
            .collect();
        let (palette, indices) = exoquant::convert_to_indexed(
            &pixels,
            rgba.width() as usize,
            colors as usize,
            &exoquant::optimizer::KMeans,
            &exoquant::ditherer::None,
        );
        let palette = palette.iter().map(|c| [c.r, c.g, c.b, c.a]).collect();
        Some((palette, indices))
    }
}

/// Peak signal-to-noise ratio (dB) of a paletted image against the original,
/// measured on alpha-premultiplied RGBA so invisible colour differences under
/// transparent pixels don't count.
fn psnr(original: &image::RgbaImage, palette: &[[u8; 4]], indices: &[u8]) -> f64 {
    let mut squared_error = 0f64;
    for (src, &i) in original.pixels().zip(indices) {
        let Some(q) = palette.get(i as usize) else {
            return 0.0;
        };
        let (sa, qa) = (src.0[3] as f64 / 255.0, q[3] as f64 / 255.0);
        for (&s, &p) in src.0[..3].iter().zip(&q[..3]) {
            let d = s as f64 * sa - p as f64 * qa;
            squared_error += d * d;
        }
        let d = src.0[3] as f64 - q[3] as f64;
        squared_error += d * d;
    }
    let mse = squared_error / (indices.len().max(1) * 4) as f64;
    if mse == 0.0 {
        return f64::INFINITY;
    }
    10.0 * (255.0 * 255.0 / mse).log10()
}

/// Encode an 8-bit paletted PNG, embedding the ICC profile / EXIF.
fn write_indexed_png(
    rgba: &image::RgbaImage,
    palette: &[[u8; 4]],
    indices: &[u8],
    meta: &Meta,
) -> Result<Vec<u8>, OptimiseError> {
    let mut buf = Vec::new();
    let mut info = png::Info::with_size(rgba.width(), rgba.height());
    info.icc_profile = meta.icc.clone().map(Into::into);
    info.exif_metadata = meta.exif.clone().map(Into::into);
    let mut enc = png::Encoder::with_info(&mut buf, info).map_err(other)?;
    enc.set_color(png::ColorType::Indexed);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_palette(
        palette
            .iter()
            .flat_map(|c| [c[0], c[1], c[2]])
            .collect::<Vec<u8>>(),
    );
    if palette.iter().any(|c| c[3] != 255) {
        enc.set_trns(palette.iter().map(|c| c[3]).collect::<Vec<u8>>());
    }
    let mut writer = enc.write_header().map_err(other)?;
    writer.write_image_data(indices).map_err(other)?;
    writer.finish().map_err(other)?;
    Ok(match &meta.xmp {
        Some(xmp) => png_with_xmp(buf, xmp),
        None => buf,
    })
}

/// JPEG: decode (upright) and re-encode at a quality derived from the
/// compression value, keeping the ICC profile and (unless stripping) the EXIF.
fn optimise_jpeg(
    src: &Path,
    out: &Path,
    cq: CompressionQuality,
    strip: Strip,
) -> Result<(), OptimiseError> {
    let (img, meta) = load(src)?;
    let q = cq.jpeg_max_quality().clamp(1, 100) as u8;
    write_jpeg(&img, out, q, &meta_for(meta, strip))
}

/// Decode and re-encode a still image in the same format (GIF/BMP/TIFF).
fn reencode(src: &Path, out: &Path, strip: Strip) -> Result<(), OptimiseError> {
    let (img, meta) = load(src)?;
    save_with_meta(&img, out, &meta_for(meta, strip))
}

/// Animated images: never decode them into a single frame. GIFs go through
/// `gifsicle` when it is installed; otherwise (and for animated WebP/APNG) the
/// file passes through unchanged so the size guard keeps the original.
fn optimise_animated(
    src: &Path,
    ext: &str,
    out: &Path,
    cq: CompressionQuality,
) -> Result<(), OptimiseError> {
    if ext == "gif" && tools::is_available(Tool::Gifsicle) {
        let mut args = cq.gifsicle_args();
        args.push("-o".into());
        args.push(out.display().to_string());
        args.push(src.display().to_string());
        tools::run(Tool::Gifsicle, &args)?;
    } else {
        std::fs::copy(src, out)?;
    }
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
    finish(
        MediaKind::Image,
        src,
        temp_out,
        src.to_path_buf(),
        old_size,
        cq.image_is_aggressive(),
        options,
        Placement {
            size_guard: true,
            backup: true,
            replace_source: false,
        },
    )
}

/// Optimise a resized/cropped intermediate and place it at the destination. The
/// user asked for the change, so it is always kept — and the original is backed
/// up *before* anything is overwritten.
pub(crate) fn finish_transformed(
    src: &Path,
    transformed: &Path,
    old_size: u64,
    options: &OptimiseOptions,
) -> Result<OptimisationResult, OptimiseError> {
    let tmp = TempDir::new()?;
    let optimised = tmp.path().join(file_name_lossy(transformed));
    let staged_opts = OptimiseOptions {
        output: Some(optimised),
        backup: false,
        allow_larger: true,
        preserve_dates: false,
        ..options.clone()
    };
    let r = optimise(transformed, &staged_opts)?;
    finish(
        MediaKind::Image,
        src,
        &r.output,
        src.to_path_buf(),
        old_size,
        r.aggressive,
        options,
        Placement {
            size_guard: false,
            backup: true,
            replace_source: false,
        },
    )
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
    // A different format is written alongside the original (which is kept);
    // only a same-format result replaces it, with a backup.
    let same_ext = extension_lower(path).as_deref() == Some(best_fmt.extension());
    let default_dest = if same_ext {
        path.to_path_buf()
    } else {
        path.with_extension(best_fmt.extension())
    };
    finish(
        MediaKind::Image,
        path,
        &best_path,
        default_dest,
        old_size,
        options.compression.image_is_aggressive(),
        options,
        Placement {
            size_guard: true,
            backup: same_ext,
            replace_source: false,
        },
    )
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

/// Produce a path the pure-Rust pipeline (or `sips`) can read. HEIC/HEIF inputs
/// are decoded to a temporary PNG first; everything else is returned unchanged.
fn readable_source(path: &Path, tmp: &TempDir) -> Result<PathBuf, OptimiseError> {
    let ext = extension_lower(path).unwrap_or_default();
    if matches!(ext.as_str(), "heic" | "heif") {
        #[cfg(target_os = "macos")]
        {
            let out = tmp.path().join("decoded_src.png");
            heic_decode(path, &out)?;
            return Ok(out);
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = tmp;
            return Err(OptimiseError::Unsupported(path.to_path_buf()));
        }
    }
    Ok(path.to_path_buf())
}

/// Run macOS `sips` with the given arguments.
#[cfg(target_os = "macos")]
fn run_sips(args: &[String]) -> Result<(), OptimiseError> {
    let output = std::process::Command::new("/usr/bin/sips")
        .args(args)
        .stdout(std::process::Stdio::null())
        .output()
        .map_err(|e| other(format!("could not run sips: {e}")))?;
    if !output.status.success() {
        return Err(other(format!(
            "sips failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

/// Decode a HEIC/HEIF image to PNG using macOS `sips`.
#[cfg(target_os = "macos")]
fn heic_decode(src: &Path, out: &Path) -> Result<(), OptimiseError> {
    run_sips(&[
        "-s".into(),
        "format".into(),
        "png".into(),
        src.display().to_string(),
        "--out".into(),
        out.display().to_string(),
    ])
}

/// Encode an image (readable by `sips`) to HEIC using macOS `sips`.
#[cfg(target_os = "macos")]
fn heic_encode(src: &Path, out: &Path, cq: CompressionQuality) -> Result<(), OptimiseError> {
    run_sips(&[
        "-s".into(),
        "format".into(),
        "heic".into(),
        "-s".into(),
        "formatOptions".into(),
        cq.conversion_quality().to_string(),
        src.display().to_string(),
        "--out".into(),
        out.display().to_string(),
    ])
}

/// Convert an image to another format.
///
/// PNG/JPEG/WebP/AVIF are handled natively in Rust. HEIC/HEIF (iPhone photos)
/// use the macOS built-in `sips` — both as input and output. JXL uses `cjxl`.
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

    // HEIC/HEIF inputs (iPhone photos) can't be read by the `image` crate, so
    // decode them to a temporary PNG first; everything downstream reads that.
    let work_src = readable_source(path, &tmp)?;

    let strip = Strip::from_options(options);
    match format {
        ImageFormat::Png => optimise_png(&work_src, &temp_out, cq, strip)?,
        ImageFormat::Jpeg => optimise_jpeg(&work_src, &temp_out, cq, strip)?,
        ImageFormat::Webp => {
            let (img, meta) = load(&work_src)?;
            write_webp(
                &img,
                &temp_out,
                cq.conversion_quality() as f32,
                &meta_for(meta, strip),
            )?;
        }
        ImageFormat::Avif => {
            let (img, meta) = load(&work_src)?;
            write_avif(
                &img,
                &temp_out,
                cq.conversion_quality().clamp(1, 100) as u8,
                &meta,
            )?;
        }
        ImageFormat::Heic => {
            #[cfg(target_os = "macos")]
            {
                // Render to a PNG sips can always read, then encode to HEIC.
                let png_in = tmp.path().join("heic_in.png");
                let (img, meta) = load(&work_src)?;
                save_with_meta(&img, &png_in, &meta_for(meta, strip))?;
                heic_encode(&png_in, &temp_out, cq)?;
            }
            #[cfg(not(target_os = "macos"))]
            {
                let q = cq.conversion_quality().to_string();
                tools::run_with_retries(
                    Tool::HeifEnc,
                    [
                        "-q",
                        &q,
                        "-o",
                        &temp_out.display().to_string(),
                        &work_src.display().to_string(),
                    ],
                    2,
                )?;
            }
        }
        ImageFormat::Jxl => {
            tools::run_with_retries(
                Tool::Cjxl,
                [
                    "-q",
                    &cq.jxl_quality().to_string(),
                    "-e",
                    &cq.jxl_effort().to_string(),
                    &work_src.display().to_string(),
                    &temp_out.display().to_string(),
                ],
                2,
            )?;
        }
    }

    // Conversions are written alongside the source, which is kept — unless the
    // target has the same extension, in which case it is backed up first.
    let default_dest = path.with_extension(format.extension());
    let overwrites_source = default_dest == path;
    finish(
        MediaKind::Image,
        path,
        &temp_out,
        default_dest,
        old_size,
        cq.image_is_aggressive(),
        options,
        Placement {
            size_guard: false,
            backup: overwrites_source,
            replace_source: false,
        },
    )
}

// ---------------------------------------------------------------------------
// Resize / crop (pure Rust, used by scale.rs and crop.rs)
// ---------------------------------------------------------------------------

/// Load an image (upright, metadata kept), run `f` to transform it, and save to
/// `out` (format from its extension).
pub fn transform(
    src: &Path,
    out: &Path,
    f: impl FnOnce(DynamicImage) -> DynamicImage,
) -> Result<(), OptimiseError> {
    let (img, meta) = load(src)?;
    save_with_meta(&f(img), out, &meta)
}

/// Scale an image by `factor` (0.0–1.0), writing to `out`.
pub fn scale_image(src: &Path, out: &Path, factor: f64) -> Result<(), OptimiseError> {
    transform(src, out, |img| {
        let w = ((img.width() as f64 * factor).round() as u32).max(1);
        let h = ((img.height() as f64 * factor).round() as u32).max(1);
        img.resize_exact(w, h, image::imageops::FilterType::Lanczos3)
    })
}

/// Resize an image to exactly `w`x`h` (keeps aspect only if caller computed it).
pub fn resize_to(src: &Path, out: &Path, w: u32, h: u32) -> Result<(), OptimiseError> {
    transform(src, out, |img| {
        img.resize_exact(w.max(1), h.max(1), image::imageops::FilterType::Lanczos3)
    })
}

/// Scale to cover `w`x`h` then crop to exactly `w`x`h` — centred, or around
/// the most salient region when `smart`.
pub fn cover_crop(
    src: &Path,
    out: &Path,
    w: u32,
    h: u32,
    smart: bool,
) -> Result<(), OptimiseError> {
    transform(src, out, |img| {
        let (iw, ih) = (img.width().max(1), img.height().max(1));
        let scale = (w as f64 / iw as f64).max(h as f64 / ih as f64);
        let sw = ((iw as f64 * scale).ceil() as u32).max(w);
        let sh = ((ih as f64 * scale).ceil() as u32).max(h);
        let scaled = img.resize_exact(sw, sh, image::imageops::FilterType::Lanczos3);
        let (x, y) = crop_origin(&scaled, w, h, smart);
        scaled.crop_imm(x, y, w, h)
    })
}

/// Crop a `w`x`h` window out of the image without scaling — centred, or around
/// the most salient region when `smart`.
pub fn crop_window(
    src: &Path,
    out: &Path,
    w: u32,
    h: u32,
    smart: bool,
) -> Result<(), OptimiseError> {
    transform(src, out, |img| {
        let (w, h) = (w.min(img.width()), h.min(img.height()));
        let (x, y) = crop_origin(&img, w, h, smart);
        img.crop_imm(x, y, w, h)
    })
}

fn crop_origin(img: &DynamicImage, w: u32, h: u32, smart: bool) -> (u32, u32) {
    if smart {
        attention_origin(img, w, h)
    } else {
        (
            img.width().saturating_sub(w) / 2,
            img.height().saturating_sub(h) / 2,
        )
    }
}

/// Top-left corner of the `w`x`h` window that captures the most "interesting"
/// content — a pure-Rust take on smartcrop / libvips `attention`.
///
/// On a small copy of the image every pixel is scored for edges (detail),
/// saturation (colourful subjects) and skin tones (people); the window with the
/// highest total score wins, with a slight pull towards the centre to break
/// ties on flat images.
pub fn attention_origin(img: &DynamicImage, w: u32, h: u32) -> (u32, u32) {
    let (iw, ih) = (img.width(), img.height());
    if w >= iw && h >= ih {
        return (0, 0);
    }
    const ANALYSIS_EDGE: f64 = 256.0;
    let k = (ANALYSIS_EDGE / iw.max(ih) as f64).min(1.0);
    let (aw, ah) = (
        ((iw as f64 * k).round() as usize).max(1),
        ((ih as f64 * k).round() as usize).max(1),
    );
    let small = img
        .resize_exact(aw as u32, ah as u32, image::imageops::FilterType::Triangle)
        .to_rgb8();

    let px = |x: usize, y: usize| {
        let p = small.get_pixel(x as u32, y as u32).0;
        [
            p[0] as f64 / 255.0,
            p[1] as f64 / 255.0,
            p[2] as f64 / 255.0,
        ]
    };
    let luma = |c: [f64; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let mut score = vec![0f64; aw * ah];
    for y in 0..ah {
        for x in 0..aw {
            let c = px(x, y);
            let l = luma(c);
            // Edges: difference to the 4-neighbourhood (Laplacian magnitude).
            let mut lap = 4.0 * l;
            lap -= luma(px(x.saturating_sub(1), y));
            lap -= luma(px((x + 1).min(aw - 1), y));
            lap -= luma(px(x, y.saturating_sub(1)));
            lap -= luma(px(x, (y + 1).min(ah - 1)));
            let edge = lap.abs();
            // Saturation, ignoring near-black/near-white.
            let (max, min) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
            let sat = if max > 0.0 { (max - min) / max } else { 0.0 };
            let sat = if (0.05..=0.95).contains(&l) { sat } else { 0.0 };
            // Skin: closeness of the normalised colour to a typical skin tone.
            let mag = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt().max(1e-6);
            let skin_ref = [0.78, 0.57, 0.44];
            let ref_mag = (0.78f64 * 0.78 + 0.57 * 0.57 + 0.44 * 0.44).sqrt();
            let d = (0..3)
                .map(|i| (c[i] / mag - skin_ref[i] / ref_mag).powi(2))
                .sum::<f64>()
                .sqrt();
            let skin = if (0.2..=0.95).contains(&l) {
                (1.0 - d / 0.2).max(0.0)
            } else {
                0.0
            };
            score[y * aw + x] = 4.0 * edge + 0.4 * sat * sat + 1.2 * skin;
        }
    }

    // Summed-area table for O(1) window sums.
    let mut sat_table = vec![0f64; (aw + 1) * (ah + 1)];
    for y in 0..ah {
        let mut row = 0.0;
        for x in 0..aw {
            row += score[y * aw + x];
            sat_table[(y + 1) * (aw + 1) + x + 1] = sat_table[y * (aw + 1) + x + 1] + row;
        }
    }
    let window_sum = |x: usize, y: usize, cw: usize, ch: usize| {
        let at = |x: usize, y: usize| sat_table[y * (aw + 1) + x];
        at(x + cw, y + ch) - at(x, y + ch) - at(x + cw, y) + at(x, y)
    };

    let cw = ((w as f64 * k).round() as usize).clamp(1, aw);
    let ch = ((h as f64 * k).round() as usize).clamp(1, ah);
    let (cx, cy) = ((aw - cw) as f64 / 2.0, (ah - ch) as f64 / 2.0);
    let total = window_sum(0, 0, aw, ah).max(1e-9);
    let mut best = (f64::MIN, 0usize, 0usize);
    for y in 0..=(ah - ch) {
        for x in 0..=(aw - cw) {
            let dist =
                ((x as f64 - cx).powi(2) + (y as f64 - cy).powi(2)).sqrt() / (aw.max(ah) as f64);
            let s = window_sum(x, y, cw, ch) / total - 0.02 * dist;
            if s > best.0 {
                best = (s, x, y);
            }
        }
    }
    let x = ((best.1 as f64 / k).round() as u32).min(iw.saturating_sub(w));
    let y = ((best.2 as f64 / k).round() as u32).min(ih.saturating_sub(h));
    (x, y)
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
    transform(src, out, |img| {
        let cropped = img.crop_imm(x, y, w, h);
        match resize_to {
            Some((rw, rh)) => cropped.resize_exact(rw, rh, image::imageops::FilterType::Lanczos3),
            None => cropped,
        }
    })
}
