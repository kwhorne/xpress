//! Removing metadata without re-encoding the pixels.
//!
//! JPEG, PNG and WebP are rewritten at the container level in pure Rust: the
//! metadata segments/chunks go, the image data is copied byte for byte. The
//! colour profile always stays (it is colour-critical), and so does the EXIF
//! orientation (as a minimal EXIF block), so a photo never ends up sideways.
//! Other formats (HEIC, AVIF, TIFF, JXL) go through `exiftool` when it is
//! installed; without it they fail loudly instead of keeping GPS silently.

use std::path::Path;

use image::metadata::Orientation;

use crate::filetype::extension_lower;
use crate::image::{Meta, Strip};
use crate::privacy::{strip_gps, strip_xmp_location};
use crate::result::OptimiseError;
use crate::tools::{self, Tool};

const EXIF_HEADER: &[u8] = b"Exif\0\0";
const XMP_HEADER: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";

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

/// HEIC, AVIF, TIFF, JXL, …: `exiftool` on a copy.
fn strip_with_exiftool(
    src: &Path,
    out: &Path,
    strip: Strip,
    ext: &str,
) -> Result<bool, OptimiseError> {
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
            // IPTC (captions, but also city/country): the re-encoding path
            // never carries it either.
            0xED => None,
            // JFIF, ICC profile / MPF, Adobe colour transform: needed to show
            // the image right.
            0xE0 | 0xE2 | 0xEE => Some(segment.to_vec()),
            // Other APPn (extended XMP, maker data, …) and comments.
            0xE1 | 0xE3..=0xEF | 0xFE if all => None,
            _ => Some(segment.to_vec()),
        };
        if marker == 0xE2 && payload.starts_with(b"MPF\0") && mpf.is_none() {
            mpf = Some((pos, out.len()));
        }
        if let Some(k) = kept {
            out.extend_from_slice(&k);
        }
        pos = end;
    }
    if let Some((orig_at, out_at)) = mpf {
        // Bytes removed between the MPF segment and the image data shift the
        // secondary images; their offsets are relative to the MPF header.
        let removed = (data.len() - orig_at) - (out.len() - out_at);
        if removed > 0 {
            patch_mpf_offsets(&mut out, out_at + 8, removed as u32)?;
        }
    }
    Some(out)
}

/// Subtract `shift` from every non-zero image offset in an MPF index whose
/// TIFF header starts at `base`.
fn patch_mpf_offsets(buf: &mut [u8], base: usize, shift: u32) -> Option<()> {
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
                let new = off.checked_sub(shift)?;
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
        patch_mpf_offsets(&mut buf, 8, 40).unwrap();
        let second = 8 + 26 + 16 + 8;
        assert_eq!(&buf[second..second + 4], &960u32.to_le_bytes());
        let first = 8 + 26 + 8;
        assert_eq!(&buf[first..first + 4], &0u32.to_le_bytes());
    }
}
