//! Taking a screenshot (macOS): the system's own area/window picker
//! (`screencapture -i`), which needs the Screen Recording permission.
//!
//! The picture goes to a private temporary file, not the screenshot folder,
//! so the history's screenshot watcher (capture.rs) doesn't record it a
//! second time; the marked-up result is recorded instead.

use std::path::PathBuf;

/// System Settings → Privacy & Security → Screen & System Audio Recording.
pub const SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";

/// Whether this platform can take screenshots.
pub fn supported() -> bool {
    cfg!(target_os = "macos")
}

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

/// Whether xpress may record the screen. Without it, a screenshot shows
/// only the desktop picture and the menu bar.
pub fn allowed() -> bool {
    #[cfg(target_os = "macos")]
    // SAFETY: no arguments; available since macOS 10.15.
    unsafe {
        CGPreflightScreenCaptureAccess()
    }
    #[cfg(not(target_os = "macos"))]
    false
}

/// Ask for the permission. macOS shows its prompt only the first time; after
/// it's turned on in System Settings, xpress must be reopened.
pub fn request() -> bool {
    #[cfg(target_os = "macos")]
    // SAFETY: as above.
    unsafe {
        CGRequestScreenCaptureAccess()
    }
    #[cfg(not(target_os = "macos"))]
    false
}

/// Let the user pick an area or a window and take it. `Ok(None)` when they
/// cancelled (esc).
pub fn capture() -> Result<Option<PathBuf>, String> {
    let file = tempfile::Builder::new()
        .prefix("xpress-shot-")
        .suffix(".png")
        .tempfile()
        .map_err(|e| e.to_string())?
        .into_temp_path()
        .keep()
        .map_err(|e| e.to_string())?;
    let status = std::process::Command::new("/usr/sbin/screencapture")
        // Interactive (area, or space for a window), no sound.
        .args(["-i", "-x"])
        .arg(&file)
        .status();
    let taken = std::fs::metadata(&file).is_ok_and(|m| m.len() > 0);
    match status {
        Ok(_) if taken => Ok(Some(file)),
        Ok(_) => {
            let _ = std::fs::remove_file(&file);
            Ok(None)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&file);
            Err(e.to_string())
        }
    }
}
