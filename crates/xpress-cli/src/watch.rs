//! Background daemon: folder automations and clipboard watching.
//!
//! * Folder watcher — when a new/changed file lands in a watched folder, run the
//!   attached pipeline on it (mirrors Clop's folder automations).
//! * Clipboard watcher — when an image is copied, optimise it and save the result
//!   to a drop folder (feature `clipboard`).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::{bail, Result};
use notify::{RecursiveMode, Watcher};

use xpress_core::filetype::{classify, MediaKind};
use xpress_core::pipeline::{self, Step};
use xpress_core::result::OptimiseOptions;
use xpress_core::store::Store;

use crate::render;

/// One thing to watch: a folder plus the pipeline to run on its files.
struct FolderTarget {
    folder: PathBuf,
    file_type: String,
    steps: Vec<Step>,
    label: String,
}

fn resolve_steps(store: &Store, spec: &str) -> Result<Vec<Step>> {
    let dsl = store.pipelines.get(spec).cloned().unwrap_or_else(|| spec.to_string());
    pipeline::parse(&dsl).map_err(|e| anyhow::anyhow!("invalid pipeline '{spec}': {e}"))
}

fn type_matches(file_type: &str, kind: MediaKind) -> bool {
    match file_type {
        "all" | "" => true,
        "image" => kind == MediaKind::Image,
        "video" => kind == MediaKind::Video,
        "audio" => kind == MediaKind::Audio,
        "pdf" => kind == MediaKind::Pdf,
        _ => true,
    }
}

/// Run the daemon. `cli_folders` + `cli_pipeline` override the saved automations
/// when provided; otherwise the store's folder automations are used.
pub fn run(
    clipboard: bool,
    recursive: bool,
    cli_folders: Vec<PathBuf>,
    cli_pipeline: Option<String>,
    options: OptimiseOptions,
) -> Result<()> {
    let store = Store::load();
    let mut targets: Vec<FolderTarget> = Vec::new();

    if !cli_folders.is_empty() {
        let spec = cli_pipeline.clone().unwrap_or_else(|| "optimise".into());
        let steps = resolve_steps(&store, &spec)?;
        for folder in cli_folders {
            if !folder.is_dir() {
                eprintln!("{} not a folder, skipping: {}", render::WARN, folder.display());
                continue;
            }
            let folder = folder.canonicalize().unwrap_or(folder);
            targets.push(FolderTarget {
                folder: folder.clone(),
                file_type: "all".into(),
                steps: steps.clone(),
                label: spec.clone(),
            });
        }
    } else {
        for a in &store.automations {
            if a.source == "clipboard" {
                continue;
            }
            let folder = PathBuf::from(shellexpand_home(&a.source));
            if !folder.is_dir() {
                eprintln!("{} automation folder missing, skipping: {}", render::WARN, folder.display());
                continue;
            }
            let folder = folder.canonicalize().unwrap_or(folder);
            match resolve_steps(&store, &a.pipeline) {
                Ok(steps) => targets.push(FolderTarget {
                    folder,
                    file_type: a.file_type.clone(),
                    steps,
                    label: a.pipeline.clone(),
                }),
                Err(e) => eprintln!("{} {e}", render::ERROR_X),
            }
        }
    }

    let clipboard_enabled = clipboard
        || store.automations.iter().any(|a| a.source == "clipboard");

    if targets.is_empty() && !clipboard_enabled {
        bail!("nothing to watch — pass folders, add automations (`xpress pipeline attach`), or use --clipboard");
    }

    // Ctrl-C handling.
    let running = Arc::new(std::sync::atomic::AtomicBool::new(true));
    {
        let running = running.clone();
        ctrlc::set_handler(move || {
            running.store(false, std::sync::atomic::Ordering::SeqCst);
        })?;
    }

    // Clipboard watcher on its own thread.
    if clipboard_enabled {
        let opts = options.clone();
        let running = running.clone();
        let steps = cli_pipeline
            .as_deref()
            .and_then(|s| resolve_steps(&store, s).ok())
            .unwrap_or_default();
        std::thread::spawn(move || clipboard_loop(running, opts, steps));
        println!("{} watching clipboard for images", render::CHECK);
    }

    if targets.is_empty() {
        // Clipboard-only: just idle until Ctrl-C.
        while running.load(std::sync::atomic::Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(200));
        }
        println!("\nstopped.");
        return Ok(());
    }

    folder_loop(running, targets, recursive, options)
}

fn folder_loop(
    running: Arc<std::sync::atomic::AtomicBool>,
    targets: Vec<FolderTarget>,
    recursive: bool,
    options: OptimiseOptions,
) -> Result<()> {
    let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })?;
    let mode = if recursive { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive };
    for t in &targets {
        watcher.watch(&t.folder, mode)?;
        println!("{} watching {} [{}] -> {}", render::CHECK, t.folder.display(), t.file_type, t.label);
    }
    println!("(press Ctrl-C to stop)\n");

    // Debounce: collect changed paths, process after a quiet period.
    let mut pending: HashSet<PathBuf> = HashSet::new();
    let mut processed: HashMap<PathBuf, SystemTime> = HashMap::new();

    while running.load(std::sync::atomic::Ordering::SeqCst) {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(Ok(event)) => {
                if matches!(
                    event.kind,
                    notify::EventKind::Create(_) | notify::EventKind::Modify(_)
                ) {
                    for p in event.paths {
                        if p.is_file() {
                            pending.insert(p);
                        }
                    }
                }
            }
            Ok(Err(_)) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if !pending.is_empty() {
                    let batch: Vec<PathBuf> = pending.drain().collect();
                    for path in batch {
                        process_path(&path, &targets, &options, &mut processed);
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    println!("\nstopped.");
    Ok(())
}

fn process_path(
    path: &Path,
    targets: &[FolderTarget],
    options: &OptimiseOptions,
    processed: &mut HashMap<PathBuf, SystemTime>,
) {
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    // Skip hidden / backup / temp files.
    if name.starts_with('.') || name.ends_with(".orig") || name.ends_with('~') {
        return;
    }
    let Some(kind) = classify(path) else { return };

    // Skip if unchanged since we last processed it (prevents re-trigger loops).
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    if let (Some(mt), Some(prev)) = (mtime, processed.get(path)) {
        if mt <= *prev {
            return;
        }
    }

    // Find the first target whose folder contains this path and type matches.
    let Some(target) = targets.iter().find(|t| {
        path.starts_with(&t.folder) && type_matches(&t.file_type, kind)
    }) else {
        return;
    };

    match pipeline::run(path, &target.steps, options) {
        Ok(r) => {
            if r.improved() {
                println!(
                    "{} {} {} {}  (-{:.0}%)",
                    render::CHECK,
                    path.display(),
                    render::ARROW,
                    r.output.display(),
                    r.saved_percent()
                );
            }
            // Record the post-write mtime so the resulting change event is ignored.
            if let Ok(mt) = std::fs::metadata(&r.output).and_then(|m| m.modified()) {
                processed.insert(path.to_path_buf(), mt);
            }
            if r.output != *path {
                if let Ok(mt) = std::fs::metadata(path).and_then(|m| m.modified()) {
                    processed.insert(path.to_path_buf(), mt);
                }
            }
        }
        Err(e) => eprintln!("{} {} {} {e}", render::ERROR_X, path.display(), render::ARROW),
    }
}

fn shellexpand_home(s: &str) -> String {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return format!("{}/{}", home.to_string_lossy(), rest);
        }
    }
    s.to_string()
}

// ---------------------------------------------------------------------------
// Clipboard watcher
// ---------------------------------------------------------------------------

#[cfg(feature = "clipboard")]
fn clipboard_loop(
    running: Arc<std::sync::atomic::AtomicBool>,
    options: OptimiseOptions,
    steps: Vec<Step>,
) {
    use std::hash::{Hash, Hasher};

    let Ok(mut clipboard) = arboard::Clipboard::new() else {
        eprintln!("{} could not access the clipboard", render::WARN);
        return;
    };

    // Where optimised clipboard images are saved.
    let drop_dir = clipboard_drop_dir();
    let _ = std::fs::create_dir_all(&drop_dir);

    let mut last_hash: u64 = 0;
    while running.load(std::sync::atomic::Ordering::SeqCst) {
        if let Ok(img) = clipboard.get_image() {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            img.width.hash(&mut hasher);
            img.height.hash(&mut hasher);
            img.bytes.len().hash(&mut hasher);
            // Sample some bytes to detect content changes cheaply.
            for b in img.bytes.iter().step_by(257).take(64) {
                b.hash(&mut hasher);
            }
            let h = hasher.finish();
            if h != last_hash {
                last_hash = h;
                if let Err(e) = handle_clipboard_image(&img, &drop_dir, &options, &steps) {
                    eprintln!("{} clipboard image: {e}", render::WARN);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(600));
    }
}

#[cfg(feature = "clipboard")]
fn handle_clipboard_image(
    img: &arboard::ImageData,
    drop_dir: &Path,
    options: &OptimiseOptions,
    steps: &[Step],
) -> Result<()> {
    // Write the raw RGBA to a temp PNG via ffmpeg (rawvideo -> png), then optimise.
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let raw = std::env::temp_dir().join(format!("xpress-clip-{ts}.rgba"));
    let png = drop_dir.join(format!("clip-{ts}.png"));
    std::fs::write(&raw, &img.bytes)?;

    // ffmpeg -f rawvideo -pix_fmt rgba -s WxH -i raw png
    let res = xpress_core::tools::run(
        xpress_core::tools::Tool::Ffmpeg,
        [
            "-y", "-f", "rawvideo", "-pix_fmt", "rgba",
            "-s", &format!("{}x{}", img.width, img.height),
            "-i", &raw.display().to_string(),
            &png.display().to_string(),
        ],
    );
    let _ = std::fs::remove_file(&raw);
    if let Err(e) = res {
        bail!("ffmpeg raw->png failed (is ffmpeg installed?): {e}");
    }

    // Optimise (or run the configured pipeline) on the saved PNG, in place.
    let opts = OptimiseOptions { backup: false, ..options.clone() };
    let result = if steps.is_empty() {
        xpress_core::image::optimise(&png, &opts)
    } else {
        pipeline::run(&png, steps, &opts)
    };
    match result {
        Ok(r) => println!(
            "{} clipboard image {} {}  (-{:.0}%)",
            render::CHECK,
            render::ARROW,
            r.output.display(),
            r.saved_percent()
        ),
        Err(e) => bail!("{e}"),
    }
    Ok(())
}

#[cfg(feature = "clipboard")]
fn clipboard_drop_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join("Pictures/xpress")
    } else {
        std::env::temp_dir().join("xpress-clipboard")
    }
}

#[cfg(not(feature = "clipboard"))]
fn clipboard_loop(
    _running: Arc<std::sync::atomic::AtomicBool>,
    _options: OptimiseOptions,
    _steps: Vec<Step>,
) {
    eprintln!("{} clipboard support not built in (enable the `clipboard` feature)", render::WARN);
}
