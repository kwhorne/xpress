//! Marking up an image: arrows, boxes, text, and redaction, drawn into the
//! pixels.
//!
//! Marks are placed in normalised coordinates (0–1 of the width and height),
//! so the same marks drawn on a small preview and on the full-resolution
//! image land in the same place. Line widths and text sizes follow the
//! image's shorter side for the same reason.
//!
//! **Redact** replaces the pixels with solid black: nothing of what was there
//! survives. **Pixelate** keeps block averages, which hides a face or a
//! name at a glance, but pixelated text can sometimes be reconstructed — use
//! redact for passwords and keys.

use std::path::Path;

use ab_glyph::{Font, FontArc, PxScale, ScaleFont};
use image::RgbaImage;

use crate::result::OptimiseError;

/// A point in normalised image coordinates (0–1 of the width and height).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// Straight (non-premultiplied) RGBA.
pub type Color = [u8; 4];

/// One mark on the image, in the order it was made.
#[derive(Debug, Clone, PartialEq)]
pub enum Mark {
    /// An arrow pointing at `to`.
    Arrow {
        from: Point,
        to: Point,
        color: Color,
    },
    /// A rectangle outline between two corners.
    Rect { a: Point, b: Point, color: Color },
    /// A line of text with its top-left corner at `at`, on a label.
    Text {
        at: Point,
        text: String,
        color: Color,
    },
    /// Solid black over the area between two corners.
    Redact { a: Point, b: Point },
    /// Coarse blocks over the area between two corners.
    Pixelate { a: Point, b: Point },
}

/// The stroke width in pixels for an image of this size.
pub fn stroke_width(width: u32, height: u32) -> f32 {
    (width.min(height) as f32 * 0.008).max(2.0)
}

/// The text height in pixels for an image of this size.
pub fn text_size(width: u32, height: u32) -> f32 {
    (width.min(height) as f32 * 0.04).max(14.0)
}

/// The pixelate block size in pixels for an image of this size.
pub fn block_size(width: u32, height: u32) -> f32 {
    (width.min(height) as f32 / 40.0).max(12.0)
}

/// Draw `marks` into `img`, in order. Text needs `font`; without one, text
/// marks are skipped.
pub fn render(img: &mut RgbaImage, marks: &[Mark], font: Option<&FontArc>) {
    let size = img.dimensions();
    render_for(img, marks, font, size);
}

/// Like [`render`], on a scaled-down preview of an image whose real size is
/// `full`: line widths, text and pixelate blocks are those of the full image,
/// scaled, so the preview looks like the result.
pub fn render_for(img: &mut RgbaImage, marks: &[Mark], font: Option<&FontArc>, full: (u32, u32)) {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 || full.0 == 0 || full.1 == 0 {
        return;
    }
    let k = w as f32 / full.0 as f32;
    let px = |p: Point| (p.x * w as f32, p.y * h as f32);
    let stroke = stroke_width(full.0, full.1) * k;
    for mark in marks {
        match mark {
            Mark::Arrow { from, to, color } => arrow(img, px(*from), px(*to), stroke, *color),
            Mark::Rect { a, b, color } => {
                let ((x0, y0), (x1, y1)) = (px(*a), px(*b));
                for (p, q) in [
                    ((x0, y0), (x1, y0)),
                    ((x1, y0), (x1, y1)),
                    ((x1, y1), (x0, y1)),
                    ((x0, y1), (x0, y0)),
                ] {
                    line(img, p, q, stroke, *color);
                }
            }
            Mark::Text { at, text, color } => {
                if let Some(font) = font {
                    label(
                        img,
                        px(*at),
                        text,
                        text_size(full.0, full.1) * k,
                        *color,
                        font,
                    );
                }
            }
            Mark::Redact { a, b } => {
                if let Some((x0, y0, x1, y1)) = pixel_box(img, px(*a), px(*b)) {
                    for y in y0..y1 {
                        for x in x0..x1 {
                            img.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
                        }
                    }
                }
            }
            Mark::Pixelate { a, b } => {
                if let Some(area) = pixel_box(img, px(*a), px(*b)) {
                    let block = (block_size(full.0, full.1) * k).round().max(1.0) as u32;
                    pixelate(img, area, block);
                }
            }
        }
    }
}

/// Mark up the image at `src` (upright) and write it as a PNG to `out`. The
/// colour profile stays — a Mac's screenshots are in Display P3 — but no
/// other metadata.
pub fn annotate_file(
    src: &Path,
    out: &Path,
    marks: &[Mark],
    font: Option<&FontArc>,
) -> Result<(), OptimiseError> {
    let (img, meta) = crate::image::load(src)?;
    let mut img = img.to_rgba8();
    render(&mut img, marks, font);
    let meta = crate::image::Meta {
        icc: meta.icc,
        exif: None,
        xmp: None,
    };
    let png = crate::image::encode_png(&image::DynamicImage::ImageRgba8(img), &meta)?;
    std::fs::write(out, png)?;
    Ok(())
}

/// A bold system font for text marks, if one is installed (loaded once).
pub fn system_font() -> Option<FontArc> {
    static FONT: std::sync::OnceLock<Option<FontArc>> = std::sync::OnceLock::new();
    FONT.get_or_init(|| {
        let candidates: &[(&str, u32)] = &[
            ("/System/Library/Fonts/Supplemental/Arial Bold.ttf", 0),
            ("/System/Library/Fonts/Helvetica.ttc", 1),
            ("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf", 0),
        ];
        candidates.iter().find_map(|(path, index)| {
            let data = std::fs::read(path).ok()?;
            ab_glyph::FontVec::try_from_vec_and_index(data, *index)
                .ok()
                .map(FontArc::new)
        })
    })
    .clone()
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/// Paint `color` over the pixel with `coverage` (0–1): "source over" in
/// straight alpha, so drawing over a transparent window shadow doesn't pull
/// in its (meaningless) black.
fn blend(img: &mut RgbaImage, x: u32, y: u32, color: Color, coverage: f32) {
    let a = coverage.clamp(0.0, 1.0) * color[3] as f32 / 255.0;
    if a <= 0.0 {
        return;
    }
    let p = img.get_pixel_mut(x, y);
    let dst_a = p[3] as f32 / 255.0;
    let out_a = a + dst_a * (1.0 - a);
    for i in 0..3 {
        let c = (color[i] as f32 * a + p[i] as f32 * dst_a * (1.0 - a)) / out_a;
        p[i] = c.round().clamp(0.0, 255.0) as u8;
    }
    p[3] = (255.0 * out_a).round() as u8;
}

/// The pixels a shape covering `[x0, x1] × [y0, y1]` (plus a margin) may touch.
fn bounds(img: &RgbaImage, x0: f32, y0: f32, x1: f32, y1: f32) -> (u32, u32, u32, u32) {
    let (w, h) = img.dimensions();
    let clamp = |v: f32, max: u32| v.max(0.0).min(max as f32) as u32;
    (
        clamp(x0.floor(), w),
        clamp(y0.floor(), h),
        clamp(x1.ceil() + 1.0, w),
        clamp(y1.ceil() + 1.0, h),
    )
}

/// A thick, anti-aliased line with round ends.
fn line(img: &mut RgbaImage, p: (f32, f32), q: (f32, f32), width: f32, color: Color) {
    let r = width / 2.0;
    let (bx0, by0, bx1, by1) = bounds(
        img,
        p.0.min(q.0) - r - 1.0,
        p.1.min(q.1) - r - 1.0,
        p.0.max(q.0) + r + 1.0,
        p.1.max(q.1) + r + 1.0,
    );
    let (dx, dy) = (q.0 - p.0, q.1 - p.1);
    let len2 = dx * dx + dy * dy;
    for y in by0..by1 {
        for x in bx0..bx1 {
            let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
            let t = if len2 > 0.0 {
                (((cx - p.0) * dx + (cy - p.1) * dy) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let (ex, ey) = (cx - (p.0 + t * dx), cy - (p.1 + t * dy));
            let d = (ex * ex + ey * ey).sqrt();
            blend(img, x, y, color, r + 0.5 - d);
        }
    }
}

/// A filled, anti-aliased triangle.
fn triangle(img: &mut RgbaImage, pts: [(f32, f32); 3], color: Color) {
    let xs = pts.map(|p| p.0);
    let ys = pts.map(|p| p.1);
    let (bx0, by0, bx1, by1) = bounds(
        img,
        xs.iter().copied().fold(f32::MAX, f32::min) - 1.0,
        ys.iter().copied().fold(f32::MAX, f32::min) - 1.0,
        xs.iter().copied().fold(f32::MIN, f32::max) + 1.0,
        ys.iter().copied().fold(f32::MIN, f32::max) + 1.0,
    );
    // Orientation, so "inside" is a positive distance from every edge.
    let [a, b, c] = pts;
    let sign = if (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0) >= 0.0 {
        1.0
    } else {
        -1.0
    };
    let edges = [(a, b), (b, c), (c, a)];
    for y in by0..by1 {
        for x in bx0..bx1 {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let d = edges
                .iter()
                .map(|&(s, e)| {
                    let (ex, ey) = (e.0 - s.0, e.1 - s.1);
                    let len = (ex * ex + ey * ey).sqrt().max(1e-6);
                    sign * (ex * (py - s.1) - ey * (px - s.0)) / len
                })
                .fold(f32::MAX, f32::min);
            blend(img, x, y, color, d + 0.5);
        }
    }
}

fn arrow(img: &mut RgbaImage, from: (f32, f32), to: (f32, f32), width: f32, color: Color) {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return;
    }
    let (ux, uy) = (dx / len, dy / len);
    let head = (width * 4.5).min(len * 0.6);
    let base = (to.0 - ux * head, to.1 - uy * head);
    // The shaft stops inside the head, so its round end doesn't poke out.
    let shaft_end = (to.0 - ux * head * 0.6, to.1 - uy * head * 0.6);
    line(img, from, shaft_end, width, color);
    let (nx, ny) = (-uy * head * 0.55, ux * head * 0.55);
    triangle(
        img,
        [to, (base.0 + nx, base.1 + ny), (base.0 - nx, base.1 - ny)],
        color,
    );
}

/// The whole pixels between two corners, rounded outwards, or `None` when
/// that is empty.
fn pixel_box(img: &RgbaImage, p: (f32, f32), q: (f32, f32)) -> Option<(u32, u32, u32, u32)> {
    let (w, h) = img.dimensions();
    let x0 = p.0.min(q.0).floor().clamp(0.0, w as f32) as u32;
    let y0 = p.1.min(q.1).floor().clamp(0.0, h as f32) as u32;
    let x1 = p.0.max(q.0).ceil().clamp(0.0, w as f32) as u32;
    let y1 = p.1.max(q.1).ceil().clamp(0.0, h as f32) as u32;
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1, y1))
}

/// Replace each `block`×`block` square of the area with its average colour.
fn pixelate(img: &mut RgbaImage, (x0, y0, x1, y1): (u32, u32, u32, u32), block: u32) {
    let mut by = y0;
    while by < y1 {
        let ey = (by + block).min(y1);
        let mut bx = x0;
        while bx < x1 {
            let ex = (bx + block).min(x1);
            let mut sum = [0u64; 4];
            for y in by..ey {
                for x in bx..ex {
                    let p = img.get_pixel(x, y);
                    for i in 0..4 {
                        sum[i] += p[i] as u64;
                    }
                }
            }
            let n = ((ex - bx) * (ey - by)) as u64;
            let avg = image::Rgba(sum.map(|s| (s / n) as u8));
            for y in by..ey {
                for x in bx..ex {
                    img.put_pixel(x, y, avg);
                }
            }
            bx = ex;
        }
        by = ey;
    }
}

/// Text in `color` on a contrasting label whose top-left corner is `at`.
fn label(img: &mut RgbaImage, at: (f32, f32), text: &str, size: f32, color: Color, font: &FontArc) {
    let scaled = font.as_scaled(PxScale::from(size));
    let line_height = scaled.ascent() - scaled.descent() + scaled.line_gap();
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return;
    }
    let width_of = |s: &str| {
        let mut w = 0.0;
        let mut prev = None;
        for c in s.chars() {
            let id = scaled.glyph_id(c);
            if let Some(p) = prev {
                w += scaled.kern(p, id);
            }
            w += scaled.h_advance(id);
            prev = Some(id);
        }
        w
    };
    let pad = size * 0.3;
    let text_w = lines.iter().map(|l| width_of(l)).fold(0.0, f32::max);
    let text_h = line_height * lines.len() as f32;
    // Light text on a dark label, dark text on a light one.
    let luma = 0.299 * color[0] as f32 + 0.587 * color[1] as f32 + 0.114 * color[2] as f32;
    let back: Color = if luma > 150.0 {
        [0, 0, 0, 190]
    } else {
        [255, 255, 255, 225]
    };
    let (bx0, by0, bx1, by1) = bounds(
        img,
        at.0,
        at.1,
        at.0 + text_w + 2.0 * pad - 1.0,
        at.1 + text_h + 2.0 * pad - 1.0,
    );
    for y in by0..by1 {
        for x in bx0..bx1 {
            blend(img, x, y, back, 1.0);
        }
    }
    let (w, h) = img.dimensions();
    for (i, text_line) in lines.iter().enumerate() {
        let mut x = at.0 + pad;
        let baseline = at.1 + pad + scaled.ascent() + line_height * i as f32;
        let mut prev = None;
        for c in text_line.chars() {
            let id = scaled.glyph_id(c);
            if let Some(p) = prev {
                x += scaled.kern(p, id);
            }
            let glyph =
                id.with_scale_and_position(PxScale::from(size), ab_glyph::point(x, baseline));
            if let Some(outline) = font.outline_glyph(glyph) {
                let b = outline.px_bounds();
                outline.draw(|gx, gy, coverage| {
                    let (px, py) = (b.min.x as i64 + gx as i64, b.min.y as i64 + gy as i64);
                    if px >= 0 && py >= 0 && (px as u32) < w && (py as u32) < h {
                        blend(img, px as u32, py as u32, color, coverage);
                    }
                });
            }
            x += scaled.h_advance(id);
            prev = Some(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Color = [230, 40, 40, 255];

    /// A noisy, opaque test image.
    fn noise(w: u32, h: u32) -> RgbaImage {
        let mut seed = 0x9e37_79b9_u32;
        RgbaImage::from_fn(w, h, |_, _| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            image::Rgba([seed as u8, (seed >> 8) as u8, (seed >> 16) as u8, 255])
        })
    }

    #[test]
    fn redact_leaves_nothing_of_the_area() {
        let mut img = noise(200, 100);
        let before = img.clone();
        let mark = Mark::Redact {
            a: Point::new(0.105, 0.2),
            b: Point::new(0.5, 0.61),
        };
        render(&mut img, &[mark], None);
        // Rounded outwards: x 21..100, y 20..61.
        for y in 20..61 {
            for x in 21..100 {
                assert_eq!(img.get_pixel(x, y).0, [0, 0, 0, 255], "({x},{y})");
            }
        }
        assert_eq!(img.get_pixel(20, 30), before.get_pixel(20, 30));
        assert_eq!(img.get_pixel(150, 80), before.get_pixel(150, 80));
    }

    #[test]
    fn pixelate_makes_uniform_blocks() {
        let mut img = noise(240, 240);
        let mark = Mark::Pixelate {
            a: Point::new(0.0, 0.0),
            b: Point::new(0.5, 0.5),
        };
        render(&mut img, &[mark], None);
        // 240 / 40 = 6, so blocks are the 12 px minimum.
        let first = *img.get_pixel(0, 0);
        for y in 0..12 {
            for x in 0..12 {
                assert_eq!(*img.get_pixel(x, y), first);
            }
        }
        assert_ne!(img.get_pixel(12, 0), img.get_pixel(24, 0), "blocks differ");
    }

    #[test]
    fn arrow_draws_along_its_line_only() {
        let mut img = RgbaImage::from_pixel(400, 200, image::Rgba([255, 255, 255, 255]));
        let mark = Mark::Arrow {
            from: Point::new(0.1, 0.5),
            to: Point::new(0.9, 0.5),
            color: RED,
        };
        render(&mut img, &[mark], None);
        assert_eq!(img.get_pixel(200, 100).0, RED, "on the shaft");
        assert_eq!(img.get_pixel(355, 100).0, RED, "in the head");
        assert_eq!(img.get_pixel(200, 20).0, [255; 4], "above the line");
        assert_eq!(img.get_pixel(5, 5).0, [255; 4], "corner untouched");
        // The head is wider than the shaft.
        let wide = |x: u32| (0..200).filter(|&y| img.get_pixel(x, y).0 == RED).count();
        assert!(wide(352) > wide(200) * 2, "{} vs {}", wide(352), wide(200));
    }

    #[test]
    fn rect_outlines_and_leaves_the_inside() {
        let mut img = RgbaImage::from_pixel(300, 300, image::Rgba([255, 255, 255, 255]));
        let mark = Mark::Rect {
            a: Point::new(0.2, 0.2),
            b: Point::new(0.8, 0.8),
            color: RED,
        };
        render(&mut img, &[mark], None);
        assert_eq!(img.get_pixel(150, 60).0, RED);
        assert_eq!(img.get_pixel(240, 150).0, RED);
        assert_eq!(img.get_pixel(150, 150).0, [255; 4], "hollow");
    }

    #[test]
    fn marks_land_in_the_same_place_at_any_size() {
        let mark = Mark::Redact {
            a: Point::new(0.25, 0.25),
            b: Point::new(0.5, 0.5),
        };
        for (w, h) in [(100, 80), (1000, 800)] {
            let mut img = RgbaImage::from_pixel(w, h, image::Rgba([255; 4]));
            render(&mut img, std::slice::from_ref(&mark), None);
            let (cx, cy) = (w * 3 / 8, h * 3 / 8);
            assert_eq!(img.get_pixel(cx, cy).0, [0, 0, 0, 255]);
            assert_eq!(img.get_pixel(w * 3 / 4, h * 3 / 4).0, [255; 4]);
        }
    }

    #[test]
    fn text_draws_on_a_label() {
        let Some(font) = system_font() else {
            return; // no system font here
        };
        let mut img = RgbaImage::from_pixel(400, 200, image::Rgba([255, 255, 255, 255]));
        let mark = Mark::Text {
            at: Point::new(0.1, 0.1),
            text: "Look here".into(),
            color: RED,
        };
        render(&mut img, &[mark], Some(&font));
        let red = img.pixels().filter(|p| p[0] > 180 && p[1] < 120).count();
        assert!(red > 50, "glyphs drawn ({red} red pixels)");
        assert_eq!(img.get_pixel(390, 190).0, [255; 4], "far away untouched");
    }

    #[test]
    fn drawing_over_transparency_keeps_the_colour() {
        // A window shadow: fully transparent (black) pixels.
        let mut img = RgbaImage::from_pixel(100, 100, image::Rgba([0, 0, 0, 0]));
        render(
            &mut img,
            &[Mark::Rect {
                a: Point::new(0.2, 0.2),
                b: Point::new(0.8, 0.8),
                color: RED,
            }],
            None,
        );
        // Edge pixels are partly covered: red, less opaque — not darker.
        let edges: Vec<_> = img.pixels().filter(|p| p[3] > 0 && p[3] < 255).collect();
        assert!(!edges.is_empty());
        for p in edges {
            assert_eq!([p[0], p[1], p[2]], [RED[0], RED[1], RED[2]], "{p:?}");
        }
    }

    #[test]
    fn a_preview_matches_the_full_image() {
        // A wide, low area: the preview is less than half the size.
        let (full, preview) = ((2400, 300), (1200, 150));
        let mark = Mark::Pixelate {
            a: Point::new(0.0, 0.0),
            b: Point::new(1.0, 1.0),
        };
        let mut small = noise(preview.0, preview.1);
        render_for(&mut small, std::slice::from_ref(&mark), None, full);
        // Full: 12 px blocks, so 6 px in the preview (not 12).
        let first = *small.get_pixel(0, 0);
        assert_eq!(*small.get_pixel(5, 5), first);
        assert_ne!(*small.get_pixel(6, 0), first, "a new block at 6 px");
    }

    #[test]
    fn the_colour_profile_stays() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("p3.png");
        let profile = b"fake ICC profile bytes".to_vec();
        let mut enc_out = Vec::new();
        {
            use image::ImageEncoder;
            let mut enc = image::codecs::png::PngEncoder::new(&mut enc_out);
            enc.set_icc_profile(profile.clone()).unwrap();
            let img = noise(16, 16);
            enc.write_image(img.as_raw(), 16, 16, image::ExtendedColorType::Rgba8)
                .unwrap();
        }
        std::fs::write(&src, enc_out).unwrap();
        let out = dir.path().join("out.png");
        annotate_file(&src, &out, &[], None).unwrap();
        let (_, meta) = crate::image::load(&out).unwrap();
        assert_eq!(meta.icc, Some(profile));
    }

    #[test]
    fn annotate_file_writes_a_png() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("shot.png");
        noise(64, 48).save(&src).unwrap();
        let out = dir.path().join("out.png");
        let marks = [Mark::Redact {
            a: Point::new(0.0, 0.0),
            b: Point::new(1.0, 1.0),
        }];
        annotate_file(&src, &out, &marks, None).unwrap();
        let img = image::open(&out).unwrap().to_rgba8();
        assert_eq!(img.dimensions(), (64, 48));
        assert!(img.pixels().all(|p| p.0 == [0, 0, 0, 255]));
    }
}
