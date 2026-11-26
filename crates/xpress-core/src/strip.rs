//! Removing metadata without re-encoding the pixels.
//!
//! JPEG, PNG and WebP are rewritten at the container level in pure Rust: the
//! metadata segments/chunks go, the image data is copied byte for byte. The
//! colour profile always stays (it is colour-critical), and so does the EXIF
//! orientation (as a minimal EXIF block), so a photo never ends up sideways.
//! Other formats: on macOS, HEIC and TIFF go through ImageIO (which copies the
//! image data unchanged), and the result is checked to have no location
//! left; what ImageIO can't write (AVIF, JXL), and everything on other
//! systems, goes through `exiftool` when it is installed. When neither can
//! do it, they fail loudly instead of keeping GPS silently.

use std::path::Path;

use image::metadata::Orientation;

use crate::filetype::extension_lower;
use crate::image::{Meta, Strip};
use crate::privacy::{strip_gps, strip_xmp_location};
use crate::result::OptimiseError;
use crate::tools::{self, Tool};

const EXIF_HEADER: &[u8] = b"Exif\0\0";
const XMP_HEADER: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
const PHOTOSHOP_HEADER: &[u8] = b"Photoshop 3.0\0";

/// IPTC-IIM datasets (record 2) that say where a photo was taken: content
/// location code and name, city, sub-location, province/state, country code
/// and country name.
const IPTC_LOCATION: &[u8] = &[26, 27, 90, 92, 95, 100, 101];

/// Write `src` without the metadata `strip` asks to remove to `out`.
///
/// Returns `Ok(false)` (and writes nothing) when there was nothing to remove,
/// or for formats that carry no removable metadata (GIF, BMP).
pub fn strip_file(src: &Path, out: &Path, strip: Strip) -> Result<bool, OptimiseError> {
    if strip == Strip::Keep {
        return Ok(false);
    }
    let ext = extension_lower(src).unwrap_or_default();
    let rewrite: fn(&[u8], Strip) -> Option<Vec<u8>> = match ext.as_str() {
        "jpg" | "jpeg" => strip_jpeg,
        "png" | "apng" => strip_png,
        "webp" => strip_webp,
        "gif" | "bmp" => return Ok(false),
        _ => return strip_with_exiftool(src, out, strip, &ext),
    };
    let data = std::fs::read(src)?;
    let stripped = rewrite(&data, strip).ok_or_else(|| {
        OptimiseError::Other(format!(
            "{}: could not read its metadata to remove it",
            src.display()
        ))
    })?;
    if stripped == data {
        return Ok(false);
    }
    std::fs::write(out, stripped)?;
    Ok(true)
}

/// HEIC, AVIF, TIFF, JXL, …: (macOS, HEIC) ImageIO, or `exiftool` on a copy.
fn strip_with_exiftool(
    src: &Path,
    out: &Path,
    strip: Strip,
    ext: &str,
) -> Result<bool, OptimiseError> {
    // ImageIO first for HEIC (much quicker than starting exiftool). Not for
    // TIFF: copying one keeps its own TIFF and EXIF tags (camera, owner,
    // serial number) whatever the metadata option says. AVIF and JXL it
    // can't write.
    #[cfg(target_os = "macos")]
    if matches!(ext, "heic" | "heif") && imageio::strip(src, out, strip).is_ok() {
        return Ok(true);
    }
    if !tools::is_available(Tool::Exiftool) {
        return Err(OptimiseError::Other(format!(
            "removing metadata from .{ext} files needs exiftool (e.g. `brew install exiftool`)"
        )));
    }
    std::fs::copy(src, out)?;
    let mut args: Vec<std::ffi::OsString> = vec!["-overwrite_original".into(), "-q".into()];
    let tags: &[&str] = match strip {
        Strip::Keep => return Ok(false),
        // Delete everything, then copy the colour profile and orientation
        // back from the original (`@`).
        Strip::All => &[
            "-all=",
            "-tagsFromFile",
            "@",
            "-icc_profile",
            "-Orientation",
        ],
        Strip::Location => &[
            "-gps:all=",
            "-xmp-exif:gps*=",
            "-xmp-photoshop:City=",
            "-xmp-photoshop:State=",
            "-xmp-photoshop:Country=",
            "-xmp-iptcCore:Location=",
            "-iptc:City=",
            "-iptc:Sub-location=",
            "-iptc:Province-State=",
            "-iptc:Country-PrimaryLocationName=",
            "-iptc:Country-PrimaryLocationCode=",
            "-iptc:ContentLocationName=",
            "-iptc:ContentLocationCode=",
            "-xmp-iptcCore:CountryCode=",
            "-xmp-iptcExt:LocationCreated=",
            "-xmp-iptcExt:LocationShown=",
        ],
    };
    args.extend(tags.iter().map(Into::into));
    // `out` is an absolute temp path, so it can't be mistaken for an option.
    args.push(out.as_os_str().to_owned());
    tools::run(Tool::Exiftool, &args)?;
    Ok(true)
}

/// A minimal big-endian EXIF (TIFF) block holding only the orientation, or
/// `None` when the image is upright anyway.
fn orientation_exif(exif: &[u8]) -> Option<Vec<u8>> {
    let o = Orientation::from_exif_chunk(exif)?.to_exif();
    if o <= 1 {
        return None;
    }
    let mut t = b"MM\0\x2a\0\0\0\x08".to_vec();
    t.extend_from_slice(&1u16.to_be_bytes()); // one entry
    t.extend_from_slice(&0x0112u16.to_be_bytes()); // Orientation
    t.extend_from_slice(&3u16.to_be_bytes()); // SHORT
    t.extend_from_slice(&1u32.to_be_bytes()); // count
    t.extend_from_slice(&[0, o, 0, 0]); // value, left-justified
    t.extend_from_slice(&0u32.to_be_bytes()); // no next IFD
    Some(t)
}

/// The EXIF block to keep under `strip`, or `None` to drop it.
fn kept_exif(exif: &[u8], strip: Strip) -> Option<Vec<u8>> {
    match strip {
        Strip::Keep => Some(exif.to_vec()),
        Strip::All => orientation_exif(exif),
        Strip::Location => {
            let mut e = exif.to_vec();
            strip_gps(&mut e);
            Some(e)
        }
    }
}

// ---------------------------------------------------------------------------
// ImageIO (macOS): HEIC, TIFF, … without exiftool
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod imageio {
    use std::path::Path;
    use std::ptr;

    use objc2_core_foundation::{CFBoolean, CFDictionary, CFString, CFType, CFURL};
    use objc2_image_io::{
        kCGImageDestinationMergeMetadata, kCGImageDestinationMetadata,
        kCGImageMetadataShouldExcludeGPS, kCGImagePropertyGPSDictionary, CGImageDestination,
        CGImageMetadata, CGImageMetadataTag, CGImageSource, CGMutableImageMetadata,
    };

    use crate::image::Strip;

    /// XMP that says where a photo was taken, besides GPS (ImageIO drops GPS
    /// itself). IPTC city/country are read and written as these too.
    const XMP_LOCATION: &[&str] = &[
        "photoshop:City",
        "photoshop:State",
        "photoshop:Country",
        "Iptc4xmpCore:Location",
        "Iptc4xmpCore:CountryCode",
        "Iptc4xmpExt:LocationCreated",
        "Iptc4xmpExt:LocationShown",
        // IPTC content location name and code, as ImageIO names them.
        "Iptc4xmpCore:IIM2-026",
        "Iptc4xmpCore:IIM2-027",
    ];

    /// What may be left in an image's properties after removing everything:
    /// its layout and orientation. Anything else (camera, owner, dates,
    /// IPTC, maker notes) means the copy kept more than it should.
    const BARE_TIFF: &[&str] = &[
        "Orientation",
        "TileWidth",
        "TileLength",
        "XResolution",
        "YResolution",
        "ResolutionUnit",
        "Compression",
        "PhotometricInterpretation",
    ];
    const BARE_EXIF: &[&str] = &["PixelXDimension", "PixelYDimension", "ColorSpace"];

    /// Copy `src` to `out` without the metadata `strip` asks to remove. The
    /// image data is copied, not re-encoded. `Err` when ImageIO can't, or
    /// when the copy would still say where the photo was taken.
    pub fn strip(src: &Path, out: &Path, strip: Strip) -> Result<(), String> {
        let src_url = CFURL::from_file_path(src).ok_or("not a file path")?;
        let out_url = CFURL::from_file_path(out).ok_or("not a file path")?;
        // SAFETY: plain ImageIO calls; every dictionary holds the value
        // types ImageIO documents for its keys.
        unsafe {
            let source = CGImageSource::with_url(&src_url, None).ok_or("ImageIO can't read it")?;
            let uti = source.r#type().ok_or("ImageIO doesn't know the format")?;
            let count = source.count();
            if count == 0 {
                return Err("no image in it".into());
            }
            let dest = CGImageDestination::with_url(&out_url, &uti, count, None)
                .ok_or("ImageIO can't write the format")?;

            // The metadata to write in place of the file's own: a copy without
            // the tags to remove. Removing everything keeps the orientation
            // (so the photo stays upright) and ImageIO's own `iio:` tags —
            // without those it keeps the file's old XMP (city, creation date)
            // instead of writing it anew.
            let remove = |path: &str| match strip {
                Strip::Keep => false,
                Strip::All => path != "tiff:Orientation" && !path.starts_with("iio:"),
                Strip::Location => XMP_LOCATION.contains(&path),
            };
            if strip == Strip::Keep {
                return Err("nothing to remove".into());
            }
            let metadata = source
                .metadata_at_index(0, None)
                .and_then(|m| CGMutableImageMetadata::new_copy(&m))
                .unwrap_or_else(|| CGMutableImageMetadata::new());
            for path in tag_paths(&metadata) {
                if remove(&path) {
                    metadata.remove_tag_with_path(None, &CFString::from_str(&path));
                }
            }
            let keys: [&CFString; 3] = [
                kCGImageMetadataShouldExcludeGPS,
                kCGImageDestinationMetadata,
                kCGImageDestinationMergeMetadata,
            ];
            let values: [&CFType; 3] = [
                CFBoolean::new(true).as_ref(),
                metadata.as_ref(),
                CFBoolean::new(false).as_ref(),
            ];
            let options = CFDictionary::<CFString, CFType>::from_slices(&keys, &values);
            if !dest.copy_image_source(&source, Some(options.as_opaque()), ptr::null_mut()) {
                let _ = std::fs::remove_file(out);
                return Err("ImageIO couldn't copy it".into());
            }
        }
        if has_location(out) {
            let _ = std::fs::remove_file(out);
            return Err("the location was still there".into());
        }
        if strip == Strip::All && !is_bare(out) {
            let _ = std::fs::remove_file(out);
            return Err("metadata was still there".into());
        }
        Ok(())
    }

    /// Whether no image in the file has more than [`BARE_TIFF`] and
    /// [`BARE_EXIF`] in its metadata dictionaries. Unreadable counts as no.
    fn is_bare(path: &Path) -> bool {
        let Some(url) = CFURL::from_file_path(path) else {
            return false;
        };
        // SAFETY: as above; property dictionaries have string keys.
        unsafe {
            let Some(source) = CGImageSource::with_url(&url, None) else {
                return false;
            };
            (0..source.count()).all(|i| {
                let Some(props) = source.properties_at_index(i, None) else {
                    return false;
                };
                let props = props.cast_unchecked::<CFString, CFType>();
                let (keys, values) = props.to_vecs();
                keys.iter().zip(values).all(|(key, value)| {
                    let key = key.to_string();
                    // Plain values (size, depth, colour model) aren't metadata.
                    if !key.starts_with('{') {
                        return true;
                    }
                    let allowed = match key.as_str() {
                        "{TIFF}" => BARE_TIFF,
                        "{Exif}" => BARE_EXIF,
                        _ => return false,
                    };
                    value
                        .downcast_ref::<CFDictionary>()
                        .map(|d| d.cast_unchecked::<CFString, CFType>().to_vecs().0)
                        .is_some_and(|inner| {
                            inner
                                .iter()
                                .all(|k| allowed.contains(&k.to_string().as_str()))
                        })
                })
            })
        }
    }

    /// The paths (`prefix:name`) of the top-level tags in `metadata`.
    unsafe fn tag_paths(metadata: &CGImageMetadata) -> Vec<String> {
        let Some(tags) = (unsafe { metadata.tags() }) else {
            return Vec::new();
        };
        let tags = unsafe { tags.cast_unchecked::<CGImageMetadataTag>() };
        tags.iter()
            .filter_map(|t| {
                let (prefix, name) = unsafe { (t.prefix()?, t.name()?) };
                Some(format!("{prefix}:{name}"))
            })
            .collect()
    }

    /// Whether the file still says where it was taken: GPS in any image, or
    /// location XMP in the first. Unreadable counts as yes.
    fn has_location(path: &Path) -> bool {
        has_gps(path)
            || CFURL::from_file_path(path)
                .and_then(|url| unsafe { CGImageSource::with_url(&url, None) })
                .and_then(|src| unsafe { src.metadata_at_index(0, None) })
                .is_some_and(|m| {
                    let tags = unsafe { tag_paths(&m) };
                    tags.iter().any(|t| XMP_LOCATION.contains(&t.as_str()))
                })
    }

    /// Whether any image in the file has a GPS dictionary.
    fn has_gps(path: &Path) -> bool {
        let Some(url) = CFURL::from_file_path(path) else {
            return true;
        };
        // SAFETY: as above.
        unsafe {
            let Some(source) = CGImageSource::with_url(&url, None) else {
                return true;
            };
            (0..source.count()).any(|i| {
                source.properties_at_index(i, None).is_none_or(|p| {
                    p.cast_unchecked::<CFString, CFType>()
                        .get(kCGImagePropertyGPSDictionary)
                        .is_some()
                })
            })
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use objc2_core_foundation::{CFNumber, CFRetained};
        use objc2_image_io::{
            kCGImagePropertyExifDateTimeOriginal, kCGImagePropertyExifDictionary,
            kCGImagePropertyGPSLatitude, kCGImagePropertyGPSLatitudeRef, kCGImagePropertyIPTCCity,
            kCGImagePropertyIPTCDictionary, kCGImagePropertyOrientation,
            kCGImagePropertyTIFFDictionary, kCGImagePropertyTIFFMake,
        };

        type Dict = CFDictionary<CFString, CFType>;

        fn dict(pairs: &[(&CFString, &CFType)]) -> CFRetained<Dict> {
            let (k, v): (Vec<_>, Vec<_>) = pairs.iter().copied().unzip();
            Dict::from_slices(&k, &v)
        }

        /// An iPhone-like HEIC: GPS, camera make, capture time, IPTC city,
        /// and a sideways orientation.
        fn heic_with_metadata(dir: &Path) -> std::path::PathBuf {
            image_with_metadata(dir, "photo.heic", "public.heic")
        }

        fn image_with_metadata(dir: &Path, name: &str, uti: &str) -> std::path::PathBuf {
            let jpg = dir.join("src.jpg");
            image::RgbImage::from_fn(64, 48, |x, y| image::Rgb([x as u8 * 4, y as u8 * 5, 90]))
                .save(&jpg)
                .unwrap();
            let heic = dir.join(name);
            unsafe {
                let s = |v: &str| CFString::from_str(v);
                let lat = CFNumber::new_f64(63.43);
                let (n, make, when, city) = (
                    s("N"),
                    s("TestCam"),
                    s("2024:05:17 12:00:00"),
                    s("Trondheim"),
                );
                let gps = dict(&[
                    (kCGImagePropertyGPSLatitude, lat.as_ref()),
                    (kCGImagePropertyGPSLatitudeRef, n.as_ref()),
                ]);
                let tiff = dict(&[(kCGImagePropertyTIFFMake, make.as_ref())]);
                let exif = dict(&[(kCGImagePropertyExifDateTimeOriginal, when.as_ref())]);
                let iptc = dict(&[(kCGImagePropertyIPTCCity, city.as_ref())]);
                let six = CFNumber::new_i32(6);
                let props = dict(&[
                    (kCGImagePropertyGPSDictionary, gps.as_ref()),
                    (kCGImagePropertyTIFFDictionary, tiff.as_ref()),
                    (kCGImagePropertyExifDictionary, exif.as_ref()),
                    (kCGImagePropertyIPTCDictionary, iptc.as_ref()),
                    (kCGImagePropertyOrientation, six.as_ref()),
                ]);
                let src =
                    CGImageSource::with_url(&CFURL::from_file_path(&jpg).unwrap(), None).unwrap();
                let dest = CGImageDestination::with_url(
                    &CFURL::from_file_path(&heic).unwrap(),
                    &s(uti),
                    1,
                    None,
                )
                .unwrap();
                dest.add_image_from_source(&src, 0, Some(props.as_opaque()));
                assert!(dest.finalize());
            }
            heic
        }

        /// The first image's value at `dict` / `key`, as a string.
        fn prop(path: &Path, dict: Option<&CFString>, key: &CFString) -> Option<String> {
            unsafe {
                let url = CFURL::from_file_path(path)?;
                let src = CGImageSource::with_url(&url, None)?;
                let props = src.properties_at_index(0, None)?;
                let props = props.cast_unchecked::<CFString, CFType>();
                let v = match dict {
                    None => props.get(key)?,
                    Some(d) => {
                        let inner = props.get(d)?.downcast::<CFDictionary>().ok()?;
                        inner.cast_unchecked::<CFString, CFType>().get(key)?
                    }
                };
                Some(match v.downcast_ref::<CFString>() {
                    Some(s) => s.to_string(),
                    None => v.downcast_ref::<CFNumber>()?.as_i64()?.to_string(),
                })
            }
        }

        #[test]
        fn a_tiff_that_keeps_its_own_tags_is_refused() {
            // Copying a TIFF keeps its TIFF/EXIF tags: removing everything
            // must not report success with the camera still in it.
            let dir = tempfile::tempdir().unwrap();
            let tiff = image_with_metadata(dir.path(), "photo.tiff", "public.tiff");
            let out = dir.path().join("all.tiff");
            assert!(strip(&tiff, &out, Strip::All).is_err());
            assert!(!out.exists());
            // And `strip_file` doesn't use ImageIO for it at all.
            let out = dir.path().join("file.tiff");
            match super::super::strip_file(&tiff, &out, Strip::All) {
                Ok(_) => assert!(!std::fs::read(&out)
                    .unwrap()
                    .windows(7)
                    .any(|w| w == b"TestCam")),
                Err(e) => assert!(e.to_string().contains("exiftool"), "{e}"),
            }
        }

        #[test]
        fn heic_without_location_or_metadata() {
            let (iptc, city, tiff, make, exif, when, orientation) = unsafe {
                (
                    Some(kCGImagePropertyIPTCDictionary),
                    kCGImagePropertyIPTCCity,
                    Some(kCGImagePropertyTIFFDictionary),
                    kCGImagePropertyTIFFMake,
                    Some(kCGImagePropertyExifDictionary),
                    kCGImagePropertyExifDateTimeOriginal,
                    kCGImagePropertyOrientation,
                )
            };
            let dir = tempfile::tempdir().unwrap();
            let heic = heic_with_metadata(dir.path());
            assert!(has_gps(&heic), "the fixture has GPS");
            assert_eq!(prop(&heic, iptc, city).as_deref(), Some("Trondheim"));
            assert_eq!(prop(&heic, tiff, make).as_deref(), Some("TestCam"));
            assert_eq!(prop(&heic, None, orientation).as_deref(), Some("6"));

            let loc = dir.path().join("loc.heic");
            strip(&heic, &loc, Strip::Location).unwrap();
            assert!(!has_gps(&loc));
            assert_eq!(prop(&loc, iptc, city), None, "city gone");
            assert_eq!(
                prop(&loc, tiff, make).as_deref(),
                Some("TestCam"),
                "the rest kept"
            );
            assert_eq!(prop(&loc, None, orientation).as_deref(), Some("6"));

            let all = dir.path().join("all.heic");
            strip(&heic, &all, Strip::All).unwrap();
            assert!(!has_gps(&all));
            assert_eq!(prop(&all, tiff, make), None, "camera gone");
            assert_eq!(prop(&all, exif, when), None, "capture time gone");
            assert_eq!(prop(&all, iptc, city), None, "city gone");
            assert_eq!(
                prop(&all, None, orientation).as_deref(),
                Some("6"),
                "still upright"
            );

            // Through the public entry point.
            let via = dir.path().join("via.heic");
            assert!(super::super::strip_file(&heic, &via, Strip::Location).unwrap());
            assert!(!has_location(&via));
        }
    }
}

// ---------------------------------------------------------------------------
// JPEG
// ---------------------------------------------------------------------------

fn jpeg_segment(marker: u8, payload: &[u8]) -> Option<Vec<u8>> {
    let len = u16::try_from(payload.len() + 2).ok()?;
    let mut s = vec![0xFF, marker];
    s.extend_from_slice(&len.to_be_bytes());
    s.extend_from_slice(payload);
    Some(s)
}

/// Rewrite a JPEG's header segments; everything from the start of scan on
/// (including MPF secondary images such as gain maps) is copied unchanged.
fn strip_jpeg(data: &[u8], strip: Strip) -> Option<Vec<u8>> {
    if !data.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut out = vec![0xFF, 0xD8];
    let mut pos = 2;
    // Where the MPF (multi-picture) segment starts, in `data` and in `out`.
    let mut mpf: Option<(usize, usize)> = None;
    loop {
        if *data.get(pos)? != 0xFF {
            return None;
        }
        let mut m = pos + 1;
        while *data.get(m)? == 0xFF {
            m += 1;
        }
        let marker = data[m];
        match marker {
            // Start of scan / end of image: the rest is image data.
            0xDA | 0xD9 => {
                out.extend_from_slice(&data[pos..]);
                break;
            }
            0x01 | 0xD0..=0xD8 => {
                out.extend_from_slice(&data[pos..=m]);
                pos = m + 1;
                continue;
            }
            _ => {}
        }
        let len = u16::from_be_bytes(data.get(m + 1..m + 3)?.try_into().ok()?) as usize;
        if len < 2 {
            return None;
        }
        let end = m + 1 + len;
        let payload = data.get(m + 3..end)?;
        let segment = &data[pos..end];
        let all = strip == Strip::All;
        let kept: Option<Vec<u8>> = match marker {
            0xE1 if payload.starts_with(EXIF_HEADER) => {
                kept_exif(&payload[EXIF_HEADER.len()..], strip).and_then(|e| {
                    let mut p = EXIF_HEADER.to_vec();
                    p.extend_from_slice(&e);
                    jpeg_segment(0xE1, &p)
                })
            }
            0xE1 if payload.starts_with(XMP_HEADER) => match strip {
                Strip::All => None,
                _ => {
                    let mut p = XMP_HEADER.to_vec();
                    p.extend_from_slice(&strip_xmp_location(&payload[XMP_HEADER.len()..]));
                    jpeg_segment(0xE1, &p)
                }
            },
            // Photoshop/IPTC: captions, copyright and keywords, but also
            // city and country. Removing location keeps the rest.
            0xED => match strip {
                Strip::Keep => Some(segment.to_vec()),
                Strip::All => None,
                Strip::Location => {
                    photoshop_without_location(payload).and_then(|p| jpeg_segment(0xED, &p))
                }
            },
            // JFIF, ICC profile / MPF, Adobe colour transform: needed to show
            // the image right.
            0xE0 | 0xE2 | 0xEE => Some(segment.to_vec()),
            // Other APPn (extended XMP, maker data, …) and comments.
            0xE1 | 0xE3..=0xEF | 0xFE if all => None,
            _ => Some(segment.to_vec()),
        };
        if marker == 0xE2 && payload.starts_with(b"MPF\0") && mpf.is_none() {
            // Its TIFF header follows `MPF\0`; the segment (fill bytes before
            // the marker included) is copied unchanged.
            mpf = Some((m + 7, out.len() + (m - pos) + 7));
        }
        if let Some(k) = kept {
            out.extend_from_slice(&k);
        }
        pos = end;
    }
    if let Some((orig_at, out_at)) = mpf {
        // Bytes removed between the MPF header and the image data shift the
        // secondary images; their offsets are relative to the MPF header.
        // (A rewritten APP13 can also grow by a pad byte.)
        let shift = (out.len() - out_at) as i64 - (data.len() - orig_at) as i64;
        if shift != 0 {
            patch_mpf_offsets(&mut out, out_at, shift)?;
        }
    }
    Some(out)
}

/// An APP13 (Photoshop) payload without where the photo was taken: the IPTC
/// location datasets go, and so does the IPTC digest when that changed the
/// IPTC (it would no longer match); embedded XMP loses its location, embedded
/// EXIF (which may hold GPS) goes. `None` when it can't be read — the caller
/// then drops the whole segment rather than keep a location.
fn photoshop_without_location(payload: &[u8]) -> Option<Vec<u8>> {
    let mut rest = payload.strip_prefix(PHOTOSHOP_HEADER)?;
    let mut blocks: Vec<(u16, &[u8], Vec<u8>)> = Vec::new();
    let mut iptc_changed = false;
    while !rest.is_empty() {
        if !rest.starts_with(b"8BIM") {
            return None;
        }
        let id = u16::from_be_bytes(rest.get(4..6)?.try_into().ok()?);
        // A Pascal string, padded to an even length.
        let name_len = *rest.get(6)? as usize;
        let name_end = 7 + name_len + (name_len + 1) % 2;
        let name = rest.get(6..name_end)?;
        let size = u32::from_be_bytes(rest.get(name_end..name_end + 4)?.try_into().ok()?) as usize;
        let start = name_end + 4;
        let data = rest.get(start..start + size)?;
        rest = rest.get(start + size + size % 2..).unwrap_or_default();
        let data = match id {
            0x0404 => {
                let iptc = iptc_without_location(data)?;
                iptc_changed |= iptc.len() != data.len();
                iptc
            }
            0x0424 => strip_xmp_location(data),
            0x0422 | 0x0423 => continue,
            _ => data.to_vec(),
        };
        blocks.push((id, name, data));
    }
    let mut out = PHOTOSHOP_HEADER.to_vec();
    for (id, name, data) in blocks {
        if id == 0x0425 && iptc_changed {
            continue;
        }
        out.extend_from_slice(b"8BIM");
        out.extend_from_slice(&id.to_be_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(&u32::try_from(data.len()).ok()?.to_be_bytes());
        out.extend_from_slice(&data);
        if data.len() % 2 == 1 {
            out.push(0);
        }
    }
    Some(out)
}

/// IPTC-IIM data without the location datasets, or `None` when it can't be
/// read.
fn iptc_without_location(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if data[i] != 0x1C {
            // Some writers pad the block with zeros.
            if data[i..].iter().all(|&b| b == 0) {
                out.extend_from_slice(&data[i..]);
                break;
            }
            return None;
        }
        let (record, dataset) = (*data.get(i + 1)?, *data.get(i + 2)?);
        let n = u16::from_be_bytes(data.get(i + 3..i + 5)?.try_into().ok()?) as usize;
        let (header, len) = if n & 0x8000 == 0 {
            (5, n)
        } else {
            // Extended dataset: the low bits say how many length bytes follow.
            let k = n & 0x7FFF;
            if k == 0 || k > 4 {
                return None;
            }
            let len = data
                .get(i + 5..i + 5 + k)?
                .iter()
                .fold(0usize, |l, &b| l << 8 | b as usize);
            (5 + k, len)
        };
        let item = data.get(i..i + header + len)?;
        if !(record == 2 && IPTC_LOCATION.contains(&dataset)) {
            out.extend_from_slice(item);
        }
        i += item.len();
    }
    Some(out)
}

/// Add `shift` (negative when bytes were removed) to every non-zero image
/// offset in an MPF index whose TIFF header starts at `base`.
fn patch_mpf_offsets(buf: &mut [u8], base: usize, shift: i64) -> Option<()> {
    let le = match buf.get(base..base + 2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let rd16 = |b: &[u8], at: usize| -> Option<u16> {
        let a: [u8; 2] = b.get(at..at + 2)?.try_into().ok()?;
        Some(if le {
            u16::from_le_bytes(a)
        } else {
            u16::from_be_bytes(a)
        })
    };
    let rd32 = |b: &[u8], at: usize| -> Option<u32> {
        let a: [u8; 4] = b.get(at..at + 4)?.try_into().ok()?;
        Some(if le {
            u32::from_le_bytes(a)
        } else {
            u32::from_be_bytes(a)
        })
    };
    let ifd = base + rd32(buf, base + 4)? as usize;
    let n = rd16(buf, ifd)? as usize;
    for i in 0..n {
        let e = ifd + 2 + i * 12;
        if rd16(buf, e)? != 0xB002 {
            continue; // not the MP Entry list
        }
        let count = rd32(buf, e + 4)? as usize / 16;
        let entries = base + rd32(buf, e + 8)? as usize;
        for j in 0..count {
            let at = entries + j * 16 + 8;
            let off = rd32(buf, at)?;
            if off != 0 {
                let new = u32::try_from(i64::from(off) + shift).ok()?;
                let b = if le {
                    new.to_le_bytes()
                } else {
                    new.to_be_bytes()
                };
                buf.get_mut(at..at + 4)?.copy_from_slice(&b);
            }
        }
    }
    Some(())
}

// ---------------------------------------------------------------------------
// PNG
// ---------------------------------------------------------------------------

fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut c = (data.len() as u32).to_be_bytes().to_vec();
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    c.extend_from_slice(&body);
    c.extend_from_slice(&crc32fast::hash(&body).to_be_bytes());
    c
}

fn strip_png(data: &[u8], strip: Strip) -> Option<Vec<u8>> {
    const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !data.starts_with(SIG) {
        return None;
    }
    let mut out = SIG.to_vec();
    let mut pos = SIG.len();
    while pos < data.len() {
        let len = u32::from_be_bytes(data.get(pos..pos + 4)?.try_into().ok()?) as usize;
        let kind: [u8; 4] = data.get(pos + 4..pos + 8)?.try_into().ok()?;
        let body = data.get(pos + 8..pos + 8 + len)?;
        let end = pos + 12 + len;
        let chunk = data.get(pos..end)?;
        let all = strip == Strip::All;
        let xmp = &kind == b"iTXt" && body.starts_with(b"XML:com.adobe.xmp\0");
        match &kind {
            b"eXIf" => {
                if let Some(e) = kept_exif(body, strip) {
                    out.extend_from_slice(&png_chunk(b"eXIf", &e));
                }
            }
            // Uncompressed XMP (flag byte 0): drop just the location.
            b"iTXt" if xmp && !all && body.get(18) == Some(&0) => {
                // keyword\0, compression flag, method, language\0, translated\0
                let mut rest = &body[18 + 2..];
                let mut head = body[..18 + 2].to_vec();
                for _ in 0..2 {
                    let nul = rest.iter().position(|&b| b == 0)?;
                    head.extend_from_slice(&rest[..=nul]);
                    rest = &rest[nul + 1..];
                }
                head.extend_from_slice(&strip_xmp_location(rest));
                out.extend_from_slice(&png_chunk(b"iTXt", &head));
            }
            b"iTXt" if xmp => {}
            b"tEXt" | b"zTXt" | b"iTXt" | b"tIME" if all => {}
            _ => out.extend_from_slice(chunk),
        }
        pos = end;
        if &kind == b"IEND" {
            break;
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// WebP
// ---------------------------------------------------------------------------

fn strip_webp(data: &[u8], strip: Strip) -> Option<Vec<u8>> {
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WEBP" {
        return None;
    }
    let mut meta = Meta::default();
    let mut has_vp8x = false;
    let mut pos = 12;
    while pos + 8 <= data.len() {
        let kind = &data[pos..pos + 4];
        let size = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().ok()?) as usize;
        let body = data.get(pos + 8..pos + 8 + size)?;
        match kind {
            b"VP8X" => has_vp8x = true,
            b"ICCP" => meta.icc = Some(body.to_vec()),
            b"EXIF" => {
                // Some writers keep JPEG's "Exif\0\0" prefix.
                let tiff = body.strip_prefix(EXIF_HEADER).unwrap_or(body);
                meta.exif = kept_exif(tiff, strip);
            }
            b"XMP " if strip == Strip::Location => meta.xmp = Some(strip_xmp_location(body)),
            _ => {}
        }
        pos += 8 + size + (size & 1);
    }
    // A simple (non-VP8X) WebP can't carry metadata.
    if !has_vp8x {
        return Some(data.to_vec());
    }
    // The canvas size comes from the existing VP8X chunk.
    crate::image::webp_with_meta(data, &meta, 1, 1, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A little-endian EXIF block: orientation 6 plus a GPS IFD.
    fn exif_with_gps() -> Vec<u8> {
        let mut t = b"II\x2a\0\x08\0\0\0".to_vec();
        t.extend_from_slice(&2u16.to_le_bytes());
        // Orientation = 6
        t.extend_from_slice(&0x0112u16.to_le_bytes());
        t.extend_from_slice(&3u16.to_le_bytes());
        t.extend_from_slice(&1u32.to_le_bytes());
        t.extend_from_slice(&[6, 0, 0, 0]);
        // GPS IFD pointer -> offset 38
        t.extend_from_slice(&0x8825u16.to_le_bytes());
        t.extend_from_slice(&4u16.to_le_bytes());
        t.extend_from_slice(&1u32.to_le_bytes());
        t.extend_from_slice(&38u32.to_le_bytes());
        t.extend_from_slice(&0u32.to_le_bytes());
        // GPS IFD: one entry, GPSLatitudeRef = "N"
        t.extend_from_slice(&1u16.to_le_bytes());
        t.extend_from_slice(&1u16.to_le_bytes());
        t.extend_from_slice(&2u16.to_le_bytes());
        t.extend_from_slice(&2u32.to_le_bytes());
        t.extend_from_slice(b"N\0\0\0");
        t.extend_from_slice(&0u32.to_le_bytes());
        t
    }

    fn jpeg(exif: &[u8]) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(8, 8, image::Rgb([200, 10, 10]));
        let mut plain = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut plain),
                image::ImageFormat::Jpeg,
            )
            .unwrap();
        let mut p = EXIF_HEADER.to_vec();
        p.extend_from_slice(exif);
        let mut out = plain[..2].to_vec();
        out.extend_from_slice(&jpeg_segment(0xE1, &p).unwrap());
        out.extend_from_slice(&jpeg_segment(0xFE, b"secret comment").unwrap());
        out.extend_from_slice(&plain[2..]);
        out
    }

    fn exif_of(jpg: &[u8]) -> Option<Vec<u8>> {
        use image::ImageDecoder;
        let mut d = image::codecs::jpeg::JpegDecoder::new(std::io::Cursor::new(jpg)).unwrap();
        d.exif_metadata().unwrap()
    }

    #[test]
    fn jpeg_strip_all_keeps_only_orientation_and_pixels() {
        let src = jpeg(&exif_with_gps());
        let out = strip_jpeg(&src, Strip::All).unwrap();
        let exif = exif_of(&out).expect("orientation kept");
        assert_eq!(
            Orientation::from_exif_chunk(&exif),
            Some(Orientation::Rotate90)
        );
        assert_eq!(exif.len(), 26);
        assert!(!out.windows(14).any(|w| w == b"secret comment"));
        let a = image::load_from_memory(&src).unwrap().to_rgb8();
        let b = image::load_from_memory(&out).unwrap().to_rgb8();
        assert_eq!(a, b);
    }

    #[test]
    fn jpeg_strip_location_keeps_the_rest() {
        let src = jpeg(&exif_with_gps());
        let out = strip_jpeg(&src, Strip::Location).unwrap();
        let exif = exif_of(&out).unwrap();
        assert_eq!(
            Orientation::from_exif_chunk(&exif),
            Some(Orientation::Rotate90)
        );
        assert!(!exif.windows(2).any(|w| w == b"N\0"));
        assert!(out.windows(14).any(|w| w == b"secret comment"));
    }

    #[test]
    fn png_strip_all_drops_text_and_exif() {
        let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([1, 2, 3, 255]));
        let mut plain = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut plain),
                image::ImageFormat::Png,
            )
            .unwrap();
        let mut src = plain[..33].to_vec(); // signature + IHDR
        src.extend_from_slice(&png_chunk(b"tEXt", b"Author\0Someone"));
        src.extend_from_slice(&png_chunk(b"eXIf", &exif_with_gps()));
        src.extend_from_slice(&plain[33..]);
        let out = strip_png(&src, Strip::All).unwrap();
        assert!(!out.windows(7).any(|w| w == b"Someone"));
        assert!(!out.windows(2).any(|w| w == b"N\0"));
        assert!(out.windows(4).any(|w| w == b"eXIf")); // orientation only
        assert_eq!(
            image::load_from_memory(&src).unwrap().to_rgba8(),
            image::load_from_memory(&out).unwrap().to_rgba8()
        );
    }

    #[test]
    fn webp_strip_all_keeps_orientation_and_pixels() {
        let img = image::RgbaImage::from_pixel(6, 4, image::Rgba([9, 8, 7, 255]));
        let mut plain = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut plain),
                image::ImageFormat::WebP,
            )
            .unwrap();
        let meta = Meta {
            icc: None,
            exif: Some(exif_with_gps()),
            xmp: Some(b"<x:xmpmeta>Someone</x:xmpmeta>".to_vec()),
        };
        let src = crate::image::webp_with_meta(&plain, &meta, 6, 4, false).unwrap();
        let out = strip_webp(&src, Strip::All).unwrap();
        assert!(!out.windows(7).any(|w| w == b"Someone"));
        assert!(!out.windows(2).any(|w| w == b"N\0"));
        assert!(out.windows(4).any(|w| w == b"EXIF"));
        assert_eq!(
            image::load_from_memory(&src).unwrap().to_rgba8(),
            image::load_from_memory(&out).unwrap().to_rgba8()
        );
    }

    fn iptc(record: u8, dataset: u8, value: &[u8]) -> Vec<u8> {
        let mut d = vec![0x1C, record, dataset];
        d.extend_from_slice(&(value.len() as u16).to_be_bytes());
        d.extend_from_slice(value);
        d
    }

    fn photoshop_block(id: u16, data: &[u8]) -> Vec<u8> {
        let mut b = b"8BIM".to_vec();
        b.extend_from_slice(&id.to_be_bytes());
        b.extend_from_slice(&[0, 0]); // empty name, padded
        b.extend_from_slice(&(data.len() as u32).to_be_bytes());
        b.extend_from_slice(data);
        if data.len() % 2 == 1 {
            b.push(0);
        }
        b
    }

    /// A JPEG with an APP13 holding a city, a caption, a copyright notice
    /// and the IPTC digest.
    fn jpeg_with_iptc() -> Vec<u8> {
        let mut data = iptc(2, 90, b"Trondheim");
        data.extend(iptc(2, 120, b"Harbour at dawn"));
        data.extend(iptc(2, 116, b"Copyright Someone"));
        data.extend(iptc(2, 101, b"Norway"));
        let mut p = PHOTOSHOP_HEADER.to_vec();
        p.extend(photoshop_block(0x0404, &data));
        p.extend(photoshop_block(0x0425, &[7; 16]));
        let src = jpeg(&exif_with_gps());
        let mut out = src[..2].to_vec();
        out.extend(jpeg_segment(0xED, &p).unwrap());
        out.extend_from_slice(&src[2..]);
        out
    }

    fn has(buf: &[u8], s: &[u8]) -> bool {
        buf.windows(s.len()).any(|w| w == s)
    }

    #[test]
    fn jpeg_strip_location_keeps_iptc_but_not_the_place() {
        let src = jpeg_with_iptc();
        let out = strip_jpeg(&src, Strip::Location).unwrap();
        assert!(has(&out, b"Harbour at dawn"), "caption kept");
        assert!(has(&out, b"Copyright Someone"), "copyright kept");
        assert!(!has(&out, b"Trondheim"), "city removed");
        assert!(!has(&out, b"Norway"), "country removed");
        assert!(!has(&out, &[7; 16]), "stale IPTC digest dropped");
        assert_eq!(
            image::load_from_memory(&src).unwrap().to_rgb8(),
            image::load_from_memory(&out).unwrap().to_rgb8()
        );

        let all = strip_jpeg(&src, Strip::All).unwrap();
        assert!(!has(&all, b"Harbour at dawn"));
        assert!(!has(&all, b"Photoshop 3.0"));
    }

    #[test]
    fn iptc_extended_and_padded_datasets_parse() {
        let mut data = iptc(2, 120, b"caption");
        data.extend_from_slice(&[0x1C, 2, 92, 0x80, 2, 0, 3]);
        data.extend_from_slice(b"Old");
        data.extend_from_slice(&[0, 0, 0]);
        let out = iptc_without_location(&data).unwrap();
        assert!(has(&out, b"caption"));
        assert!(!has(&out, b"Old"));
        assert!(iptc_without_location(b"\x1C\x02").is_none(), "truncated");
    }

    #[test]
    fn mpf_after_fill_bytes_is_patched_in_place() {
        // MPF TIFF (LE) with one MP Entry list of two images.
        let mut m = b"MPF\0II\x2a\0\x08\0\0\0".to_vec();
        m.extend_from_slice(&1u16.to_le_bytes());
        m.extend_from_slice(&0xB002u16.to_le_bytes());
        m.extend_from_slice(&7u16.to_le_bytes());
        m.extend_from_slice(&32u32.to_le_bytes());
        m.extend_from_slice(&26u32.to_le_bytes());
        m.extend_from_slice(&0u32.to_le_bytes());
        m.resize(4 + 26, 0);
        for off in [0u32, 5000] {
            m.extend_from_slice(&[0; 8]);
            m.extend_from_slice(&off.to_le_bytes());
            m.extend_from_slice(&[0; 4]);
        }
        let src = jpeg(&exif_with_gps());
        let mut data = src[..2].to_vec();
        data.push(0xFF); // a fill byte before the marker
        data.extend(jpeg_segment(0xE2, &m).unwrap());
        let comment = jpeg_segment(0xFE, b"to be removed").unwrap();
        data.extend_from_slice(&comment);
        data.extend_from_slice(&src[2..]);

        let out = strip_jpeg(&data, Strip::All).expect("MPF found and patched");
        let hdr = out.windows(4).position(|w| w == b"MPF\0").unwrap() + 4;
        assert_eq!(&out[hdr..hdr + 2], b"II");
        let second = hdr + 26 + 16 + 8;
        let removed = (data.len() - out.len()) as u32;
        assert!(removed >= comment.len() as u32);
        assert_eq!(&out[second..second + 4], &(5000 - removed).to_le_bytes());
    }

    #[test]
    fn mpf_offsets_follow_a_segment_that_grows() {
        // An APP13 whose odd-sized IPTC block lacks its pad byte gets one
        // when rewritten: the output is longer after the MPF header.
        let mut m = b"MPF\0II\x2a\0\x08\0\0\0".to_vec();
        m.extend_from_slice(&1u16.to_le_bytes());
        m.extend_from_slice(&0xB002u16.to_le_bytes());
        m.extend_from_slice(&7u16.to_le_bytes());
        m.extend_from_slice(&32u32.to_le_bytes());
        m.extend_from_slice(&26u32.to_le_bytes());
        m.extend_from_slice(&0u32.to_le_bytes());
        m.resize(4 + 26, 0);
        for off in [0u32, 5000] {
            m.extend_from_slice(&[0; 8]);
            m.extend_from_slice(&off.to_le_bytes());
            m.extend_from_slice(&[0; 4]);
        }
        let caption = iptc(2, 120, b"Hi"); // 7 bytes
        let mut p = PHOTOSHOP_HEADER.to_vec();
        p.extend_from_slice(b"8BIM\x04\x04\0\0");
        p.extend_from_slice(&(caption.len() as u32).to_be_bytes());
        p.extend_from_slice(&caption); // no pad byte
        let src = jpeg(&[]);
        let mut data = src[..2].to_vec();
        data.extend(jpeg_segment(0xE2, &m).unwrap());
        data.extend(jpeg_segment(0xED, &p).unwrap());
        data.extend_from_slice(&src[2..]);

        let out = strip_jpeg(&data, Strip::Location).expect("rewritten");
        assert!(has(&out, b"Hi"), "caption kept");
        let hdr = out.windows(4).position(|w| w == b"MPF\0").unwrap() + 4;
        let second = hdr + 26 + 16 + 8;
        let shift = out.len() as i64 - data.len() as i64;
        assert_eq!(
            &out[second..second + 4],
            &((5000 + shift) as u32).to_le_bytes()
        );
    }

    #[test]
    fn mpf_offsets_follow_removed_bytes() {
        // MPF TIFF (LE) with one MP Entry list of two images.
        let mut m = b"II\x2a\0\x08\0\0\0".to_vec();
        m.extend_from_slice(&1u16.to_le_bytes());
        m.extend_from_slice(&0xB002u16.to_le_bytes());
        m.extend_from_slice(&7u16.to_le_bytes());
        m.extend_from_slice(&32u32.to_le_bytes());
        m.extend_from_slice(&26u32.to_le_bytes());
        m.extend_from_slice(&0u32.to_le_bytes());
        m.resize(26, 0);
        for off in [0u32, 1000] {
            m.extend_from_slice(&[0; 8]);
            m.extend_from_slice(&off.to_le_bytes());
            m.extend_from_slice(&[0; 4]);
        }
        let mut buf = vec![0u8; 8];
        buf.extend_from_slice(&m);
        patch_mpf_offsets(&mut buf, 8, -40).unwrap();
        let second = 8 + 26 + 16 + 8;
        assert_eq!(&buf[second..second + 4], &960u32.to_le_bytes());
        let first = 8 + 26 + 8;
        assert_eq!(&buf[first..first + 4], &0u32.to_le_bytes());
    }
}
