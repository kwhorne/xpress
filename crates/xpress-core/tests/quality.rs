//! Perceptual quality targets (SSIMULACRA2).

use std::path::{Path, PathBuf};

use image::DynamicImage;
use xpress_core::image::ImageFormat;
use xpress_core::quality::{convert_to_quality, optimise_to_quality, parse_target, score};
use xpress_core::result::OptimiseOptions;

fn tmpdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("xpress-q-{}-{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn opts() -> OptimiseOptions {
    OptimiseOptions {
        backup: false,
        ..Default::default()
    }
}

/// Smooth shapes with mild texture — compresses like a simple photo.
fn picture(w: u32, h: u32) -> DynamicImage {
    DynamicImage::ImageRgb8(image::RgbImage::from_fn(w, h, |x, y| {
        let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
        let d = ((fx - 0.4).powi(2) + (fy - 0.5).powi(2)).sqrt();
        let t = ((x * 7 + y * 13) % 17) as f32;
        image::Rgb([
            (200.0 * (1.0 - d) + t) as u8,
            (120.0 + 100.0 * fy + t) as u8,
            (60.0 + 150.0 * fx) as u8,
        ])
    }))
}

fn write_jpeg(path: &Path, img: &DynamicImage) {
    let mut buf = Vec::new();
    img.write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(
        &mut buf, 98,
    ))
    .unwrap();
    std::fs::write(path, buf).unwrap();
}

#[test]
fn parses_named_and_numeric_targets() {
    assert_eq!(parse_target("visually-lossless"), Ok(90.0));
    assert_eq!(parse_target("High"), Ok(80.0));
    assert_eq!(parse_target("72.5"), Ok(72.5));
    assert!(parse_target("0").is_err());
    assert!(parse_target("great").is_err());
}

#[test]
fn score_ranks_distortion() {
    let img = picture(128, 128);
    let same = score(&img, &img).unwrap();
    let blurred = score(&img, &img.blur(2.0)).unwrap();
    assert!(same > 99.0, "identical images score ~100, got {same}");
    assert!(blurred < same - 10.0, "blur is penalised, got {blurred}");
}

#[test]
fn optimise_meets_the_target_and_higher_targets_cost_bytes() {
    let dir = tmpdir("optimise");
    let mut sizes = Vec::new();
    for target in [70.0, 85.0] {
        let f = dir.join(format!("photo-{target}.jpg"));
        write_jpeg(&f, &picture(256, 256));
        let before = std::fs::metadata(&f).unwrap().len();

        let r = optimise_to_quality(&f, target, &opts()).unwrap();

        let s = r.score.expect("score reported");
        assert!(s >= target, "score {s} below target {target}");
        assert!(r.new_size < before, "should shrink a q98 JPEG");
        sizes.push(r.new_size);
    }
    assert!(
        sizes[0] <= sizes[1],
        "a lower target is never bigger: {sizes:?}"
    );
}

#[test]
fn convert_to_webp_meets_the_target() {
    let dir = tmpdir("webp");
    let f = dir.join("photo.png");
    picture(256, 256).save(&f).unwrap();

    let r = convert_to_quality(&f, ImageFormat::Webp, 80.0, &opts()).unwrap();

    assert_eq!(r.output, dir.join("photo.webp"));
    assert!(r.score.unwrap() >= 80.0);
    let decoded = xpress_core::image::open_oriented(&r.output).unwrap();
    let original = image::open(&f).unwrap();
    assert!(
        score(&original, &decoded).unwrap() >= 80.0,
        "independently re-scored"
    );
}

#[test]
fn quality_targets_are_for_images_only() {
    let dir = tmpdir("not-image");
    let f = dir.join("song.wav");
    std::fs::write(&f, vec![0u8; 100]).unwrap();
    assert!(optimise_to_quality(&f, 80.0, &opts()).is_err());
}
