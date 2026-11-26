//! Removing metadata through a real `exiftool` (TIFF): skipped when it isn't
//! installed, unless `XPRESS_REQUIRE_EXIFTOOL` is set (the CI smoke job).

use std::path::Path;
use std::process::Command;

use xpress_core::image::Strip;
use xpress_core::tools::{self, Tool};

const PLACE: &[&str] = &[
    "Trondheim",
    "Harbour",
    "ExtCity",
    "ExtCountry",
    "NOR",
    "GPS",
];

/// Camera, people and dates: kept by "location only", gone with "all".
const PERSONAL: &[&str] = &[
    "TestCam",
    "ModelX",
    "ArtistX",
    "SoftX",
    "HostX",
    "CopyX",
    "SerialX",
    "OwnerX",
    "2024:05:17",
];

fn exiftool() -> Option<std::path::PathBuf> {
    match tools::resolve(Tool::Exiftool) {
        Ok(path) => Some(path),
        Err(_) if std::env::var_os("XPRESS_REQUIRE_EXIFTOOL").is_some() => {
            panic!("XPRESS_REQUIRE_EXIFTOOL is set, but exiftool isn't installed")
        }
        Err(_) => None,
    }
}

/// All the tags exiftool reads, one per line (`[group] name : value`).
fn tags(exiftool: &Path, file: &Path) -> String {
    let out = Command::new(exiftool)
        .args(["-a", "-G1"])
        .arg(file)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A TIFF with a camera, GPS, and a place in IPTC, XMP and the IPTC
/// Extension.
fn tiff_with_place(exiftool: &Path, dir: &Path) -> std::path::PathBuf {
    let tiff = dir.join("place.tiff");
    image::RgbImage::from_fn(32, 24, |x, y| image::Rgb([x as u8 * 8, y as u8 * 10, 90]))
        .save(&tiff)
        .unwrap();
    let ok = Command::new(exiftool)
        .args([
            "-q",
            "-overwrite_original",
            "-Make=TestCam",
            "-Model=ModelX",
            "-Artist=ArtistX",
            "-Software=SoftX",
            "-HostComputer=HostX",
            "-Copyright=CopyX",
            "-DateTime=2024:05:17 12:00:00",
            "-ExifIFD:SerialNumber=SerialX",
            "-ExifIFD:OwnerName=OwnerX",
            "-ExifIFD:DateTimeOriginal=2024:05:17 12:00:00",
            "-Orientation#=6",
            "-GPSLatitude=63.4",
            "-GPSLatitudeRef=N",
            "-IPTC:City=Trondheim",
            "-IPTC:ContentLocationName=Harbour",
            "-IPTC:Country-PrimaryLocationCode=NOR",
            "-XMP-iptcCore:CountryCode=NOR",
            "-XMP-iptcExt:LocationCreated={City=ExtCity}",
            "-XMP-iptcExt:LocationShown={CountryName=ExtCountry}",
        ])
        .arg(&tiff)
        .status()
        .unwrap()
        .success();
    assert!(ok, "exiftool wrote the fixture");
    let before = tags(exiftool, &tiff);
    for tag in PLACE.iter().chain(PERSONAL) {
        assert!(before.contains(tag), "fixture has {tag}:\n{before}");
    }
    tiff
}

#[test]
fn a_tiffs_location_goes_and_its_camera_stays() {
    let Some(exiftool) = exiftool() else {
        eprintln!("exiftool not installed: skipped");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let tiff = tiff_with_place(&exiftool, dir.path());
    let out = dir.path().join("out.tiff");
    assert!(xpress_core::strip::strip_file(&tiff, &out, Strip::Location).unwrap());
    let after = tags(&exiftool, &out);
    for tag in PERSONAL {
        assert!(after.contains(tag), "{tag} kept:\n{after}");
    }
    for place in PLACE {
        assert!(!after.contains(place), "{place} left:\n{after}");
    }
    assert_same_pixels(&tiff, &out);
}

/// Removing metadata doesn't touch the image.
fn assert_same_pixels(a: &Path, b: &Path) {
    let a = image::open(a).unwrap().to_rgb8();
    let b = image::open(b).unwrap().to_rgb8();
    assert_eq!(a, b);
}

#[test]
fn removing_everything_from_a_tiff_leaves_no_camera() {
    let Some(exiftool) = exiftool() else {
        eprintln!("exiftool not installed: skipped");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let tiff = tiff_with_place(&exiftool, dir.path());
    let out = dir.path().join("out.tiff");
    assert!(xpress_core::strip::strip_file(&tiff, &out, Strip::All).unwrap());
    let after = tags(&exiftool, &out);
    for tag in PLACE.iter().chain(PERSONAL) {
        assert!(!after.contains(tag), "{tag} left:\n{after}");
    }
    assert!(after.contains("Rotate 90 CW"), "orientation kept:\n{after}");
    assert_same_pixels(&tiff, &out);
}
