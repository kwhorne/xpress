//! Responsive image sets (`xpress web`).

use image::DynamicImage;
use xpress_core::image::ImageFormat;
use xpress_core::result::OptimiseOptions;
use xpress_core::web::{generate, WebOptions};

fn tmpdir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("xpress-web-{}-{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn generates_widths_formats_and_markup() {
    let dir = tmpdir("set");
    let src = dir.join("hero.jpg");
    let img = image::RgbImage::from_fn(300, 150, |x, y| {
        image::Rgb([(x % 256) as u8, (y * 2) as u8, ((x + y) % 256) as u8])
    });
    DynamicImage::ImageRgb8(img).save(&src).unwrap();

    let web = WebOptions {
        widths: vec![128, 256, 1024],
        out_dir: dir.join("out"),
        alt: "Hero".into(),
        ..Default::default()
    };
    let set = generate(&src, &web, &OptimiseOptions::default()).unwrap();

    // 1024 would upscale a 300 px source, so only 128 and 256, in three formats.
    let mut got: Vec<(u32, &str)> = set
        .variants
        .iter()
        .map(|v| (v.width, v.format.extension()))
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec![
            (128, "avif"),
            (128, "jpg"),
            (128, "webp"),
            (256, "avif"),
            (256, "jpg"),
            (256, "webp")
        ]
    );
    for v in &set.variants {
        assert!(v.path.exists() && v.size > 0, "{}", v.path.display());
        assert_eq!(v.height, v.width / 2, "aspect kept");
        if v.format != ImageFormat::Avif {
            let decoded = image::open(&v.path).unwrap();
            assert_eq!((decoded.width(), decoded.height()), (v.width, v.height));
        }
    }
    assert!(set
        .html
        .contains(r#"srcset="hero-128.avif 128w, hero-256.avif 256w""#));
    assert!(set.html.contains(r#"<img src="hero-256.jpg""#));
    assert!(set.html.contains(r#"width="256" height="128""#));
    assert!(set.html.contains(r#"alt="Hero""#));
}

#[test]
fn transparent_images_fall_back_to_png() {
    let dir = tmpdir("alpha");
    let src = dir.join("logo.png");
    let img =
        image::RgbaImage::from_fn(200, 100, |x, _| image::Rgba([200, 40, 40, (x % 256) as u8]));
    DynamicImage::ImageRgba8(img).save(&src).unwrap();

    let web = WebOptions {
        widths: vec![100],
        formats: vec![ImageFormat::Webp],
        out_dir: dir.join("out"),
        ..Default::default()
    };
    let set = generate(&src, &web, &OptimiseOptions::default()).unwrap();

    let formats: Vec<&str> = set.variants.iter().map(|v| v.format.extension()).collect();
    assert_eq!(formats, vec!["webp", "png"]);
    assert!(set.html.contains(r#"<img src="logo-100.png""#));
}
