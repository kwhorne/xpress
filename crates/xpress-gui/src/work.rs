//! Background worker: runs optimisation off the UI thread and streams results back.

use std::path::PathBuf;
use std::sync::mpsc::Sender;

use eframe::egui;
use xpress_core::audio::AudioFormat;
use xpress_core::filetype::{classify, MediaKind};
use xpress_core::result::{OptimisationResult, OptimiseOptions};

/// A finished (or failed) job, plus an optional preview of the output.
pub struct Done {
    pub source: PathBuf,
    pub result: Result<OptimisationResult, String>,
    pub thumbnail: Option<egui::ColorImage>,
}

pub enum Msg {
    Done(Box<Done>),
}

/// Optimise a single file on a background thread.
pub fn spawn(path: PathBuf, options: OptimiseOptions, ctx: egui::Context, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        let result = xpress_core::optimise_file(&path, &options, AudioFormat::SameAsInput, None)
            .map_err(|e| e.to_string());
        let thumbnail = result
            .as_ref()
            .ok()
            .filter(|r| r.kind == MediaKind::Image)
            .and_then(|r| make_thumbnail(&r.output));
        let _ = tx.send(Msg::Done(Box::new(Done {
            source: path,
            result,
            thumbnail,
        })));
        ctx.request_repaint();
    });
}

/// Run a pipeline DSL on a file on a background thread.
pub fn spawn_pipeline(
    path: PathBuf,
    steps: Vec<xpress_core::pipeline::Step>,
    options: OptimiseOptions,
    ctx: egui::Context,
    tx: Sender<Msg>,
) {
    std::thread::spawn(move || {
        let result = xpress_core::pipeline::run(&path, &steps, &options).map_err(|e| e.to_string());
        let thumbnail = result
            .as_ref()
            .ok()
            .filter(|r| classify(&r.output) == Some(MediaKind::Image))
            .and_then(|r| make_thumbnail(&r.output));
        let _ = tx.send(Msg::Done(Box::new(Done {
            source: path,
            result,
            thumbnail,
        })));
        ctx.request_repaint();
    });
}

/// Decode an image and downscale it to a small preview for the result card.
pub fn make_thumbnail(path: &std::path::Path) -> Option<egui::ColorImage> {
    let img = image::open(path).ok()?;
    let thumb = img.thumbnail(96, 96).to_rgba8();
    let (w, h) = thumb.dimensions();
    Some(egui::ColorImage::from_rgba_unmultiplied(
        [w as usize, h as usize],
        thumb.as_raw(),
    ))
}
