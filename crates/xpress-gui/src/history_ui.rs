//! The History view: search, filters, categories and the list of clips —
//! with several clips selected at once, multi-clips and collecting.

use std::collections::HashMap;
use std::path::PathBuf;

use eframe::egui;
use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Color32, FontId, Key, Layout, Modifiers, Rect, RichText, Sense, Vec2};
use xpress_core::history::{parse_color, Category, Clip, ClipKind, History, Query, Rule};
use xpress_core::intelligence::{Status as AiStatus, Task};

const ROW_HEIGHT: f32 = 62.0;
const PREVIEW: f32 = 46.0;

/// Colours offered for categories.
pub const PALETTE: [[u8; 3]; 8] = [
    [122, 162, 247],
    [158, 206, 106],
    [255, 158, 100],
    [247, 118, 142],
    [187, 154, 247],
    [115, 218, 202],
    [224, 175, 104],
    [169, 177, 214],
];

/// What the user asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum HistoryAction {
    /// Put the clip on the clipboard; `hide` when chosen with Enter, ⌘1–9 or a
    /// double-click (so it can be pasted straight away).
    Copy {
        id: i64,
        hide: bool,
    },
    /// Make a multi-clip of these clips; `copy` also puts it on the clipboard.
    Combine {
        ids: Vec<i64>,
        copy: bool,
    },
    /// Copy some text (e.g. the text recognised in an image).
    CopyText(String),
    TogglePin(i64),
    Pin(Vec<i64>),
    Delete(Vec<i64>),
    Reveal(PathBuf),
    Open(String),
    Enable,
    Hide,
    SetCollecting(bool),
    SetCategory {
        clips: Vec<i64>,
        category: i64,
        on: bool,
    },
    SaveCategory {
        id: Option<i64>,
        name: String,
        color: [u8; 3],
        rule: Rule,
    },
    DeleteCategory(i64),
    /// Run an Apple Intelligence task on a clip's text.
    Intelligence {
        id: i64,
        task: Task,
    },
    /// Add text to the history (e.g. an Apple Intelligence result).
    SaveText(String),
}

/// An Apple Intelligence request and its result, shown in a dialog.
#[derive(Debug, Clone, PartialEq)]
pub struct AiView {
    pub task: Task,
    /// What it was asked about (the clip's title).
    pub source: String,
    pub state: AiState,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AiState {
    Working,
    Done(String),
    Failed(String),
}

/// The text Apple Intelligence can work on for a clip, if any.
pub fn ai_text(clip: &Clip) -> Option<String> {
    let text = match clip.kind {
        ClipKind::Text | ClipKind::Code | ClipKind::Multi => clip.text.as_str(),
        ClipKind::Image | ClipKind::Screenshot => clip.ocr.as_str(),
        _ => "",
    };
    (!text.trim().is_empty()).then(|| text.to_string())
}

/// The "New category" / "Edit category" dialog.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CategoryEditor {
    pub id: Option<i64>,
    pub name: String,
    pub color: [u8; 3],
    pub app: Option<String>,
    pub kind: Option<ClipKind>,
    pub contains: String,
    pub error: Option<String>,
}

impl CategoryEditor {
    fn new(used: usize) -> Self {
        Self {
            color: PALETTE[used % PALETTE.len()],
            ..Default::default()
        }
    }

    fn edit(c: &Category) -> Self {
        Self {
            id: Some(c.id),
            name: c.name.clone(),
            color: c.color,
            app: c.rule.app.clone(),
            kind: c.rule.kind,
            contains: c.rule.contains.clone().unwrap_or_default(),
            error: None,
        }
    }

    fn rule(&self) -> Rule {
        Rule {
            app: self.app.clone(),
            kind: self.kind,
            contains: Some(self.contains.trim().to_string()).filter(|s| !s.is_empty()),
        }
    }
}

#[derive(Default)]
pub struct HistoryPanel {
    pub query: String,
    pub kind: Option<ClipKind>,
    pub pinned_only: bool,
    pub app: Option<String>,
    pub category: Option<i64>,
    pub results: Vec<Clip>,
    pub apps: Vec<(String, usize)>,
    pub categories: Vec<Category>,
    pub stats: (usize, u64),
    pub selected: usize,
    /// Clips picked with ⌘-click / ⇧-click, in the order picked.
    pub marked: Vec<i64>,
    /// Showing the items of this multi-clip.
    pub parent: Option<i64>,
    /// Items of the multi-clips in `results`.
    pub multi_items: HashMap<i64, Vec<Clip>>,
    pub editor: Option<CategoryEditor>,
    /// Whether Apple Intelligence can be used (kept up to date by the app).
    pub ai_status: Option<AiStatus>,
    pub ai: Option<AiView>,
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
        let results = match self.parent {
            Some(parent) => history.items(parent),
            None => history.search(&Query {
                text: self.query.clone(),
                kind: self.kind,
                app: self.app.clone(),
                pinned_only: self.pinned_only,
                category: self.category,
                limit: 500,
            }),
        };
        match results {
            Ok(results) => {
                self.results = results;
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
        if self.parent.is_some() && self.results.is_empty() {
            // The multi-clip is gone.
            self.parent = None;
            return self.refresh(history);
        }
        self.multi_items = self
            .results
            .iter()
            .filter(|c| c.kind == ClipKind::Multi)
            .map(|c| (c.id, history.items(c.id).unwrap_or_default()))
            .collect();
        self.apps = history.apps().unwrap_or_default();
        self.categories = history.categories().unwrap_or_default();
        if self
            .category
            .is_some_and(|id| !self.categories.iter().any(|c| c.id == id))
        {
            self.category = None;
        }
        self.stats = history.stats().unwrap_or_default();
        self.selected = self.selected.min(self.results.len().saturating_sub(1));
        let ids: Vec<i64> = self.results.iter().map(|c| c.id).collect();
        self.marked.retain(|id| ids.contains(id));
        self.textures.retain(|id, _| ids.contains(id));
        self.dirty = false;
    }

    /// Start over: empty search, first clip selected, cursor in the field.
    pub fn reset_for_paste(&mut self) {
        self.query.clear();
        self.selected = 0;
        self.marked.clear();
        self.parent = None;
        self.dirty = true;
        self.focus_search = true;
    }

    /// A category save failed (e.g. the name is taken): keep the dialog open.
    pub fn editor_failed(&mut self, error: String) {
        if let Some(editor) = &mut self.editor {
            editor.error = Some(error);
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        enabled: bool,
        collecting: bool,
        now_ms: i64,
    ) -> Option<HistoryAction> {
        let mut action = None;
        ui.horizontal(|ui| {
            ui.heading("History");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if enabled
                    && ui
                        .add(egui::Button::selectable(collecting, "⧉ Collect"))
                        .on_hover_text(
                            "Put everything you copy from now on into one multi-clip, \
                             to paste all of it at once",
                        )
                        .clicked()
                {
                    action = Some(HistoryAction::SetCollecting(!collecting));
                }
            });
        });
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

        if collecting {
            crate::app::card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Collecting — everything you copy goes into one multi-clip.")
                            .strong(),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Done").clicked() {
                            action = Some(HistoryAction::SetCollecting(false));
                        }
                    });
                });
            });
            ui.add_space(8.0);
        }

        if let Some(parent) = self.parent {
            ui.horizontal(|ui| {
                if ui.button("← Back").clicked() {
                    self.parent = None;
                    self.selected = 0;
                    self.dirty = true;
                }
                ui.label(
                    RichText::new(format!("Multi-clip · {} items", self.results.len())).strong(),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Copy all").clicked() {
                        action = Some(HistoryAction::Copy {
                            id: parent,
                            hide: false,
                        });
                    }
                });
            });
            ui.add_space(8.0);
        } else {
            self.filters(ui, &mut action);
        }

        if let Some(e) = &self.error {
            ui.colored_label(crate::app::ERR_RED, e);
        }

        if !self.marked.is_empty() {
            self.selection_bar(ui, &mut action);
        }

        // Keyboard: ↑/↓ choose, Enter copies, ⌘1–9 copies that row, Esc clears.
        let n = self.results.len();
        let (down, up, enter, escape, number) = ui.input(|i| {
            let number = [
                Key::Num1,
                Key::Num2,
                Key::Num3,
                Key::Num4,
                Key::Num5,
                Key::Num6,
                Key::Num7,
                Key::Num8,
                Key::Num9,
            ]
            .iter()
            .position(|k| i.modifiers.command && i.key_pressed(*k));
            (
                i.key_pressed(Key::ArrowDown),
                i.key_pressed(Key::ArrowUp),
                i.key_pressed(Key::Enter),
                i.key_pressed(Key::Escape),
                number,
            )
        });
        let editing = self.editor.is_some() || self.ai.is_some();
        let mut scroll_to_selected = false;
        if !editing {
            if n > 0 && down {
                self.selected = (self.selected + 1).min(n - 1);
                scroll_to_selected = true;
            }
            if n > 0 && up {
                self.selected = self.selected.saturating_sub(1);
                scroll_to_selected = true;
            }
            if enter {
                if self.marked.len() > 1 {
                    action = Some(HistoryAction::Combine {
                        ids: self.marked.clone(),
                        copy: true,
                    });
                } else if let Some(clip) = self.results.get(self.selected) {
                    action = Some(HistoryAction::Copy {
                        id: clip.id,
                        hide: true,
                    });
                }
            }
            if let Some(clip) = number.and_then(|k| self.results.get(k)) {
                action = Some(HistoryAction::Copy {
                    id: clip.id,
                    hide: true,
                });
            }
            if escape {
                if !self.marked.is_empty() {
                    self.marked.clear();
                } else if self.parent.is_some() {
                    self.parent = None;
                    self.dirty = true;
                } else if !self.query.is_empty() {
                    self.query.clear();
                    self.dirty = true;
                } else {
                    action = Some(HistoryAction::Hide);
                }
            }
        }

        if let Some(a) = self.category_editor(ui.ctx()) {
            action = Some(a);
        }
        if let Some(a) = self.ai_dialog(ui.ctx()) {
            action = Some(a);
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
            "{} clips · {}   ↑↓ choose · ⏎ copy · ⌘1–9 · ⌘-click to pick several · esc",
            self.stats.0,
            crate::app::human(self.stats.1)
        );
        let list_height = ui.available_height() - 22.0;
        let modifiers = ui.input(|i| i.modifiers);
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
                    if let Some(a) = self.row(ui, rect, &resp, &clip, selected, now_ms, i) {
                        action = Some(a);
                    }
                    if resp.clicked() {
                        self.click(i, modifiers);
                    }
                    if resp.double_clicked() {
                        action = Some(HistoryAction::Copy {
                            id: clip.id,
                            hide: true,
                        });
                    }
                    resp.context_menu(|ui| {
                        if let Some(a) = self.context_menu(ui, &clip) {
                            action = Some(a);
                        }
                    });
                }
            });
        ui.label(RichText::new(footer).weak().small());
        action
    }

    /// Plain click selects; ⌘-click picks or unpicks; ⇧-click picks a range.
    pub fn click(&mut self, i: usize, modifiers: Modifiers) {
        let Some(id) = self.results.get(i).map(|c| c.id) else {
            return;
        };
        if modifiers.command {
            if self.marked.is_empty() {
                if let Some(current) = self.results.get(self.selected) {
                    if current.id != id {
                        self.marked.push(current.id);
                    }
                }
            }
            match self.marked.iter().position(|m| *m == id) {
                Some(pos) => {
                    self.marked.remove(pos);
                }
                None => self.marked.push(id),
            }
        } else if modifiers.shift {
            let (from, to) = (self.selected.min(i), self.selected.max(i));
            for clip in &self.results[from..=to] {
                if !self.marked.contains(&clip.id) {
                    self.marked.push(clip.id);
                }
            }
        } else {
            self.marked.clear();
        }
        self.selected = i;
    }

    fn filters(&mut self, ui: &mut egui::Ui, action: &mut Option<HistoryAction>) {
        // Search and the app filter.
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

        // Kinds.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let chip = |ui: &mut egui::Ui, on: bool, label: &str| {
                ui.add(egui::Button::selectable(on, label)).clicked()
            };
            let everything = self.kind.is_none() && !self.pinned_only && self.category.is_none();
            if chip(ui, everything, "All") {
                self.kind = None;
                self.pinned_only = false;
                self.category = None;
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

        // Categories.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for category in self.categories.clone() {
                let on = self.category == Some(category.id);
                let resp = ui.add(egui::Button::selectable(
                    on,
                    category_label(ui, &category, Some(category.count)),
                ));
                if resp.clicked() {
                    self.category = (!on).then_some(category.id);
                    self.selected = 0;
                    self.dirty = true;
                }
                resp.context_menu(|ui| {
                    if ui.button("Edit…").clicked() {
                        self.editor = Some(CategoryEditor::edit(&category));
                        ui.close();
                    }
                    if ui.button("Delete category").clicked() {
                        *action = Some(HistoryAction::DeleteCategory(category.id));
                        ui.close();
                    }
                });
            }
            if ui
                .add(egui::Button::new("+ Category").small())
                .on_hover_text("Group clips by hand, or automatically by app, kind or words")
                .clicked()
            {
                self.editor = Some(CategoryEditor::new(self.categories.len()));
            }
        });
        ui.add_space(8.0);
    }

    fn selection_bar(&mut self, ui: &mut egui::Ui, action: &mut Option<HistoryAction>) {
        let ids = self.marked.clone();
        crate::app::card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("{} selected", ids.len())).strong());
                ui.add_space(8.0);
                if ui
                    .button("Copy together")
                    .on_hover_text("Combine into a multi-clip and copy it (⏎)")
                    .clicked()
                {
                    *action = Some(HistoryAction::Combine {
                        ids: ids.clone(),
                        copy: true,
                    });
                }
                if ui
                    .button("Combine")
                    .on_hover_text("Keep them together as one multi-clip")
                    .clicked()
                {
                    *action = Some(HistoryAction::Combine {
                        ids: ids.clone(),
                        copy: false,
                    });
                }
                if !self.categories.is_empty() {
                    ui.menu_button("Add to category", |ui| {
                        for category in &self.categories {
                            if ui.button(category_label(ui, category, None)).clicked() {
                                *action = Some(HistoryAction::SetCategory {
                                    clips: ids.clone(),
                                    category: category.id,
                                    on: true,
                                });
                                ui.close();
                            }
                        }
                    });
                }
                if ui.button("Pin").clicked() {
                    *action = Some(HistoryAction::Pin(ids.clone()));
                }
                if ui.button("Delete").clicked() {
                    *action = Some(HistoryAction::Delete(ids.clone()));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .small_button("×")
                        .on_hover_text("Clear selection")
                        .clicked()
                    {
                        self.marked.clear();
                    }
                });
            });
        });
        ui.add_space(6.0);
    }

    fn category_editor(&mut self, ctx: &egui::Context) -> Option<HistoryAction> {
        let editor = self.editor.as_mut()?;
        let mut action = None;
        let mut close = false;
        let apps: Vec<String> = self.apps.iter().map(|(a, _)| a.clone()).collect();
        let modal = egui::Modal::new(egui::Id::new("category_editor")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading(if editor.id.is_some() {
                "Edit category"
            } else {
                "New category"
            });
            ui.add_space(8.0);
            let name = ui.add(
                egui::TextEdit::singleline(&mut editor.name)
                    .hint_text("Name, e.g. Receipts")
                    .desired_width(f32::INFINITY),
            );
            if editor.name.is_empty() && editor.id.is_none() {
                name.request_focus();
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                for color in PALETTE {
                    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
                    let c = Color32::from_rgb(color[0], color[1], color[2]);
                    ui.painter().circle_filled(rect.center(), 9.0, c);
                    if editor.color == color {
                        ui.painter().circle_stroke(
                            rect.center(),
                            11.0,
                            egui::Stroke::new(2.0, Color32::WHITE),
                        );
                    }
                    if resp.clicked() {
                        editor.color = color;
                    }
                }
            });
            ui.add_space(10.0);
            ui.label(RichText::new("Add clips automatically").strong());
            ui.label(
                RichText::new("New clips that match everything set here join by themselves.")
                    .weak()
                    .small(),
            );
            ui.add_space(4.0);
            egui::Grid::new("category_rule")
                .num_columns(2)
                .spacing([10.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Copied in");
                    egui::ComboBox::from_id_salt("rule_app")
                        .selected_text(editor.app.as_deref().unwrap_or("Any app"))
                        .width(220.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut editor.app, None, "Any app");
                            for app in &apps {
                                ui.selectable_value(&mut editor.app, Some(app.clone()), app);
                            }
                        });
                    ui.end_row();
                    ui.label("Kind");
                    egui::ComboBox::from_id_salt("rule_kind")
                        .selected_text(editor.kind.map_or("Any kind", |k| k.label()))
                        .width(220.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut editor.kind, None, "Any kind");
                            for kind in ClipKind::ALL {
                                if kind != ClipKind::Multi {
                                    ui.selectable_value(&mut editor.kind, Some(kind), kind.label());
                                }
                            }
                        });
                    ui.end_row();
                    ui.label("Containing");
                    ui.add(
                        egui::TextEdit::singleline(&mut editor.contains)
                            .hint_text("words, also in images")
                            .desired_width(220.0),
                    );
                    ui.end_row();
                });
            if let Some(error) = &editor.error {
                ui.add_space(4.0);
                ui.colored_label(crate::app::ERR_RED, error);
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let can_save = !editor.name.trim().is_empty();
                if ui
                    .add_enabled(can_save, egui::Button::new("Save"))
                    .clicked()
                {
                    action = Some(HistoryAction::SaveCategory {
                        id: editor.id,
                        name: editor.name.trim().to_string(),
                        color: editor.color,
                        rule: editor.rule(),
                    });
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
                if let Some(id) = editor.id {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Delete").clicked() {
                            action = Some(HistoryAction::DeleteCategory(id));
                        }
                    });
                }
            });
        });
        if close || modal.should_close() {
            self.editor = None;
        }
        action
    }

    fn ai_dialog(&mut self, ctx: &egui::Context) -> Option<HistoryAction> {
        let view = self.ai.clone()?;
        let mut action = None;
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("ai_result")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.heading(view.task.result_title());
            ui.label(
                RichText::new(format!("Apple Intelligence · {}", view.source))
                    .weak()
                    .small(),
            );
            ui.add_space(10.0);
            match &view.state {
                AiState::Working => {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new());
                        ui.label(format!("{}…", view.task.label()));
                    });
                    ui.add_space(10.0);
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                }
                AiState::Done(text) => {
                    egui::ScrollArea::vertical()
                        .max_height(320.0)
                        .show(ui, |ui| {
                            ui.add(egui::Label::new(text.as_str()).selectable(true).wrap());
                        });
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.button("Copy").clicked() {
                            action = Some(HistoryAction::CopyText(text.clone()));
                            close = true;
                        }
                        if ui
                            .button("Save to history")
                            .on_hover_text("Keep it as a new clip")
                            .clicked()
                        {
                            action = Some(HistoryAction::SaveText(text.clone()));
                            close = true;
                        }
                        if ui.button("Close").clicked() {
                            close = true;
                        }
                    });
                }
                AiState::Failed(error) => {
                    ui.colored_label(crate::app::ERR_RED, error);
                    ui.add_space(10.0);
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                }
            }
        });
        if close || modal.should_close() {
            self.ai = None;
        }
        action
    }

    #[allow(clippy::too_many_arguments)]
    fn row(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        resp: &egui::Response,
        clip: &Clip,
        selected: bool,
        now_ms: i64,
        index: usize,
    ) -> Option<HistoryAction> {
        let marked = self.marked.contains(&clip.id);
        let fill = if selected || marked {
            crate::app::ACCENT2
        } else if resp.hovered() {
            crate::app::BG3
        } else {
            Color32::TRANSPARENT
        };
        let row = rect.shrink2(Vec2::new(0.0, 3.0));
        ui.painter().rect_filled(row, 8.0, fill);
        if marked {
            ui.painter().rect_stroke(
                row,
                8.0,
                egui::Stroke::new(1.5, crate::app::ACCENT),
                egui::StrokeKind::Inside,
            );
        }

        // Preview.
        let preview = Rect::from_min_size(
            egui::pos2(row.min.x + 8.0, row.center().y - PREVIEW / 2.0),
            Vec2::splat(PREVIEW),
        );
        self.paint_preview(ui, preview, clip);
        if marked {
            let dot = preview.left_top() + Vec2::new(4.0, 4.0);
            ui.painter().circle_filled(dot, 7.0, crate::app::ACCENT);
            // A tick, drawn (not every font has ✓).
            ui.painter().line(
                vec![
                    dot + Vec2::new(-3.0, 0.0),
                    dot + Vec2::new(-1.0, 2.5),
                    dot + Vec2::new(3.5, -2.5),
                ],
                egui::Stroke::new(1.6, Color32::WHITE),
            );
        }

        let items = self.multi_items.get(&clip.id).cloned();
        let (title, subtitle) = match &items {
            Some(items) => (multi_title(items), multi_subtitle(clip, items, now_ms)),
            None => (title(clip), subtitle(clip, now_ms)),
        };
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
                        .on_hover_text(if index < 9 {
                            format!("Copy to the clipboard (⌘{})", index + 1)
                        } else {
                            "Copy to the clipboard".into()
                        })
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
                    for id in &clip.categories {
                        if let Some(c) = self.categories.iter().find(|c| c.id == *id) {
                            ui.label(RichText::new("●").color(rgb(c.color)))
                                .on_hover_text(&c.name);
                        }
                    }
                    ui.with_layout(Layout::top_down(Align::LEFT), |ui| {
                        ui.add(egui::Label::new(RichText::new(title).strong()).truncate());
                        ui.add(egui::Label::new(RichText::new(subtitle).weak().small()).truncate());
                    });
                });
            });
        });
        action
    }

    fn context_menu(&mut self, ui: &mut egui::Ui, clip: &Clip) -> Option<HistoryAction> {
        let mut action = None;
        if ui.button("Copy").clicked() {
            action = Some(HistoryAction::Copy {
                id: clip.id,
                hide: false,
            });
        }
        if clip.kind == ClipKind::Multi && ui.button("Show items").clicked() {
            self.parent = Some(clip.id);
            self.selected = 0;
            self.marked.clear();
            self.dirty = true;
            ui.close();
        }
        if !clip.ocr.is_empty() && ui.button("Copy text in image").clicked() {
            action = Some(HistoryAction::CopyText(clip.ocr.clone()));
        }
        if ai_text(clip).is_some() {
            match self.ai_status {
                Some(AiStatus::Available) => {
                    ui.menu_button("Apple Intelligence", |ui| {
                        for task in Task::ALL {
                            if ui.button(task.label()).clicked() {
                                action = Some(HistoryAction::Intelligence { id: clip.id, task });
                                ui.close();
                            }
                        }
                    });
                }
                // Not on this macOS: don't mention it.
                None | Some(AiStatus::Missing) => {}
                Some(status) => {
                    ui.add_enabled(false, egui::Button::new("Apple Intelligence"))
                        .on_disabled_hover_text(status.explain().unwrap_or_default());
                }
            }
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
        ui.menu_button("Categories", |ui| {
            for category in &self.categories {
                let mut on = clip.categories.contains(&category.id);
                if ui
                    .checkbox(&mut on, category_label(ui, category, None))
                    .clicked()
                {
                    action = Some(HistoryAction::SetCategory {
                        clips: vec![clip.id],
                        category: category.id,
                        on,
                    });
                }
            }
            if !self.categories.is_empty() {
                ui.separator();
            }
            if ui.button("New category…").clicked() {
                self.editor = Some(CategoryEditor::new(self.categories.len()));
                ui.close();
            }
        });
        ui.separator();
        if ui.button("Delete").clicked() {
            action = Some(HistoryAction::Delete(vec![clip.id]));
        }
        if action.is_some() {
            ui.close();
        }
        action
    }

    fn paint_preview(&mut self, ui: &egui::Ui, rect: Rect, clip: &Clip) {
        let painter = ui.painter();
        if clip.kind == ClipKind::Multi {
            let tile = Color32::from_rgb(60, 160, 170);
            let back = rect.translate(Vec2::new(4.0, -4.0)).shrink(2.0);
            painter.rect_filled(back, 8.0, tile.gamma_multiply(0.45));
            let front = rect.translate(Vec2::new(-2.0, 2.0)).shrink(2.0);
            painter.rect_filled(front, 8.0, tile);
            let n = self.multi_items.get(&clip.id).map_or(0, Vec::len);
            painter.text(
                front.center(),
                egui::Align2::CENTER_CENTER,
                n.to_string(),
                FontId::proportional(18.0),
                Color32::WHITE,
            );
            return;
        }
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

fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// "● Receipts  4": a coloured dot, the name and (optionally) a count.
fn category_label(ui: &egui::Ui, category: &Category, count: Option<usize>) -> LayoutJob {
    let font = FontId::proportional(14.0);
    let mut job = LayoutJob::default();
    job.append(
        "● ",
        0.0,
        TextFormat::simple(font.clone(), rgb(category.color)),
    );
    job.append(
        &category.name,
        0.0,
        TextFormat::simple(font.clone(), ui.visuals().text_color()),
    );
    if let Some(n) = count {
        job.append(
            &format!("  {n}"),
            0.0,
            TextFormat::simple(font, ui.visuals().weak_text_color()),
        );
    }
    job
}

fn first_line(s: &str, max: usize) -> String {
    let line = s
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let mut out: String = line.chars().take(max).collect();
    if line.chars().count() > max {
        out.push('…');
    }
    out
}

/// The main line of a row.
pub fn title(clip: &Clip) -> String {
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
            first_line(&names.join(", "), 140)
        }
        ClipKind::Image | ClipKind::Screenshot if !clip.ocr.is_empty() => {
            format!("“{}”", first_line(&clip.ocr, 140))
        }
        ClipKind::Screenshot if !clip.text.is_empty() => first_line(&clip.text, 140),
        ClipKind::Image | ClipKind::Screenshot => "Image".into(),
        ClipKind::Multi if clip.text.is_empty() => "Multi-clip".into(),
        _ => first_line(&clip.text, 140),
    }
}

/// A multi-clip's main line: its items, short.
pub fn multi_title(items: &[Clip]) -> String {
    let parts: Vec<String> = items.iter().map(|c| first_line(&title(c), 40)).collect();
    first_line(&parts.join("  ·  "), 160)
}

pub fn multi_subtitle(clip: &Clip, items: &[Clip], now_ms: i64) -> String {
    format!(
        "Multi-clip  ·  {}  ·  {} items",
        ago(now_ms, clip.last_used),
        items.len()
    )
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
        ClipKind::Multi => "Multi-clip",
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
            categories: Vec::new(),
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

        let items = [clip(ClipKind::Text, "Hello"), img.clone()];
        assert_eq!(multi_title(&items), "Hello  ·  “Invoice 4711”");
        assert_eq!(
            multi_subtitle(&clip(ClipKind::Multi, ""), &items, 0),
            "Multi-clip  ·  just now  ·  2 items"
        );
    }

    fn panel(n: i64) -> HistoryPanel {
        let mut p = HistoryPanel::new();
        p.results = (1..=n)
            .map(|id| Clip {
                id,
                ..clip(ClipKind::Text, "x")
            })
            .collect();
        p
    }

    #[test]
    fn what_apple_intelligence_works_on() {
        assert_eq!(
            ai_text(&clip(ClipKind::Text, "hello")).as_deref(),
            Some("hello")
        );
        assert_eq!(ai_text(&clip(ClipKind::Text, "  ")), None);
        assert_eq!(ai_text(&clip(ClipKind::Link, "https://x.y")), None);
        let mut shot = clip(ClipKind::Screenshot, "Screenshot.png");
        assert_eq!(ai_text(&shot), None, "the file name isn't the content");
        shot.ocr = "Invoice 4711".into();
        assert_eq!(ai_text(&shot).as_deref(), Some("Invoice 4711"));
    }

    #[test]
    fn picking_several_clips() {
        let mut p = panel(5);
        let cmd = Modifiers {
            command: true,
            ..Default::default()
        };
        let shift = Modifiers {
            shift: true,
            ..Default::default()
        };
        p.click(0, Modifiers::NONE);
        assert!(p.marked.is_empty());
        p.click(2, cmd);
        assert_eq!(p.marked, [1, 3], "⌘-click also picks the current row");
        p.click(4, cmd);
        p.click(2, cmd);
        assert_eq!(p.marked, [1, 5], "⌘-click again unpicks");
        p.selected = 1;
        p.click(3, shift);
        assert_eq!(p.marked, [1, 5, 2, 3, 4]);
        p.click(0, Modifiers::NONE);
        assert!(p.marked.is_empty(), "a plain click starts over");
    }
}
