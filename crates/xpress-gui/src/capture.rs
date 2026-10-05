//! Recording the clipboard and screenshots into the history, on a background
//! thread. The clipboard is checked twice a second (macOS has no notification
//! for it, only a change counter); the screenshot folder every two seconds.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicIsize, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use xpress_core::history::{History, NewClip, Retention};

use crate::pasteboard::{self, Snapshot, IGNORED_APPS};
use crate::settings::IgnoredApp;

const TICK: Duration = Duration::from_millis(500);
const SCREENSHOT_EVERY: u32 = 4;
const PRUNE_EVERY: Duration = Duration::from_secs(60 * 60);
/// The history never grows past this, however long clips are kept.
pub const MAX_HISTORY_BYTES: u64 = 2_000_000_000;

/// What to record, shared with the UI (which updates it from the settings).
#[derive(Debug, Default)]
pub struct CaptureFlags {
    pub enabled: AtomicBool,
    pub screenshots: AtomicBool,
    pub ocr: AtomicBool,
    /// 0 = keep until the size limit.
    pub keep_days: AtomicU32,
    /// Add everything copied to one multi-clip.
    pub collecting: AtomicBool,
    /// The multi-clip being collected into (0 = start a new one).
    pub collection: AtomicI64,
    /// The pasteboard change xpress made itself (not to be recorded).
    pub own_change: AtomicIsize,
    /// Sync through iCloud Drive.
    pub sync: AtomicBool,
    /// Start the next sync from a fresh snapshot (syncing was just turned on).
    pub sync_fresh: AtomicBool,
    /// Apps the user chose not to record.
    pub ignored: Mutex<Vec<IgnoredApp>>,
}

/// How syncing is going, for Preferences.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SyncStatus {
    /// Unix milliseconds of the last successful round.
    pub last: Option<i64>,
    pub devices: Vec<String>,
    /// Changes waiting for images still on their way.
    pub waiting: usize,
    pub error: Option<String>,
}

const SYNC_EVERY: Duration = Duration::from_secs(10);

/// Sync the history through iCloud Drive while `flags.sync` is on.
pub fn start_sync(
    history: Arc<Mutex<History>>,
    flags: Arc<CaptureFlags>,
    status: Arc<Mutex<SyncStatus>>,
    changed: impl Fn() + Send + 'static,
) {
    std::thread::Builder::new()
        .name("xpress-sync".into())
        .spawn(move || {
            let name = xpress_core::sync::device_name();
            loop {
                if flags.sync.load(Ordering::Relaxed) {
                    let fresh = flags.sync_fresh.swap(false, Ordering::Relaxed);
                    let result = match xpress_core::sync::default_root() {
                        Some(root) => xpress_core::sync::sync_with(
                            &history,
                            &root,
                            &name,
                            flags.retention(),
                            fresh,
                        )
                        .map_err(|e| e.to_string()),
                        None => Err("iCloud Drive is off — turn it on in System Settings → \
                                     Apple Account → iCloud."
                            .into()),
                    };
                    let mut s = status.lock().unwrap();
                    match result {
                        Ok(report) => {
                            s.last = Some(xpress_core::history::now_ms());
                            s.devices = report.devices;
                            s.waiting = report.waiting;
                            s.error = None;
                            if report.received > 0 {
                                changed();
                            }
                        }
                        Err(e) => {
                            if fresh {
                                flags.sync_fresh.store(true, Ordering::Relaxed);
                            }
                            s.error = Some(e);
                        }
                    }
                }
                std::thread::sleep(SYNC_EVERY);
            }
        })
        .expect("start the sync thread");
}

impl CaptureFlags {
    /// Remember that the clipboard now holds what xpress put there.
    pub fn mark_own_change(&self) {
        self.own_change
            .store(pasteboard::change_count(), Ordering::Relaxed);
    }

    /// Record a clip (and while collecting, add it to the collection).
    pub fn record(&self, history: &Mutex<History>, clip: NewClip) -> bool {
        let Some(id) = record(history, clip, self.ocr.load(Ordering::Relaxed)) else {
            return false;
        };
        if self.collecting.load(Ordering::Relaxed) {
            let current = self.collection.load(Ordering::Relaxed);
            let into = (current != 0).then_some(current);
            match history.lock().unwrap().append_to(into, id) {
                Ok(multi) => self.collection.store(multi, Ordering::Relaxed),
                Err(e) => eprintln!("xpress: could not collect clip: {e}"),
            }
        }
        true
    }

    pub fn retention(&self) -> Retention {
        let days = self.keep_days.load(Ordering::Relaxed);
        Retention {
            max_age: (days > 0).then(|| Duration::from_secs(days as u64 * 24 * 60 * 60)),
            max_bytes: Some(MAX_HISTORY_BYTES),
        }
    }
}

/// Turn a clipboard snapshot into a clip, or `None` if it shouldn't be kept:
/// marked private, from a password manager, or empty. Files win over images,
/// and text over images (apps such as Word or Excel put a picture of copied
/// text on the clipboard too), except that a lone link next to an image is
/// what browsers add when you copy an image.
pub fn clip_from_snapshot(
    snap: Snapshot,
    app: Option<String>,
    bundle: Option<String>,
    ignored: &[IgnoredApp],
) -> Option<NewClip> {
    if snap.concealed
        || snap.from_xpress
        || bundle.as_deref().is_some_and(|b| IGNORED_APPS.contains(&b))
        || ignored
            .iter()
            .any(|i| i.matches(app.as_deref(), bundle.as_deref()))
    {
        return None;
    }
    let text = snap.text.filter(|t| !t.trim().is_empty());
    let clip = if !snap.files.is_empty() {
        NewClip::files(&snap.files)
    } else {
        match (text, snap.image_png) {
            (Some(t), Some(png)) if is_link(&t) => NewClip::image_png(png),
            (Some(t), _) => NewClip::text(t),
            (None, Some(png)) => NewClip::image_png(png),
            (None, None) => return None,
        }
    };
    Some(clip.from_app(app, bundle))
}

pub fn is_link(text: &str) -> bool {
    xpress_core::history::classify_text(text) == xpress_core::history::ClipKind::Link
}

/// Where screenshots are saved: on macOS the user's choice, or the Desktop;
/// elsewhere the Pictures folder's `Screenshots` (GNOME, KDE).
#[cfg(not(target_os = "macos"))]
pub fn screenshot_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let pictures = std::process::Command::new("xdg-user-dir")
        .arg("PICTURES")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
        .filter(|p| p.is_dir());
    linux_screenshot_dir(&home, pictures)
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
fn linux_screenshot_dir(home: &Path, pictures: Option<PathBuf>) -> PathBuf {
    pictures
        .unwrap_or_else(|| home.join("Pictures"))
        .join("Screenshots")
}

/// GNOME ("Screenshot from 2026-10-04 12-00-00.png") and KDE Spectacle
/// ("Screenshot_20261004_120000.png") names.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn looks_like_screenshot(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !name.starts_with('.')
        && lower.starts_with("screenshot")
        && [".png", ".jpg", ".jpeg", ".webp"]
            .iter()
            .any(|ext| lower.ends_with(ext))
}

#[cfg(target_os = "macos")]
pub fn screenshot_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let custom = std::process::Command::new("defaults")
        .args(["read", "com.apple.screencapture", "location"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .map(|s| match s.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None => PathBuf::from(s),
        })
        .filter(|p| p.is_dir());
    custom.unwrap_or_else(|| home.join("Desktop"))
}

/// macOS tags screenshots with this attribute, whatever the language of the
/// file name.
#[cfg(target_os = "macos")]
pub fn is_screenshot(path: &Path) -> bool {
    let visible = path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| !n.starts_with('.'));
    visible
        && xattr::get(path, "com.apple.metadata:kMDItemIsScreenCapture")
            .ok()
            .flatten()
            .is_some()
}

#[cfg(not(target_os = "macos"))]
pub fn is_screenshot(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(looks_like_screenshot)
}

/// New, finished screenshots in `dir`: made after `since`, untouched for a
/// second, not seen before.
fn new_screenshots(dir: &Path, since: SystemTime, seen: &mut HashSet<PathBuf>) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let now = SystemTime::now();
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        let settled = now
            .duration_since(modified)
            .is_ok_and(|age| age >= Duration::from_secs(1));
        if modified > since && settled && !seen.contains(&path) && is_screenshot(&path) {
            seen.insert(path.clone());
            found.push(path);
        }
    }
    found.sort();
    found
}

/// Add a clip; for a new image, recognise its text (outside the lock).
/// Returns the clip's id.
pub fn record(history: &Mutex<History>, clip: NewClip, ocr: bool) -> Option<i64> {
    let is_image = clip.kind.is_image();
    let added = match history.lock().unwrap().add(clip) {
        Ok(added) => added,
        Err(e) => {
            eprintln!("xpress: could not record clip: {e}");
            return None;
        }
    };
    if added.new && is_image && ocr && xpress_core::ocr::available() {
        let image = history
            .lock()
            .unwrap()
            .get(added.id)
            .ok()
            .flatten()
            .and_then(|c| c.image);
        if let Some(text) = image.and_then(|p| xpress_core::ocr::recognize_file(&p).ok()) {
            if !text.is_empty() {
                let _ = history.lock().unwrap().set_ocr(added.id, &text);
            }
        }
    }
    Some(added.id)
}

/// Start recording. `changed` is called after each change to the history.
pub fn start(
    history: Arc<Mutex<History>>,
    flags: Arc<CaptureFlags>,
    changed: impl Fn() + Send + 'static,
) {
    std::thread::Builder::new()
        .name("xpress-history".into())
        .spawn(move || {
            // Only what's copied from now on.
            let mut last = pasteboard::change_count();
            let since = SystemTime::now();
            let shots_dir = screenshot_dir();
            let mut seen = HashSet::new();
            let mut last_prune: Option<Instant> = None;
            let mut tick: u32 = 0;
            loop {
                std::thread::sleep(TICK);
                tick = tick.wrapping_add(1);
                let count = pasteboard::change_count();
                if !flags.enabled.load(Ordering::Relaxed) {
                    last = count;
                    continue;
                }
                let mut dirty = false;

                if count != last {
                    last = count;
                    let own = count == flags.own_change.load(Ordering::Relaxed);
                    let (app, bundle) = pasteboard::frontmost_app();
                    let ignored = flags.ignored.lock().unwrap().clone();
                    if let (false, Some(clip)) = (
                        own,
                        clip_from_snapshot(pasteboard::read(), app, bundle, &ignored),
                    ) {
                        dirty |= flags.record(&history, clip);
                    }
                }

                if tick.is_multiple_of(SCREENSHOT_EVERY)
                    && flags.screenshots.load(Ordering::Relaxed)
                {
                    for path in new_screenshots(&shots_dir, since, &mut seen) {
                        if let Ok(clip) = NewClip::screenshot(&path) {
                            dirty |= flags.record(&history, clip);
                        }
                    }
                }

                if last_prune.is_none_or(|t| t.elapsed() >= PRUNE_EVERY) {
                    last_prune = Some(Instant::now());
                    let removed = history.lock().unwrap().prune(flags.retention());
                    dirty |= removed.is_ok_and(|n| n > 0);
                }

                if dirty {
                    changed();
                }
            }
        })
        .expect("start the history thread");
}

#[cfg(test)]
mod tests {
    use super::*;
    use xpress_core::history::ClipKind;

    fn png() -> Vec<u8> {
        let mut out = Vec::new();
        image::RgbaImage::from_pixel(4, 4, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    fn kind(snap: Snapshot) -> Option<ClipKind> {
        clip_from_snapshot(
            snap,
            Some("Notes".into()),
            Some("com.apple.Notes".into()),
            &[],
        )
        .map(|c| c.kind)
    }

    #[test]
    fn what_gets_recorded() {
        let text = |t: &str| Some(t.to_string());
        assert_eq!(
            kind(Snapshot {
                text: text("hello"),
                ..Default::default()
            }),
            Some(ClipKind::Text)
        );
        // Office apps add a picture of the copied text: keep the text.
        assert_eq!(
            kind(Snapshot {
                text: text("Q3 revenue"),
                image_png: Some(png()),
                ..Default::default()
            }),
            Some(ClipKind::Text)
        );
        // Browsers add the image's address when copying an image.
        assert_eq!(
            kind(Snapshot {
                text: text("https://example.com/cat.png"),
                image_png: Some(png()),
                ..Default::default()
            }),
            Some(ClipKind::Image)
        );
        assert_eq!(
            kind(Snapshot {
                files: vec!["/tmp/a.txt".into()],
                text: text("a.txt"),
                ..Default::default()
            }),
            Some(ClipKind::Files)
        );
        assert_eq!(
            kind(Snapshot {
                text: text("   \n"),
                ..Default::default()
            }),
            None
        );
    }

    #[test]
    fn copies_made_by_xpress_are_not_recorded_again() {
        assert_eq!(
            kind(Snapshot {
                from_xpress: true,
                text: Some("copied back".into()),
                ..Default::default()
            }),
            None
        );
    }

    #[test]
    fn private_content_is_never_recorded() {
        assert_eq!(
            kind(Snapshot {
                concealed: true,
                text: Some("hunter2".into()),
                ..Default::default()
            }),
            None
        );
        let from_password_manager = clip_from_snapshot(
            Snapshot {
                text: Some("hunter2".into()),
                ..Default::default()
            },
            Some("1Password".into()),
            Some("com.1password.1password".into()),
            &[],
        );
        assert!(from_password_manager.is_none());
        let bank = IgnoredApp {
            name: "Bank".into(),
            bundle: Some("com.bank.app".into()),
        };
        let from_ignored_app = clip_from_snapshot(
            Snapshot {
                text: Some("account 1234".into()),
                ..Default::default()
            },
            Some("Bank".into()),
            Some("com.bank.app".into()),
            &[bank],
        );
        assert!(from_ignored_app.is_none(), "an app the user ignores");
        assert!(from_password_manager.is_none());
    }

    #[test]
    fn records_the_source_app() {
        let clip = clip_from_snapshot(
            Snapshot {
                text: Some("x".into()),
                ..Default::default()
            },
            Some("Notes".into()),
            Some("com.apple.Notes".into()),
            &[],
        )
        .unwrap();
        assert_eq!(clip.source_app.as_deref(), Some("Notes"));
    }

    #[test]
    fn retention_from_flags() {
        let flags = CaptureFlags::default();
        assert_eq!(flags.retention().max_age, None);
        flags.keep_days.store(7, Ordering::Relaxed);
        assert_eq!(
            flags.retention().max_age,
            Some(Duration::from_secs(7 * 86_400))
        );
        assert_eq!(flags.retention().max_bytes, Some(MAX_HISTORY_BYTES));
    }

    #[test]
    fn linux_screenshots_by_name_and_folder() {
        assert!(looks_like_screenshot(
            "Screenshot from 2026-10-04 12-00-00.png"
        ));
        assert!(looks_like_screenshot("Screenshot_20261004_120000.png"));
        assert!(!looks_like_screenshot("photo.png"));
        assert!(!looks_like_screenshot("Screenshot notes.txt"));
        assert!(!looks_like_screenshot(".Screenshot.png"));
        assert_eq!(
            linux_screenshot_dir(Path::new("/home/a"), None),
            Path::new("/home/a/Pictures/Screenshots")
        );
        assert_eq!(
            linux_screenshot_dir(Path::new("/home/a"), Some("/home/a/Bilder".into())),
            Path::new("/home/a/Bilder/Screenshots")
        );
    }

    #[test]
    fn collecting_puts_every_copy_in_one_multi_clip() {
        let dir = tempfile::tempdir().unwrap();
        let history = Mutex::new(History::open(dir.path()).unwrap());
        let flags = CaptureFlags::default();
        assert!(flags.record(&history, NewClip::text("before")));
        assert_eq!(flags.collection.load(Ordering::Relaxed), 0);

        flags.collecting.store(true, Ordering::Relaxed);
        flags.record(&history, NewClip::text("one"));
        flags.record(&history, NewClip::text("two"));
        let multi = flags.collection.load(Ordering::Relaxed);
        let h = history.lock().unwrap();
        let items: Vec<String> = h
            .items(multi)
            .unwrap()
            .into_iter()
            .map(|c| c.text)
            .collect();
        assert_eq!(items, ["one", "two"]);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn finds_only_new_settled_screenshots() {
        let dir = tempfile::tempdir().unwrap();
        let since = SystemTime::now() - Duration::from_secs(60);
        let shot = dir.path().join("Skjermbilde 2026-10-04 kl. 12.00.00.png");
        let other = dir.path().join("photo.png");
        for p in [&shot, &other] {
            std::fs::write(p, png()).unwrap();
            let old = SystemTime::now() - Duration::from_secs(5);
            std::fs::File::options()
                .write(true)
                .open(p)
                .unwrap()
                .set_modified(old)
                .unwrap();
        }
        xattr::set(
            &shot,
            "com.apple.metadata:kMDItemIsScreenCapture",
            b"bplist",
        )
        .unwrap();

        let mut seen = HashSet::new();
        assert_eq!(
            new_screenshots(dir.path(), since, &mut seen),
            std::slice::from_ref(&shot)
        );
        assert!(
            new_screenshots(dir.path(), since, &mut seen).is_empty(),
            "once"
        );
        let later = SystemTime::now();
        let mut fresh = HashSet::new();
        assert!(
            new_screenshots(dir.path(), later, &mut fresh).is_empty(),
            "made before recording started"
        );
    }

    #[test]
    fn records_images_with_their_text() {
        let dir = tempfile::tempdir().unwrap();
        let history = Mutex::new(History::open(dir.path()).unwrap());
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../xpress-core/tests/fixtures/ocr.png");
        let png = std::fs::read(fixture).unwrap();
        assert!(record(&history, NewClip::image_png(png.clone()), true).is_some());
        let h = history.lock().unwrap();
        let clips = h.search(&Default::default()).unwrap();
        assert_eq!(clips.len(), 1);
        if xpress_core::ocr::available() {
            assert!(clips[0].ocr.contains("Invoice 4711"), "{:?}", clips[0].ocr);
        }
    }
}
