//! The mark-up view: arrows, boxes, text and redaction on a screenshot,
//! before it's shrunk and copied.
//!
//! What you see is drawn by the same code that draws the final image
//! (`xpress_core::annotate`), on a preview at most 1400 px wide; the marks
//! are kept in normalised coordinates and drawn again at full resolution
//! when copying.

use std::path::PathBuf;

use eframe::egui::{self, Align, Color32, Key, Layout, Pos2, Rect, RichText, Sense};
use xpress_core::annotate::{self, Mark, Point};

use crate::i18n::tr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Arrow,
    Rect,
    Text,
    Redact,
    Pixelate,
}

impl Tool {
    pub const ALL: [Tool; 5] = [
        Tool::Arrow,
        Tool::Rect,
        Tool::Text,
        Tool::Redact,
        Tool::Pixelate,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Arrow => "Arrow",
            Tool::Rect => "Box",
            Tool::Text => "Text",
            Tool::Redact => "Redact",
            Tool::Pixelate => "Pixelate",
        }
    }

    fn key(self) -> Key {
        match self {
            Tool::Arrow => Key::A,
            Tool::Rect => Key::R,
            Tool::Text => Key::T,
            Tool::Redact => Key::X,
            Tool::Pixelate => Key::P,
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Tool::Arrow => "Drag from where the arrow starts to what it points at.",
            Tool::Rect => "Drag to draw a box around something.",
            Tool::Text => "Click where the text should go, type, then press ↩.",
            Tool::Redact => "Drag over what must not be seen: a black box, safe for passwords and keys.",
            Tool::Pixelate => {
                "Drag to blur faces or names. Pixelated text can sometimes be read again — use Redact for secrets."
            }
        }
    }
}

/// Red, yellow, blue, black.
pub const COLORS: [annotate::Color; 4] = [
    [230, 45, 45, 255],
    [250, 200, 30, 255],
    [40, 110, 230, 255],
    [25, 25, 25, 255],
];

/// What the view asks the app to do.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Keep,
    Cancel,
    Copy,
}

pub struct MarkupState {
    /// The screenshot (a temporary file).
    pub path: PathBuf,
    /// Hide xpress again when done (it was taken from another app).
    pub back_to_app: bool,
    pub marks: Vec<Mark>,
    pub tool: Tool,
    pub color: usize,
    /// The preview's own pixels, before any marks.
    base: image::RgbaImage,
    texture: egui::TextureHandle,
    /// The marks the texture shows.
    shown: Vec<Mark>,
    /// A shape being dragged (normalised start and end).
    drag: Option<(Point, Point)>,
    /// Text being typed, and where.
    text: Option<(Point, String)>,
}

impl MarkupState {
    pub fn new(
        ctx: &egui::Context,
        path: PathBuf,
        base: image::RgbaImage,
        back_to_app: bool,
    ) -> Self {
        let texture = ctx.load_texture("markup", color_image(&base), egui::TextureOptions::LINEAR);
        Self {
            path,
            back_to_app,
            marks: Vec::new(),
            tool: Tool::Arrow,
            color: 0,
            base,
            texture,
            shown: Vec::new(),
            drag: None,
            text: None,
        }
    }

    pub fn undo(&mut self) {
        self.marks.pop();
    }

    /// Redraw the preview when the marks changed.
    fn refresh(&mut self) {
        if self.shown == self.marks {
            return;
        }
        let mut img = self.base.clone();
        annotate::render(&mut img, &self.marks, annotate::system_font().as_ref());
        self.texture
            .set(color_image(&img), egui::TextureOptions::LINEAR);
        self.shown = self.marks.clone();
    }

    /// The mark a drag from `a` to `b` makes with the current tool.
    fn mark_for(&self, a: Point, b: Point) -> Option<Mark> {
        let color = COLORS[self.color];
        Some(match self.tool {
            Tool::Arrow => Mark::Arrow {
                from: a,
                to: b,
                color,
            },
            Tool::Rect => Mark::Rect { a, b, color },
            Tool::Redact => Mark::Redact { a, b },
            Tool::Pixelate => Mark::Pixelate { a, b },
            Tool::Text => return None,
        })
    }

    fn commit_text(&mut self) {
        if let Some((at, text)) = self.text.take() {
            let text = text.trim().to_string();
            if !text.is_empty() {
                self.marks.push(Mark::Text {
                    at,
                    text,
                    color: COLORS[self.color],
                });
            }
        }
    }

    /// Draw the view; returns what the app should do.
    pub fn draw(&mut self, root: &mut egui::Ui) -> Outcome {
        let mut outcome = Outcome::Keep;
        // Keys, unless text is being typed.
        if self.text.is_none() {
            root.input(|i| {
                for tool in Tool::ALL {
                    if i.key_pressed(tool.key()) && !i.modifiers.command {
                        self.tool = tool;
                    }
                }
                if i.modifiers.command && i.key_pressed(Key::Z) {
                    self.undo();
                }
                if i.key_pressed(Key::Enter) {
                    outcome = Outcome::Copy;
                }
                if i.key_pressed(Key::Escape) {
                    outcome = Outcome::Cancel;
                }
            });
        }

        egui::Panel::top("markup_top").show(root, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.heading(tr("Mark up"));
                ui.add_space(8.0);
                for tool in Tool::ALL {
                    let hover = format!("{}  ({:?})", tr(tool.hint()), tool.key());
                    if ui
                        .selectable_label(self.tool == tool, tr(tool.label()))
                        .on_hover_text(hover)
                        .clicked()
                    {
                        self.commit_text();
                        self.tool = tool;
                    }
                }
                ui.add_space(8.0);
                for (i, c) in COLORS.iter().enumerate() {
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(18.0, 18.0), Sense::click());
                    let color = Color32::from_rgb(c[0], c[1], c[2]);
                    let painter = ui.painter();
                    painter.circle_filled(rect.center(), 7.0, color);
                    if self.color == i {
                        painter.circle_stroke(
                            rect.center(),
                            8.5,
                            egui::Stroke::new(2.0, ui.visuals().strong_text_color()),
                        );
                    }
                    if resp.clicked() {
                        self.color = i;
                    }
                }
                if ui
                    .add_enabled(!self.marks.is_empty(), egui::Button::new(tr("Undo")))
                    .on_hover_text("⌘Z")
                    .clicked()
                {
                    self.undo();
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .button(RichText::new(tr("Copy")).strong())
                        .on_hover_text(tr("Shrink it and copy it (↩)"))
                        .clicked()
                    {
                        outcome = Outcome::Copy;
                    }
                    if ui.button(tr("Cancel")).on_hover_text("esc").clicked() {
                        outcome = Outcome::Cancel;
                    }
                });
            });
            ui.label(RichText::new(tr(self.tool.hint())).weak().small());
            ui.add_space(4.0);
        });

        self.refresh();
        egui::CentralPanel::default().show(root, |ui| self.canvas(ui));

        if outcome == Outcome::Copy {
            self.commit_text();
        }
        outcome
    }

    fn canvas(&mut self, ui: &mut egui::Ui) {
        let size = self.texture.size_vec2();
        let avail = ui.available_size();
        let scale = (avail.x / size.x).min(avail.y / size.y).clamp(0.01, 1.0);
        let (rect, resp) = ui.allocate_exact_size(size * scale, Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.image(
            self.texture.id(),
            rect,
            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        let to_norm = |p: Pos2| {
            Point::new(
                ((p.x - rect.min.x) / rect.width()).clamp(0.0, 1.0),
                ((p.y - rect.min.y) / rect.height()).clamp(0.0, 1.0),
            )
        };
        let to_screen = |p: Point| {
            egui::pos2(
                rect.min.x + p.x * rect.width(),
                rect.min.y + p.y * rect.height(),
            )
        };

        if self.tool == Tool::Text {
            if resp.clicked() {
                if let Some(p) = resp.interact_pointer_pos() {
                    self.commit_text();
                    self.text = Some((to_norm(p), String::new()));
                }
            }
        } else if let Some(p) = resp.interact_pointer_pos() {
            let n = to_norm(p);
            if resp.drag_started() {
                self.drag = Some((n, n));
            } else if let Some((start, _)) = self.drag {
                self.drag = Some((start, n));
            }
        }
        if resp.drag_stopped() {
            if let Some((a, b)) = self.drag.take() {
                // Ignore a click without a real drag.
                let (dx, dy) = ((a.x - b.x) * rect.width(), (a.y - b.y) * rect.height());
                if dx.abs() + dy.abs() > 4.0 {
                    if let Some(mark) = self.mark_for(a, b) {
                        self.marks.push(mark);
                    }
                }
            }
        }

        // The shape being dragged, drawn on screen until it's let go.
        if let Some((a, b)) = self.drag {
            let (pa, pb) = (to_screen(a), to_screen(b));
            let c = COLORS[self.color];
            let color = Color32::from_rgb(c[0], c[1], c[2]);
            let (w, h) = self.base.dimensions();
            let width = annotate::stroke_width(w, h) * scale;
            match self.tool {
                Tool::Arrow => painter.arrow(pa, pb - pa, egui::Stroke::new(width, color)),
                Tool::Rect => {
                    painter.rect_stroke(
                        Rect::from_two_pos(pa, pb),
                        0.0,
                        egui::Stroke::new(width, color),
                        egui::StrokeKind::Middle,
                    );
                }
                Tool::Redact => {
                    painter.rect_filled(Rect::from_two_pos(pa, pb), 0.0, Color32::BLACK);
                }
                Tool::Pixelate | Tool::Text => {
                    painter.rect_filled(
                        Rect::from_two_pos(pa, pb),
                        0.0,
                        Color32::from_gray(128).gamma_multiply(0.7),
                    );
                }
            }
        }

        // Typing text: a field where it will go.
        let mut commit = false;
        let mut cancel = false;
        if let Some((at, text)) = &mut self.text {
            egui::Area::new(egui::Id::new("markup_text"))
                .fixed_pos(to_screen(*at))
                .order(egui::Order::Foreground)
                .show(ui.ctx(), |ui| {
                    let edit = ui.add(
                        egui::TextEdit::singleline(text)
                            .hint_text(tr("Text"))
                            .desired_width(220.0),
                    );
                    edit.request_focus();
                    if ui.input(|i| i.key_pressed(Key::Enter)) {
                        commit = true;
                    }
                    if ui.input(|i| i.key_pressed(Key::Escape)) {
                        cancel = true;
                    }
                });
        }
        if commit {
            self.commit_text();
        } else if cancel {
            self.text = None;
        }
    }
}

fn color_image(img: &image::RgbaImage) -> egui::ColorImage {
    let (w, h) = img.dimensions();
    egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw())
}
