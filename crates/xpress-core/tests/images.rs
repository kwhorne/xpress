//! Real-codec image tests: animation, orientation and colour-profile handling.
//!
//! Kept apart from `integration.rs` (which installs stub tools process-wide) so
//! these always exercise the pure-Rust encoders.

use std::path::{Path, PathBuf};

use image::metadata::Orientation;
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageEncoder, RgbImage};
use xpress_core::image::{self as ximage, ImageFormat};
use xpress_core::result::OptimiseOptions;
use xpress_core::{crop, scale};

const FAKE_ICC: &[u8] = b"xpress-test-icc-profile: opaque bytes carried through verbatim";

fn tmpdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("xpress-img-{}-{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn opts() -> OptimiseOptions {
    OptimiseOptions {
        backup: true,
        ..Default::default()
    }
}

/// A noisy, photo-like picture that lossy codecs can actually shrink.
fn photo(w: u32, h: u32) -> DynamicImage {
    let mut seed = 0x2545_f491_u32;
    let img = RgbImage::from_fn(w, h, |x, y| {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let n = (seed % 48) as u8;
        image::Rgb([
            (x * 255 / w) as u8 ^ n,
            (y * 255 / h) as u8 ^ n,
            ((x + y) % 256) as u8 ^ n,
        ])
    });
    DynamicImage::ImageRgb8(img)
}

/// A minimal little-endian EXIF (TIFF) block with `Make = "Test"` and the given
/// orientation.
fn exif_block(orientation: u16) -> Vec<u8> {
    let mut v = b"II*\0".to_vec();
    v.extend(8u32.to_le_bytes());
    v.extend(2u16.to_le_bytes());
    // Make (ASCII, 5 bytes) stored after the IFD at offset 8 + 2 + 2*12 + 4 = 38.
    v.extend(0x010Fu16.to_le_bytes());
    v.extend(2u16.to_le_bytes());
    v.extend(5u32.to_le_bytes());
    v.extend(38u32.to_le_bytes());
    // Orientation (SHORT, inline).
    v.extend(0x0112u16.to_le_bytes());
    v.extend(3u16.to_le_bytes());
    v.extend(1u32.to_le_bytes());
    v.extend((orientation as u32).to_le_bytes());
    v.extend(0u32.to_le_bytes());
    v.extend(b"Test\0");
    v
}

fn write_jpeg_with_meta(path: &Path, img: &DynamicImage, orientation: u16) {
    let f = std::fs::File::create(path).unwrap();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(f, 98);
    enc.set_icc_profile(FAKE_ICC.to_vec()).unwrap();
    enc.set_exif_metadata(exif_block(orientation)).unwrap();
    img.write_with_encoder(enc).unwrap();
}

fn write_png_with_icc(path: &Path, img: &DynamicImage) {
    let f = std::fs::File::create(path).unwrap();
    let mut enc = image::codecs::png::PngEncoder::new(f);
    enc.set_icc_profile(FAKE_ICC.to_vec()).unwrap();
    img.write_with_encoder(enc).unwrap();
}

fn write_animated_gif(path: &Path, frames: u32) {
    let f = std::fs::File::create(path).unwrap();
    let mut enc = image::codecs::gif::GifEncoder::new(f);
    enc.set_repeat(image::codecs::gif::Repeat::Infinite)
        .unwrap();
    for i in 0..frames {
        let buf = image::RgbaImage::from_fn(40, 40, |x, _| {
            if x / 8 == i {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 255, 255])
            }
        });
        enc.encode_frame(image::Frame::new(buf)).unwrap();
    }
}

fn gif_frames(path: &Path) -> usize {
    let f = std::io::BufReader::new(std::fs::File::open(path).unwrap());
    image::codecs::gif::GifDecoder::new(f)
        .unwrap()
        .into_frames()
        .count()
}

/// (dimensions, orientation, ICC, EXIF) as a decoder sees them.
type Inspected = ((u32, u32), Orientation, Option<Vec<u8>>, Option<Vec<u8>>);

fn inspect(path: &Path) -> Inspected {
    let mut d = image::ImageReader::open(path)
        .unwrap()
        .with_guessed_format()
        .unwrap()
        .into_decoder()
        .unwrap();
    let icc = d.icc_profile().unwrap();
    let exif = d.exif_metadata().unwrap();
    let orientation = d.orientation().unwrap();
    (d.dimensions(), orientation, icc, exif)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn animated_gif_keeps_every_frame() {
    let dir = tmpdir("anim");
    let f = dir.join("anim.gif");
    write_animated_gif(&f, 5);
    assert!(ximage::is_animated(&f));

    let r = ximage::optimise(&f, &opts()).unwrap();

    assert_eq!(gif_frames(&r.output), 5, "animation must not be flattened");
    assert!(r.new_size <= r.old_size);
}

#[test]
fn animated_gif_resize_is_refused_not_flattened() {
    let dir = tmpdir("anim-resize");
    let f = dir.join("anim.gif");
    write_animated_gif(&f, 4);
    let before = std::fs::read(&f).unwrap();

    assert!(scale::downscale_file(&f, 0.5, &opts()).is_err());
    assert!(ximage::convert(&f, ImageFormat::Png, &opts()).is_err());
    assert_eq!(
        std::fs::read(&f).unwrap(),
        before,
        "source must be untouched"
    );
}

#[test]
fn still_gif_is_not_animated() {
    let dir = tmpdir("still-gif");
    let f = dir.join("still.gif");
    photo(32, 32).save(&f).unwrap();
    assert!(!ximage::is_animated(&f));
}

#[test]
fn jpeg_optimise_keeps_icc_and_exif_and_applies_orientation() {
    let dir = tmpdir("jpeg-meta");
    let f = dir.join("portrait.jpg");
    // Stored 64x32, orientation 6 = "rotate 90° clockwise to display".
    write_jpeg_with_meta(&f, &photo(64, 32), 6);

    let r = ximage::optimise(&f, &opts()).unwrap();
    assert!(r.improved(), "q98 source should shrink");

    let (dims, orientation, icc, exif) = inspect(&r.output);
    assert_eq!(dims, (32, 64), "pixels are rotated upright");
    assert_eq!(
        orientation,
        Orientation::NoTransforms,
        "tag reset, no double rotation"
    );
    assert_eq!(icc.as_deref(), Some(FAKE_ICC), "colour profile kept");
    assert!(
        contains(&exif.expect("EXIF kept"), b"Test"),
        "camera EXIF kept"
    );
}

#[test]
fn jpeg_strip_metadata_drops_exif_but_keeps_icc() {
    let dir = tmpdir("jpeg-strip");
    let f = dir.join("portrait.jpg");
    write_jpeg_with_meta(&f, &photo(64, 32), 6);

    let o = OptimiseOptions {
        strip_metadata: true,
        ..opts()
    };
    let r = ximage::optimise(&f, &o).unwrap();

    let (dims, _, icc, exif) = inspect(&r.output);
    assert_eq!(dims, (32, 64), "still upright without the tag");
    assert!(exif.is_none(), "EXIF stripped");
    assert_eq!(
        icc.as_deref(),
        Some(FAKE_ICC),
        "colour profile is not metadata"
    );
}

#[test]
fn png_optimise_keeps_icc() {
    let dir = tmpdir("png-icc");
    let f = dir.join("shot.png");
    write_png_with_icc(&f, &photo(96, 96));

    let o = OptimiseOptions {
        allow_larger: true,
        ..opts()
    };
    let r = ximage::optimise(&f, &o).unwrap();

    let (_, _, icc, _) = inspect(&r.output);
    assert_eq!(icc.as_deref(), Some(FAKE_ICC));
}

#[test]
fn convert_to_webp_is_lossy_and_keeps_icc() {
    let dir = tmpdir("webp");
    let f = dir.join("photo.png");
    write_png_with_icc(&f, &photo(256, 256));

    let r = ximage::convert(&f, ImageFormat::Webp, &opts()).unwrap();

    assert_eq!(r.output, dir.join("photo.webp"));
    assert!(
        r.new_size * 2 < r.old_size,
        "lossy WebP should be far smaller than the PNG ({} vs {})",
        r.new_size,
        r.old_size
    );
    let (dims, _, icc, _) = inspect(&r.output);
    assert_eq!(dims, (256, 256));
    assert_eq!(icc.as_deref(), Some(FAKE_ICC));
}

#[test]
fn convert_to_webp_keeps_alpha() {
    let dir = tmpdir("webp-alpha");
    let f = dir.join("logo.png");
    let img = image::RgbaImage::from_fn(64, 64, |x, _| image::Rgba([200, 30, 30, (x * 4) as u8]));
    DynamicImage::ImageRgba8(img).save(&f).unwrap();

    let r = ximage::convert(&f, ImageFormat::Webp, &opts()).unwrap();

    let decoded = image::open(&r.output).unwrap();
    assert!(decoded.color().has_alpha());
}

#[test]
fn convert_to_avif_works() {
    let dir = tmpdir("avif");
    let f = dir.join("photo.png");
    photo(64, 64).save(&f).unwrap();

    let r = ximage::convert(&f, ImageFormat::Avif, &opts()).unwrap();

    let bytes = std::fs::read(&r.output).unwrap();
    assert_eq!(&bytes[4..12], b"ftypavif", "a real AVIF container");
}

#[test]
fn convert_transparent_png_to_jpeg_flattens_on_white() {
    let dir = tmpdir("jpeg-alpha");
    let f = dir.join("clear.png");
    DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        16,
        16,
        image::Rgba([0, 0, 0, 0]),
    ))
    .save(&f)
    .unwrap();

    let r = ximage::convert(&f, ImageFormat::Jpeg, &opts()).unwrap();

    let px = image::open(&r.output).unwrap().to_rgb8().get_pixel(8, 8).0;
    assert!(
        px.iter().all(|&c| c > 240),
        "transparent -> white, got {px:?}"
    );
}

#[test]
fn crop_uses_displayed_dimensions_of_rotated_jpeg() {
    let dir = tmpdir("crop-rotated");
    let f = dir.join("portrait.jpg");
    write_jpeg_with_meta(&f, &photo(64, 32), 6);
    assert_eq!(scale::image_dimensions(&f), Some((32, 64)));

    let o = OptimiseOptions {
        output: Some(dir.join("out.jpg")),
        ..opts()
    };
    // Width-only resize keeps aspect: a 32x64 portrait at width 16 is 16x32.
    let r = crop::crop_file(&f, &crop::CropSpec::size(16, 0), &o).unwrap();

    let (dims, orientation, _, _) = inspect(&r.output);
    assert_eq!(dims, (16, 32));
    assert_eq!(orientation, Orientation::NoTransforms);
}

#[test]
fn in_place_crop_and_downscale_back_up_the_real_original() {
    let dir = tmpdir("backup-before-write");
    for (name, op) in [("crop.png", 0), ("scale.png", 1)] {
        let f = dir.join(name);
        photo(80, 60).save(&f).unwrap();
        let original = std::fs::read(&f).unwrap();

        let r = if op == 0 {
            crop::crop_file(&f, &crop::CropSpec::size(40, 0), &opts()).unwrap()
        } else {
            scale::downscale_file(&f, 0.5, &opts()).unwrap()
        };

        let backup = r.backup.expect("backup made");
        assert_eq!(
            std::fs::read(&backup).unwrap(),
            original,
            "{name}: the backup must hold the untouched original"
        );
        assert_ne!(std::fs::read(&f).unwrap(), original);
    }
}

/// A flat gray `w`x`h` canvas with a `pw`-wide patch painted by `f` at `px`.
fn canvas_with_patch(
    w: u32,
    h: u32,
    px: u32,
    pw: u32,
    f: impl Fn(u32, u32) -> image::Rgb<u8>,
) -> DynamicImage {
    DynamicImage::ImageRgb8(RgbImage::from_fn(w, h, |x, y| {
        if (px..px + pw).contains(&x) && (20..h - 20).contains(&y) {
            f(x, y)
        } else {
            image::Rgb([128, 128, 128])
        }
    }))
}

#[test]
fn smart_crop_finds_detail_off_centre() {
    // Busy checkerboard on the right of a flat 300x100 canvas.
    let img = canvas_with_patch(300, 100, 220, 60, |x, y| {
        if (x / 3 + y / 3) % 2 == 0 {
            image::Rgb([20, 20, 20])
        } else {
            image::Rgb([235, 235, 235])
        }
    });
    let (x, y) = ximage::attention_origin(&img, 100, 100);
    assert_eq!(y, 0);
    assert!(
        (180..=200).contains(&x),
        "window should cover the patch, got x={x}"
    );
}

#[test]
fn smart_crop_finds_skin_tones() {
    // A smooth skin-coloured blob on the left, no edges inside it.
    let img = canvas_with_patch(300, 100, 10, 70, |_, _| image::Rgb([220, 160, 125]));
    let (x, _) = ximage::attention_origin(&img, 100, 100);
    assert!(x <= 10, "window should cover the skin patch, got x={x}");
}

#[test]
fn smart_crop_of_flat_image_stays_centred() {
    let img = DynamicImage::ImageRgb8(RgbImage::from_pixel(300, 100, image::Rgb([90, 90, 90])));
    assert_eq!(ximage::attention_origin(&img, 100, 100), (100, 0));
}

#[test]
fn crop_file_smart_ratio_keeps_the_subject() {
    let dir = tmpdir("smart-crop");
    let f = dir.join("wide.png");
    canvas_with_patch(300, 100, 220, 60, |x, y| {
        if (x / 3 + y / 3) % 2 == 0 {
            image::Rgb([20, 20, 20])
        } else {
            image::Rgb([235, 235, 235])
        }
    })
    .save(&f)
    .unwrap();

    let spread = |p: &Path| {
        let img = image::open(p).unwrap().to_luma8();
        let (lo, hi) = img
            .pixels()
            .fold((255u8, 0u8), |(lo, hi), p| (lo.min(p.0[0]), hi.max(p.0[0])));
        hi - lo
    };
    let spec = crop::CropSpec::parse("1:1").unwrap();
    let centred = dir.join("centred.png");
    let smart = dir.join("smart.png");
    let o = |out: &Path| OptimiseOptions {
        output: Some(out.to_path_buf()),
        ..opts()
    };
    crop::crop_file(&f, &spec, &o(&centred)).unwrap();
    crop::crop_file(&f, &spec.with_smart(true), &o(&smart)).unwrap();

    assert!(spread(&centred) < 10, "the centre is flat");
    assert!(
        spread(&smart) > 150,
        "smart crop contains the detailed subject"
    );
}
