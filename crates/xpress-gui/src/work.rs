//! Background worker: runs optimisation off the UI thread and streams results back.

use std::path::PathBuf;
use std::sync::mpsc::Sender;

use eframe::egui;
use xpress_core::audio::AudioFormat;
use xpress_core::filetype::{classify, MediaKind};
use xpress_core::image::ImageFormat;
use xpress_core::result::{OptimisationResult, OptimiseOptions};

/// A finished (or failed) job, plus an optional preview of the output.
pub struct Done {
    pub source: PathBuf,
    pub result: Result<OptimisationResult, String>,
    pub thumbnail: Option<egui::ColorImage>,
}

pub enum Msg {
    Done(Box<Done>),
    /// The clipboard history changed (something was copied or captured).
    HistoryChanged,
    /// Files sent from Finder ("Optimise with xpress"; macOS).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    OpenFiles(Vec<PathBuf>),
    /// An Apple Intelligence answer (for the request with this number).
    Ai {
        request: u64,
        result: Result<String, String>,
    },
    /// The clipboard image, saved as a file to optimise.
    ClipboardFile(Result<PathBuf, String>),
    /// An image decoded for the crop view (or why it couldn't be).
    CropReady {
        path: PathBuf,
        image: Result<egui::ColorImage, String>,
    },
    /// A screenshot was taken (`None`: cancelled). `back_to_app`: it was
    /// started from another app, which gets the focus back when done.
    ScreenshotTaken {
        result: Result<Option<PathBuf>, String>,
        back_to_app: bool,
    },
    /// An image decoded for the mark-up view (or why it couldn't be).
    MarkupReady {
        source: crate::markup::Source,
        image: Result<image::RgbaImage, String>,
    },
    /// The marked-up image, written as a PNG (to optimise and copy).
    Annotated {
        result: Result<PathBuf, String>,
        source: crate::markup::Source,
    },
}

type Job = Box<dyn FnOnce() + Send>;

/// Run `job` on a small, fixed pool of worker threads (one per CPU core):
/// dropping a folder of 500 files queues 500 jobs instead of starting 500
/// heavy encodes at once.
fn run_queued(job: impl FnOnce() + Send + 'static) {
    use std::sync::{mpsc, Arc, Mutex, OnceLock};
    static QUEUE: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
    let queue = QUEUE.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        let workers = std::thread::available_parallelism().map_or(2, |n| n.get());
        for i in 0..workers {
            let rx = rx.clone();
            let _ = std::thread::Builder::new()
                .name(format!("xpress-worker-{i}"))
                .spawn(move || loop {
                    let job = match rx.lock() {
                        Ok(rx) => rx.recv(),
                        Err(_) => return,
                    };
                    match job {
                        Ok(job) => job(),
                        Err(_) => return,
                    }
                });
        }
        Mutex::new(tx)
    });
    if let Ok(tx) = queue.lock() {
        let _ = tx.send(Box::new(job));
    }
}

/// Optimise a single file on a background thread. With a `quality` target,
/// images get the smallest file that still meets it (SSIMULACRA2).
pub fn spawn(
    path: PathBuf,
    options: OptimiseOptions,
    quality: Option<f64>,
    ctx: egui::Context,
    tx: Sender<Msg>,
) {
    run_queued(move || {
        let result = match quality {
            Some(target) if classify(&path) == Some(MediaKind::Image) => {
                xpress_core::quality::optimise_to_quality(&path, target, &options)
            }
            _ => xpress_core::optimise_file(&path, &options, AudioFormat::SameAsInput, None),
        }
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

/// Convert an image to `format` on a background thread, keeping the original.
/// PNG is lossless; JPEG/WebP use the `quality` target when one is set
/// (falling back to the compression slider if it can't be met).
pub fn spawn_convert(
    path: PathBuf,
    format: ImageFormat,
    options: OptimiseOptions,
    quality: Option<f64>,
    ctx: egui::Context,
    tx: Sender<Msg>,
) {
    run_queued(move || {
        let by_quality = match (quality, format) {
            (Some(q), ImageFormat::Jpeg | ImageFormat::Webp) => {
                xpress_core::quality::convert_to_quality(&path, format, q, &options).ok()
            }
            _ => None,
        };
        let result = match by_quality {
            Some(r) => Ok(r),
            None => xpress_core::image::convert_with(&path, format, &options, true),
        }
        .map_err(|e| e.to_string());
        let thumbnail = result.as_ref().ok().and_then(|r| make_thumbnail(&r.output));
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
    run_queued(move || {
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

/// Crop a file to a normalised rectangle on a background thread.
#[allow(clippy::too_many_arguments)]
pub fn spawn_crop(
    path: PathBuf,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    options: OptimiseOptions,
    ctx: egui::Context,
    tx: Sender<Msg>,
) {
    run_queued(move || {
        let result =
            xpress_core::crop::crop_rect(&path, x, y, w, h, &options).map_err(|e| e.to_string());
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
    let img = xpress_core::image::open_oriented(path).ok()?;
    let thumb = img.thumbnail(96, 96).to_rgba8();
    let (w, h) = thumb.dimensions();
    Some(egui::ColorImage::from_rgba_unmultiplied(
        [w as usize, h as usize],
        thumb.as_raw(),
    ))
}
