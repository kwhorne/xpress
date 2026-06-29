//! Write an optimised image back onto the system clipboard so "paste" yields the
//! smaller file. macOS writes the PNG bytes as `public.png` via NSPasteboard;
//! other platforms are best-effort no-ops for now.

use std::path::Path;

/// Put the PNG at `path` onto the clipboard. Returns true on success.
pub fn set_clipboard_png(path: &Path) -> bool {
    match std::fs::read(path) {
        Ok(bytes) => set_png_bytes(&bytes),
        Err(_) => false,
    }
}

#[cfg(target_os = "macos")]
fn set_png_bytes(bytes: &[u8]) -> bool {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypePNG};
    use objc2_foundation::NSData;
    unsafe {
        let pb = NSPasteboard::generalPasteboard();
        pb.clearContents();
        let data = NSData::with_bytes(bytes);
        pb.setData_forType(Some(&data), NSPasteboardTypePNG)
    }
}

#[cfg(not(target_os = "macos"))]
fn set_png_bytes(_bytes: &[u8]) -> bool {
    false
}
