//! "Optimise with xpress" in Finder: a macOS service (Finder → right-click →
//! Quick Actions / Services). The app's Info.plist declares it (NSServices,
//! see scripts/make-app.sh); this provides the object that receives the files
//! and hands them to the app, as if they had been dropped on the window.

use std::path::PathBuf;
use std::sync::mpsc::Sender;

use eframe::egui;

use crate::work::Msg;

/// Keeps the provider alive (macOS only holds a weak reference).
pub struct Services {
    #[cfg(target_os = "macos")]
    _provider: objc2::rc::Retained<imp::Provider>,
}

/// Register the service handler. Call on the main thread, at launch.
#[cfg(target_os = "macos")]
pub fn install(tx: Sender<Msg>, ctx: egui::Context) -> Option<Services> {
    use objc2_foundation::MainThreadMarker;
    let mtm = MainThreadMarker::new()?;
    let provider = imp::Provider::new(mtm, tx, ctx);
    let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
    unsafe { app.setServicesProvider(Some(&provider)) };
    objc2_app_kit::NSUpdateDynamicServices();
    Some(Services {
        _provider: provider,
    })
}

#[cfg(not(target_os = "macos"))]
pub fn install(_tx: Sender<Msg>, _ctx: egui::Context) -> Option<Services> {
    None
}

/// The files a service request carries.
#[cfg(target_os = "macos")]
pub fn files_on(pboard: &objc2_app_kit::NSPasteboard) -> Vec<PathBuf> {
    use objc2_app_kit::NSPasteboardTypeFileURL;
    use objc2_foundation::NSURL;
    pboard
        .pasteboardItems()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.stringForType(unsafe { NSPasteboardTypeFileURL }))
                .filter_map(|s| NSURL::URLWithString(&s))
                .filter_map(|url| url.path())
                .map(|p| PathBuf::from(p.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use objc2::rc::Retained;
    use objc2::runtime::NSObject;
    use objc2::{define_class, msg_send, DefinedClass, MainThreadOnly};
    use objc2_app_kit::NSPasteboard;
    use objc2_foundation::{MainThreadMarker, NSString};

    pub struct Ivars {
        tx: Sender<Msg>,
        ctx: egui::Context,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "XpressServicesProvider"]
        #[ivars = Ivars]
        pub struct Provider;

        impl Provider {
            /// NSMessage "optimiseFiles" in Info.plist.
            #[unsafe(method(optimiseFiles:userData:error:))]
            fn optimise_files(
                &self,
                pboard: &NSPasteboard,
                _user_data: Option<&NSString>,
                _error: *mut *mut NSString,
            ) {
                let files = files_on(pboard);
                if !files.is_empty() {
                    let _ = self.ivars().tx.send(Msg::OpenFiles(files));
                    self.ivars().ctx.request_repaint();
                }
            }
        }
    );

    impl Provider {
        pub fn new(mtm: MainThreadMarker, tx: Sender<Msg>, ctx: egui::Context) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(Ivars { tx, ctx });
            unsafe { msg_send![super(this), init] }
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use objc2_app_kit::NSPasteboard;

    #[test]
    fn the_files_of_a_service_request() {
        // A private pasteboard, like the one macOS hands a service.
        let pb = NSPasteboard::pasteboardWithUniqueName();
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a photo.png");
        let b = dir.path().join("b.mov");
        std::fs::write(&a, b"x").unwrap();
        std::fs::write(&b, b"y").unwrap();
        xpress_core::clipboard::write_to(
            &pb,
            &[
                xpress_core::clipboard::Part::File(a.clone()),
                xpress_core::clipboard::Part::File(b.clone()),
            ],
        );
        let canon = |p: &PathBuf| std::fs::canonicalize(p).unwrap();
        assert_eq!(
            files_on(&pb).iter().map(canon).collect::<Vec<_>>(),
            [canon(&a), canon(&b)]
        );
        unsafe {
            let _: () = objc2::msg_send![&*pb, releaseGlobally];
        }
    }

    #[test]
    fn the_provider_answers_the_service_message() {
        use objc2::runtime::Sel;
        use objc2::ClassType;
        // Registers the class (no main thread needed for that).
        let class = imp::Provider::class();
        assert_eq!(class.name().to_str().unwrap(), "XpressServicesProvider");
        // The NSMessage in Info.plist, with its two arguments.
        let method = class
            .instance_method(Sel::register(c"optimiseFiles:userData:error:"))
            .expect("the service method");
        assert_eq!(
            method.arguments_count(),
            5,
            "self, _cmd, pboard, userData, error"
        );
        let plist = include_str!("../../../scripts/make-app.sh");
        assert!(plist.contains("<key>NSMessage</key>        <string>optimiseFiles</string>"));
    }
}
