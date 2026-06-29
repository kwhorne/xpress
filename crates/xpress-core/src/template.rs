//! Output filename templates.
//!
//! Tokens (case-sensitive), expanded against the source path and the clock:
//!
//! | Token | Meaning                         |
//! |-------|---------------------------------|
//! | `%f`  | source file name (no extension) |
//! | `%e`  | source file extension           |
//! | `%P`  | source parent directory path    |
//! | `%y`  | year (4 digits)                 |
//! | `%m`  | month (2 digits)                |
//! | `%d`  | day (2 digits)                  |
//! | `%H`  | hour (24h, 2 digits)            |
//! | `%M`  | minute (2 digits)               |
//! | `%S`  | second (2 digits)               |
//! | `%i`  | auto-incrementing number        |
//! | `%r`  | 6 random lowercase/digit chars  |
//! | `%%`  | a literal `%`                   |

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Whether a string looks like a template (contains a `%` token).
pub fn is_template(s: &str) -> bool {
    s.contains('%')
}

/// Expand `template` for `source`, using `counter` for `%i` (and incrementing it).
pub fn expand(template: &str, source: &Path, counter: &mut u64) -> PathBuf {
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    let ext = source
        .extension()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let parent = source
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();

    let (y, mo, d, h, mi, s) = local_datetime_parts();

    let mut out = String::with_capacity(template.len() + 16);
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('f') => out.push_str(&stem),
            Some('e') => out.push_str(&ext),
            Some('P') => out.push_str(&parent),
            Some('y') => out.push_str(&format!("{y:04}")),
            Some('m') => out.push_str(&format!("{mo:02}")),
            Some('d') => out.push_str(&format!("{d:02}")),
            Some('H') => out.push_str(&format!("{h:02}")),
            Some('M') => out.push_str(&format!("{mi:02}")),
            Some('S') => out.push_str(&format!("{s:02}")),
            Some('i') => {
                out.push_str(&counter.to_string());
                *counter += 1;
            }
            Some('r') => out.push_str(&random_chars(6)),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }

    let expanded = shellexpand_home(&out);
    PathBuf::from(expanded)
}

fn shellexpand_home(s: &str) -> String {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return format!("{}/{}", home.to_string_lossy(), rest);
        }
    }
    s.to_string()
}

/// Best-effort local date/time. Falls back to UTC-from-epoch math (no chrono dep).
fn local_datetime_parts() -> (i64, u32, u32, u32, u32, u32) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // Civil-from-days algorithm (Howard Hinnant), UTC.
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, mi, s) = (
        (rem / 3600) as u32,
        ((rem % 3600) / 60) as u32,
        (rem % 60) as u32,
    );

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d, h, mi, s)
}

fn random_chars(n: usize) -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    // Seed from the high-resolution clock; sufficient for filename uniqueness.
    let mut seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1)
        | 1;
    let mut s = String::with_capacity(n);
    for _ in 0..n {
        // xorshift
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        s.push(ALPHABET[(seed % ALPHABET.len() as u64) as usize] as char);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_tokens() {
        let mut c = 1;
        let out = expand("%f-thumb.%e", Path::new("/a/b/photo.png"), &mut c);
        assert_eq!(out, PathBuf::from("photo-thumb.png"));
    }

    #[test]
    fn auto_increment_and_literal_percent() {
        let mut c = 5;
        let p = Path::new("img.jpg");
        assert_eq!(expand("n%i", p, &mut c), PathBuf::from("n5"));
        assert_eq!(expand("n%i", p, &mut c), PathBuf::from("n6"));
        assert_eq!(expand("100%%", p, &mut c), PathBuf::from("100%"));
    }
}
