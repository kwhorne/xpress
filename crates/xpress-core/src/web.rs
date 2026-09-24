//! Responsive images for the web: one source image becomes a set of widths in
//! modern formats plus a fallback, with a ready-to-paste `<picture>` element.
//!
//! JPEG/PNG/WebP variants are encoded to a perceptual quality target (see
//! [`crate::quality`]); AVIF (which can't be decoded here to be scored) uses
//! the compression factor. Widths larger than the source are never produced.

use std::path::{Path, PathBuf};

use crate::image::{self as ximage, ImageFormat};
use crate::result::{file_size, file_stem_lossy, OptimiseError, OptimiseOptions};

/// What to generate.
#[derive(Debug, Clone)]
pub struct WebOptions {
    /// Target widths in pixels (capped at the source width).
    pub widths: Vec<u32>,
    /// Modern formats to offer, in order of preference (e.g. AVIF, WebP).
    pub formats: Vec<ImageFormat>,
    /// SSIMULACRA2 target for JPEG/PNG/WebP variants.
    pub quality: f64,
    /// The `sizes` attribute (how wide the image is shown).
    pub sizes: String,
    /// The `alt` text.
    pub alt: String,
    /// Directory to write the variants to.
    pub out_dir: PathBuf,
}

impl Default for WebOptions {
    fn default() -> Self {
        Self {
            widths: vec![640, 1024, 1600, 2048],
            formats: vec![ImageFormat::Avif, ImageFormat::Webp],
            quality: 80.0,
            sizes: "100vw".into(),
            alt: String::new(),
            out_dir: PathBuf::from("."),
        }
    }
}

/// One generated file.
#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub path: PathBuf,
    pub format: ImageFormat,
    pub width: u32,
    pub height: u32,
    pub size: u64,
}

/// The generated set: all variants plus the `<picture>` markup.
#[derive(Debug, Clone)]
pub struct WebSet {
    pub variants: Vec<Variant>,
    pub html: String,
    /// Size of the source image, for reporting.
    pub source_size: u64,
}

/// The widths to produce for a `source_width`-wide image: the requested ones
/// that fit, plus the source width itself when every requested width is larger
/// (so there is always at least one).
pub fn plan_widths(requested: &[u32], source_width: u32) -> Vec<u32> {
    let mut w: Vec<u32> = requested
        .iter()
        .copied()
        .filter(|&w| w > 0 && w <= source_width)
        .collect();
    if w.is_empty() {
        w.push(source_width);
    }
    w.sort_unstable();
    w.dedup();
    w
}

fn mime(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Avif => "image/avif",
        ImageFormat::Webp => "image/webp",
        ImageFormat::Png => "image/png",
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::Heic => "image/heic",
        ImageFormat::Jxl => "image/jxl",
    }
}

fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Build the `<picture>` element. `sources` are the modern formats (in order),
/// `fallback` the `<img>` format; file names are relative to the HTML.
pub fn picture_html(
    variants: &[Variant],
    sources: &[ImageFormat],
    fallback: ImageFormat,
    sizes: &str,
    alt: &str,
) -> String {
    let srcset = |format: ImageFormat| -> String {
        variants
            .iter()
            .filter(|v| v.format == format)
            .map(|v| {
                let name = v.path.file_name().unwrap_or_default().to_string_lossy();
                format!("{} {}w", escape_attr(&name), v.width)
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let sizes = escape_attr(sizes);
    let mut html = String::from("<picture>\n");
    for &f in sources {
        let set = srcset(f);
        if !set.is_empty() {
            html.push_str(&format!(
                "  <source type=\"{}\" srcset=\"{set}\" sizes=\"{sizes}\">\n",
                mime(f)
            ));
        }
    }
    let largest = variants
        .iter()
        .filter(|v| v.format == fallback)
        .max_by_key(|v| v.width);
    if let Some(img) = largest {
        let src = img.path.file_name().unwrap_or_default().to_string_lossy();
        html.push_str(&format!(
            "  <img src=\"{}\" srcset=\"{}\" sizes=\"{sizes}\" width=\"{}\" height=\"{}\" \
             alt=\"{}\" loading=\"lazy\" decoding=\"async\">\n",
            escape_attr(&src),
            srcset(fallback),
            img.width,
            img.height,
            escape_attr(alt),
        ));
    }
    html.push_str("</picture>\n");
    html
}

/// Generate the responsive set for `src`.
pub fn generate(
    src: &Path,
    web: &WebOptions,
    base: &OptimiseOptions,
) -> Result<WebSet, OptimiseError> {
    let (img, _) = ximage::load(src)?;
    let (sw, sh) = (img.width(), img.height());
    let fallback = if img.color().has_alpha() {
        ImageFormat::Png
    } else {
        ImageFormat::Jpeg
    };
    let stem = file_stem_lossy(src);
    std::fs::create_dir_all(&web.out_dir)?;
    let tmp = tempfile::TempDir::new()?;

    let mut formats: Vec<ImageFormat> = web
        .formats
        .iter()
        .copied()
        .filter(|&f| f != fallback)
        .collect();
    formats.push(fallback);

    let mut variants = Vec::new();
    for w in plan_widths(&web.widths, sw) {
        let h = ((sh as f64 * w as f64 / sw as f64).round() as u32).max(1);
        // A resized, metadata-carrying intermediate (lossless PNG).
        let scaled = tmp.path().join(format!("{stem}-{w}.png"));
        ximage::resize_to(src, &scaled, w, h)?;
        for &format in &formats {
            let out = web
                .out_dir
                .join(format!("{stem}-{w}.{}", format.extension()));
            let opts = OptimiseOptions {
                output: Some(out.clone()),
                backup: false,
                ..base.clone()
            };
            match format {
                ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::Webp => {
                    // If the target can't be met, fall back to the compression
                    // factor rather than failing the whole set.
                    if crate::quality::convert_to_quality(&scaled, format, web.quality, &opts)
                        .is_err()
                    {
                        ximage::convert(&scaled, format, &opts)?;
                    }
                }
                _ => {
                    ximage::convert(&scaled, format, &opts)?;
                }
            }
            variants.push(Variant {
                size: file_size(&out),
                path: out,
                format,
                width: w,
                height: h,
            });
        }
    }

    let sources: Vec<ImageFormat> = formats.iter().copied().filter(|&f| f != fallback).collect();
    let html = picture_html(&variants, &sources, fallback, &web.sizes, &web.alt);
    Ok(WebSet {
        variants,
        html,
        source_size: file_size(src),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_are_capped_at_the_source() {
        assert_eq!(plan_widths(&[640, 1024, 1600, 2048], 1200), vec![640, 1024]);
        assert_eq!(plan_widths(&[640, 1024], 300), vec![300], "never upscale");
        assert_eq!(plan_widths(&[1024, 640, 640], 4000), vec![640, 1024]);
    }

    #[test]
    fn picture_markup() {
        let v = |f: ImageFormat, w: u32| Variant {
            path: PathBuf::from(format!("hero-{w}.{}", f.extension())),
            format: f,
            width: w,
            height: w / 2,
            size: 1,
        };
        let variants = vec![
            v(ImageFormat::Avif, 640),
            v(ImageFormat::Avif, 1024),
            v(ImageFormat::Jpeg, 640),
            v(ImageFormat::Jpeg, 1024),
        ];
        let html = picture_html(
            &variants,
            &[ImageFormat::Avif, ImageFormat::Webp],
            ImageFormat::Jpeg,
            "(max-width: 800px) 100vw, 800px",
            "A \"hero\" & more",
        );
        assert!(html.contains(
            r#"<source type="image/avif" srcset="hero-640.avif 640w, hero-1024.avif 1024w""#
        ));
        assert!(!html.contains("image/webp"), "no empty source");
        assert!(html.contains(r#"<img src="hero-1024.jpg""#));
        assert!(html.contains(r#"width="1024" height="512""#));
        assert!(html.contains(r#"alt="A &quot;hero&quot; &amp; more""#));
        assert!(html.contains(r#"sizes="(max-width: 800px) 100vw, 800px""#));
    }
}
