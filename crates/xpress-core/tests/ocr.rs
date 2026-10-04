//! Text recognition (Apple Vision; macOS only).

#[test]
#[cfg(target_os = "macos")]
fn recognises_text_in_an_image() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ocr.png");
    let text = xpress_core::ocr::recognize_file(&path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines,
        ["Invoice 4711 paid", "error: file not found"],
        "got {text:?}"
    );
}

#[test]
#[cfg(target_os = "macos")]
fn an_image_without_text_gives_nothing() {
    let mut png = Vec::new();
    image::GrayImage::from_pixel(64, 64, image::Luma([255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    assert_eq!(xpress_core::ocr::recognize(&png).unwrap(), "");
}

#[test]
fn garbage_is_an_error_not_a_crash() {
    assert!(
        xpress_core::ocr::recognize(b"not an image").is_err() || !xpress_core::ocr::available()
    );
}
