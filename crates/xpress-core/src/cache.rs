//! "Already optimised" markers, so re-running over a folder is instant and
//! never re-encodes a file that xpress has already squeezed (which would only
//! add generation loss).
//!
//! After a plain optimise, the resulting file gets an extended attribute
//! recording the settings used and a CRC32 fingerprint of its content. Next
//! time, a file whose content still matches and that was optimised at least as
//! hard (same or higher compression, metadata stripped if asked, PDF DPI at or
//! below the requested one) is skipped. Any edit changes the fingerprint and
//! invalidates the marker. Filesystems without xattr support simply get no
//! cache.

use std::io::Read;
use std::path::Path;

use crate::result::OptimiseOptions;

/// Attribute name: Linux only allows user attributes in the `user.` namespace.
#[cfg(unix)]
const ATTR: &str = if cfg!(target_os = "linux") {
    "user.xpress.optimised"
} else {
    "com.xpress.optimised"
};

/// What a marker records about the optimisation that produced a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Mark {
    factor: i32,
    strip: bool,
    /// Location removed (implied by `strip`).
    location: bool,
    /// PDF downsampling DPI (0 = none).
    dpi: i32,
    len: u64,
    crc: u32,
}

impl Mark {
    fn encode(&self) -> String {
        format!(
            "v1 f={} s={} d={} len={} crc={:08x} l={}",
            self.factor, self.strip as u8, self.dpi, self.len, self.crc, self.location as u8
        )
    }

    fn decode(s: &str) -> Option<Mark> {
        let mut it = s.split_whitespace();
        if it.next()? != "v1" {
            return None;
        }
        let mut field = |key: &str| it.next()?.strip_prefix(key).map(str::to_owned);
        let factor = field("f=")?.parse().ok()?;
        let strip = field("s=")? == "1";
        let dpi = field("d=")?.parse().ok()?;
        let len = field("len=")?.parse().ok()?;
        let crc = u32::from_str_radix(&field("crc=")?, 16).ok()?;
        // Added later: older markers have no `l=` field.
        let location = field("l=").is_some_and(|v| v == "1");
        Some(Mark {
            factor,
            strip,
            location,
            dpi,
            len,
            crc,
        })
    }

    /// Whether a file optimised with this mark needs nothing more for a request.
    fn covers(&self, factor: i32, strip: bool, location: bool, dpi: Option<i32>) -> bool {
        let dpi_ok = match dpi {
            None => true,
            Some(want) => self.dpi != 0 && self.dpi <= want,
        };
        let location_ok = self.strip || self.location || !location;
        self.factor >= factor && (self.strip || !strip) && location_ok && dpi_ok
    }
}

/// CRC32 of the file's content, streamed.
fn fingerprint(path: &Path) -> std::io::Result<(u64, u32)> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = crc32fast::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut len = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        len += n as u64;
    }
    Ok((len, hasher.finalize()))
}

#[cfg(unix)]
fn read_mark(path: &Path) -> Option<Mark> {
    let raw = xattr::get(path, ATTR).ok()??;
    Mark::decode(std::str::from_utf8(&raw).ok()?)
}

#[cfg(not(unix))]
fn read_mark(_path: &Path) -> Option<Mark> {
    None
}

/// Whether `path` is already optimised well enough for these options (its
/// marker matches its current content and settings).
pub fn is_optimised(path: &Path, options: &OptimiseOptions, pdf_dpi: Option<i32>) -> bool {
    let Some(mark) = read_mark(path) else {
        return false;
    };
    if !mark.covers(
        options.compression.factor,
        options.strip_metadata,
        options.strip_location,
        pdf_dpi,
    ) {
        return false;
    }
    // Cheap size check first; only hash when it could still match.
    if std::fs::metadata(path).map(|m| m.len()).ok() != Some(mark.len) {
        return false;
    }
    fingerprint(path).is_ok_and(|(len, crc)| len == mark.len && crc == mark.crc)
}

/// Record that `path` has been optimised with these options. Best-effort.
pub fn mark(path: &Path, options: &OptimiseOptions, pdf_dpi: Option<i32>) {
    #[cfg(unix)]
    if let Ok((len, crc)) = fingerprint(path) {
        let m = Mark {
            factor: options.compression.factor,
            strip: options.strip_metadata,
            location: options.strip_location,
            dpi: pdf_dpi.unwrap_or(0),
            len,
            crc,
        };
        let _ = xattr::set(path, ATTR, m.encode().as_bytes());
    }
    #[cfg(not(unix))]
    let _ = (path, options, pdf_dpi);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_roundtrip() {
        let m = Mark {
            factor: 64,
            strip: true,
            location: false,
            dpi: 150,
            len: 12345,
            crc: 0xdead_beef,
        };
        assert_eq!(Mark::decode(&m.encode()), Some(m));
        assert_eq!(Mark::decode("v0 junk"), None);
    }

    #[test]
    fn covers_only_equal_or_gentler_requests() {
        let m = Mark {
            factor: 50,
            strip: false,
            location: false,
            dpi: 0,
            len: 1,
            crc: 1,
        };
        assert!(
            m.covers(30, false, false, None),
            "already compressed harder"
        );
        assert!(m.covers(50, false, false, None));
        assert!(
            !m.covers(64, false, false, None),
            "asked for more compression"
        );
        assert!(!m.covers(30, true, false, None), "asked to strip metadata");
        assert!(!m.covers(30, false, true, None), "asked to remove location");
        let loc = Mark {
            location: true,
            ..m
        };
        assert!(loc.covers(30, false, true, None));
        assert!(
            !loc.covers(30, true, false, None),
            "location only isn't everything"
        );
        assert_eq!(
            Mark::decode("v1 f=30 s=0 d=0 len=1 crc=00000001").map(|m| m.location),
            Some(false),
            "older markers without l= still parse"
        );
        assert!(
            !m.covers(30, false, false, Some(150)),
            "asked to downsample"
        );
        let d = Mark { dpi: 144, ..m };
        assert!(d.covers(30, false, false, Some(150)));
        assert!(!d.covers(30, false, false, Some(96)));
    }
}
