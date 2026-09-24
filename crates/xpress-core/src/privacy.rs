//! Location removal: strip where a photo was taken while keeping the rest of
//! its metadata (camera, date, orientation, colour profile).
//!
//! EXIF: the GPS sub-IFD is wiped (its entries *and* the out-of-line values
//! they point to are zeroed, so no coordinates linger in the bytes) and the
//! pointer to it is removed from IFD0. XMP: `exif:GPS*`, `photoshop:City`,
//! `State`, `Country` and `Iptc4xmpCore:Location` are removed.

/// EXIF tag of the pointer to the GPS IFD.
const GPS_IFD_POINTER: u16 = 0x8825;

struct Tiff<'a> {
    data: &'a mut [u8],
    le: bool,
}

impl Tiff<'_> {
    fn u16(&self, at: usize) -> Option<u16> {
        let b: [u8; 2] = self.data.get(at..at + 2)?.try_into().ok()?;
        Some(if self.le {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        })
    }

    fn u32(&self, at: usize) -> Option<u32> {
        let b: [u8; 4] = self.data.get(at..at + 4)?.try_into().ok()?;
        Some(if self.le {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        })
    }

    fn zero(&mut self, from: usize, len: usize) {
        if let Some(s) = self.data.get_mut(from..from + len) {
            s.fill(0);
        }
    }
}

/// Bytes per value for a TIFF field type.
fn type_size(t: u16) -> usize {
    match t {
        1 | 2 | 6 | 7 => 1,
        3 | 8 => 2,
        4 | 9 | 11 => 4,
        5 | 10 | 12 => 8,
        _ => 1,
    }
}

/// Remove GPS data from a raw EXIF (TIFF) block in place. Returns whether any
/// was found. Malformed blocks are left as they are.
pub fn strip_gps(exif: &mut [u8]) -> bool {
    let le = match exif.get(0..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return false,
    };
    let mut t = Tiff { data: exif, le };
    let Some(ifd0) = t.u32(4).map(|o| o as usize) else {
        return false;
    };
    let Some(n) = t.u16(ifd0).map(usize::from) else {
        return false;
    };
    let entry = |i: usize| ifd0 + 2 + i * 12;
    let Some(idx) = (0..n).find(|&i| t.u16(entry(i)) == Some(GPS_IFD_POINTER)) else {
        return false;
    };
    let Some(gps) = t.u32(entry(idx) + 8).map(|o| o as usize) else {
        return false;
    };

    // Wipe the GPS IFD: out-of-line values first, then the directory itself.
    if let Some(gn) = t.u16(gps).map(usize::from) {
        for i in 0..gn {
            let e = gps + 2 + i * 12;
            let (Some(ty), Some(count), Some(off)) = (t.u16(e + 2), t.u32(e + 4), t.u32(e + 8))
            else {
                break;
            };
            let len = type_size(ty).saturating_mul(count as usize);
            if len > 4 {
                t.zero(off as usize, len);
            }
        }
        t.zero(gps, 2 + gn * 12 + 4);
    }

    // Drop the pointer entry from IFD0: shift the later entries (and the
    // next-IFD offset that follows them) up by one slot.
    let tail_start = entry(idx + 1);
    let tail_end = entry(n) + 4;
    if tail_end <= t.data.len() {
        t.data.copy_within(tail_start..tail_end, entry(idx));
        t.zero(tail_end - 12, 12);
        let new_n = (n - 1) as u16;
        let b = if le {
            new_n.to_le_bytes()
        } else {
            new_n.to_be_bytes()
        };
        t.data[ifd0..ifd0 + 2].copy_from_slice(&b);
    }
    true
}

/// Remove location properties from an XMP packet (attribute and element forms).
pub fn strip_xmp_location(xmp: &[u8]) -> Vec<u8> {
    use std::sync::OnceLock;
    static RE: OnceLock<[regex::bytes::Regex; 2]> = OnceLock::new();
    let names = r"(?:exif:GPS[A-Za-z]*|photoshop:(?:City|State|Country)|Iptc4xmpCore:Location)";
    let [attr, elem] = RE.get_or_init(|| {
        [
            regex::bytes::Regex::new(&format!(r#"\s+{names}\s*=\s*("[^"]*"|'[^']*')"#)).unwrap(),
            // (No backreferences in `regex`; these elements never nest.)
            regex::bytes::Regex::new(&format!(r"(?s)<{names}\b[^>]*?(?:/>|>.*?</{names}\s*>)"))
                .unwrap(),
        ]
    });
    let without_attrs = attr.replace_all(xmp, &b""[..]);
    elem.replace_all(&without_attrs, &b""[..]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A little-endian EXIF block: IFD0 {Make, GPS pointer, Orientation},
    /// GPS IFD {GPSLatitude (3 rationals, out of line)}.
    fn exif_with_gps() -> Vec<u8> {
        let mut v = b"II*\0".to_vec();
        v.extend(8u32.to_le_bytes());
        // IFD0 at 8: 3 entries (sorted by tag) + next offset = 2 + 36 + 4 = 42 -> ends at 50.
        v.extend(3u16.to_le_bytes());
        // Make (ASCII, 5) -> at 50
        v.extend(0x010Fu16.to_le_bytes());
        v.extend(2u16.to_le_bytes());
        v.extend(5u32.to_le_bytes());
        v.extend(50u32.to_le_bytes());
        // Orientation = 6
        v.extend(0x0112u16.to_le_bytes());
        v.extend(3u16.to_le_bytes());
        v.extend(1u32.to_le_bytes());
        v.extend(6u32.to_le_bytes());
        // GPS IFD pointer -> 56
        v.extend(0x8825u16.to_le_bytes());
        v.extend(4u16.to_le_bytes());
        v.extend(1u32.to_le_bytes());
        v.extend(56u32.to_le_bytes());
        v.extend(0u32.to_le_bytes());
        v.extend(b"Test\0\0"); // 50..56 (padded)
                               // GPS IFD at 56: 1 entry + next = 18 -> values at 74
        v.extend(1u16.to_le_bytes());
        v.extend(0x0002u16.to_le_bytes()); // GPSLatitude
        v.extend(5u16.to_le_bytes()); // RATIONAL
        v.extend(3u32.to_le_bytes());
        v.extend(74u32.to_le_bytes());
        v.extend(0u32.to_le_bytes());
        for n in [59u32, 1, 54, 1, 3012, 100] {
            v.extend(n.to_le_bytes()); // 59° 54' 30.12" — Oslo
        }
        v
    }

    #[test]
    fn removes_gps_and_keeps_the_rest() {
        let mut exif = exif_with_gps();
        assert!(strip_gps(&mut exif));
        let t = Tiff {
            data: &mut exif.clone(),
            le: true,
        };
        assert_eq!(t.u16(8), Some(2), "one entry fewer");
        let tags: Vec<u16> = (0..2).map(|i| t.u16(10 + i * 12).unwrap()).collect();
        assert_eq!(tags, vec![0x010F, 0x0112], "Make and Orientation kept");
        assert_eq!(t.u32(10 + 24), Some(0), "next-IFD offset moved up");
        assert!(exif.windows(4).any(|w| w == b"Test"), "Make value intact");
        let lat = 3012u32.to_le_bytes();
        assert!(!exif.windows(4).any(|w| w == lat), "coordinates wiped");
        assert_eq!(
            image::metadata::Orientation::from_exif_chunk(&exif),
            image::metadata::Orientation::from_exif(6)
        );
    }

    #[test]
    fn no_gps_no_change() {
        let mut exif = exif_with_gps();
        strip_gps(&mut exif);
        let once = exif.clone();
        assert!(!strip_gps(&mut exif));
        assert_eq!(exif, once);
        assert!(!strip_gps(&mut [0u8; 3]), "malformed input is ignored");
    }

    #[test]
    fn strips_xmp_location_attributes_and_elements() {
        let xmp = br#"<rdf:Description xmp:Rating="5" exif:GPSLatitude="59,54.5N" exif:GPSLongitude='10,45E' photoshop:City="Oslo"><photoshop:Country>Norway</photoshop:Country><dc:title>Fjord</dc:title><Iptc4xmpCore:Location/></rdf:Description>"#;
        let out = String::from_utf8(strip_xmp_location(xmp)).unwrap();
        assert!(out.contains(r#"xmp:Rating="5""#));
        assert!(out.contains("<dc:title>Fjord</dc:title>"));
        for gone in [
            "GPSLatitude",
            "GPSLongitude",
            "Oslo",
            "Norway",
            "Iptc4xmpCore",
        ] {
            assert!(!out.contains(gone), "{gone} still in {out}");
        }
    }
}
