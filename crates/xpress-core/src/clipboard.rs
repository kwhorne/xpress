//! Putting things on the system clipboard: optimised images ("copy large,
//! paste small"), and clips from the history — text, images, files, or
//! several of them at once. macOS writes through NSPasteboard; other
//! platforms are best-effort no-ops here (the desktop app has its own
//! fallback).

use std::path::{Path, PathBuf};

/// Put the PNG at `path` onto the clipboard. Returns true on success.
pub fn set_clipboard_png(path: &Path) -> bool {
    match std::fs::read(path) {
        Ok(bytes) => write(&[Part::Png(bytes)]),
        Err(_) => false,
    }
}

/// A pasteboard type xpress adds to everything it puts on the clipboard, so
/// the app's history doesn't record copies made by xpress itself (from the
/// app, `xpress history copy` or `xpress watch --clipboard`).
pub const XPRESS_MARKER: &str = "com.kwhorne.xpress.copy";

/// One piece of what goes on the clipboard.
#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    Text(String),
    Png(Vec<u8>),
    File(PathBuf),
}

/// How the parts go onto the clipboard together.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Composed {
    /// All parts are files: put them on as files (Finder pastes them all).
    pub files: Vec<PathBuf>,
    /// Every text part (and file path), for plain-text fields.
    pub text: String,
    /// Several parts: text and images in order, for rich editors (Mail,
    /// Notes, Pages, documents in the browser).
    pub html: Option<String>,
    /// The first image, for apps that only take a picture.
    pub png: Option<Vec<u8>>,
}

pub fn compose(parts: &[Part]) -> Composed {
    if !parts.is_empty() && parts.iter().all(|p| matches!(p, Part::File(_))) {
        return Composed {
            files: parts
                .iter()
                .filter_map(|p| match p {
                    Part::File(f) => Some(f.clone()),
                    _ => None,
                })
                .collect(),
            ..Default::default()
        };
    }
    let texts: Vec<String> = parts
        .iter()
        .filter_map(|p| match p {
            Part::Text(t) => Some(t.trim_end().to_string()),
            Part::File(f) => Some(f.display().to_string()),
            Part::Png(_) => None,
        })
        .collect();
    let separator = if texts.iter().any(|t| t.contains('\n')) {
        "\n\n"
    } else {
        "\n"
    };
    let png = parts.iter().find_map(|p| match p {
        Part::Png(png) => Some(png.clone()),
        _ => None,
    });
    let html = (parts.len() > 1).then(|| {
        parts
            .iter()
            .map(|p| match p {
                Part::Text(t) => {
                    let t = t.trim();
                    if crate::history::classify_text(t) == crate::history::ClipKind::Link {
                        format!("<p><a href=\"{0}\">{0}</a></p>", html_escape(t))
                    } else {
                        format!("<p>{}</p>", html_escape(t).replace('\n', "<br>"))
                    }
                }
                Part::File(f) => format!("<p>{}</p>", html_escape(&f.display().to_string())),
                Part::Png(png) => {
                    format!("<p><img src=\"data:image/png;base64,{}\"></p>", base64(png))
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    });
    Composed {
        files: Vec::new(),
        text: texts.join(separator),
        html,
        png,
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (*b as u32) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Put `parts` on the clipboard. Returns true on success.
#[cfg(target_os = "macos")]
pub fn write(parts: &[Part]) -> bool {
    write_to(&objc2_app_kit::NSPasteboard::generalPasteboard(), parts)
}

#[cfg(not(target_os = "macos"))]
pub fn write(_parts: &[Part]) -> bool {
    false
}

/// [`write`] to a given pasteboard (tests use a private one).
#[cfg(target_os = "macos")]
pub fn write_to(pb: &objc2_app_kit::NSPasteboard, parts: &[Part]) -> bool {
    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2_app_kit::{
        NSPasteboardItem, NSPasteboardTypeFileURL, NSPasteboardTypeHTML, NSPasteboardTypePNG,
        NSPasteboardTypeString, NSPasteboardWriting,
    };
    use objc2_foundation::{NSArray, NSData, NSString, NSURL};

    let composed = compose(parts);
    let marker = NSString::from_str(XPRESS_MARKER);
    pb.clearContents();
    if !composed.files.is_empty() {
        let items: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = composed
            .files
            .iter()
            .filter_map(|p| {
                let url = NSURL::fileURLWithPath(&NSString::from_str(&p.to_string_lossy()));
                let link = url.absoluteString()?;
                let item = NSPasteboardItem::new();
                item.setString_forType(&NSString::from_str(""), &marker);
                item.setString_forType(&link, unsafe { NSPasteboardTypeFileURL })
                    .then(|| ProtocolObject::from_retained(item))
            })
            .collect();
        return !items.is_empty() && pb.writeObjects(&NSArray::from_retained_slice(&items));
    }
    let mut ok = false;
    pb.setString_forType(&NSString::from_str(""), &marker);
    if !composed.text.is_empty() {
        ok |= pb.setString_forType(&NSString::from_str(&composed.text), unsafe {
            NSPasteboardTypeString
        });
    }
    if let Some(html) = &composed.html {
        ok |= pb.setString_forType(&NSString::from_str(html), unsafe { NSPasteboardTypeHTML });
    }
    if let Some(png) = &composed.png {
        ok |= pb.setData_forType(Some(&NSData::with_bytes(png)), unsafe {
            NSPasteboardTypePNG
        });
    }
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composing_parts() {
        let files = compose(&[Part::File("/a.pdf".into()), Part::File("/b.txt".into())]);
        assert_eq!(files.files.len(), 2);
        assert!(files.text.is_empty() && files.html.is_none());

        let lines = compose(&[Part::Text("one".into()), Part::Text("two".into())]);
        assert_eq!(lines.text, "one\ntwo");
        assert_eq!(lines.html.as_deref(), Some("<p>one</p>\n<p>two</p>"));

        let blocks = compose(&[
            Part::Text("a\nb".into()),
            Part::Text("<c & d>".into()),
            Part::File("/x y.txt".into()),
        ]);
        assert_eq!(blocks.text, "a\nb\n\n<c & d>\n\n/x y.txt");
        let html = blocks.html.unwrap();
        assert!(html.contains("<p>a<br>b</p>"));
        assert!(html.contains("&lt;c &amp; d&gt;"));

        let single = compose(&[Part::Text("just this".into())]);
        assert_eq!(single.text, "just this");
        assert!(single.html.is_none());
        let image = compose(&[Part::Png(vec![1, 2, 3])]);
        assert_eq!(image.png, Some(vec![1, 2, 3]));
        assert!(image.html.is_none() && image.text.is_empty());
    }

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe, 0x00]), "//4A");
    }
}
