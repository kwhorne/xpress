//! Terminal output helpers (status icons + savings summary).

use std::path::PathBuf;

use xpress_core::result::{OptimisationResult, OptimiseError};
use xpress_core::tools::{self, Tool};

pub const CHECK: &str = "\u{2705}"; // ✅
pub const ERROR_X: &str = "\u{274C}"; // ❌
pub const WARN: &str = "\u{26A0}\u{FE0F}"; // ⚠️
pub const ARROW: &str = "\u{2192}"; // →

fn human_size(bytes: u64) -> String {
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

/// How to render results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    Normal,
    Quiet,
    Json,
}

fn kind_str(kind: xpress_core::filetype::MediaKind) -> &'static str {
    use xpress_core::filetype::MediaKind::*;
    match kind {
        Image => "image",
        Video => "video",
        Audio => "audio",
        Pdf => "pdf",
    }
}

/// Emit results as a JSON array (one object per file).
fn summarise_json(results: &[(PathBuf, Result<OptimisationResult, OptimiseError>)]) {
    let items: Vec<serde_json::Value> = results
        .iter()
        .map(|(path, res)| match res {
            Ok(r) => serde_json::json!({
                "source": path.display().to_string(),
                "output": r.output.display().to_string(),
                "kind": kind_str(r.kind),
                "ok": true,
                "old_size": r.old_size,
                "new_size": r.new_size,
                "saved_bytes": r.saved_bytes(),
                "saved_percent": (r.saved_percent() * 100.0).round() / 100.0,
                "aggressive": r.aggressive,
                "improved": r.improved(),
            }),
            Err(e) => serde_json::json!({
                "source": path.display().to_string(),
                "ok": false,
                "error": e.to_string(),
            }),
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".into())
    );
}

/// Print a per-file summary plus aggregate savings.
pub fn summarise(
    results: &[(PathBuf, Result<OptimisationResult, OptimiseError>)],
    mode: OutputMode,
) {
    if mode == OutputMode::Json {
        summarise_json(results);
        return;
    }
    let quiet = mode == OutputMode::Quiet;
    let mut total_old = 0u64;
    let mut total_new = 0u64;
    let mut ok = 0usize;
    let mut failed = 0usize;

    for (path, res) in results {
        match res {
            Ok(r) => {
                total_old += r.old_size;
                total_new += r.new_size;
                ok += 1;
                if quiet {
                    // no per-file lines in quiet mode
                } else if r.improved() {
                    println!(
                        "{CHECK} {} {ARROW} {}  ({} {ARROW} {}, -{:.0}%){}",
                        path.display(),
                        r.output.display(),
                        human_size(r.old_size),
                        human_size(r.new_size),
                        r.saved_percent(),
                        if r.aggressive { "  [aggressive]" } else { "" },
                    );
                } else {
                    println!(
                        "{WARN} {} already optimal ({})",
                        path.display(),
                        human_size(r.old_size)
                    );
                }
            }
            Err(e) => {
                failed += 1;
                eprintln!("{ERROR_X} {} {ARROW} {e}", path.display());
            }
        }
    }

    let saved = total_old.saturating_sub(total_new);
    let pct = if total_old > 0 {
        saved as f64 / total_old as f64 * 100.0
    } else {
        0.0
    };
    println!(
        "\n{ok} optimised, {failed} failed — saved {} ({:.0}%)",
        human_size(saved),
        pct
    );
}

/// Report tool availability.
pub fn doctor() {
    let tools = [
        ("ffmpeg (video/audio)", Tool::Ffmpeg),
        ("pngquant (png)", Tool::Pngquant),
        ("jpegoptim (jpeg)", Tool::Jpegoptim),
        ("gifsicle (gif)", Tool::Gifsicle),
        ("gs / ghostscript (pdf)", Tool::Ghostscript),
        ("gifski (video->gif)", Tool::Gifski),
        ("vips (resize)", Tool::Vips),
        ("vipsthumbnail (resize)", Tool::Vipsthumbnail),
        ("cwebp (webp)", Tool::Cwebp),
        ("heif-enc (heic/avif)", Tool::HeifEnc),
        ("cjxl (jxl)", Tool::Cjxl),
        ("exiftool (metadata)", Tool::Exiftool),
    ];
    println!("Tool availability:");
    for (label, tool) in tools {
        match tools::resolve(tool) {
            Ok(path) => println!("  {CHECK} {label}: {}", path.display()),
            Err(_) => println!("  {ERROR_X} {label}: not found"),
        }
    }
    println!(
        "\nArch: {}, CPUs: {}",
        if tools::is_arm64() { "arm64" } else { "x86_64" },
        tools::cpu_count()
    );
}
