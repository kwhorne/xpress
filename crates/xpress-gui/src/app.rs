//! The egui application: controls, drop zone, result cards, and the global hotkey.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

use eframe::egui;
use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};
use xpress_core::compression::{CompressionQuality, CompressionTier};
use xpress_core::result::OptimiseOptions;

use crate::work::{self, Msg};

struct Card {
    title: String,
    detail: String,
    saved_pct: f64,
    ok: bool,
    texture: Option<egui::TextureHandle>,
    pending_thumb: Option<egui::ColorImage>,
}

pub struct XpressApp {
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
}

impl XpressApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let (tx, rx) = channel();

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
        }
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
                    self.cards.insert(
                        0,
                        Card {
                            title: "Invalid pipeline".into(),
                            detail: e,
                            saved_pct: 0.0,
                            ok: false,
                            texture: None,
                            pending_thumb: None,
                        },
                    );
                }
            }
        } else {
            work::spawn(path, options, ctx.clone(), self.tx.clone());
        }
    }

    fn optimise_clipboard(&mut self, ctx: &egui::Context) {
        match clipboard_image_to_file() {
            Ok(path) => self.submit(path, ctx),
            Err(e) => self.cards.insert(
                0,
                Card {
                    title: "Clipboard".into(),
                    detail: e,
                    saved_pct: 0.0,
                    ok: false,
                    texture: None,
                    pending_thumb: None,
                },
            ),
        }
    }

    fn drain_results(&mut self) {
        while let Ok(Msg::Done(done)) = self.rx.try_recv() {
            self.in_flight = self.in_flight.saturating_sub(1);
            let name = done
                .source
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let card = match done.result {
                Ok(r) => Card {
                    title: name,
                    detail: format!(
                        "{} → {}{}",
                        human(r.old_size),
                        human(r.new_size),
                        if r.aggressive { "  (aggressive)" } else { "" }
                    ),
                    saved_pct: r.saved_percent(),
                    ok: true,
                    texture: None,
                    pending_thumb: done.thumbnail,
                },
                Err(e) => Card {
                    title: name,
                    detail: e,
                    saved_pct: 0.0,
                    ok: false,
                    texture: None,
                    pending_thumb: None,
                },
            };
            self.cards.insert(0, card);
            self.cards.truncate(50);
        }
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

        // Dropped files.
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

        self.draw_controls(ctx);
        self.draw_results(ctx);

        // Keep a steady repaint while work is in flight.
        if self.in_flight > 0 {
            ctx.request_repaint_after(std::time::Duration::from_millis(120));
        }
    }
}

impl XpressApp {
    fn draw_controls(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("controls").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.heading("xpress");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.in_flight > 0 {
                        ui.add(egui::Spinner::new());
                        ui.label(format!("{} working", self.in_flight));
                    }
                });
            });

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_enabled(
                    !self.aggressive,
                    egui::Slider::new(&mut self.factor, 5..=100).text("compression"),
                );
                ui.checkbox(&mut self.aggressive, "aggressive");
            });

            ui.horizontal(|ui| {
                ui.checkbox(&mut self.backup, "backup");
                ui.checkbox(&mut self.strip_metadata, "strip metadata");
                if ui
                    .checkbox(&mut self.always_on_top, "float on top")
                    .changed()
                {
                    let level = if self.always_on_top {
                        egui::WindowLevel::AlwaysOnTop
                    } else {
                        egui::WindowLevel::Normal
                    };
                    ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
                }
            });

            ui.horizontal(|ui| {
                ui.checkbox(&mut self.use_pipeline, "pipeline");
                ui.add_enabled(
                    self.use_pipeline,
                    egui::TextEdit::singleline(&mut self.pipeline_dsl)
                        .desired_width(f32::INFINITY)
                        .hint_text("crop(width: 1600) -> convert(to: webp)"),
                );
            });

            ui.horizontal(|ui| {
                if ui.button("Open files…").clicked() {
                    if let Some(paths) = rfd::FileDialog::new().pick_files() {
                        for p in paths {
                            self.submit(p, ctx);
                        }
                    }
                }
                if ui.button("Optimise clipboard").clicked() {
                    self.optimise_clipboard(ctx);
                }
                if !self.cards.is_empty() && ui.button("Clear").clicked() {
                    self.cards.clear();
                }
            });
            ui.add_space(6.0);
        });
    }

    fn draw_results(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.cards.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        egui::RichText::new("Drop images, videos, PDFs or audio here\n\n⌘⇧O optimises the clipboard image")
                            .size(15.0)
                            .weak(),
                    );
                });
                return;
            }

            egui::ScrollArea::vertical().show(ui, |ui| {
                for card in &mut self.cards {
                    // Realise any pending thumbnail into a texture (must happen with ctx).
                    if card.texture.is_none() {
                        if let Some(image) = card.pending_thumb.take() {
                            card.texture = Some(ctx.load_texture(
                                "thumb",
                                image,
                                egui::TextureOptions::LINEAR,
                            ));
                        }
                    }

                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if let Some(tex) = &card.texture {
                                let size = tex.size_vec2();
                                let scale = 56.0 / size.y.max(1.0);
                                ui.image((tex.id(), size * scale));
                            } else {
                                let icon = if card.ok { "🗎" } else { "⚠" };
                                ui.label(egui::RichText::new(icon).size(28.0));
                            }
                            ui.vertical(|ui| {
                                ui.label(egui::RichText::new(&card.title).strong());
                                let color = if card.ok {
                                    egui::Color32::from_rgb(60, 160, 90)
                                } else {
                                    egui::Color32::from_rgb(190, 80, 80)
                                };
                                ui.label(egui::RichText::new(&card.detail).color(color));
                            });
                            if card.ok && card.saved_pct > 0.0 {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            egui::RichText::new(format!("-{:.0}%", card.saved_pct))
                                                .size(18.0)
                                                .strong()
                                                .color(egui::Color32::from_rgb(60, 160, 90)),
                                        );
                                    },
                                );
                            }
                        });
                    });
                }
            });
        });
    }
}

/// Read an image from the clipboard, encode it to PNG in `~/Pictures/xpress`,
/// and return the path. Uses the `image` crate (no ffmpeg needed).
fn clipboard_image_to_file() -> Result<PathBuf, String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    let img = clipboard
        .get_image()
        .map_err(|_| "no image on the clipboard".to_string())?;
    let buf =
        image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned())
            .ok_or_else(|| "could not decode clipboard image".to_string())?;

    let dir = dirs_pictures().join("xpress");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("clip-{ts}.png"));
    buf.save(&path).map_err(|e| e.to_string())?;
    Ok(path)
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
