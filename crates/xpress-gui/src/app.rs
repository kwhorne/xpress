//! The egui application: a sidebar-driven UI with an Optimise view, Settings,
//! an About view, an interactive crop tool, and a global hotkey.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use egui::{Align, Align2, Color32, FontId, Layout, Pos2, Rect, RichText, Sense, Vec2};
use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};
use xpress_core::compression::{CompressionQuality, CompressionTier};
use xpress_core::result::OptimiseOptions;

use crate::work::{self, Msg};

// Palette matching elyra-conductor (Tokyo Night).
const BG: Color32 = Color32::from_rgb(0x16, 0x16, 0x1e);
const BG2: Color32 = Color32::from_rgb(0x1a, 0x1b, 0x26);
const BG3: Color32 = Color32::from_rgb(0x1f, 0x20, 0x30);
const PANEL: Color32 = Color32::from_rgb(0x1e, 0x1f, 0x2b);
const BORDER: Color32 = Color32::from_rgb(0x2a, 0x2b, 0x3c);
const TEXT: Color32 = Color32::from_rgb(0xc0, 0xca, 0xf5);
const TEXT_DIM: Color32 = Color32::from_rgb(0x78, 0x7c, 0x99);
const ACCENT: Color32 = Color32::from_rgb(0x7a, 0xa2, 0xf7);
const ACCENT2: Color32 = Color32::from_rgb(0x2f, 0x36, 0x50);
const OK_GREEN: Color32 = Color32::from_rgb(0x9e, 0xce, 0x6a);
const ERR_RED: Color32 = Color32::from_rgb(0xf7, 0x76, 0x8e);

const UPDATE_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Optimise,
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
    always_on_top: bool,
    pipeline_dsl: String,
    use_pipeline: bool,

    cards: Vec<Card>,
    in_flight: usize,

    tx: Sender<Msg>,
    rx: Receiver<Msg>,

    _hotkey_manager: Option<GlobalHotKeyManager>,
    clipboard_hotkey: Option<HotKey>,

    crop: Option<CropState>,

    update_info: Arc<Mutex<Option<xpress_core::update::UpdateInfo>>>,
    update_checking: Arc<AtomicBool>,
    last_update_check: Instant,
    update_dismissed: bool,
    updating: Arc<AtomicBool>,
    update_status: Arc<Mutex<Option<String>>>,
}

impl XpressApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_style(&cc.egui_ctx);
        let (tx, rx) = channel();

        let update_info: Arc<Mutex<Option<xpress_core::update::UpdateInfo>>> =
            Arc::new(Mutex::new(None));
        let update_checking = Arc::new(AtomicBool::new(false));
        spawn_update_check(
            cc.egui_ctx.clone(),
            update_info.clone(),
            update_checking.clone(),
        );

        // Register Cmd/Ctrl + Shift + O to optimise the clipboard image.
        let (manager, hotkey) = match GlobalHotKeyManager::new() {
            Ok(m) => {
                let hk = HotKey::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::KeyO);
                match m.register(hk) {
                    Ok(()) => (Some(m), Some(hk)),
                    Err(_) => (Some(m), None),
                }
            }
            Err(_) => (None, None),
        };

        Self {
            tab: Tab::Optimise,
            factor: xpress_core::compression::COMPRESSION_FACTOR_NORMAL,
            aggressive: false,
            backup: true,
            strip_metadata: false,
            always_on_top: false,
            pipeline_dsl: "crop(longEdge: 2000) -> convert(to: webp)".to_string(),
            use_pipeline: false,
            cards: Vec::new(),
            in_flight: 0,
            tx,
            rx,
            _hotkey_manager: manager,
            clipboard_hotkey: hotkey,
            crop: None,
            update_info,
            update_checking,
            last_update_check: Instant::now(),
            update_dismissed: false,
            updating: Arc::new(AtomicBool::new(false)),
            update_status: Arc::new(Mutex::new(None)),
        }
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
            preserve_dates: true,
            output: None,
            allow_larger: false,
        }
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
        if self.use_pipeline {
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
            work::spawn(path, options, ctx.clone(), self.tx.clone());
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
        while let Ok(Msg::Done(done)) = self.rx.try_recv() {
            self.in_flight = self.in_flight.saturating_sub(1);
            let name = done
                .source
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let from_clipboard = done.source.starts_with(clipboard_dir());
            let card = match done.result {
                Ok(r) => {
                    let mut detail = format!(
                        "{} → {}{}",
                        human(r.old_size),
                        human(r.new_size),
                        if r.aggressive { "  ·  aggressive" } else { "" }
                    );
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
        let Ok(img) = image::open(&path) else { return };
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
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Global hotkey events.
        if let Some(hk) = self.clipboard_hotkey {
            while let Ok(ev) = GlobalHotKeyEvent::receiver().try_recv() {
                if ev.id == hk.id() && ev.state == HotKeyState::Pressed {
                    self.optimise_clipboard(ctx);
                }
            }
        }

        self.drain_results();

        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        for path in dropped {
            self.submit(path, ctx);
        }

        if self.last_update_check.elapsed() >= UPDATE_INTERVAL
            && !self.update_checking.load(Ordering::Relaxed)
        {
            self.check_for_updates(ctx);
        }

        self.draw_update_banner(ctx);

        if self.crop.is_some() {
            self.draw_crop(ctx);
        } else {
            self.draw_sidebar(ctx);
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::central_panel(&ctx.style())
                        .fill(BG)
                        .inner_margin(egui::Margin::same(22)),
                )
                .show(ctx, |ui| match self.tab {
                    Tab::Optimise => self.optimise_view(ui),
                    Tab::Settings => self.settings_view(ui),
                    Tab::About => self.about_view(ui),
                });
        }

        if self.in_flight > 0 {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
    }
}

impl XpressApp {
    // ---- Sidebar -----------------------------------------------------------

    fn draw_sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("sidebar")
            .exact_width(212.0)
            .resizable(false)
            .frame(
                egui::Frame::default()
                    .fill(sidebar_fill(ctx))
                    .inner_margin(egui::Margin::symmetric(12, 16)),
            )
            .show(ctx, |ui| {
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
                    false,
                    Color32::from_rgb(90, 140, 240),
                    "⛶",
                    "Crop image…",
                ) {
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter(
                            "images",
                            &["png", "jpg", "jpeg", "webp", "gif", "bmp", "tiff"],
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
            egui::Stroke::new(2.0, ACCENT)
        } else {
            egui::Stroke::new(1.5, ui.visuals().widgets.noninteractive.bg_stroke.color)
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
        ui.painter().text(
            rect.center() + egui::vec2(0.0, 14.0),
            Align2::CENTER_CENTER,
            "images · video · PDF · audio      ⌘⇧O for clipboard",
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
                ui.add_enabled(
                    !self.aggressive,
                    egui::Slider::new(&mut self.factor, 5..=100).show_value(true),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    toggle_labeled(ui, &mut self.aggressive, "Aggressive");
                });
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
        egui::ScrollArea::vertical().show(ui, |ui| {
            for card in &mut self.cards {
                if card.texture.is_none() {
                    if let Some(image) = card.pending_thumb.take() {
                        card.texture = Some(ui.ctx().load_texture(
                            "thumb",
                            image,
                            egui::TextureOptions::LINEAR,
                        ));
                    }
                }
                result_card(ui, card);
            }
        });
    }

    // ---- Settings view -----------------------------------------------------

    fn settings_view(&mut self, ui: &mut egui::Ui) {
        ui.heading("Preferences");
        ui.label(RichText::new("Defaults applied to every optimisation.").weak());
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

    fn draw_update_banner(&mut self, ctx: &egui::Context) {
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
        egui::TopBottomPanel::top("update_banner")
            .frame(
                egui::Frame::default()
                    .fill(ACCENT)
                    .inner_margin(egui::Margin::symmetric(12, 7)),
            )
            .show(ctx, |ui| {
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

    fn draw_crop(&mut self, ctx: &egui::Context) {
        let mut apply = false;
        let mut cancel = false;
        egui::TopBottomPanel::top("crop_top").show(ctx, |ui| {
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

        egui::CentralPanel::default().show(ctx, |ui| {
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
                    egui::Stroke::new(2.0, ACCENT),
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

fn install_style(ctx: &egui::Context) {
    use egui::{CornerRadius, Stroke};
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.spacing.interact_size.y = 26.0;

    let mut v = egui::Visuals::dark();
    v.dark_mode = true;
    v.override_text_color = Some(TEXT);
    v.panel_fill = BG2;
    v.window_fill = PANEL;
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_corner_radius = CornerRadius::same(12);
    v.extreme_bg_color = BG; // text-edit background
    v.faint_bg_color = BG3;
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = ACCENT.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, ACCENT);

    let border = Stroke::new(1.0, BORDER);
    let text = Stroke::new(1.0, TEXT);
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
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.active.bg_fill = ACCENT2;
    v.widgets.active.weak_bg_fill = ACCENT2;
    v.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.noninteractive.bg_stroke = border;

    style.visuals = v;
    ctx.set_style(style);
}

fn sidebar_fill(_ctx: &egui::Context) -> Color32 {
    BG2
}

fn card_fill(_ctx: &egui::Context) -> Color32 {
    BG3
}

fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) {
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

fn result_card(ui: &mut egui::Ui, card: &Card) {
    egui::Frame::default()
        .fill(card_fill(ui.ctx()))
        .corner_radius(10)
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(tex) = &card.texture {
                    let s = tex.size_vec2();
                    let scale = 52.0 / s.y.max(1.0);
                    ui.image((tex.id(), s * scale));
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
    ui.add_space(6.0);
}

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t)
        .round()
        .clamp(0.0, 255.0) as u8
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

    let bytes = xpress_core::update::download(url)?;
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

fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
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
