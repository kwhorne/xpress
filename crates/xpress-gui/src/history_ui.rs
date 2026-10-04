//! The History view: search, filters and the list of clips.

use std::collections::HashMap;
use std::path::PathBuf;

use eframe::egui;
use egui::{Align, Color32, FontId, Key, Layout, Rect, RichText, Sense, Vec2};
use xpress_core::history::{parse_color, Clip, ClipKind, History, Query};

const ROW_HEIGHT: f32 = 62.0;
const PREVIEW: f32 = 46.0;

/// What the user asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum HistoryAction {
    /// Put the clip on the clipboard; `hide` when chosen with Enter or a
    /// double-click (so you can paste straight away).
    Copy {
        id: i64,
        hide: bool,
    },
    /// Copy some text (e.g. the text recognised in an image).
    CopyText(String),
    TogglePin(i64),
    Delete(i64),
    Reveal(PathBuf),
    Open(String),
    Enable,
    Hide,
}

#[derive(Default)]
pub struct HistoryPanel {
    pub query: String,
    pub kind: Option<ClipKind>,
    pub pinned_only: bool,
    pub app: Option<String>,
    pub results: Vec<Clip>,
    pub apps: Vec<(String, usize)>,
    pub stats: (usize, u64),
    pub selected: usize,
    /// Re-run the query on the next frame.
    pub dirty: bool,
    /// Put the cursor in the search field on the next frame.
    pub focus_search: bool,
    textures: HashMap<i64, Option<egui::TextureHandle>>,
    error: Option<String>,
}

impl HistoryPanel {
    pub fn new() -> Self {
        Self {
            dirty: true,
            ..Default::default()
        }
    }

    pub fn refresh(&mut self, history: &History) {
        let query = Query {
            text: self.query.clone(),
            kind: self.kind,
            app: self.app.clone(),
            pinned_only: self.pinned_only,
            limit: 500,
        };
        match history.search(&query) {
            Ok(results) => {
                self.results = results;
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
        self.apps = history.apps().unwrap_or_default();
        self.stats = history.stats().unwrap_or_default();
        self.selected = self.selected.min(self.results.len().saturating_sub(1));
        let ids: Vec<i64> = self.results.iter().map(|c| c.id).collect();
        self.textures.retain(|id, _| ids.contains(id));
        self.dirty = false;
    }

    /// Start over: empty search, first clip selected, cursor in the field.
    pub fn reset_for_paste(&mut self) {
        self.query.clear();
        self.selected = 0;
        self.dirty = true;
        self.focus_search = true;
    }

    pub fn show(&mut self, ui: &mut egui::Ui, enabled: bool, now_ms: i64) -> Option<HistoryAction> {
        let mut action = None;
        ui.heading("History");
        ui.label(
            RichText::new("Everything you copy and every screenshot — search, then copy it back.")
                .weak(),
        );
        ui.add_space(12.0);

        if !enabled {
            crate::app::card(ui, |ui| {
                ui.label(RichText::new("Clipboard history is off").strong());
                ui.label(
                    RichText::new(
                        "Turn it on to keep what you copy and your screenshots, and find them \
                         again — also by the text inside images. Everything stays on this Mac; \
                         passwords and other private clipboard content are never saved.",
                    )
                    .weak()
                    .small(),
                );
                ui.add_space(8.0);
                if ui.button("Turn on clipboard history").clicked() {
                    action = Some(HistoryAction::Enable);
                }
            });
            ui.add_space(12.0);
            if self.stats.0 == 0 {
                return action;
            }
        }

        // Search, the app filter, and the kind filters.
        let app_filter_width = if self.apps.is_empty() { 0.0 } else { 170.0 };
        let mut search = None;
        ui.horizontal(|ui| {
            search = Some(
                ui.add(
                    egui::TextEdit::singleline(&mut self.query)
                        .hint_text("Search text, links, files and words in images…")
                        .desired_width(ui.available_width() - app_filter_width),
                ),
            );
            if !self.apps.is_empty() {
                egui::ComboBox::from_id_salt("history_app")
                    .selected_text(self.app.as_deref().unwrap_or("All apps"))
                    .width(app_filter_width - 16.0)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(self.app.is_none(), "All apps")
                            .clicked()
                        {
                            self.app = None;
                            self.dirty = true;
                        }
                        for (app, count) in &self.apps {
                            let on = self.app.as_deref() == Some(app.as_str());
                            if ui
                                .selectable_label(on, format!("{app}  ({count})"))
                                .clicked()
                            {
                                self.app = Some(app.clone());
                                self.dirty = true;
                            }
                        }
                    });
            }
        });
        let search = search.expect("search field");
        if self.focus_search {
            search.request_focus();
            self.focus_search = false;
        }
        if search.changed() {
            self.selected = 0;
            self.dirty = true;
        }
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let chip = |ui: &mut egui::Ui, on: bool, label: &str| {
                ui.add(egui::Button::selectable(on, label)).clicked()
            };
            if chip(ui, self.kind.is_none() && !self.pinned_only, "All") {
                self.kind = None;
                self.pinned_only = false;
                self.dirty = true;
            }
            if chip(ui, self.pinned_only, "Pinned") {
                self.pinned_only = !self.pinned_only;
                self.dirty = true;
            }
            for kind in ClipKind::ALL {
                if chip(ui, self.kind == Some(kind), kind.label()) {
                    self.kind = (self.kind != Some(kind)).then_some(kind);
                    self.dirty = true;
                }
            }
        });
        ui.add_space(8.0);

        if let Some(e) = &self.error {
            ui.colored_label(crate::app::ERR_RED, e);
        }

        // Keyboard: ↑/↓ choose, Enter copies, Esc clears or hides.
        let n = self.results.len();
        let (down, up, enter, escape) = ui.input(|i| {
            (
                i.key_pressed(Key::ArrowDown),
                i.key_pressed(Key::ArrowUp),
                i.key_pressed(Key::Enter),
                i.key_pressed(Key::Escape),
            )
        });
        let mut scroll_to_selected = false;
        if n > 0 && down {
            self.selected = (self.selected + 1).min(n - 1);
            scroll_to_selected = true;
        }
        if n > 0 && up {
            self.selected = self.selected.saturating_sub(1);
            scroll_to_selected = true;
        }
        if enter {
            if let Some(clip) = self.results.get(self.selected) {
                action = Some(HistoryAction::Copy {
                    id: clip.id,
                    hide: true,
                });
            }
        }
        if escape {
            if self.query.is_empty() {
                action = Some(HistoryAction::Hide);
            } else {
                self.query.clear();
                self.dirty = true;
            }
        }

        if n == 0 {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                let empty = if self.stats.0 == 0 {
                    "Nothing yet — copy something or take a screenshot."
                } else {
                    "No clips match."
                };
                ui.label(RichText::new(empty).weak());
            });
            return action;
        }

        let footer = format!(
            "{} clips · {}   ↑↓ choose · ⏎ copy · esc close",
            self.stats.0,
            crate::app::human(self.stats.1)
        );
        let list_height = ui.available_height() - 22.0;
        egui::ScrollArea::vertical()
            .max_height(list_height)
            .auto_shrink([false, false])
            .show_rows(ui, ROW_HEIGHT, n, |ui, range| {
                ui.style_mut().interaction.selectable_labels = false;
                for i in range {
                    let clip = self.results[i].clone();
                    let selected = i == self.selected;
                    let (rect, resp) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), ROW_HEIGHT),
                        Sense::click(),
                    );
                    if selected && scroll_to_selected {
                        ui.scroll_to_rect(rect, None);
                    }
                    if let Some(a) = self.row(ui, rect, &resp, &clip, selected, now_ms) {
                        action = Some(a);
                    }
                    if resp.clicked() {
                        self.selected = i;
                    }
                    if resp.double_clicked() {
                        action = Some(HistoryAction::Copy {
                            id: clip.id,
                            hide: true,
                        });
                    }
                    resp.context_menu(|ui| {
                        if let Some(a) = context_menu(ui, &clip) {
                            action = Some(a);
                        }
                    });
                }
            });
        ui.label(RichText::new(footer).weak().small());
        action
    }

    fn row(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        resp: &egui::Response,
        clip: &Clip,
        selected: bool,
        now_ms: i64,
    ) -> Option<HistoryAction> {
        let fill = if selected {
            crate::app::ACCENT2
        } else if resp.hovered() {
            crate::app::BG3
        } else {
            Color32::TRANSPARENT
        };
        let row = rect.shrink2(Vec2::new(0.0, 3.0));
        ui.painter().rect_filled(row, 8.0, fill);

        // Preview.
        let preview = Rect::from_min_size(
            egui::pos2(row.min.x + 8.0, row.center().y - PREVIEW / 2.0),
            Vec2::splat(PREVIEW),
        );
        self.paint_preview(ui, preview, clip);

        let mut action = None;
        let content = Rect::from_min_max(
            egui::pos2(preview.max.x + 12.0, row.min.y + 6.0),
            egui::pos2(row.max.x - 8.0, row.max.y - 6.0),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .small_button("Copy")
                        .on_hover_text("Copy to the clipboard")
                        .clicked()
                    {
                        action = Some(HistoryAction::Copy {
                            id: clip.id,
                            hide: false,
                        });
                    }
                    let star = if clip.pinned { "★" } else { "☆" };
                    if ui
                        .small_button(star)
                        .on_hover_text(if clip.pinned {
                            "Unpin"
                        } else {
                            "Pin — keep forever"
                        })
                        .clicked()
                    {
                        action = Some(HistoryAction::TogglePin(clip.id));
                    }
                    ui.with_layout(Layout::top_down(Align::LEFT), |ui| {
                        ui.add(egui::Label::new(RichText::new(title(clip)).strong()).truncate());
                        ui.add(
                            egui::Label::new(RichText::new(subtitle(clip, now_ms)).weak().small())
                                .truncate(),
                        );
                    });
                });
            });
        });
        action
    }

    fn paint_preview(&mut self, ui: &egui::Ui, rect: Rect, clip: &Clip) {
        let painter = ui.painter();
        if clip.kind == ClipKind::Color {
            if let Some([r, g, b, a]) = parse_color(&clip.text) {
                painter.rect_filled(rect, 8.0, Color32::from_rgba_unmultiplied(r, g, b, a));
                painter.rect_stroke(
                    rect,
                    8.0,
                    egui::Stroke::new(1.0, crate::app::BORDER),
                    egui::StrokeKind::Inside,
                );
                return;
            }
        }
        if let Some(texture) = self.texture(ui.ctx(), clip) {
            let size = texture.size_vec2();
            let scale = (PREVIEW / size.x).min(PREVIEW / size.y);
            let fit = Rect::from_center_size(rect.center(), size * scale);
            painter.rect_filled(rect, 8.0, crate::app::BG3);
            painter.image(
                texture.id(),
                fit,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            return;
        }
        let (glyph, tile) = match clip.kind {
            ClipKind::Link => ("↗", Color32::from_rgb(90, 140, 240)),
            ClipKind::Code => ("{ }", Color32::from_rgb(155, 110, 230)),
            ClipKind::Files => ("▤", Color32::from_rgb(230, 160, 70)),
            ClipKind::Image | ClipKind::Screenshot => ("▣", Color32::from_rgb(90, 180, 140)),
            _ => ("T", Color32::from_rgb(110, 115, 135)),
        };
        painter.rect_filled(rect, 8.0, tile);
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            glyph,
            FontId::proportional(18.0),
            Color32::WHITE,
        );
    }

    fn texture(&mut self, ctx: &egui::Context, clip: &Clip) -> Option<egui::TextureHandle> {
        let thumb = clip.thumb.as_ref()?;
        self.textures
            .entry(clip.id)
            .or_insert_with(|| {
                let img = image::open(thumb).ok()?.to_rgba8();
                let (w, h) = img.dimensions();
                let color = egui::ColorImage::from_rgba_unmultiplied(
                    [w as usize, h as usize],
                    img.as_raw(),
                );
                Some(ctx.load_texture(
                    format!("clip-{}", clip.id),
                    color,
                    egui::TextureOptions::LINEAR,
                ))
            })
            .clone()
    }
}

fn context_menu(ui: &mut egui::Ui, clip: &Clip) -> Option<HistoryAction> {
    let mut action = None;
    if ui.button("Copy").clicked() {
        action = Some(HistoryAction::Copy {
            id: clip.id,
            hide: false,
        });
    }
    if !clip.ocr.is_empty() && ui.button("Copy text in image").clicked() {
        action = Some(HistoryAction::CopyText(clip.ocr.clone()));
    }
    if clip.kind == ClipKind::Link && ui.button("Open link").clicked() {
        action = Some(HistoryAction::Open(clip.text.trim().to_string()));
    }
    let reveal = clip
        .paths()
        .into_iter()
        .next()
        .or_else(|| clip.image.clone());
    if let Some(path) = reveal {
        if ui.button("Show in Finder").clicked() {
            action = Some(HistoryAction::Reveal(path));
        }
    }
    if ui
        .button(if clip.pinned { "Unpin" } else { "Pin" })
        .clicked()
    {
        action = Some(HistoryAction::TogglePin(clip.id));
    }
    ui.separator();
    if ui.button("Delete").clicked() {
        action = Some(HistoryAction::Delete(clip.id));
    }
    if action.is_some() {
        ui.close();
    }
    action
}

/// The main line of a row.
pub fn title(clip: &Clip) -> String {
    let first_line = |s: &str| -> String {
        let line = s
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("");
        let mut out: String = line.chars().take(140).collect();
        if line.chars().count() > 140 {
            out.push('…');
        }
        out
    };
    match clip.kind {
        ClipKind::Files => {
            let names: Vec<String> = clip
                .paths()
                .iter()
                .map(|p| {
                    p.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| p.display().to_string())
                })
                .collect();
            first_line(&names.join(", "))
        }
        ClipKind::Image | ClipKind::Screenshot if !clip.ocr.is_empty() => {
            format!("“{}”", first_line(&clip.ocr))
        }
        ClipKind::Screenshot if !clip.text.is_empty() => first_line(&clip.text),
        ClipKind::Image | ClipKind::Screenshot => "Image".into(),
        _ => first_line(&clip.text),
    }
}

/// The detail line: kind · app · when · size.
pub fn subtitle(clip: &Clip, now_ms: i64) -> String {
    let kind = match clip.kind {
        ClipKind::Text => "Text",
        ClipKind::Link => "Link",
        ClipKind::Code => "Code",
        ClipKind::Color => "Colour",
        ClipKind::Image => "Image",
        ClipKind::Screenshot => "Screenshot",
        ClipKind::Files => "Files",
    };
    let mut parts = vec![kind.to_string()];
    if let Some(app) = &clip.source_app {
        if clip.kind != ClipKind::Screenshot {
            parts.push(app.clone());
        }
    }
    parts.push(ago(now_ms, clip.last_used));
    if clip.kind.is_image() {
        parts.push(crate::app::human(clip.bytes));
    }
    if clip.kind == ClipKind::Files {
        let n = clip.paths().len();
        if n > 1 {
            parts.push(format!("{n} items"));
        }
    }
    parts.join("  ·  ")
}

/// "just now", "5 min ago", "3 h ago", "yesterday", "4 days ago".
pub fn ago(now_ms: i64, then_ms: i64) -> String {
    let secs = (now_ms - then_ms).max(0) / 1000;
    match secs {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86_400 => format!("{} h ago", secs / 3600),
        86_400..172_800 => "yesterday".into(),
        _ if secs < 60 * 86_400 => format!("{} days ago", secs / 86_400),
        _ => format!("{} months ago", secs / (30 * 86_400)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_times() {
        let min = 60_000;
        assert_eq!(ago(10 * min, 10 * min), "just now");
        assert_eq!(ago(10 * min, 5 * min), "5 min ago");
        assert_eq!(ago(200 * min, 20 * min), "3 h ago");
        assert_eq!(ago(2000 * min, 20 * min), "yesterday");
        assert_eq!(ago(10_000 * min, 20 * min), "6 days ago");
        assert_eq!(ago(200_000 * min, 0), "4 months ago");
    }

    fn clip(kind: ClipKind, text: &str) -> Clip {
        Clip {
            id: 1,
            kind,
            text: text.into(),
            ocr: String::new(),
            image: None,
            thumb: None,
            bytes: 2048,
            source_app: Some("Safari".into()),
            source_bundle: None,
            created: 0,
            last_used: 0,
            pinned: false,
        }
    }

    #[test]
    fn titles_and_subtitles() {
        assert_eq!(
            title(&clip(ClipKind::Text, "\n  first line \nsecond")),
            "first line"
        );
        assert_eq!(
            title(&clip(ClipKind::Files, "/a/report.pdf\n/b/photo.jpg")),
            "report.pdf, photo.jpg"
        );
        let mut img = clip(ClipKind::Image, "");
        assert_eq!(title(&img), "Image");
        img.ocr = "Invoice 4711\npaid".into();
        assert_eq!(title(&img), "“Invoice 4711”");
        assert_eq!(
            subtitle(&img, 0),
            "Image  ·  Safari  ·  just now  ·  2.0 KB"
        );
        assert_eq!(
            subtitle(&clip(ClipKind::Files, "/a\n/b"), 0),
            "Files  ·  Safari  ·  just now  ·  2 items"
        );
        assert!(title(&clip(ClipKind::Text, &"x".repeat(500))).ends_with('…'));
    }
}
