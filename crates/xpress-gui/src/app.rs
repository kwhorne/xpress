//! The egui application: a sidebar-driven UI with an Optimise view, Settings,
//! an About view, an interactive crop tool, and a global hotkey.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use egui::{Align, Align2, Color32, FontId, Layout, Pos2, Rect, RichText, Sense, Vec2};
use global_hotkey::{hotkey::HotKey, GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use xpress_core::compression::{CompressionQuality, CompressionTier};
use xpress_core::filetype::MediaKind;
use xpress_core::image::ImageFormat;
use xpress_core::result::OptimiseOptions;

use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::capture::{CaptureFlags, SyncStatus};
use crate::history_ui::{AiState, AiView, HistoryAction, HistoryPanel};
use crate::settings::{IgnoredApp, Settings};
use crate::shortcuts::{self, Action};
use crate::work::{self, Msg};
use xpress_core::history::History;
use xpress_core::intelligence::Status as AiStatus;

// Palette matching elyra-conductor (Tokyo Night).
const BG: Color32 = Color32::from_rgb(0x16, 0x16, 0x1e);
const BG2: Color32 = Color32::from_rgb(0x1a, 0x1b, 0x26);
pub(crate) const BG3: Color32 = Color32::from_rgb(0x1f, 0x20, 0x30);
const PANEL: Color32 = Color32::from_rgb(0x1e, 0x1f, 0x2b);
pub(crate) const BORDER: Color32 = Color32::from_rgb(0x2a, 0x2b, 0x3c);
const TEXT: Color32 = Color32::from_rgb(0xc0, 0xca, 0xf5);
const TEXT_DIM: Color32 = Color32::from_rgb(0x78, 0x7c, 0x99);
pub(crate) const ACCENT: Color32 = Color32::from_rgb(0x7a, 0xa2, 0xf7);
pub(crate) const ACCENT2: Color32 = Color32::from_rgb(0x2f, 0x36, 0x50);
const OK_GREEN: Color32 = Color32::from_rgb(0x9e, 0xce, 0x6a);
pub(crate) const ERR_RED: Color32 = Color32::from_rgb(0xf7, 0x76, 0x8e);

const UPDATE_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
/// Changed settings are written once they've been left alone this long.
const SAVE_DELAY: Duration = Duration::from_millis(500);

/// Perceptual quality targets offered in Preferences (SSIMULACRA2 scores).
const QUALITY_TARGETS: [(&str, Option<f64>); 5] = [
    ("Off — use the compression slider", None),
    ("Visually lossless", Some(90.0)),
    ("High", Some(80.0)),
    ("Medium", Some(70.0)),
    ("Low", Some(50.0)),
];
/// "Keep history" choices in Preferences (days; 0 = until the size limit).
const HISTORY_DAYS: [(u32, &str); 6] = [
    (1, "1 day"),
    (7, "1 week"),
    (30, "1 month"),
    (90, "3 months"),
    (365, "1 year"),
    (0, "Forever"),
];
/// How each entry of [`QUALITY_TARGETS`] is stored in the settings file.
const QUALITY_KEYS: [&str; 5] = ["off", "visually-lossless", "high", "medium", "low"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Optimise,
    History,
    Settings,
    About,
}

struct Card {
    title: String,
    detail: String,
    saved_pct: f64,
    ok: bool,
    output: Option<PathBuf>,
    texture: Option<egui::TextureHandle>,
    pending_thumb: Option<egui::ColorImage>,
}

struct CropState {
    path: PathBuf,
    texture: egui::TextureHandle,
    tex_size: Vec2,
    start: Option<Pos2>,
    sel: Option<Rect>,
}

pub struct XpressApp {
    tab: Tab,

    factor: i32,
    aggressive: bool,
    backup: bool,
    strip_metadata: bool,
    strip_location: bool,
    /// Index into [`QUALITY_TARGETS`].
    quality_target: usize,
    /// Convert dropped/opened images to this format instead of optimising them.
    convert_to: Option<ImageFormat>,
    skip_optimised: bool,
    always_on_top: bool,
    pipeline_dsl: String,
    use_pipeline: bool,

    /// Clipboard history.
    history: Option<Arc<Mutex<History>>>,
    history_panel: HistoryPanel,
    capture: Arc<CaptureFlags>,
    history_enabled: bool,
    history_screenshots: bool,
    history_ocr: bool,
    history_days: u32,
    /// "Clear history" asks once more before deleting.
    confirm_clear: bool,
    /// The clip last put on the clipboard (tests read this).
    last_copied: Option<i64>,
    /// Every copy goes into one multi-clip.
    collecting: bool,
    /// Apple Intelligence, once checked (in the background at launch).
    ai_status: Arc<Mutex<Option<AiStatus>>>,
    /// The latest Apple Intelligence request (older answers are dropped).
    ai_request: u64,
    /// A status check is running, and when the last one started.
    ai_checking: Arc<AtomicBool>,
    ai_checked_at: Instant,
    /// Press ⌘V in the previous app after choosing a clip.
    paste_directly: bool,
    /// Sync the history through iCloud Drive.
    history_sync: bool,
    /// Apps whose copies aren't recorded.
    history_ignored: Vec<IgnoredApp>,
    /// "Delete its clips" asks once more (the app's index in the list).
    confirm_delete_app: Option<usize>,
    /// Whether the history records changes for syncing right now.
    journal_on: bool,
    sync_status: Arc<Mutex<SyncStatus>>,
    /// Platform side (tray, hotkeys, clipboard, recording) is set up.
    integrations: bool,

    /// Where settings are remembered (none in tests).
    settings_path: Option<PathBuf>,
    /// The settings as last written, and when they started to differ.
    saved_settings: Settings,
    settings_changed: Option<Instant>,

    cards: Vec<Card>,
    in_flight: usize,

    tx: Sender<Msg>,
    rx: Receiver<Msg>,

    hotkey_manager: Option<GlobalHotKeyManager>,
    /// The shortcuts as set (stored strings, "" = off), by [`Action`].
    shortcuts: [String; 3],
    /// What's registered with the system right now.
    registered: [Option<HotKey>; 3],
    /// Why a shortcut couldn't be used.
    shortcut_errors: [Option<String>; 3],
    /// Waiting for the keys of a new shortcut.
    recording: Option<Action>,
    /// Menu-bar items that show their shortcut.
    tray_items: Option<[MenuItem; 3]>,

    crop: Option<CropState>,

    update_info: Arc<Mutex<Option<xpress_core::update::UpdateInfo>>>,
    update_checking: Arc<AtomicBool>,
    last_update_check: Instant,
    update_dismissed: bool,
    updating: Arc<AtomicBool>,
    update_status: Arc<Mutex<Option<String>>>,

    _tray: Option<TrayIcon>,
    tray_open_id: String,
    tray_clip_id: String,
    tray_history_id: String,
    tray_collect_id: String,
    tray_collect_item: Option<CheckMenuItem>,
    tray_update_id: String,
    tray_quit_id: String,
}

impl XpressApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self::build(&cc.egui_ctx, true)
    }

    /// The app state. `integrations` sets up the platform side — menu-bar
    /// icon, global hotkeys and the background update check — which tests
    /// leave out (they need the main thread and the network).
    fn build(ctx: &egui::Context, integrations: bool) -> Self {
        let (settings_path, history_dir) = if integrations {
            (Settings::path(), History::default_dir())
        } else {
            (None, None)
        };
        Self::build_with(ctx, integrations, settings_path, history_dir)
    }

    fn build_with(
        ctx: &egui::Context,
        integrations: bool,
        settings_path: Option<PathBuf>,
        history_dir: Option<PathBuf>,
    ) -> Self {
        install_style(ctx);
        let (tx, rx) = channel();

        let update_info: Arc<Mutex<Option<xpress_core::update::UpdateInfo>>> =
            Arc::new(Mutex::new(None));
        let update_checking = Arc::new(AtomicBool::new(false));
        if integrations {
            spawn_update_check(ctx.clone(), update_info.clone(), update_checking.clone());
        }
        let (
            tray,
            [open_id, clip_id, history_id, collect_id, update_id, quit_id],
            collect_item,
            tray_items,
        ) = if integrations {
            build_tray()
        } else {
            (None, Default::default(), None, None)
        };
        let manager = if integrations {
            GlobalHotKeyManager::new().ok()
        } else {
            None
        };

        let history = history_dir.and_then(|dir| match History::open(&dir) {
            Ok(h) => Some(Arc::new(Mutex::new(h))),
            Err(e) => {
                eprintln!(
                    "xpress: could not open the history in {}: {e}",
                    dir.display()
                );
                None
            }
        });
        let capture = Arc::new(CaptureFlags::default());
        let ai_status = Arc::new(Mutex::new(None));
        let ai_checking = Arc::new(AtomicBool::new(false));
        let sync_status = Arc::new(Mutex::new(SyncStatus::default()));
        if let (true, Some(history)) = (integrations, &history) {
            let (tx, ctx) = (tx.clone(), ctx.clone());
            crate::capture::start_sync(
                history.clone(),
                capture.clone(),
                sync_status.clone(),
                move || {
                    let _ = tx.send(Msg::HistoryChanged);
                    ctx.request_repaint();
                },
            );
        }
        if integrations {
            spawn_ai_check(ai_status.clone(), ai_checking.clone(), ctx.clone());
        }
        if let (true, Some(history)) = (integrations, &history) {
            let (tx, ctx) = (tx.clone(), ctx.clone());
            crate::capture::start(history.clone(), capture.clone(), move || {
                let _ = tx.send(Msg::HistoryChanged);
                ctx.request_repaint();
            });
        }

        let settings = match &settings_path {
            Some(path) => Settings::load(path, || {
                Settings::from_config(&xpress_core::config::Config::load())
            }),
            None => Settings::default(),
        };

        let mut app = Self {
            tab: Tab::Optimise,
            factor: xpress_core::compression::COMPRESSION_FACTOR_NORMAL,
            aggressive: false,
            backup: true,
            strip_metadata: false,
            strip_location: false,
            quality_target: 0,
            convert_to: None,
            skip_optimised: true,
            always_on_top: false,
            pipeline_dsl: String::new(),
            use_pipeline: false,
            history,
            history_panel: HistoryPanel::new(),
            capture,
            history_enabled: false,
            history_screenshots: true,
            history_ocr: true,
            history_days: 30,
            confirm_clear: false,
            last_copied: None,
            collecting: false,
            ai_status,
            ai_request: 0,
            ai_checking,
            ai_checked_at: Instant::now(),
            paste_directly: false,
            history_sync: false,
            history_ignored: Vec::new(),
            confirm_delete_app: None,
            journal_on: false,
            sync_status,
            integrations,
            settings_path,
            saved_settings: settings.clone(),
            settings_changed: None,
            cards: Vec::new(),
            in_flight: 0,
            tx,
            rx,
            hotkey_manager: manager,
            shortcuts: Action::ALL.map(|a| a.default_shortcut().to_string()),
            registered: [None, None, None],
            shortcut_errors: [None, None, None],
            recording: None,
            tray_items,
            crop: None,
            update_info,
            update_checking,
            last_update_check: Instant::now(),
            update_dismissed: false,
            updating: Arc::new(AtomicBool::new(false)),
            update_status: Arc::new(Mutex::new(None)),
            _tray: tray,
            tray_open_id: open_id,
            tray_clip_id: clip_id,
            tray_history_id: history_id,
            tray_collect_id: collect_id,
            tray_collect_item: collect_item,
            tray_update_id: update_id,
            tray_quit_id: quit_id,
        };
        app.apply_settings(&settings);
        app.sync_capture();
        app.apply_shortcuts();
        if app.always_on_top {
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                egui::WindowLevel::AlwaysOnTop,
            ));
        }
        app
    }

    fn settings(&self) -> Settings {
        Settings {
            compression: self.factor,
            aggressive: self.aggressive,
            backup: self.backup,
            strip_metadata: self.strip_metadata,
            strip_location: self.strip_location,
            quality_target: QUALITY_KEYS[self.quality_target].to_string(),
            convert_to: self.convert_to.map(|f| f.extension().to_string()),
            skip_optimised: self.skip_optimised,
            always_on_top: self.always_on_top,
            pipeline: self.pipeline_dsl.clone(),
            use_pipeline: self.use_pipeline,
            history_enabled: self.history_enabled,
            history_screenshots: self.history_screenshots,
            history_ocr: self.history_ocr,
            history_days: self.history_days,
            paste_directly: self.paste_directly,
            history_sync: self.history_sync,
            history_ignored: self.history_ignored.clone(),
            shortcut_clipboard: self.shortcuts[Action::Clipboard.index()].clone(),
            shortcut_show: self.shortcuts[Action::Show.index()].clone(),
            shortcut_history: self.shortcuts[Action::History.index()].clone(),
        }
    }

    fn apply_settings(&mut self, s: &Settings) {
        self.factor = s.compression.clamp(5, 100);
        self.aggressive = s.aggressive;
        self.backup = s.backup;
        self.strip_metadata = s.strip_metadata;
        self.strip_location = s.strip_location;
        self.quality_target = QUALITY_KEYS
            .iter()
            .position(|k| *k == s.quality_target)
            .unwrap_or(0);
        self.convert_to = s.convert_to.as_deref().and_then(ImageFormat::from_str);
        self.skip_optimised = s.skip_optimised;
        self.always_on_top = s.always_on_top;
        self.pipeline_dsl = s.pipeline.clone();
        self.use_pipeline = s.use_pipeline;
        self.history_enabled = s.history_enabled;
        self.history_screenshots = s.history_screenshots;
        self.history_ocr = s.history_ocr;
        self.history_days = s.history_days;
        self.paste_directly = s.paste_directly;
        self.history_sync = s.history_sync;
        self.history_ignored = s.history_ignored.clone();
        self.shortcuts = [
            s.shortcut_clipboard.clone(),
            s.shortcut_show.clone(),
            s.shortcut_history.clone(),
        ];
    }

    /// Register the shortcuts as set (none while recording a new one, so its
    /// keys reach the app), and show them in the menu-bar menu.
    fn apply_shortcuts(&mut self) {
        if let Some(manager) = &self.hotkey_manager {
            let current: Vec<HotKey> = self.registered.iter().flatten().copied().collect();
            let _ = manager.unregister_all(&current);
        }
        self.registered = [None, None, None];
        for action in Action::ALL {
            let i = action.index();
            self.shortcut_errors[i] = None;
            let Some(hotkey) = shortcuts::parse(&self.shortcuts[i]) else {
                continue;
            };
            if self.recording.is_some() {
                continue;
            }
            match &self.hotkey_manager {
                Some(manager) => match manager.register(hotkey) {
                    Ok(()) => self.registered[i] = Some(hotkey),
                    Err(_) => {
                        self.shortcut_errors[i] = Some(format!(
                            "{} couldn't be set up — another app may be using it.",
                            shortcuts::display(&hotkey)
                        ))
                    }
                },
                // Tests: no system shortcuts, but keep track of them.
                None => self.registered[i] = Some(hotkey),
            }
        }
        if let Some(items) = &self.tray_items {
            for (item, action) in items.iter().zip(Action::ALL) {
                item.set_text(match shortcuts::parse(&self.shortcuts[action.index()]) {
                    Some(hk) => format!("{}\t{}", action.label(), shortcuts::display(&hk)),
                    None => action.label().to_string(),
                });
            }
        }
    }

    /// The drop zone's second line, with the shortcuts that are on.
    fn drop_hint(&self) -> String {
        let mut hint = "images · video · PDF · audio".to_string();
        let keys: Vec<String> = [(Action::Clipboard, "clipboard"), (Action::Show, "show")]
            .into_iter()
            .filter_map(|(a, what)| self.shortcut_label(a).map(|k| format!("{k} {what}")))
            .collect();
        if !keys.is_empty() {
            hint.push_str("      ");
            hint.push_str(&keys.join("  ·  "));
        }
        hint
    }

    /// The shortcut for `action` as shown to the user, or None when off.
    fn shortcut_label(&self, action: Action) -> Option<String> {
        shortcuts::parse(&self.shortcuts[action.index()]).map(|h| shortcuts::display(&h))
    }

    /// Take the keys of a new shortcut while recording one.
    fn record_shortcut(&mut self, ui: &egui::Ui) {
        let Some(action) = self.recording else { return };
        let pressed = ui.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key {
                    key,
                    physical_key,
                    pressed: true,
                    modifiers,
                    ..
                } => Some((physical_key.unwrap_or(*key), *modifiers)),
                _ => None,
            })
        });
        let Some((key, mods)) = pressed else { return };
        let i = action.index();
        let plain = !(mods.mac_cmd || mods.ctrl || mods.alt || mods.command);
        if key == egui::Key::Escape && plain {
            self.recording = None;
        } else if matches!(key, egui::Key::Backspace | egui::Key::Delete) && plain {
            self.shortcuts[i].clear();
            self.recording = None;
        } else {
            match shortcuts::from_key(key, mods) {
                Ok(hotkey) => {
                    let taken = Action::ALL.into_iter().find(|other| {
                        *other != action
                            && shortcuts::parse(&self.shortcuts[other.index()]) == Some(hotkey)
                    });
                    match taken {
                        Some(other) => {
                            self.shortcut_errors[i] =
                                Some(format!("Already used for “{}”.", other.label()));
                            return;
                        }
                        None => {
                            self.shortcuts[i] = shortcuts::store(&hotkey);
                            self.recording = None;
                        }
                    }
                }
                Err(refused) => {
                    self.shortcut_errors[i] = Some(refused.message().to_string());
                    return;
                }
            }
        }
        self.apply_shortcuts();
    }

    fn shortcut_settings(&mut self, ui: &mut egui::Ui) {
        self.record_shortcut(ui);
        card(ui, |ui| {
            ui.label(RichText::new("Shortcuts").strong());
            ui.label(
                RichText::new("Work in every app. Click one to change it.")
                    .weak()
                    .small(),
            );
            ui.add_space(6.0);
            for action in Action::ALL {
                let i = action.index();
                let desc = match action {
                    Action::Clipboard => "Optimise the image you copied",
                    Action::Show => "Bring the window to the front",
                    Action::History => "Search what you copied and paste it again",
                };
                let mut start = false;
                let mut reset = false;
                setting_row(ui, action.label(), desc, |ui| {
                    let recording = self.recording == Some(action);
                    let label = if recording {
                        "Press keys…".to_string()
                    } else {
                        shortcuts::display_str(&self.shortcuts[i])
                    };
                    if ui
                        .add(
                            egui::Button::selectable(recording, label)
                                .min_size(egui::vec2(96.0, 0.0)),
                        )
                        .on_hover_text("Click, then press the new shortcut")
                        .clicked()
                    {
                        start = true;
                    }
                    if self.shortcuts[i] != action.default_shortcut()
                        && ui.small_button("Reset").clicked()
                    {
                        reset = true;
                    }
                });
                if self.recording == Some(action) {
                    ui.label(
                        RichText::new("Press the new shortcut · ⌫ turns it off · esc cancels")
                            .weak()
                            .small(),
                    );
                }
                if let Some(error) = &self.shortcut_errors[i] {
                    ui.label(RichText::new(error).small().color(ERR_RED));
                }
                if start {
                    self.recording = Some(action);
                    self.shortcut_errors[i] = None;
                    self.apply_shortcuts();
                }
                if reset {
                    self.shortcuts[i] = action.default_shortcut().to_string();
                    self.recording = None;
                    self.apply_shortcuts();
                }
                if action != Action::History {
                    ui.separator();
                }
            }
        });
    }

    /// Tell the recording thread what the settings say.
    fn sync_capture(&mut self) {
        let c = &self.capture;
        c.enabled.store(
            self.history_enabled && self.history.is_some(),
            Ordering::Relaxed,
        );
        c.screenshots
            .store(self.history_screenshots, Ordering::Relaxed);
        c.ocr.store(self.history_ocr, Ordering::Relaxed);
        c.keep_days.store(self.history_days, Ordering::Relaxed);
        {
            let mut ignored = c.ignored.lock().unwrap();
            if *ignored != self.history_ignored {
                *ignored = self.history_ignored.clone();
            }
        }

        let sync = self.history_sync && self.history_enabled && self.history.is_some();
        c.sync.store(sync, Ordering::Relaxed);
        if sync != self.journal_on {
            if let Some(history) = &self.history {
                history.lock().unwrap().set_journal(sync);
            }
            if sync {
                // Changes made while it was off weren't recorded.
                c.sync_fresh.store(true, Ordering::Relaxed);
            }
            self.journal_on = sync;
        }
    }

    /// Show the window on History, ready to type (the ⌃⌘V hotkey).
    fn open_history(&mut self, ctx: &egui::Context) {
        Self::show_window(ctx);
        self.crop = None;
        self.tab = Tab::History;
        self.history_panel.reset_for_paste();
    }

    fn handle_history_action(&mut self, action: HistoryAction, ctx: &egui::Context) {
        let Some(history) = self.history.clone() else {
            return;
        };
        self.history_panel.dirty = true;
        match action {
            HistoryAction::Copy { id, hide } => self.copy_clip(&history, id, hide, ctx),
            HistoryAction::Combine { ids, copy } => {
                let combined = history.lock().unwrap().combine(&ids);
                match combined {
                    Ok(multi) => {
                        self.history_panel.marked.clear();
                        if copy {
                            self.copy_clip(&history, multi, true, ctx);
                        }
                    }
                    Err(e) => eprintln!("xpress: could not combine clips: {e}"),
                }
            }
            HistoryAction::CopyText(text) => {
                if self.integrations
                    && crate::pasteboard::write(&[crate::pasteboard::Part::Text(text)])
                {
                    self.capture.mark_own_change();
                }
            }
            HistoryAction::TogglePin(id) => {
                let h = history.lock().unwrap();
                if let Ok(Some(clip)) = h.get(id) {
                    let _ = h.set_pinned(id, !clip.pinned);
                }
            }
            HistoryAction::Pin(ids) => {
                let h = history.lock().unwrap();
                for id in ids {
                    let _ = h.set_pinned(id, true);
                }
                self.history_panel.marked.clear();
            }
            HistoryAction::Delete(ids) => {
                let h = history.lock().unwrap();
                for id in ids {
                    let _ = h.delete(id);
                }
                self.history_panel.marked.clear();
            }
            HistoryAction::Reveal(path) => reveal_in_file_manager(&path),
            HistoryAction::Open(url) => ctx.open_url(egui::OpenUrl::new_tab(url)),
            HistoryAction::Enable => {
                self.history_enabled = true;
                self.sync_capture();
            }
            HistoryAction::Hide => {
                if self.integrations {
                    hide_app(ctx);
                }
            }
            HistoryAction::SetCollecting(on) => self.set_collecting(on),
            HistoryAction::SetCategory {
                clips,
                category,
                on,
            } => {
                let h = history.lock().unwrap();
                for clip in clips {
                    let _ = h.set_category(clip, category, on);
                }
            }
            HistoryAction::SaveCategory {
                id,
                name,
                color,
                rule,
            } => {
                let saved = history
                    .lock()
                    .unwrap()
                    .save_category(id, &name, color, &rule);
                match saved {
                    Ok(_) => self.history_panel.editor = None,
                    Err(e) => self.history_panel.editor_failed(category_error(&e, &name)),
                }
            }
            HistoryAction::DeleteCategory(id) => {
                let _ = history.lock().unwrap().delete_category(id);
                self.history_panel.editor = None;
            }
            HistoryAction::Intelligence { id, task } => {
                let clip = history.lock().unwrap().get(id).ok().flatten();
                let Some((clip, text)) =
                    clip.and_then(|c| crate::history_ui::ai_text(&c).map(|t| (c, t)))
                else {
                    return;
                };
                self.ai_request += 1;
                self.history_panel.ai = Some(AiView {
                    task,
                    source: crate::history_ui::title(&clip),
                    state: AiState::Working,
                });
                if self.integrations {
                    let (request, tx, ctx) = (self.ai_request, self.tx.clone(), ctx.clone());
                    std::thread::spawn(move || {
                        let result = xpress_core::intelligence::run(task, &text);
                        let _ = tx.send(Msg::Ai { request, result });
                        ctx.request_repaint();
                    });
                }
            }
            HistoryAction::IgnoreApp { name, bundle } => {
                self.ignore_app(IgnoredApp { name, bundle })
            }
            HistoryAction::SaveText(text) => {
                let clip = xpress_core::history::NewClip::text(text)
                    .from_app(Some("Apple Intelligence".into()), None);
                let _ = history.lock().unwrap().add(clip);
            }
        }
    }

    /// Put a clip on the clipboard; with `hide`, step aside so it can be
    /// pasted — and paste it, when "Paste directly" is on and allowed.
    fn copy_clip(
        &mut self,
        history: &Arc<Mutex<History>>,
        id: i64,
        hide: bool,
        ctx: &egui::Context,
    ) {
        let parts = history.lock().unwrap().parts(id).unwrap_or_default();
        if parts.is_empty() {
            return;
        }
        if self.integrations {
            if !crate::pasteboard::write(&parts) {
                return;
            }
            self.capture.mark_own_change();
        }
        let _ = history.lock().unwrap().touch(id);
        self.last_copied = Some(id);
        self.history_panel.selected = 0;
        if hide && self.integrations {
            hide_app(ctx);
            if self.paste_directly && crate::autopaste::allowed() {
                crate::autopaste::paste_soon();
            }
        }
    }

    /// Add an app to the ignore list (once).
    fn ignore_app(&mut self, app: IgnoredApp) {
        let known = self
            .history_ignored
            .iter()
            .any(|i| i.matches(Some(&app.name), app.bundle.as_deref()));
        if !known {
            self.history_ignored.push(app);
            self.sync_capture();
        }
    }

    fn ignored_apps_settings(&mut self, ui: &mut egui::Ui) {
        let Some(history) = self.history.clone() else {
            return;
        };
        let sources = history.lock().unwrap().app_sources().unwrap_or_default();
        let mut add = None;
        setting_row(
            ui,
            "Ignore apps",
            "Don't record what you copy in these apps — password managers are always left out",
            |ui| {
                ui.menu_button("Add app…", |ui| {
                    let mut listed = false;
                    for (name, bundle, count) in &sources {
                        let ignored = self
                            .history_ignored
                            .iter()
                            .any(|i| i.matches(Some(name), bundle.as_deref()));
                        if !ignored {
                            listed = true;
                            if ui.button(format!("{name}  ({count})")).clicked() {
                                add = Some(IgnoredApp {
                                    name: name.clone(),
                                    bundle: bundle.clone(),
                                });
                                ui.close();
                            }
                        }
                    }
                    if listed {
                        ui.separator();
                    }
                    if ui.button("Choose from Applications…").clicked() {
                        add = pick_app();
                        ui.close();
                    }
                });
            },
        );
        if let Some(app) = add {
            self.ignore_app(app);
        }
        let mut remove = None;
        let mut delete = None;
        for (i, app) in self.history_ignored.iter().enumerate() {
            let count = history
                .lock()
                .unwrap()
                .count_from_app(&app.name, app.bundle.as_deref())
                .unwrap_or(0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(&app.name).strong());
                if let Some(bundle) = &app.bundle {
                    ui.label(RichText::new(bundle).weak().small());
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .small_button("×")
                        .on_hover_text("Record it again")
                        .clicked()
                    {
                        remove = Some(i);
                    }
                    if count > 0 {
                        let label = if self.confirm_delete_app == Some(i) {
                            "Click again to delete".to_string()
                        } else {
                            format!(
                                "Delete its {count} clip{}",
                                if count == 1 { "" } else { "s" }
                            )
                        };
                        if ui.small_button(label).clicked() {
                            if self.confirm_delete_app == Some(i) {
                                delete = Some(i);
                            } else {
                                self.confirm_delete_app = Some(i);
                            }
                        }
                    }
                });
            });
        }
        if let Some(i) = delete {
            let app = &self.history_ignored[i];
            let _ = history
                .lock()
                .unwrap()
                .delete_from_app(&app.name, app.bundle.as_deref());
            self.confirm_delete_app = None;
            self.history_panel.dirty = true;
        }
        if let Some(i) = remove {
            self.history_ignored.remove(i);
            self.confirm_delete_app = None;
            self.sync_capture();
        }
    }

    /// Start or stop collecting every copy into one multi-clip.
    fn set_collecting(&mut self, on: bool) {
        self.collecting = on;
        self.capture.collecting.store(on, Ordering::Relaxed);
        if on {
            self.capture.collection.store(0, Ordering::Relaxed);
        }
        if let Some(item) = &self.tray_collect_item {
            item.set_checked(on);
        }
        self.history_panel.dirty = true;
    }

    /// Write the settings once they've changed and settled (or now, with
    /// `force`, e.g. before quitting).
    fn persist_settings(&mut self, force: bool) {
        let Some(path) = self.settings_path.clone() else {
            return;
        };
        let current = self.settings();
        if current == self.saved_settings {
            self.settings_changed = None;
            return;
        }
        let since = *self.settings_changed.get_or_insert_with(Instant::now);
        if force || since.elapsed() >= SAVE_DELAY {
            if let Err(e) = current.save(&path) {
                eprintln!("xpress: could not save settings to {}: {e}", path.display());
            }
            self.saved_settings = current;
            self.settings_changed = None;
        }
    }

    fn show_window(ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        activate_app();
    }

    /// Download the latest release, replace this .app, and relaunch. macOS only.
    fn start_self_update(&mut self, url: String) {
        if self.updating.swap(true, Ordering::SeqCst) {
            return;
        }
        *self.update_status.lock().unwrap() = Some("Downloading update…".into());
        let updating = self.updating.clone();
        let status = self.update_status.clone();
        std::thread::spawn(move || {
            if let Err(e) = perform_self_update(&url, &status) {
                *status.lock().unwrap() = Some(format!("Update failed: {e}"));
                updating.store(false, Ordering::SeqCst);
            }
            // On success the process re-launches and exits; nothing to do here.
        });
    }

    fn options(&self) -> OptimiseOptions {
        let compression = if self.aggressive {
            CompressionQuality::aggressive()
        } else {
            CompressionQuality::new(CompressionTier::Custom, self.factor)
        };
        OptimiseOptions {
            compression,
            backup: self.backup,
            strip_metadata: self.strip_metadata,
            strip_location: self.strip_location,
            preserve_dates: true,
            output: None,
            allow_larger: false,
            use_cache: self.skip_optimised,
        }
    }

    fn quality(&self) -> Option<f64> {
        QUALITY_TARGETS
            .get(self.quality_target)
            .and_then(|(_, q)| *q)
    }

    fn check_for_updates(&mut self, ctx: &egui::Context) {
        self.last_update_check = Instant::now();
        spawn_update_check(
            ctx.clone(),
            self.update_info.clone(),
            self.update_checking.clone(),
        );
    }

    fn submit(&mut self, path: PathBuf, ctx: &egui::Context) {
        if xpress_core::filetype::classify(&path).is_none() {
            return;
        }
        self.in_flight += 1;
        let options = self.options();
        let is_image = xpress_core::filetype::classify(&path) == Some(MediaKind::Image);
        if let (Some(format), true) = (self.convert_to, is_image) {
            work::spawn_convert(
                path,
                format,
                options,
                self.quality(),
                ctx.clone(),
                self.tx.clone(),
            );
        } else if self.use_pipeline {
            match xpress_core::pipeline::parse(&self.pipeline_dsl) {
                Ok(steps) => {
                    work::spawn_pipeline(path, steps, options, ctx.clone(), self.tx.clone())
                }
                Err(e) => {
                    self.in_flight -= 1;
                    self.push_card(Card {
                        title: "Invalid pipeline".into(),
                        detail: e,
                        saved_pct: 0.0,
                        ok: false,
                        output: None,
                        texture: None,
                        pending_thumb: None,
                    });
                }
            }
        } else {
            work::spawn(path, options, self.quality(), ctx.clone(), self.tx.clone());
        }
    }

    fn optimise_clipboard(&mut self, ctx: &egui::Context) {
        match clipboard_image_to_file() {
            Ok(path) => self.submit(path, ctx),
            Err(e) => self.push_card(Card {
                title: "Clipboard".into(),
                detail: e,
                saved_pct: 0.0,
                ok: false,
                output: None,
                texture: None,
                pending_thumb: None,
            }),
        }
    }

    fn push_card(&mut self, card: Card) {
        self.cards.insert(0, card);
        self.cards.truncate(50);
    }

    fn drain_results(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            let done = match msg {
                Msg::Done(done) => done,
                Msg::HistoryChanged => {
                    self.history_panel.dirty = true;
                    continue;
                }
                Msg::Ai { request, result } => {
                    if let (true, Some(view)) =
                        (request == self.ai_request, &mut self.history_panel.ai)
                    {
                        view.state = match result {
                            Ok(text) => AiState::Done(text),
                            Err(e) => AiState::Failed(e),
                        };
                    }
                    continue;
                }
            };
            self.in_flight = self.in_flight.saturating_sub(1);
            let name = done
                .source
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let from_clipboard = done.source.starts_with(clipboard_dir());
            let card = match done.result {
                Ok(r) => {
                    // A conversion: show "photo.png → photo.jpg".
                    let name = if r.output.extension() != done.source.extension() {
                        let out = r
                            .output
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        format!("{name} → {out}")
                    } else {
                        name
                    };
                    let mut detail = if r.cached {
                        format!("{}  ·  already optimised — skipped", human(r.old_size))
                    } else {
                        format!(
                            "{} → {}{}",
                            human(r.old_size),
                            human(r.new_size),
                            if r.aggressive { "  ·  aggressive" } else { "" }
                        )
                    };
                    if let Some(score) = r.score {
                        detail.push_str(&format!("  ·  SSIMULACRA2 {score:.0}"));
                    }
                    if from_clipboard && xpress_core::clipboard::set_clipboard_png(&r.output) {
                        detail.push_str("  ·  copied back");
                    }
                    Card {
                        title: name,
                        detail,
                        saved_pct: r.saved_percent(),
                        ok: true,
                        output: Some(r.output.clone()),
                        texture: None,
                        pending_thumb: done.thumbnail,
                    }
                }
                Err(e) => Card {
                    title: name,
                    detail: e,
                    saved_pct: 0.0,
                    ok: false,
                    output: None,
                    texture: None,
                    pending_thumb: None,
                },
            };
            self.push_card(card);
        }
    }

    fn enter_crop(&mut self, path: PathBuf, ctx: &egui::Context) {
        let Ok(img) = xpress_core::image::open_oriented(&path) else {
            return;
        };
        let disp = img.thumbnail(1400, 1400).to_rgba8();
        let (w, h) = (disp.width(), disp.height());
        let color =
            egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], disp.as_raw());
        let texture = ctx.load_texture("crop", color, egui::TextureOptions::LINEAR);
        self.crop = Some(CropState {
            path,
            texture,
            tex_size: egui::vec2(w as f32, h as f32),
            start: None,
            sel: None,
        });
    }

    fn apply_crop(&mut self, ctx: &egui::Context) {
        let Some(crop) = self.crop.take() else { return };
        let Some(sel) = crop.sel else { return };
        let img_rect: Option<Rect> =
            ctx.memory_mut(|m| m.data.get_temp(egui::Id::new("crop_rect")));
        let Some(rect) = img_rect else { return };
        let nx = ((sel.min.x - rect.min.x) / rect.width()).clamp(0.0, 1.0) as f64;
        let ny = ((sel.min.y - rect.min.y) / rect.height()).clamp(0.0, 1.0) as f64;
        let nw = (sel.width() / rect.width()).clamp(0.01, 1.0) as f64;
        let nh = (sel.height() / rect.height()).clamp(0.01, 1.0) as f64;
        self.in_flight += 1;
        work::spawn_crop(
            crop.path,
            nx,
            ny,
            nw,
            nh,
            self.options(),
            ctx.clone(),
            self.tx.clone(),
        );
    }
}

impl eframe::App for XpressApp {
    /// Non-drawing work. eframe calls this even while the window is hidden, so
    /// menu-bar clicks, hotkeys and finished jobs are handled from the tray too.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Closing the window hides it to the menu bar instead of quitting; use the
        // tray's “Quit” to actually exit.
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

        // Menu-bar icon: menu selections and clicks.
        while let Ok(ev) = MenuEvent::receiver().try_recv() {
            let id = ev.id.0.as_str();
            if id == self.tray_open_id {
                Self::show_window(ctx);
            } else if id == self.tray_clip_id {
                Self::show_window(ctx);
                self.optimise_clipboard(ctx);
            } else if id == self.tray_history_id {
                self.open_history(ctx);
            } else if id == self.tray_collect_id {
                let on = !self.collecting;
                self.set_collecting(on);
            } else if id == self.tray_update_id {
                Self::show_window(ctx);
                self.tab = Tab::About;
                self.check_for_updates(ctx);
            } else if id == self.tray_quit_id {
                self.persist_settings(true);
                std::process::exit(0);
            }
        }
        while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click { .. } = ev {
                Self::show_window(ctx);
            }
        }

        // Global hotkey events (poll once; dispatch by id).
        while let Ok(ev) = GlobalHotKeyEvent::receiver().try_recv() {
            if ev.state != HotKeyState::Pressed {
                continue;
            }
            let action = Action::ALL
                .into_iter()
                .find(|a| self.registered[a.index()].map(|h| h.id()) == Some(ev.id));
            match action {
                Some(Action::Clipboard) => {
                    Self::show_window(ctx);
                    self.optimise_clipboard(ctx);
                }
                Some(Action::Show) => Self::show_window(ctx),
                Some(Action::History) => self.open_history(ctx),
                None => {}
            }
        }

        self.drain_results();
        self.sync_capture();
        self.persist_settings(false);

        if self.last_update_check.elapsed() >= UPDATE_INTERVAL
            && !self.update_checking.load(Ordering::Relaxed)
        {
            self.check_for_updates(ctx);
        }

        // Poll steadily so menu-bar clicks and the global hotkey are handled even
        // when the window is idle or in the background.
        let interval = if self.in_flight > 0 { 120 } else { 250 };
        ctx.request_repaint_after(Duration::from_millis(interval));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }
}

impl XpressApp {
    /// Draw the whole window (and accept dropped files).
    fn draw(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        for path in dropped {
            self.submit(path, &ctx);
        }

        self.draw_update_banner(ui);

        if self.crop.is_some() {
            self.draw_crop(ui);
        } else {
            self.draw_sidebar(ui);
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::central_panel(&ctx.global_style())
                        .fill(BG)
                        .inner_margin(egui::Margin::same(22)),
                )
                .show(ui, |ui| match self.tab {
                    Tab::Optimise => self.optimise_view(ui),
                    Tab::History => self.history_view(ui),
                    // Taller than the window: scroll.
                    Tab::Settings => {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| self.settings_view(ui));
                    }
                    Tab::About => self.about_view(ui),
                });
        }
    }
}

impl XpressApp {
    // ---- Sidebar -----------------------------------------------------------

    fn draw_sidebar(&mut self, root: &mut egui::Ui) {
        let ctx = &root.ctx().clone();
        egui::Panel::left("sidebar")
            .exact_size(212.0)
            .resizable(false)
            .frame(
                egui::Frame::default()
                    .fill(sidebar_fill(ctx))
                    .inner_margin(egui::Margin::symmetric(12, 16)),
            )
            .show(root, |ui| {
                // Brand.
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(32.0, 32.0), Sense::hover());
                    draw_x_logo(ui.painter(), rect.center(), 32.0);
                    ui.add_space(2.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new("xpress").size(17.0).strong());
                        ui.label(
                            RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                                .size(11.0)
                                .weak(),
                        );
                    });
                });

                ui.add_space(16.0);
                section_header(ui, "WORKSPACE");
                if nav_item(ui, self.tab == Tab::Optimise, ACCENT, "⤓", "Optimise") {
                    self.tab = Tab::Optimise;
                }
                if nav_item(
                    ui,
                    self.tab == Tab::History,
                    Color32::from_rgb(90, 180, 140),
                    "≡",
                    "History",
                ) {
                    self.tab = Tab::History;
                    self.history_panel.dirty = true;
                    self.history_panel.focus_search = true;
                }
                if nav_item(
                    ui,
                    false,
                    Color32::from_rgb(90, 140, 240),
                    "⛶",
                    "Crop image…",
                ) {
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter(
                            "images",
                            &[
                                "png", "jpg", "jpeg", "webp", "gif", "bmp", "tiff", "tif", "heic",
                                "heif", "avif",
                            ],
                        )
                        .pick_file()
                    {
                        self.enter_crop(p, ctx);
                    }
                }

                ui.add_space(14.0);
                section_header(ui, "SETTINGS");
                if nav_item(
                    ui,
                    self.tab == Tab::Settings,
                    Color32::from_rgb(120, 120, 130),
                    "⚙",
                    "Preferences",
                ) {
                    self.tab = Tab::Settings;
                }

                ui.add_space(14.0);
                section_header(ui, "SUPPORT");
                if nav_item(
                    ui,
                    self.tab == Tab::About,
                    Color32::from_rgb(230, 90, 110),
                    "i",
                    "About",
                ) {
                    self.tab = Tab::About;
                }

                // Footer status.
                ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
                    ui.add_space(4.0);
                    if self.in_flight > 0 {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new().size(14.0));
                            ui.label(
                                RichText::new(format!("{} working…", self.in_flight))
                                    .weak()
                                    .small(),
                            );
                        });
                    }
                });
            });
    }

    // ---- Optimise view -----------------------------------------------------

    fn optimise_view(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.heading("Optimise");
        ui.label(RichText::new("Make images, video, PDF and audio smaller.").weak());
        ui.add_space(16.0);

        // Drop zone.
        let drop_h = 150.0;
        let (rect, resp) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), drop_h), Sense::hover());
        let hovering_files = ctx.input(|i| !i.raw.hovered_files.is_empty());
        let stroke = if hovering_files {
            egui::Stroke::new(2.0_f32, ACCENT)
        } else {
            egui::Stroke::new(1.5_f32, ui.visuals().widgets.noninteractive.bg_stroke.color)
        };
        ui.painter().rect(
            rect,
            12.0,
            card_fill(ui.ctx()),
            stroke,
            egui::StrokeKind::Inside,
        );
        let _ = resp;
        ui.painter().text(
            rect.center() - egui::vec2(0.0, 12.0),
            Align2::CENTER_CENTER,
            "Drop files here",
            FontId::proportional(18.0),
            ui.visuals().text_color(),
        );
        let hint = self.drop_hint();
        ui.painter().text(
            rect.center() + egui::vec2(0.0, 14.0),
            Align2::CENTER_CENTER,
            hint,
            FontId::proportional(12.0),
            ui.visuals().weak_text_color(),
        );

        ui.add_space(14.0);
        ui.horizontal(|ui| {
            if ui.button("  Open files…  ").clicked() {
                if let Some(paths) = rfd::FileDialog::new().pick_files() {
                    for p in paths {
                        self.submit(p, &ctx);
                    }
                }
            }
            if ui.button("Optimise clipboard").clicked() {
                self.optimise_clipboard(&ctx);
            }
            if !self.cards.is_empty() && ui.button("Clear").clicked() {
                self.cards.clear();
            }
        });

        ui.add_space(14.0);
        // Quick compression control.
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Compression").strong());
                let quality = self.quality();
                ui.add_enabled(
                    !self.aggressive && quality.is_none(),
                    egui::Slider::new(&mut self.factor, 5..=100).show_value(true),
                );
                if quality.is_some() {
                    let (name, _) = QUALITY_TARGETS[self.quality_target];
                    ui.label(
                        RichText::new(format!("images: quality target “{name}”"))
                            .weak()
                            .small(),
                    );
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    toggle_labeled(ui, &mut self.aggressive, "Aggressive");
                });
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Convert to").strong());
                convert_picker(ui, &mut self.convert_to);
                if let Some(format) = self.convert_to {
                    ui.label(RichText::new(convert_note(format)).weak().small());
                }
            });
            ui.horizontal(|ui| {
                toggle_labeled(ui, &mut self.use_pipeline, "Pipeline");
                ui.add_enabled(
                    self.use_pipeline,
                    egui::TextEdit::singleline(&mut self.pipeline_dsl)
                        .desired_width(f32::INFINITY)
                        .hint_text("crop(width: 1600) -> convert(to: webp)"),
                );
            });
        });

        ui.add_space(16.0);
        if self.cards.is_empty() {
            return;
        }
        ui.label(RichText::new("RESULTS").size(11.0).weak());
        ui.add_space(6.0);
        let mut action = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (i, card) in self.cards.iter_mut().enumerate() {
                if card.texture.is_none() {
                    if let Some(image) = card.pending_thumb.take() {
                        card.texture = Some(ui.ctx().load_texture(
                            "thumb",
                            image,
                            egui::TextureOptions::LINEAR,
                        ));
                    }
                }
                if let Some(a) = result_card(ui, card, i) {
                    action = Some(a);
                }
            }
        });
        match action {
            Some(CardAction::Convert(path, format)) => {
                self.in_flight += 1;
                work::spawn_convert(
                    path,
                    format,
                    self.options(),
                    self.quality(),
                    ctx.clone(),
                    self.tx.clone(),
                );
            }
            Some(CardAction::Crop(path)) => self.enter_crop(path, &ctx),
            None => {}
        }
    }

    // ---- History view ------------------------------------------------------

    fn history_view(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let Some(history) = self.history.clone() else {
            ui.heading("History");
            ui.label(RichText::new("The history could not be opened.").color(ERR_RED));
            return;
        };
        if self.history_panel.dirty {
            self.history_panel.refresh(&history.lock().unwrap());
        }
        let ai = *self.ai_status.lock().unwrap();
        self.history_panel.ai_status = ai;
        // Apple Intelligence may have been turned on (or finished getting
        // ready) since: look again now and then.
        if self.integrations
            && !matches!(ai, Some(AiStatus::Available) | Some(AiStatus::Missing))
            && self.ai_checked_at.elapsed() >= Duration::from_secs(30)
        {
            self.ai_checked_at = Instant::now();
            spawn_ai_check(
                self.ai_status.clone(),
                self.ai_checking.clone(),
                ctx.clone(),
            );
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        if let Some(action) =
            self.history_panel
                .show(ui, self.history_enabled, self.collecting, now)
        {
            self.handle_history_action(action, &ctx);
        }
    }

    fn history_settings(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            setting_row(
                ui,
                "Clipboard history",
                &match self.shortcut_label(Action::History) {
                    Some(k) => format!("Keep what you copy, searchable under History ({k})"),
                    None => "Keep what you copy, searchable under History".to_string(),
                },
                |ui| {
                    toggle(ui, &mut self.history_enabled);
                },
            );
            ui.separator();
            ui.add_enabled_ui(self.history_enabled, |ui| {
                setting_row(
                    ui,
                    "Include screenshots",
                    "Add new screenshots to the history",
                    |ui| {
                        toggle(ui, &mut self.history_screenshots);
                    },
                );
                if xpress_core::ocr::available() {
                    ui.separator();
                    setting_row(
                        ui,
                        "Find text in images",
                        "Recognise words in screenshots and images, on this Mac",
                        |ui| {
                            toggle(ui, &mut self.history_ocr);
                        },
                    );
                }
                ui.separator();
                if crate::autopaste::supported() {
                    setting_row(
                        ui,
                        "Paste directly",
                        "After you choose a clip, paste it into the app you were using",
                        |ui| {
                            if toggle(ui, &mut self.paste_directly).changed()
                                && self.paste_directly
                                && self.integrations
                                && !crate::autopaste::allowed()
                            {
                                crate::autopaste::ask();
                            }
                        },
                    );
                    if self.paste_directly && self.integrations && !crate::autopaste::allowed() {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(
                                    "Needs permission: System Settings → Privacy & Security → \
                                     Accessibility → xpress",
                                )
                                .color(ERR_RED)
                                .small(),
                            );
                            if ui.small_button("Open Settings").clicked() {
                                ui.ctx().open_url(egui::OpenUrl::new_tab(
                                    crate::autopaste::SETTINGS_URL,
                                ));
                            }
                        });
                    }
                    ui.separator();
                }
                setting_row(
                    ui,
                    "Sync with iCloud",
                    "Share the history between your Macs through iCloud Drive",
                    |ui| {
                        toggle(ui, &mut self.history_sync);
                    },
                );
                if self.history_sync {
                    let status = self.sync_status.lock().unwrap().clone();
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as i64)
                        .unwrap_or(0);
                    let (text, color) = sync_status_line(&status, now);
                    ui.label(RichText::new(text).small().color(color));
                }
                ui.separator();
                self.ignored_apps_settings(ui);
                ui.separator();
                setting_row(ui, "Keep history", "Pinned clips are always kept", |ui| {
                    let current = HISTORY_DAYS
                        .iter()
                        .find(|(d, _)| *d == self.history_days)
                        .map(|(_, l)| *l)
                        .unwrap_or("1 month");
                    egui::ComboBox::from_id_salt("history_days")
                        .selected_text(current)
                        .width(140.0)
                        .show_ui(ui, |ui| {
                            for (days, label) in HISTORY_DAYS {
                                ui.selectable_value(&mut self.history_days, days, label);
                            }
                        });
                });
            });
            if let Some(history) = self.history.clone() {
                ui.separator();
                let (count, bytes) = history.lock().unwrap().stats().unwrap_or_default();
                setting_row(
                    ui,
                    "Clear history",
                    &format!("{count} clips · {} · pinned clips stay", human(bytes)),
                    |ui| {
                        let label = if self.confirm_clear {
                            "Click again to clear"
                        } else {
                            "Clear…"
                        };
                        if ui.button(label).clicked() {
                            if self.confirm_clear {
                                let _ = history.lock().unwrap().clear(true);
                                self.history_panel.dirty = true;
                                self.confirm_clear = false;
                            } else {
                                self.confirm_clear = true;
                            }
                        }
                    },
                );
            }
        });
    }

    // ---- Settings view -----------------------------------------------------

    fn settings_view(&mut self, ui: &mut egui::Ui) {
        ui.heading("Preferences");
        ui.label(
            RichText::new(
                "Defaults applied to every optimisation. Changes are saved automatically.",
            )
            .weak(),
        );
        ui.add_space(16.0);

        card(ui, |ui| {
            setting_row(
                ui,
                "Keep a backup",
                "Save the original as .name.orig",
                |ui| {
                    toggle(ui, &mut self.backup);
                },
            );
            ui.separator();
            setting_row(
                ui,
                "Strip metadata",
                "Remove EXIF (camera, location, date)",
                |ui| {
                    toggle(ui, &mut self.strip_metadata);
                },
            );
            ui.separator();
            setting_row(
                ui,
                "Remove location",
                "Drop GPS / where it was taken, keep the rest",
                |ui| {
                    ui.add_enabled_ui(!self.strip_metadata, |ui| {
                        toggle(ui, &mut self.strip_location);
                    });
                },
            );
        });

        ui.add_space(12.0);
        card(ui, |ui| {
            setting_row(
                ui,
                "Quality target",
                "Images: the smallest file that still looks this good",
                |ui| {
                    egui::ComboBox::from_id_salt("quality_target")
                        .selected_text(QUALITY_TARGETS[self.quality_target].0)
                        .width(230.0)
                        .show_ui(ui, |ui| {
                            for (i, (name, _)) in QUALITY_TARGETS.iter().enumerate() {
                                ui.selectable_value(&mut self.quality_target, i, *name);
                            }
                        });
                },
            );
            ui.separator();
            setting_row(
                ui,
                "Skip already-optimised files",
                "Leave files xpress already squeezed with these settings",
                |ui| {
                    toggle(ui, &mut self.skip_optimised);
                },
            );
        });

        ui.add_space(12.0);
        card(ui, |ui| {
            setting_row(
                ui,
                "Aggressive by default",
                "Trade a little quality for smaller files",
                |ui| {
                    toggle(ui, &mut self.aggressive);
                },
            );
            ui.separator();
            setting_row(ui, "Float on top", "Keep the window above others", |ui| {
                if toggle(ui, &mut self.always_on_top).changed() {
                    let level = if self.always_on_top {
                        egui::WindowLevel::AlwaysOnTop
                    } else {
                        egui::WindowLevel::Normal
                    };
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
                }
            });
        });

        ui.add_space(12.0);
        self.history_settings(ui);

        ui.add_space(12.0);
        self.shortcut_settings(ui);

        ui.add_space(12.0);
        card(ui, |ui| {
            ui.label(RichText::new("Default pipeline").strong());
            ui.label(
                RichText::new("Runs when “Pipeline” is enabled on the Optimise screen.")
                    .weak()
                    .small(),
            );
            ui.add_space(6.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.pipeline_dsl)
                    .desired_width(f32::INFINITY)
                    .hint_text("crop(width: 1600) -> convert(to: webp)"),
            );
        });
    }

    // ---- About view --------------------------------------------------------

    fn about_view(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.add_space(10.0);
        ui.vertical_centered(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(72.0, 72.0), Sense::hover());
            draw_x_logo(ui.painter(), rect.center(), 72.0);
            ui.add_space(10.0);
            ui.heading("xpress");
            ui.label(
                RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
                    .weak()
                    .monospace(),
            );
            ui.add_space(10.0);
            ui.label("Make your media smaller — images, video, PDF and audio.");
            ui.add_space(16.0);
            ui.hyperlink_to("Website · kwhorne.com", "https://kwhorne.com");
            ui.hyperlink_to(
                "GitHub · github.com/kwhorne/xpress",
                "https://github.com/kwhorne/xpress",
            );
            ui.add_space(6.0);
            ui.label(RichText::new("Developed by Knut W. Horne").strong());
            ui.add_space(18.0);

            let checking = self.update_checking.load(Ordering::Relaxed);
            let updating = self.updating.load(Ordering::Relaxed);
            let info = self.update_info.lock().unwrap().clone();
            let mut start_update: Option<String> = None;
            if updating {
                let s = self.update_status.lock().unwrap().clone();
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(14.0));
                    ui.label(s.unwrap_or_else(|| "Updating…".into()));
                });
            } else if checking {
                ui.label(RichText::new("Checking for updates…").weak());
            } else if let Some(info) = &info {
                if info.newer {
                    ui.label(
                        RichText::new(format!("Update available — v{}", info.latest)).color(ACCENT),
                    );
                    if can_self_update(info) {
                        if ui.button("Update & Restart").clicked() {
                            start_update = info.download_url.clone();
                        }
                    } else {
                        ui.hyperlink_to("Download", &info.url);
                    }
                } else {
                    ui.label(RichText::new("You're on the latest version").weak());
                }
            }
            if !updating
                && ui
                    .add_enabled(!checking, egui::Button::new("Check for updates"))
                    .clicked()
            {
                self.check_for_updates(&ctx);
            }
            if let Some(url) = start_update {
                self.start_self_update(url);
            }
        });
    }

    // ---- Update banner -----------------------------------------------------

    fn draw_update_banner(&mut self, root: &mut egui::Ui) {
        if self.update_dismissed {
            return;
        }
        let info = { self.update_info.lock().unwrap().clone() };
        let Some(info) = info else { return };
        if !info.newer {
            return;
        }
        let updating = self.updating.load(Ordering::Relaxed);
        let status = self.update_status.lock().unwrap().clone();
        let can_auto = can_self_update(&info);
        let dl = info.download_url.clone();
        let mut start_update: Option<String> = None;
        egui::Panel::top("update_banner")
            .frame(
                egui::Frame::default()
                    .fill(ACCENT)
                    .inner_margin(egui::Margin::symmetric(12, 7)),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("Update available — v{}", info.latest))
                            .color(Color32::WHITE)
                            .strong(),
                    );
                    if updating {
                        ui.add(egui::Spinner::new().size(14.0).color(Color32::WHITE));
                        ui.label(
                            RichText::new(status.unwrap_or_else(|| "Updating…".into()))
                                .color(Color32::WHITE),
                        );
                    } else if can_auto {
                        if ui
                            .button(RichText::new("Update & Restart").strong())
                            .clicked()
                        {
                            start_update = dl.clone();
                        }
                    } else {
                        ui.hyperlink_to(
                            RichText::new("Download").color(Color32::WHITE).underline(),
                            &info.url,
                        );
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !updating
                            && ui
                                .button(RichText::new("✕").color(Color32::WHITE))
                                .on_hover_text("Dismiss")
                                .clicked()
                        {
                            self.update_dismissed = true;
                        }
                    });
                });
            });
        if let Some(url) = start_update {
            self.start_self_update(url);
        }
    }

    // ---- Crop overlay ------------------------------------------------------

    fn draw_crop(&mut self, root: &mut egui::Ui) {
        let ctx = &root.ctx().clone();
        let mut apply = false;
        let mut cancel = false;
        egui::Panel::top("crop_top").show(root, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.heading("Crop");
                if let Some(c) = &self.crop {
                    ui.label(
                        RichText::new(
                            c.path
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_default(),
                        )
                        .weak(),
                    );
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    let has_sel = self.crop.as_ref().and_then(|c| c.sel).is_some();
                    if ui
                        .add_enabled(has_sel, egui::Button::new("Apply crop"))
                        .clicked()
                    {
                        apply = true;
                    }
                });
            });
            ui.label(RichText::new("Drag to select a region.").weak().small());
            ui.add_space(4.0);
        });

        egui::CentralPanel::default().show(root, |ui| {
            let Some(crop) = self.crop.as_mut() else {
                return;
            };
            let avail = ui.available_size();
            let fit = (avail.x / crop.tex_size.x).min(avail.y / crop.tex_size.y);
            let scale = fit.clamp(0.01, 1.0);
            let img_size = crop.tex_size * scale;
            let (rect, resp) = ui.allocate_exact_size(img_size, Sense::drag());
            let painter = ui.painter_at(rect);
            painter.image(
                crop.texture.id(),
                rect,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );

            if let Some(p) = resp.interact_pointer_pos() {
                let p = egui::pos2(
                    p.x.clamp(rect.min.x, rect.max.x),
                    p.y.clamp(rect.min.y, rect.max.y),
                );
                if resp.drag_started() {
                    crop.start = Some(p);
                }
                if let Some(s) = crop.start {
                    crop.sel = Some(Rect::from_two_pos(s, p));
                }
            }

            if let Some(sel) = crop.sel {
                let dim = Color32::from_black_alpha(120);
                painter.rect_filled(
                    Rect::from_min_max(rect.min, egui::pos2(rect.max.x, sel.min.y)),
                    0.0,
                    dim,
                );
                painter.rect_filled(
                    Rect::from_min_max(egui::pos2(rect.min.x, sel.max.y), rect.max),
                    0.0,
                    dim,
                );
                painter.rect_filled(
                    Rect::from_min_max(
                        egui::pos2(rect.min.x, sel.min.y),
                        egui::pos2(sel.min.x, sel.max.y),
                    ),
                    0.0,
                    dim,
                );
                painter.rect_filled(
                    Rect::from_min_max(
                        egui::pos2(sel.max.x, sel.min.y),
                        egui::pos2(rect.max.x, sel.max.y),
                    ),
                    0.0,
                    dim,
                );
                painter.rect_stroke(
                    sel,
                    0.0,
                    egui::Stroke::new(2.0_f32, ACCENT),
                    egui::StrokeKind::Inside,
                );
            }

            ui.memory_mut(|m| m.data.insert_temp(egui::Id::new("crop_rect"), rect));
        });

        if cancel {
            self.crop = None;
        } else if apply {
            self.apply_crop(ctx);
        }
    }
}

// ---- Small UI helpers ------------------------------------------------------

/// The menu-bar (status bar) icon and its menu; returns the menu item ids
/// (open, clipboard, update, quit).
#[allow(clippy::type_complexity)]
fn build_tray() -> (
    Option<TrayIcon>,
    [String; 6],
    Option<CheckMenuItem>,
    Option<[MenuItem; 3]>,
) {
    let menu = Menu::new();
    // Their shortcuts are added by `apply_shortcuts`.
    let open_item = MenuItem::new("Open xpress", true, None);
    let clip_item = MenuItem::new("Optimise clipboard", true, None);
    let history_item = MenuItem::new("Clipboard history", true, None);
    let collect_item = CheckMenuItem::new("Collect clips", true, false, None);
    let update_item = MenuItem::new("Check for updates", true, None);
    let quit_item = MenuItem::new("Quit xpress", true, None);
    let _ = menu.append_items(&[
        &open_item,
        &PredefinedMenuItem::separator(),
        &clip_item,
        &history_item,
        &collect_item,
        &update_item,
        &PredefinedMenuItem::separator(),
        &quit_item,
    ]);
    let (open_id, clip_id, history_id, collect_id, update_id, quit_id) = (
        open_item.id().0.clone(),
        clip_item.id().0.clone(),
        history_item.id().0.clone(),
        collect_item.id().0.clone(),
        update_item.id().0.clone(),
        quit_item.id().0.clone(),
    );
    let tray = tray_icon_image().and_then(|(rgba, w, h)| {
        let icon = tray_icon::Icon::from_rgba(rgba, w, h).ok()?;
        TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_icon(icon)
            .with_tooltip("xpress")
            .build()
            .ok()
    });
    (
        tray,
        [open_id, clip_id, history_id, collect_id, update_id, quit_id],
        Some(collect_item),
        // In `Action` order.
        Some([clip_item, open_item, history_item]),
    )
}

/// egui's bundled fonts lack symbols we show (⇧ in hotkey hints, → in result
/// cards), which render as boxes. On macOS, add the system's Apple Symbols
/// font as a fallback for both families.
fn install_fallback_fonts(ctx: &egui::Context) {
    const APPLE_SYMBOLS: &str = "/System/Library/Fonts/Apple Symbols.ttf";
    let Ok(bytes) = std::fs::read(APPLE_SYMBOLS) else {
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "apple-symbols".into(),
        std::sync::Arc::new(egui::FontData::from_owned(bytes)),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("apple-symbols".into());
    }
    ctx.set_fonts(fonts);
}

fn install_style(ctx: &egui::Context) {
    use egui::{CornerRadius, Stroke};
    install_fallback_fonts(ctx);
    let mut style = (*ctx.global_style()).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.spacing.interact_size.y = 26.0;

    let mut v = egui::Visuals::dark();
    v.dark_mode = true;
    v.override_text_color = Some(TEXT);
    v.panel_fill = BG2;
    v.window_fill = PANEL;
    v.window_stroke = Stroke::new(1.0_f32, BORDER);
    v.window_corner_radius = CornerRadius::same(12);
    v.extreme_bg_color = BG; // text-edit background
    v.faint_bg_color = BG3;
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = ACCENT.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0_f32, ACCENT);

    let border = Stroke::new(1.0_f32, BORDER);
    let text = Stroke::new(1.0_f32, TEXT);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.bg_fill = BG3;
        w.weak_bg_fill = BG3;
        w.bg_stroke = border;
        w.fg_stroke = text;
        w.corner_radius = CornerRadius::same(8);
    }
    v.widgets.hovered.bg_fill = ACCENT2;
    v.widgets.hovered.weak_bg_fill = ACCENT2;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    v.widgets.active.bg_fill = ACCENT2;
    v.widgets.active.weak_bg_fill = ACCENT2;
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    v.widgets.noninteractive.bg_stroke = border;

    style.visuals = v;
    ctx.set_global_style(style);
}

fn sidebar_fill(_ctx: &egui::Context) -> Color32 {
    BG2
}

fn card_fill(_ctx: &egui::Context) -> Color32 {
    BG3
}

pub(crate) fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) {
    egui::Frame::default()
        .fill(card_fill(ui.ctx()))
        .corner_radius(10)
        .inner_margin(egui::Margin::same(14))
        .show(ui, add);
}

fn section_header(ui: &mut egui::Ui, text: &str) {
    ui.add_space(2.0);
    ui.label(RichText::new(text).size(10.5).color(TEXT_DIM).strong());
    ui.add_space(2.0);
}

/// Draw the colourful "x" logo (two crossing gradient-ish strokes) centred at
/// `center`, sized to fit a `size`×`size` box, on a transparent background.
fn draw_x_logo(painter: &egui::Painter, center: Pos2, size: f32) {
    let h = size * 0.34;
    let w = size * 0.2;
    let blue = ACCENT;
    let orange = Color32::from_rgb(0xff, 0x9e, 0x64);
    painter.line_segment(
        [center + egui::vec2(-h, -h), center + egui::vec2(h, h)],
        egui::Stroke::new(w, blue),
    );
    painter.line_segment(
        [center + egui::vec2(h, -h), center + egui::vec2(-h, h)],
        egui::Stroke::new(w, orange),
    );
}

/// A sidebar navigation row: colored icon tile + label, highlighted when active.
fn nav_item(ui: &mut egui::Ui, selected: bool, tile: Color32, icon: &str, label: &str) -> bool {
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.0), Sense::click());
    // Painted by hand, so describe it for screen readers (VoiceOver) as a
    // selectable item carrying its label.
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, label)
    });
    if selected {
        ui.painter().rect_filled(rect, 8.0, ACCENT2);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 8.0, BG3);
    }
    let tile_rect = Rect::from_min_size(
        egui::pos2(rect.min.x + 6.0, rect.center().y - 11.0),
        egui::vec2(22.0, 22.0),
    );
    ui.painter().rect_filled(tile_rect, 6.0, tile);
    ui.painter().text(
        tile_rect.center(),
        Align2::CENTER_CENTER,
        icon,
        FontId::proportional(13.0),
        Color32::WHITE,
    );
    let text_color = if selected {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().text_color()
    };
    ui.painter().text(
        egui::pos2(tile_rect.max.x + 10.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(14.0),
        text_color,
    );
    resp.clicked()
}

/// A macOS-style toggle switch.
fn toggle(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let size = egui::vec2(40.0, 22.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool(resp.id, *on);
    let off = Color32::from_gray(90);
    let bg = Color32::from_rgb(
        lerp_u8(off.r(), ACCENT.r(), t),
        lerp_u8(off.g(), ACCENT.g(), t),
        lerp_u8(off.b(), ACCENT.b(), t),
    );
    ui.painter().rect_filled(rect, 11.0, bg);
    let cx = egui::lerp((rect.left() + 11.0)..=(rect.right() - 11.0), t);
    ui.painter()
        .circle_filled(egui::pos2(cx, rect.center().y), 8.5, Color32::WHITE);
    resp
}

fn toggle_labeled(ui: &mut egui::Ui, on: &mut bool, label: &str) {
    ui.horizontal(|ui| {
        toggle(ui, on);
        ui.label(label);
    });
}

fn setting_row(ui: &mut egui::Ui, label: &str, desc: &str, control: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(label).strong());
            ui.label(RichText::new(desc).weak().small());
        });
        ui.with_layout(Layout::right_to_left(Align::Center), control);
    });
}

/// What the user asked for from a result card.
enum CardAction {
    Convert(PathBuf, ImageFormat),
    Crop(PathBuf),
}

/// The "Convert to" entries for a file currently in `current` format.
fn convert_menu(
    ui: &mut egui::Ui,
    path: &Path,
    current: Option<ImageFormat>,
    action: &mut Option<CardAction>,
) {
    for format in ImageFormat::convertible() {
        if Some(format) == current {
            continue;
        }
        if ui
            .button(format.label())
            .on_hover_text(format.description())
            .clicked()
        {
            *action = Some(CardAction::Convert(path.to_path_buf(), format));
            ui.close();
        }
    }
    ui.separator();
    ui.label(
        RichText::new("Saved next to it; the original is kept.")
            .weak()
            .small(),
    );
}

fn result_card(ui: &mut egui::Ui, card: &Card, index: usize) -> Option<CardAction> {
    let mut action = None;
    let image_out = card
        .output
        .as_deref()
        .filter(|p| xpress_core::filetype::classify(p) == Some(MediaKind::Image));
    let current = image_out.and_then(ImageFormat::of);
    // A clickable container (children stay on top and keep their clicks), so a
    // right-click anywhere on the card opens its menu.
    let resp = ui
        .scope_builder(
            egui::UiBuilder::new()
                .id_salt(("result_card", index))
                .sense(Sense::click()),
            |ui| {
                // Selectable labels would swallow the right-click meant for
                // the card's menu.
                ui.style_mut().interaction.selectable_labels = false;
                egui::Frame::default()
                    .fill(card_fill(ui.ctx()))
                    .corner_radius(10)
                    .inner_margin(egui::Margin::same(10))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if let Some(tex) = &card.texture {
                                // Fit a fixed 52×52 slot, so wide or tall
                                // images don't push the text around.
                                let (slot, _) =
                                    ui.allocate_exact_size(egui::vec2(52.0, 52.0), Sense::hover());
                                let s = tex.size_vec2();
                                let scale = (52.0 / s.x.max(1.0)).min(52.0 / s.y.max(1.0));
                                ui.painter().image(
                                    tex.id(),
                                    Rect::from_center_size(slot.center(), s * scale),
                                    Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
                                    Color32::WHITE,
                                );
                            } else {
                                let icon = if card.ok { "🗎" } else { "⚠" };
                                ui.label(RichText::new(icon).size(26.0));
                            }
                            ui.vertical(|ui| {
                                ui.label(RichText::new(&card.title).strong());
                                let color = if card.ok { OK_GREEN } else { ERR_RED };
                                ui.label(RichText::new(&card.detail).color(color).small());
                                if let Some(out) = &card.output {
                                    ui.horizontal(|ui| {
                                        if let Some(img) = image_out {
                                            let label =
                                                current.map(|f| f.label()).unwrap_or("Image");
                                            ui.menu_button(format!("{label} ▾"), |ui| {
                                                ui.label(RichText::new("Convert to").strong());
                                                convert_menu(ui, img, current, &mut action);
                                            })
                                            .response
                                            .on_hover_text("Convert to another format");
                                        }
                                        if ui.small_button("Reveal").clicked() {
                                            reveal_in_file_manager(out);
                                        }
                                        if ui.small_button("Copy").clicked() {
                                            xpress_core::clipboard::set_clipboard_png(out);
                                        }
                                    });
                                }
                            });
                            if card.ok && card.saved_pct > 0.0 {
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.label(
                                        RichText::new(format!("−{:.0}%", card.saved_pct))
                                            .size(18.0)
                                            .strong()
                                            .color(OK_GREEN),
                                    );
                                });
                            }
                        });
                    });
            },
        )
        .response;

    if let Some(out) = card.output.clone() {
        resp.context_menu(|ui| {
            if let Some(img) = image_out {
                ui.menu_button("Convert to", |ui| {
                    convert_menu(ui, img, current, &mut action);
                });
                if ui.button("Crop…").clicked() {
                    action = Some(CardAction::Crop(img.to_path_buf()));
                    ui.close();
                }
                ui.separator();
            }
            if ui.button("Show in Finder").clicked() {
                reveal_in_file_manager(&out);
                ui.close();
            }
            if ui.button("Copy").clicked() {
                xpress_core::clipboard::set_clipboard_png(&out);
                ui.close();
            }
        });
    }
    ui.add_space(6.0);
    action
}

/// The "Convert to" picker on the Optimise screen.
fn convert_picker(ui: &mut egui::Ui, value: &mut Option<ImageFormat>) {
    let selected = value.map(|f| f.label()).unwrap_or("Keep format");
    egui::ComboBox::from_id_salt("convert_to")
        .selected_text(selected)
        .width(150.0)
        // Tall enough to list every format without scrolling.
        .height(480.0)
        .show_ui(ui, |ui| {
            ui.selectable_value(value, None, "Keep format — just optimise");
            ui.separator();
            for format in ImageFormat::convertible() {
                ui.selectable_value(
                    value,
                    Some(format),
                    format!("{}  ·  {}", format.label(), format.description()),
                );
            }
        });
}

/// What happens to images converted to `format` (shown next to the picker).
fn convert_note(format: ImageFormat) -> String {
    let caveat = match format {
        ImageFormat::Jpeg => " · transparent areas become white",
        ImageFormat::Gif => " · limited to 256 colours",
        ImageFormat::Bmp => " · uncompressed, large files",
        ImageFormat::Png | ImageFormat::Tiff => " · lossless",
        _ => "",
    };
    format!(
        "Images are saved as {} next to the originals{caveat}",
        format.label()
    )
}

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t)
        .round()
        .clamp(0.0, 255.0) as u8
}

/// Bring the app to the front. For a menu-bar (agent) app this is needed so the
/// window actually appears in front when shown.
#[cfg(target_os = "macos")]
fn activate_app() {
    use objc2_foundation::MainThreadMarker;
    if let Some(mtm) = MainThreadMarker::new() {
        let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
    }
}

#[cfg(not(target_os = "macos"))]
fn activate_app() {}

/// Hide xpress so the app you were in comes back to the front, ready for ⌘V.
#[cfg(target_os = "macos")]
fn hide_app(_ctx: &egui::Context) {
    use objc2_foundation::MainThreadMarker;
    if let Some(mtm) = MainThreadMarker::new() {
        objc2_app_kit::NSApplication::sharedApplication(mtm).hide(None);
    }
}

#[cfg(not(target_os = "macos"))]
fn hide_app(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
}

/// Ask the Apple Intelligence helper whether the model can be used.
fn spawn_ai_check(
    slot: Arc<Mutex<Option<AiStatus>>>,
    checking: Arc<AtomicBool>,
    ctx: egui::Context,
) {
    if checking.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        *slot.lock().unwrap() = Some(xpress_core::intelligence::status());
        checking.store(false, Ordering::SeqCst);
        ctx.request_repaint();
    });
}

/// Pick an app in /Applications to ignore.
fn pick_app() -> Option<IgnoredApp> {
    let path = rfd::FileDialog::new()
        .set_directory("/Applications")
        .add_filter("Apps", &["app"])
        .pick_file()?;
    let name = path.file_stem()?.to_string_lossy().into_owned();
    Some(IgnoredApp {
        name,
        bundle: bundle_id(&path),
    })
}

#[cfg(target_os = "macos")]
fn bundle_id(app: &Path) -> Option<String> {
    use objc2_foundation::{NSBundle, NSString};
    let bundle = NSBundle::bundleWithPath(&NSString::from_str(&app.to_string_lossy()))?;
    bundle.bundleIdentifier().map(|id| id.to_string())
}

#[cfg(not(target_os = "macos"))]
fn bundle_id(_app: &Path) -> Option<String> {
    None
}

/// "Synced 2 min ago with MacBook Air", or what's wrong.
fn sync_status_line(status: &SyncStatus, now: i64) -> (String, Color32) {
    if let Some(error) = &status.error {
        return (error.clone(), ERR_RED);
    }
    let Some(last) = status.last else {
        return ("Syncing…".into(), TEXT_DIM);
    };
    let mut text = format!("Synced {}", crate::history_ui::ago(now, last));
    match status.devices.as_slice() {
        [] => text.push_str(" — no other Mac yet"),
        [one] => text.push_str(&format!(" with {one}")),
        many => text.push_str(&format!(" with {} Macs", many.len())),
    }
    if status.waiting > 0 {
        text.push_str(&format!(" · {} changes waiting for iCloud", status.waiting));
    }
    (text, TEXT_DIM)
}

/// "Invoices" exists already, rather than SQLite's wording.
fn category_error(e: &dyn std::fmt::Display, name: &str) -> String {
    let text = e.to_string();
    if text.contains("UNIQUE") {
        format!("There's already a category called “{name}”.")
    } else {
        text
    }
}

/// Build a 32×32 RGBA menu-bar icon: the colourful "x" on a transparent field.
fn tray_icon_image() -> Option<(Vec<u8>, u32, u32)> {
    let size = 32usize;
    let mut buf = vec![0u8; size * size * 4];
    let blue = [0x7a, 0xa2, 0xf7u8];
    let orange = [0xff, 0x9e, 0x64u8];
    let thickness = 3.2f32;
    let (lo, hi) = (6.0f32, (size as f32) - 6.0);
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32, y as f32);
            // distance to the two diagonals within the [lo, hi] box
            let in_box = fx >= lo - thickness && fx <= hi + thickness;
            let d1 = ((fx - fy).abs()) / std::f32::consts::SQRT_2; // line y = x
            let d2 = ((fx + fy) - (size as f32 - 1.0)).abs() / std::f32::consts::SQRT_2; // y = N-1-x
            let on1 = in_box && fy >= lo - thickness && fy <= hi + thickness && d1 <= thickness;
            let on2 = in_box && fy >= lo - thickness && fy <= hi + thickness && d2 <= thickness;
            let idx = (y * size + x) * 4;
            if on1 {
                buf[idx] = blue[0];
                buf[idx + 1] = blue[1];
                buf[idx + 2] = blue[2];
                buf[idx + 3] = 255;
            } else if on2 {
                buf[idx] = orange[0];
                buf[idx + 1] = orange[1];
                buf[idx + 2] = orange[2];
                buf[idx + 3] = 255;
            }
        }
    }
    Some((buf, size as u32, size as u32))
}

/// The `.app` bundle this executable lives in, if any (…/xpress.app).
fn app_bundle_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    exe.ancestors()
        .find(|a| a.extension().map(|e| e == "app").unwrap_or(false))
        .map(|p| p.to_path_buf())
}

/// True when we're running from an installed .app (so self-update makes sense).
fn can_self_update(info: &xpress_core::update::UpdateInfo) -> bool {
    cfg!(target_os = "macos") && info.download_url.is_some() && app_bundle_path().is_some()
}

/// Download the new .app zip, swap it into place, and relaunch. macOS only.
fn perform_self_update(url: &str, status: &Arc<Mutex<Option<String>>>) -> Result<(), String> {
    let old_app = app_bundle_path().ok_or("not running from an .app bundle")?;

    let bytes = xpress_core::update::download_verified(url)?;
    *status.lock().unwrap() = Some("Installing update…".into());

    let tmp = std::env::temp_dir().join(format!("xpress-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let zip = tmp.join("update.zip");
    std::fs::write(&zip, &bytes).map_err(|e| e.to_string())?;

    // Extract with ditto (preserves signatures/permissions).
    let extract = tmp.join("extract");
    std::fs::create_dir_all(&extract).map_err(|e| e.to_string())?;
    let ok = std::process::Command::new("ditto")
        .args(["-x", "-k"])
        .arg(&zip)
        .arg(&extract)
        .status()
        .map_err(|e| e.to_string())?
        .success();
    if !ok {
        return Err("could not extract update archive".into());
    }
    let new_app = std::fs::read_dir(&extract)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.extension().map(|e| e == "app").unwrap_or(false))
        .ok_or("no .app found in update archive")?;

    // A helper that waits for us to quit, swaps the bundle, and relaunches.
    let script = tmp.join("apply.sh");
    let script_body = format!(
        "#!/bin/bash\nset -e\nOLD={old}\nNEW={new}\nPID={pid}\n\
for i in $(seq 1 200); do kill -0 \"$PID\" 2>/dev/null || break; sleep 0.1; done\n\
rm -rf \"$OLD\"\nditto \"$NEW\" \"$OLD\"\nopen \"$OLD\"\n",
        old = shell_quote(&old_app),
        new = shell_quote(&new_app),
        pid = std::process::id(),
    );
    std::fs::write(&script, script_body).map_err(|e| e.to_string())?;

    *status.lock().unwrap() = Some("Restarting…".into());
    std::process::Command::new("/bin/bash")
        .arg(&script)
        .spawn()
        .map_err(|e| e.to_string())?;

    // Give the helper a moment to start, then quit so it can replace us.
    std::thread::sleep(Duration::from_millis(300));
    std::process::exit(0);
}

fn shell_quote(p: &Path) -> String {
    format!("'{}'", p.display().to_string().replace('\'', "'\\''"))
}

/// Run an update check on a background thread, storing the result (newer or not).
fn spawn_update_check(
    ctx: egui::Context,
    slot: Arc<Mutex<Option<xpress_core::update::UpdateInfo>>>,
    checking: Arc<AtomicBool>,
) {
    if checking.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        if let Ok(info) = xpress_core::update::check(env!("CARGO_PKG_VERSION")) {
            *slot.lock().unwrap() = Some(info);
        }
        checking.store(false, Ordering::SeqCst);
        ctx.request_repaint();
    });
}

fn reveal_in_file_manager(path: &Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer")
        .arg(format!("/select,{}", path.display()))
        .spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let dir = path.parent().unwrap_or(path);
        let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
    }
}

fn clipboard_dir() -> PathBuf {
    dirs_pictures().join("xpress")
}

fn dirs_pictures() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join("Pictures")
    } else {
        std::env::temp_dir()
    }
}

pub fn human(bytes: u64) -> String {
    xpress_core::result::human_size(bytes)
}

fn clipboard_image_to_file() -> Result<PathBuf, String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    let img = clipboard
        .get_image()
        .map_err(|_| "no image on the clipboard".to_string())?;
    let buf =
        image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned())
            .ok_or_else(|| "could not decode clipboard image".to_string())?;

    let dir = clipboard_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("clip-{ts}.png"));
    buf.save(&path).map_err(|e| e.to_string())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;
    use egui_kittest::Harness;

    fn harness() -> Harness<'static, XpressApp> {
        let app = XpressApp::build(&egui::Context::default(), false);
        let mut h = Harness::builder()
            .with_size(egui::vec2(960.0, 692.0))
            .build_ui_state(|ui, app: &mut XpressApp| app.draw(ui), app);
        h.run();
        h
    }

    #[test]
    fn sidebar_navigates_between_views() {
        let mut h = harness();
        assert!(
            h.query_by_label("Optimise clipboard").is_some(),
            "starts on Optimise"
        );

        h.get_by_label("Preferences").click();
        h.run();
        assert_eq!(h.state().tab, Tab::Settings);
        assert!(h.query_by_label("Quality target").is_some());
        assert!(h.query_by_label("Skip already-optimised files").is_some());
        assert!(h.query_by_label("Remove location").is_some());

        h.get_by_label("About").click();
        h.run();
        assert_eq!(h.state().tab, Tab::About);
        assert!(h.query_by_label("Developed by Knut W. Horne").is_some());

        h.get_by_label("Optimise").click();
        h.run();
        assert_eq!(h.state().tab, Tab::Optimise);
    }

    fn harness_with_settings(path: &Path) -> Harness<'static, XpressApp> {
        let app = XpressApp::build_with(&egui::Context::default(), false, Some(path.into()), None);
        let mut h = Harness::builder()
            .with_size(egui::vec2(960.0, 692.0))
            .build_ui_state(|ui, app: &mut XpressApp| app.draw(ui), app);
        h.run();
        h
    }

    #[test]
    fn settings_are_remembered_between_launches() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui.json");
        {
            let mut h = harness_with_settings(&path);
            let app = h.state_mut();
            app.factor = 70;
            app.backup = false;
            app.strip_location = true;
            app.quality_target = 2;
            app.convert_to = Some(ImageFormat::Jpeg);
            app.pipeline_dsl = "convert(to: avif)".into();
            app.use_pipeline = true;
            app.persist_settings(true);
        }
        let h = harness_with_settings(&path);
        let app = h.state();
        assert_eq!(app.factor, 70);
        assert!(!app.backup);
        assert!(app.strip_location);
        assert_eq!(app.quality(), Some(80.0));
        assert_eq!(app.convert_to, Some(ImageFormat::Jpeg));
        assert_eq!(app.pipeline_dsl, "convert(to: avif)");
        assert!(app.use_pipeline);
        assert!(
            h.query_by_label_contains("saved as JPEG").is_some(),
            "the Convert to picker shows the remembered format"
        );
    }

    #[test]
    fn settings_are_written_once_they_settle() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui.json");
        let mut h = harness_with_settings(&path);
        let app = h.state_mut();
        app.persist_settings(false);
        assert!(!path.exists(), "nothing changed, nothing written");

        app.factor = 55;
        app.persist_settings(false);
        assert!(!path.exists(), "waits for the slider to settle");

        app.settings_changed = Instant::now().checked_sub(SAVE_DELAY);
        app.persist_settings(false);
        let saved = Settings::load(&path, Settings::default);
        assert_eq!(saved.compression, 55);
        assert!(app.settings_changed.is_none());
    }

    #[test]
    fn tests_do_not_touch_the_real_settings() {
        let h = harness();
        assert!(h.state().settings_path.is_none());
        assert!(h.state().history.is_none());
    }

    /// An app with a history in a temp folder, holding a few clips.
    fn history_harness(dir: &Path) -> Harness<'static, XpressApp> {
        use xpress_core::history::NewClip;
        let history = History::open(&dir.join("history")).unwrap();
        for clip in [
            NewClip::text("Invoice 4711 is paid").from_app(Some("Mail".into()), None),
            NewClip::text("https://kwhorne.com/xpress").from_app(Some("Safari".into()), None),
            NewClip::text("#7aa2f7").from_app(Some("Figma".into()), None),
        ] {
            history.add(clip).unwrap();
            std::thread::sleep(Duration::from_millis(2));
        }
        drop(history);
        let mut app = XpressApp::build_with(
            &egui::Context::default(),
            false,
            Some(dir.join("gui.json")),
            Some(dir.join("history")),
        );
        app.tab = Tab::History;
        let mut h = Harness::builder()
            .with_size(egui::vec2(960.0, 692.0))
            .build_ui_state(|ui, app: &mut XpressApp| app.draw(ui), app);
        h.run();
        h
    }

    fn shown(h: &Harness<'static, XpressApp>) -> Vec<String> {
        h.state()
            .history_panel
            .results
            .iter()
            .map(|c| c.text.clone())
            .collect()
    }

    #[test]
    fn history_is_off_until_turned_on() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = history_harness(dir.path());
        assert!(!h.state().capture.enabled.load(Ordering::Relaxed));
        assert!(h.query_by_label("Clipboard history is off").is_some());
        h.get_by_label("Turn on clipboard history").click();
        h.run();
        assert!(h.state().history_enabled);
        assert!(h.state().capture.enabled.load(Ordering::Relaxed));
        assert!(h.query_by_label("Clipboard history is off").is_none());
    }

    #[test]
    fn history_lists_searches_and_filters() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = history_harness(dir.path());
        h.state_mut().history_enabled = true;
        h.run();
        assert_eq!(
            shown(&h),
            [
                "#7aa2f7",
                "https://kwhorne.com/xpress",
                "Invoice 4711 is paid"
            ]
        );
        assert!(h.query_by_label("Invoice 4711 is paid").is_some());
        assert!(h.query_by_label_contains("Link  ·  Safari").is_some());

        h.state_mut().history_panel.query = "inv".into();
        h.state_mut().history_panel.dirty = true;
        h.run();
        assert_eq!(shown(&h), ["Invoice 4711 is paid"]);

        h.state_mut().history_panel.query.clear();
        h.state_mut().history_panel.dirty = true;
        h.run();
        h.get_by_label("Links").click();
        h.run();
        assert_eq!(shown(&h), ["https://kwhorne.com/xpress"]);
        h.get_by_label("All").click();
        h.run();
        assert_eq!(shown(&h).len(), 3);
    }

    #[test]
    fn history_keyboard_copy_pin_and_delete() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = history_harness(dir.path());
        h.state_mut().history_enabled = true;
        h.run();
        let ids: Vec<i64> = h
            .state()
            .history_panel
            .results
            .iter()
            .map(|c| c.id)
            .collect();

        h.key_press(egui::Key::ArrowDown);
        h.run();
        assert_eq!(h.state().history_panel.selected, 1);
        h.key_press(egui::Key::Enter);
        h.run();
        assert_eq!(h.state().last_copied, Some(ids[1]));
        // Copying moves it to the top.
        h.run();
        assert_eq!(shown(&h)[0], "https://kwhorne.com/xpress");

        let invoice = ids[2];
        h.state_mut()
            .handle_history_action(HistoryAction::TogglePin(invoice), &egui::Context::default());
        h.run();
        h.get_by_label("Pinned").click();
        h.run();
        assert_eq!(shown(&h), ["Invoice 4711 is paid"]);

        h.state_mut().handle_history_action(
            HistoryAction::Delete(vec![invoice]),
            &egui::Context::default(),
        );
        h.run();
        assert!(shown(&h).is_empty());
    }

    fn enabled_history(dir: &Path) -> Harness<'static, XpressApp> {
        let mut h = history_harness(dir);
        h.state_mut().history_enabled = true;
        h.run();
        h
    }

    fn ids(h: &Harness<'static, XpressApp>) -> Vec<i64> {
        h.state()
            .history_panel
            .results
            .iter()
            .map(|c| c.id)
            .collect()
    }

    #[test]
    fn pick_several_and_copy_them_together() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = enabled_history(dir.path());
        let all = ids(&h);
        let cmd = egui::Modifiers {
            command: true,
            ..Default::default()
        };
        h.state_mut().history_panel.click(0, egui::Modifiers::NONE);
        h.state_mut().history_panel.click(2, cmd);
        h.run();
        assert!(h.query_by_label("2 selected").is_some());
        h.key_press(egui::Key::Enter);
        h.run();
        h.run();

        let multi = h.state().last_copied.unwrap();
        assert!(!all.contains(&multi), "a new multi-clip");
        let first = &h.state().history_panel.results[0];
        assert_eq!(
            (first.id, first.kind),
            (multi, xpress_core::history::ClipKind::Multi)
        );
        assert!(h.state().history_panel.marked.is_empty());
        assert!(h
            .query_by_label("#7aa2f7  ·  Invoice 4711 is paid")
            .is_some());
        assert!(h.query_by_label_contains("Multi-clip  ·").is_some());

        // Its items, and back.
        h.state_mut().history_panel.parent = Some(multi);
        h.state_mut().history_panel.dirty = true;
        h.run();
        assert_eq!(shown(&h), ["#7aa2f7", "Invoice 4711 is paid"]);
        h.get_by_label("← Back").click();
        h.run();
        h.run();
        assert_eq!(h.state().history_panel.results.len(), 4);
    }

    #[test]
    fn number_keys_copy_that_row() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = enabled_history(dir.path());
        let second = ids(&h)[1];
        h.key_press_modifiers(
            egui::Modifiers {
                command: true,
                ..Default::default()
            },
            egui::Key::Num2,
        );
        h.run();
        assert_eq!(h.state().last_copied, Some(second));
    }

    #[test]
    fn collecting_from_the_history_view() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = enabled_history(dir.path());
        h.get_by_label("⧉ Collect").click();
        h.run();
        assert!(h.state().collecting);
        assert!(h.state().capture.collecting.load(Ordering::Relaxed));
        assert!(h
            .query_by_label_contains("Collecting — everything you copy")
            .is_some());
        h.get_by_label("Done").click();
        h.run();
        assert!(!h.state().capture.collecting.load(Ordering::Relaxed));
    }

    #[test]
    fn categories_are_made_used_and_filtered() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = enabled_history(dir.path());
        h.get_by_label("+ Category").click();
        h.run();
        assert!(h.query_by_label("New category").is_some());
        h.state_mut().history_panel.editor.as_mut().unwrap().name = "Work".into();
        h.state_mut().history_panel.editor.as_mut().unwrap().app = Some("Safari".into());
        h.run();
        h.get_by_label("Save").click();
        h.run();
        h.run();
        assert!(h.state().history_panel.editor.is_none());
        let work = h.state().history_panel.categories[0].clone();
        assert_eq!(
            (work.name.as_str(), work.count),
            ("Work", 1),
            "the Safari link by rule"
        );

        // By hand, then filter.
        let invoice = h
            .state()
            .history_panel
            .results
            .iter()
            .find(|c| c.text.starts_with("Invoice"))
            .unwrap()
            .id;
        h.state_mut().handle_history_action(
            HistoryAction::SetCategory {
                clips: vec![invoice],
                category: work.id,
                on: true,
            },
            &egui::Context::default(),
        );
        h.run();
        h.get_by_label_contains("Work").click();
        h.run();
        h.run();
        assert_eq!(
            shown(&h),
            ["https://kwhorne.com/xpress", "Invoice 4711 is paid"]
        );

        // The same name again is refused, and the dialog stays open.
        h.state_mut().history_panel.editor = Some(Default::default());
        h.state_mut().history_panel.editor.as_mut().unwrap().name = "work".into();
        h.run();
        h.get_by_label("Save").click();
        h.run();
        h.run();
        assert!(h
            .query_by_label_contains("already a category called")
            .is_some());
    }

    #[test]
    fn apple_intelligence_result_can_be_saved() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = enabled_history(dir.path());
        *h.state().ai_status.lock().unwrap() = Some(AiStatus::Available);
        let invoice = h
            .state()
            .history_panel
            .results
            .iter()
            .find(|c| c.text.starts_with("Invoice"))
            .unwrap()
            .id;
        h.state_mut().handle_history_action(
            HistoryAction::Intelligence {
                id: invoice,
                task: xpress_core::intelligence::Task::Professional,
            },
            &egui::Context::default(),
        );
        // The spinner keeps repainting while it works.
        h.run_steps(3);
        assert!(h.query_by_label("Rewritten").is_some());
        assert!(h.query_by_label("Make professional…").is_some());

        // An answer to an older request is ignored; the current one is shown.
        let request = h.state().ai_request;
        let tx = h.state().tx.clone();
        tx.send(Msg::Ai {
            request: request - 1,
            result: Ok("stale".into()),
        })
        .unwrap();
        tx.send(Msg::Ai {
            request,
            result: Ok("Invoice 4711 has been paid.".into()),
        })
        .unwrap();
        h.state_mut().drain_results();
        h.run();
        assert!(h.query_by_label("Invoice 4711 has been paid.").is_some());
        h.get_by_label("Save to history").click();
        h.run();
        h.run();
        assert!(h.state().history_panel.ai.is_none());
        let first = &h.state().history_panel.results[0];
        assert_eq!(first.text, "Invoice 4711 has been paid.");
        assert_eq!(first.source_app.as_deref(), Some("Apple Intelligence"));
    }

    #[test]
    fn turning_sync_on_records_changes_and_starts_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = enabled_history(dir.path());
        assert!(!h
            .state()
            .history
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .journal());
        h.state_mut().history_sync = true;
        h.state_mut().sync_capture();
        let app = h.state();
        assert!(app.history.as_ref().unwrap().lock().unwrap().journal());
        assert!(app.capture.sync.load(Ordering::Relaxed));
        assert!(app.capture.sync_fresh.load(Ordering::Relaxed));
        h.state_mut().history_sync = false;
        h.state_mut().sync_capture();
        assert!(!h.state().capture.sync.load(Ordering::Relaxed));
    }

    #[test]
    fn sync_status_lines() {
        let min = 60_000;
        let mut s = SyncStatus::default();
        assert_eq!(sync_status_line(&s, 0).0, "Syncing…");
        s.last = Some(0);
        assert_eq!(
            sync_status_line(&s, 2 * min).0,
            "Synced 2 min ago — no other Mac yet"
        );
        s.devices = vec!["MacBook Air".into()];
        s.waiting = 3;
        assert_eq!(
            sync_status_line(&s, 0).0,
            "Synced just now with MacBook Air · 3 changes waiting for iCloud"
        );
        s.error = Some("iCloud Drive is off".into());
        assert_eq!(
            sync_status_line(&s, 0),
            ("iCloud Drive is off".into(), ERR_RED)
        );
    }

    fn preferences(dir: &Path) -> Harness<'static, XpressApp> {
        let mut h = harness_with_settings(&dir.join("gui.json"));
        h.state_mut().tab = Tab::Settings;
        h.set_size(egui::vec2(960.0, 2200.0));
        h.run();
        h
    }

    fn cmd_alt() -> egui::Modifiers {
        egui::Modifiers {
            alt: true,
            mac_cmd: true,
            command: true,
            ..Default::default()
        }
    }

    #[test]
    fn shortcuts_can_be_changed_turned_off_and_reset() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut h = preferences(dir.path());
            assert!(h.query_by_label("Shortcuts").is_some());
            assert!(
                h.query_by_label_contains("⇧⌘O clipboard").is_none(),
                "not on Optimise"
            );

            // Record ⌥⌘H for the history.
            h.get_by_label("⌃⌘V").click();
            h.run();
            assert_eq!(h.state().recording, Some(Action::History));
            assert!(h.query_by_label("Press keys…").is_some());
            assert!(
                h.state().registered.iter().all(Option::is_none),
                "paused while recording"
            );
            h.key_press_modifiers(cmd_alt(), egui::Key::H);
            h.run();
            h.run();
            assert_eq!(h.state().recording, None);
            assert_eq!(
                h.state().shortcuts[Action::History.index()],
                "alt+super+KeyH"
            );
            assert!(h.query_by_label("⌥⌘H").is_some());
            assert!(h.state().registered.iter().all(Option::is_some), "back on");

            // A plain key is refused; the same keys as another shortcut too.
            h.get_by_label("⇧⌘X").click();
            h.run();
            h.key_press(egui::Key::K);
            h.run();
            assert!(h.query_by_label("Use ⌘, ⌃ or ⌥ with the key.").is_some());
            assert_eq!(h.state().recording, Some(Action::Show));
            h.key_press_modifiers(cmd_alt(), egui::Key::H);
            h.run();
            assert!(h
                .query_by_label("Already used for “Clipboard history”.")
                .is_some());
            // ⌫ turns it off.
            h.key_press(egui::Key::Backspace);
            h.run();
            h.run();
            assert_eq!(h.state().shortcuts[Action::Show.index()], "");
            assert_eq!(h.state().registered[Action::Show.index()], None);
            assert!(h.query_by_label("Off").is_some());

            // Esc cancels a recording.
            h.get_by_label("⇧⌘O").click();
            h.run();
            h.key_press(egui::Key::Escape);
            h.run();
            assert_eq!(h.state().recording, None);
            assert_eq!(
                h.state().shortcuts[Action::Clipboard.index()],
                Action::Clipboard.default_shortcut()
            );
            h.state_mut().persist_settings(true);
        }

        // Remembered; Reset brings the default back.
        let mut h = preferences(dir.path());
        assert_eq!(
            h.state().shortcuts[Action::History.index()],
            "alt+super+KeyH"
        );
        assert_eq!(h.state().shortcuts[Action::Show.index()], "");
        assert_eq!(
            h.state().drop_hint(),
            "images · video · PDF · audio      ⇧⌘O clipboard",
            "the drop zone shows the shortcuts that are on"
        );
        let resets = h.query_all_by_label("Reset").count();
        assert_eq!(resets, 2);
        h.query_all_by_label("Reset").next().unwrap().click();
        h.run();
        h.run();
        assert_eq!(
            h.state().shortcuts[Action::Show.index()],
            Action::Show.default_shortcut()
        );
    }

    #[test]
    fn ignoring_an_app_from_a_clip_and_in_preferences() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut h = enabled_history(dir.path());
            let safari = h
                .state()
                .history_panel
                .results
                .iter()
                .find(|c| c.source_app.as_deref() == Some("Safari"))
                .unwrap()
                .id;
            assert!(!h
                .state()
                .capture
                .ignored
                .lock()
                .unwrap()
                .iter()
                .any(|i| i.name == "Safari"));
            let action = HistoryAction::IgnoreApp {
                name: "Safari".into(),
                bundle: None,
            };
            h.state_mut()
                .handle_history_action(action.clone(), &egui::Context::default());
            h.state_mut()
                .handle_history_action(action, &egui::Context::default());
            assert_eq!(h.state().history_ignored.len(), 1, "once");
            assert_eq!(h.state().capture.ignored.lock().unwrap()[0].name, "Safari");

            // Preferences list it, with its clips to delete (asking twice).
            h.state_mut().tab = Tab::Settings;
            h.set_size(egui::vec2(960.0, 2400.0));
            h.run();
            assert!(h.query_by_label("Ignore apps").is_some());
            h.get_by_label("Delete its 1 clip").click();
            h.run();
            h.get_by_label("Click again to delete").click();
            h.run();
            let h_ref = h.state().history.as_ref().unwrap().clone();
            assert!(h_ref.lock().unwrap().get(safari).unwrap().is_none());
            h.state_mut().persist_settings(true);
        }
        let mut h = enabled_history(dir.path());
        assert_eq!(h.state().history_ignored[0].name, "Safari");
        h.state_mut().tab = Tab::Settings;
        h.set_size(egui::vec2(960.0, 2400.0));
        h.run();
        h.get_by_label("×").click();
        h.run();
        assert!(h.state().history_ignored.is_empty());
        assert!(h.state().capture.ignored.lock().unwrap().is_empty());
    }

    #[test]
    fn history_settings_are_remembered_and_clear_asks_twice() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut h = history_harness(dir.path());
            h.state_mut().tab = Tab::Settings;
            h.state_mut().history_enabled = true;
            // Preferences scroll; show all of it.
            h.set_size(egui::vec2(960.0, 1800.0));
            h.state_mut().history_days = 7;
            h.state_mut().paste_directly = true;
            h.run();
            // The setting, and its shortcut.
            assert_eq!(h.query_all_by_label("Clipboard history").count(), 2);
            h.get_by_label("Clear…").click();
            h.run();
            assert_eq!(
                h.state()
                    .history
                    .as_ref()
                    .unwrap()
                    .lock()
                    .unwrap()
                    .stats()
                    .unwrap()
                    .0,
                3
            );
            h.get_by_label("Click again to clear").click();
            h.run();
            assert_eq!(
                h.state()
                    .history
                    .as_ref()
                    .unwrap()
                    .lock()
                    .unwrap()
                    .stats()
                    .unwrap()
                    .0,
                0
            );
            h.state_mut().persist_settings(true);
        }
        let h = history_harness(dir.path());
        assert!(h.state().history_enabled);
        assert_eq!(h.state().capture.keep_days.load(Ordering::Relaxed), 7);
        assert!(h.state().paste_directly);
    }

    #[test]
    fn quality_target_disables_the_slider_and_is_used() {
        let mut h = harness();
        h.state_mut().quality_target = 2;
        h.run();
        assert!(h.query_by_label_contains("quality target “High”").is_some());
        assert_eq!(h.state().quality(), Some(80.0));
        h.state_mut().skip_optimised = false;
        assert!(!h.state().options().use_cache);
    }

    #[test]
    fn crop_view_opens_and_cancels() {
        let dir = std::env::temp_dir().join(format!("xpress-gui-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let img = dir.join("pic.png");
        image::RgbImage::from_pixel(64, 48, image::Rgb([200, 100, 50]))
            .save(&img)
            .unwrap();

        let mut h = harness();
        let ctx = h.ctx.clone();
        h.state_mut().enter_crop(img, &ctx);
        h.run();
        assert!(h.state().crop.is_some());
        assert!(h.query_by_label("Apply crop").is_some());
        assert!(h.query_by_label("Drag to select a region.").is_some());

        h.get_by_label("Cancel").click();
        h.run();
        assert!(h.state().crop.is_none(), "back to the main view");
        assert!(h.query_by_label("Optimise clipboard").is_some());
    }

    fn temp_png(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xpress-gui-conv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        image::RgbaImage::from_fn(48, 32, |x, y| {
            image::Rgba([x as u8 * 5, y as u8 * 7, 90, 255])
        })
        .save(&path)
        .unwrap();
        path
    }

    /// Run frames and collect background results until a card arrives.
    fn wait_for_card(h: &mut Harness<'static, XpressApp>, cards: usize) {
        for _ in 0..200 {
            h.state_mut().drain_results();
            if h.state().cards.len() >= cards {
                h.run();
                return;
            }
            // A spinner keeps repainting while a job runs: step, don't run.
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        panic!("no result arrived");
    }

    #[test]
    fn convert_to_picker_converts_dropped_images() {
        let mut h = harness();
        // The combo box carries its selection as its value.
        h.get_by_value("Keep format").click();
        h.run();
        h.get_by_label_contains("JPEG  ·").click();
        h.run();
        assert_eq!(h.state().convert_to, Some(ImageFormat::Jpeg));
        assert!(h
            .query_by_label_contains("transparent areas become white")
            .is_some());

        let src = temp_png("dropped.png");
        let ctx = h.ctx.clone();
        h.state_mut().submit(src.clone(), &ctx);
        wait_for_card(&mut h, 1);

        let jpg = src.with_extension("jpg");
        assert!(jpg.exists(), "converted next to the original");
        assert!(src.exists(), "original kept");
        assert!(h.query_by_label("dropped.png → dropped.jpg").is_some());
    }

    #[test]
    fn card_format_chip_and_context_menu_convert() {
        let mut h = harness();
        let src = temp_png("card.png");
        h.state_mut().push_card(Card {
            title: "card.png".into(),
            detail: "1 KB → 1 KB".into(),
            saved_pct: 0.0,
            ok: true,
            output: Some(src.clone()),
            texture: None,
            pending_thumb: None,
        });
        h.run();

        // The chip shows the current format and offers the others.
        h.get_by_label("PNG ▾").click();
        h.run();
        assert!(h.query_by_label("JPEG").is_some());
        h.get_by_label("WebP").click();
        h.step();
        wait_for_card(&mut h, 2);
        assert!(src.with_extension("webp").exists());

        // Right-click on the card opens the same choices plus file actions.
        h.get_by_label("card.png").click_secondary();
        h.run();
        assert!(h.query_by_label("Convert to").is_some());
        assert!(h.query_by_label("Show in Finder").is_some());
        assert!(h.query_by_label("Crop…").is_some());
    }

    #[test]
    fn result_cards_show_cached_and_score() {
        let mut h = harness();
        h.state_mut().push_card(Card {
            title: "photo.jpg".into(),
            detail: "244.8 KB → 94.6 KB  ·  SSIMULACRA2 80".into(),
            saved_pct: 61.0,
            ok: true,
            output: None,
            texture: None,
            pending_thumb: None,
        });
        h.run();
        assert!(h.query_by_label("photo.jpg").is_some());
        assert!(h.query_by_label_contains("SSIMULACRA2 80").is_some());
    }
}
