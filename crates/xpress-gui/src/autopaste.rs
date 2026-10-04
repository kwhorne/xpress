//! Pasting into the app you were using: after xpress steps aside, press ⌘V
//! for you. macOS only lets apps send keystrokes to other apps with the
//! Accessibility permission (System Settings → Privacy & Security →
//! Accessibility), so this checks for it and can ask.

use std::time::Duration;

/// Time for the previous app to come back to the front before ⌘V.
pub const DELAY: Duration = Duration::from_millis(200);

/// Where the permission is granted.
pub const SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::c_void;

    use objc2_foundation::{NSDictionary, NSNumber, NSString};

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
        fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceCreate(state: i32) -> *mut c_void;
        fn CGEventCreateKeyboardEvent(source: *mut c_void, key: u16, down: bool) -> *mut c_void;
        fn CGEventSetFlags(event: *mut c_void, flags: u64);
        fn CGEventPost(tap: u32, event: *mut c_void);
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(cf: *const c_void);
    }

    const COMBINED_SESSION_STATE: i32 = 0;
    const ANNOTATED_SESSION_TAP: u32 = 2;
    const COMMAND: u64 = 1 << 20;
    /// The V key (by position, as macOS' own ⌘V does on QWERTY-style layouts).
    const KEY_V: u16 = 9;

    pub fn allowed() -> bool {
        unsafe { AXIsProcessTrusted() }
    }

    /// Show the system prompt that leads to the Accessibility settings.
    pub fn ask() -> bool {
        // NSDictionary is toll-free bridged with CFDictionary.
        let key = NSString::from_str("AXTrustedCheckOptionPrompt");
        let yes = NSNumber::numberWithBool(true);
        let options = NSDictionary::from_slices(&[&*key], &[&*yes]);
        unsafe { AXIsProcessTrustedWithOptions(objc2::rc::Retained::as_ptr(&options).cast()) }
    }

    pub fn press_cmd_v() {
        unsafe {
            let source = CGEventSourceCreate(COMBINED_SESSION_STATE);
            for down in [true, false] {
                let event = CGEventCreateKeyboardEvent(source, KEY_V, down);
                if event.is_null() {
                    continue;
                }
                CGEventSetFlags(event, COMMAND);
                CGEventPost(ANNOTATED_SESSION_TAP, event);
                CFRelease(event);
            }
            if !source.is_null() {
                CFRelease(source);
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    pub fn allowed() -> bool {
        false
    }
    pub fn ask() -> bool {
        false
    }
    pub fn press_cmd_v() {}
}

pub use imp::{allowed, ask};

/// Whether pasting for you is possible on this platform at all.
pub fn supported() -> bool {
    cfg!(target_os = "macos")
}

/// Press ⌘V after [`DELAY`], on a background thread.
pub fn paste_soon() {
    std::thread::spawn(|| {
        std::thread::sleep(DELAY);
        imp::press_cmd_v();
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn checking_the_permission_neither_prompts_nor_types() {
        // Only reads the Accessibility state; never asks and never presses keys.
        let allowed = super::allowed();
        assert!(super::supported() || !allowed);
    }
}
