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
    for place in PLACE {
        assert!(before.contains(place), "fixture has {place}:\n{before}");
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
    assert!(after.contains("TestCam"), "camera kept:\n{after}");
    for place in PLACE {
        assert!(!after.contains(place), "{place} left:\n{after}");
    }
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
    assert!(!after.contains("TestCam"), "camera removed:\n{after}");
    for place in PLACE {
        assert!(!after.contains(place), "{place} left:\n{after}");
    }
}
