//! Open xpress at login, through macOS' login items (SMAppService, macOS 13+).
//! It only works for the installed app bundle, not a bare binary.

#[cfg(target_os = "macos")]
mod imp {
    use objc2_service_management::{SMAppService, SMAppServiceStatus};

    /// SMAppService arrived in macOS 13; the app still runs on 11.
    fn available() -> bool {
        objc2::runtime::AnyClass::get(c"SMAppService").is_some()
    }

    fn in_app_bundle() -> bool {
        std::env::current_exe().is_ok_and(|exe| {
            exe.ancestors()
                .any(|a| a.extension().is_some_and(|e| e == "app"))
        })
    }

    pub fn supported() -> bool {
        available() && in_app_bundle()
    }

    pub fn enabled() -> bool {
        supported()
            && unsafe { SMAppService::mainAppService().status() } == SMAppServiceStatus::Enabled
    }

    pub fn set(on: bool) -> Result<(), String> {
        if !supported() {
            return Err("Opening at login needs macOS 13 or later and the installed app.".into());
        }
        let service = unsafe { SMAppService::mainAppService() };
        let result = unsafe {
            if on {
                service.registerAndReturnError()
            } else {
                service.unregisterAndReturnError()
            }
        };
        result.map_err(|e| e.localizedDescription().to_string())
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    pub fn supported() -> bool {
        false
    }
    pub fn enabled() -> bool {
        false
    }
    pub fn set(_on: bool) -> Result<(), String> {
        Err("Opening at login isn't available here.".into())
    }
}

pub use imp::{enabled, set, supported};

#[cfg(test)]
mod tests {
    #[test]
    fn a_test_binary_is_not_the_installed_app() {
        // Read-only: never registers anything.
        assert!(!super::supported());
        assert!(!super::enabled());
        assert!(super::set(true).is_err());
    }
}
