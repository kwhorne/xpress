//! PDF image recompression: colour-space safety and DPI downsampling.

use std::path::{Path, PathBuf};

use lopdf::{dictionary, Document, Object, Stream};
use xpress_core::pdf;
use xpress_core::result::OptimiseOptions;

fn tmpdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("xpress-pdf-{}-{}", std::process::id(), name));
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

/// A noisy q95 JPEG (so recompressing at the normal preset clearly shrinks it).
fn noisy_jpeg(w: u32, h: u32, gray: bool) -> Vec<u8> {
    let mut seed = 0x1234_5678u32;
    let img = image::RgbImage::from_fn(w, h, |_, _| {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        image::Rgb([(seed >> 16) as u8, (seed >> 8) as u8, seed as u8])
    });
    let img = if gray {
        image::DynamicImage::ImageLuma8(image::DynamicImage::ImageRgb8(img).to_luma8())
    } else {
        image::DynamicImage::ImageRgb8(img)
    };
    let mut jpeg = Vec::new();
    img.write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(
        &mut jpeg, 95,
    ))
    .unwrap();
    assert_eq!(jpeg_components(&jpeg), Some(if gray { 1 } else { 3 }));
    jpeg
}

/// A one-page PDF (`page` points square) drawing one JPEG XObject at `drawn`
/// points square. Returns the image's object id.
fn write_pdf(
    path: &Path,
    jpeg: Vec<u8>,
    (w, h): (u32, u32),
    colorspace: Object,
    page: i64,
    drawn: i64,
) -> (u32, u16) {
    let mut doc = Document::with_version("1.5");
    let img_id = doc.add_object(Object::Stream(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => w as i64,
            "Height" => h as i64,
            "ColorSpace" => colorspace,
            "BitsPerComponent" => 8,
            "Filter" => "DCTDecode",
        },
        jpeg,
    )));
    let content = format!("q {drawn} 0 0 {drawn} 10 10 cm /Im0 Do Q");
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));
    let pages_id = doc.new_object_id();
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), page.into(), page.into()],
        "Contents" => content_id,
        "Resources" => dictionary! { "XObject" => dictionary! { "Im0" => img_id } },
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
        }),
    );
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    doc.save(path).unwrap();
    img_id
}

/// (Width, Height, stream bytes) of the image XObject after optimisation.
/// Object ids can be renumbered on save, so find the (only) image stream.
fn image_after(path: &Path) -> (i64, i64, Vec<u8>) {
    let doc = Document::load(path).unwrap();
    doc.objects
        .values()
        .find_map(|o| match o {
            Object::Stream(s)
                if matches!(s.dict.get(b"Subtype"), Ok(Object::Name(n)) if n == b"Image") =>
            {
                Some((
                    s.dict.get(b"Width").unwrap().as_i64().unwrap(),
                    s.dict.get(b"Height").unwrap().as_i64().unwrap(),
                    s.content.clone(),
                ))
            }
            _ => None,
        })
        .expect("image stream present")
}

/// Number of colour components in a JPEG's start-of-frame header.
fn jpeg_components(jpeg: &[u8]) -> Option<u8> {
    let mut i = 2;
    while i + 4 <= jpeg.len() {
        if jpeg[i] != 0xFF {
            return None;
        }
        let marker = jpeg[i + 1];
        let len = u16::from_be_bytes([jpeg[i + 2], jpeg[i + 3]]) as usize;
        if matches!(marker, 0xC0..=0xC3) {
            return jpeg.get(i + 9).copied();
        }
        i += 2 + len;
    }
    None
}

#[test]
fn cmyk_jpeg_is_left_untouched() {
    let dir = tmpdir("cmyk");
    let f = dir.join("print.pdf");
    // Declared DeviceCMYK: decoding would yield RGB, so re-encoding would
    // corrupt the colours. It must be copied through byte for byte.
    let jpeg = noisy_jpeg(128, 128, false);
    write_pdf(&f, jpeg.clone(), (128, 128), "DeviceCMYK".into(), 200, 200);

    let o = OptimiseOptions {
        allow_larger: true,
        ..opts()
    };
    pdf::optimise(&f, &o, Some(36)).unwrap();

    let (w, h, bytes) = image_after(&f);
    assert_eq!((w, h), (128, 128));
    assert_eq!(bytes, jpeg, "CMYK image must not be re-encoded");
}

#[test]
fn gray_jpeg_is_recompressed() {
    let dir = tmpdir("gray");
    let f = dir.join("scan.pdf");
    let jpeg = noisy_jpeg(192, 192, true);
    write_pdf(&f, jpeg.clone(), (192, 192), "DeviceGray".into(), 200, 200);

    let r = pdf::optimise(&f, &opts(), None).unwrap();

    assert!(r.improved());
    let (_, _, bytes) = image_after(&f);
    assert_eq!(jpeg_components(&bytes), Some(1), "still a 1-component JPEG");
}

#[test]
fn dpi_downsamples_to_the_drawn_size() {
    let dir = tmpdir("dpi");
    let f = dir.join("photo.pdf");
    // 600 px drawn at 144 pt (2 in): at 72 dpi it needs only 144 px.
    write_pdf(
        &f,
        noisy_jpeg(600, 600, false),
        (600, 600),
        "DeviceRGB".into(),
        612,
        144,
    );

    pdf::optimise(&f, &opts(), Some(72)).unwrap();

    let (w, h, bytes) = image_after(&f);
    assert_eq!((w, h), (144, 144));
    let decoded = image::load_from_memory(&bytes).unwrap();
    assert_eq!(
        (decoded.width(), decoded.height()),
        (144, 144),
        "dict matches data"
    );
}

#[test]
fn no_dpi_keeps_resolution() {
    let dir = tmpdir("no-dpi");
    let f = dir.join("photo.pdf");
    write_pdf(
        &f,
        noisy_jpeg(300, 300, false),
        (300, 300),
        "DeviceRGB".into(),
        612,
        144,
    );

    pdf::optimise(&f, &opts(), None).unwrap();

    let (w, h, _) = image_after(&f);
    assert_eq!((w, h), (300, 300));
}

#[test]
fn image_already_under_the_dpi_is_not_resampled() {
    let dir = tmpdir("dpi-under");
    let f = dir.join("photo.pdf");
    // 200 px drawn at 288 pt (4 in) = 50 dpi, below the 150 dpi cap.
    write_pdf(
        &f,
        noisy_jpeg(200, 200, false),
        (200, 200),
        "DeviceRGB".into(),
        612,
        288,
    );

    pdf::optimise(&f, &opts(), Some(150)).unwrap();

    let (w, h, _) = image_after(&f);
    assert_eq!((w, h), (200, 200));
}
