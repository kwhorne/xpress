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
        fn CFDataGetBytePtr(data: *const c_void) -> *const u8;
    }

    #[link(name = "Carbon", kind = "framework")]
    unsafe extern "C" {
        fn TISCopyCurrentKeyboardLayoutInputSource() -> *mut c_void;
        fn TISGetInputSourceProperty(source: *mut c_void, key: *const c_void) -> *const c_void;
        static kTISPropertyUnicodeKeyLayoutData: *const c_void;
        fn LMGetKbdType() -> u8;
        #[allow(clippy::too_many_arguments)]
        fn UCKeyTranslate(
            layout: *const c_void,
            key: u16,
            action: u16,
            modifiers: u32,
            keyboard_type: u32,
            options: u32,
            dead_key_state: *mut u32,
            max_len: usize,
            len: *mut usize,
            chars: *mut u16,
        ) -> i32;
    }

    const COMBINED_SESSION_STATE: i32 = 0;
    const ANNOTATED_SESSION_TAP: u32 = 2;
    const COMMAND: u64 = 1 << 20;
    /// The V key's position on QWERTY, if the layout can't be read.
    const QWERTY_V: u16 = 9;
    const KEY_ACTION_DISPLAY: u16 = 3;
    const NO_DEAD_KEYS: u32 = 1;
    /// ⌘ in UCKeyTranslate's modifier state (`cmdKey >> 8`).
    const COMMAND_STATE: u32 = 1;

    /// The key that types ⌘V with the current keyboard layout — asked with ⌘
    /// held, so layouts such as “Dvorak – QWERTY ⌘” give the QWERTY key and
    /// plain Dvorak its own. Call on the main thread (macOS requires it).
    pub fn v_key() -> u16 {
        unsafe {
            let source = TISCopyCurrentKeyboardLayoutInputSource();
            if source.is_null() {
                return QWERTY_V;
            }
            let data = TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData);
            let found = (!data.is_null()).then(|| {
                let layout = CFDataGetBytePtr(data).cast::<c_void>();
                (0u16..128).find(|&key| {
                    let (mut dead, mut len, mut chars) = (0u32, 0usize, [0u16; 4]);
                    let status = UCKeyTranslate(
                        layout,
                        key,
                        KEY_ACTION_DISPLAY,
                        COMMAND_STATE,
                        LMGetKbdType() as u32,
                        NO_DEAD_KEYS,
                        &mut dead,
                        chars.len(),
                        &mut len,
                        chars.as_mut_ptr(),
                    );
                    status == 0 && len == 1 && chars[0] == u16::from(b'v')
                })
            });
            CFRelease(source);
            found.flatten().unwrap_or(QWERTY_V)
        }
    }

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

    pub fn press_cmd_v(key: u16) {
        unsafe {
            let source = CGEventSourceCreate(COMBINED_SESSION_STATE);
            for down in [true, false] {
                let event = CGEventCreateKeyboardEvent(source, key, down);
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
    pub fn v_key() -> u16 {
        9
    }
    pub fn press_cmd_v(_key: u16) {}
}

pub use imp::{allowed, ask, v_key};

/// Whether pasting for you is possible on this platform at all.
pub fn supported() -> bool {
    cfg!(target_os = "macos")
}

/// Press ⌘V after [`DELAY`], on a background thread. Call from the main
/// thread: the key is looked up in the keyboard layout first.
pub fn paste_soon() {
    let key = v_key();
    std::thread::spawn(move || {
        std::thread::sleep(DELAY);
        imp::press_cmd_v(key);
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
