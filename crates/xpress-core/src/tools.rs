//! External tool resolution and process execution.
//!
//! Resolution order for each tool:
//!  1. `$XPRESS_BIN_DIR/<name>`
//!  2. a sibling `bin/` directory next to the executable
//!  3. the per-user bundle/cache dir (see [`bundle_dir`])
//!  4. the system `PATH` (via `which`)

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;

use thiserror::Error;

/// The external tools xpress drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Ffmpeg,
    Ffprobe,
    Pngquant,
    Jpegoptim,
    JpegoptimOld,
    Gifsicle,
    Gifski,
    Vips,
    Vipsthumbnail,
    Ghostscript,
    Cwebp,
    HeifEnc,
    Cjxl,
    Exiftool,
}

impl Tool {
    pub fn binary_name(&self) -> &'static str {
        match self {
            Tool::Ffmpeg => "ffmpeg",
            Tool::Ffprobe => "ffprobe",
            Tool::Pngquant => "pngquant",
            Tool::Jpegoptim => "jpegoptim",
            Tool::JpegoptimOld => "jpegoptim-old",
            Tool::Gifsicle => "gifsicle",
            Tool::Gifski => "gifski",
            Tool::Vips => "vips",
            Tool::Vipsthumbnail => "vipsthumbnail",
            Tool::Ghostscript => "gs",
            Tool::Cwebp => "cwebp",
            Tool::HeifEnc => "heif-enc",
            Tool::Cjxl => "cjxl",
            Tool::Exiftool => "exiftool",
        }
    }
}

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("`{0}` not found. Install it (e.g. `brew install {0}`) or set $XPRESS_BIN_DIR")]
    NotFound(&'static str),
    #[error("`{tool}` exited with status {code}: {stderr}")]
    Failed {
        tool: &'static str,
        code: i32,
        stderr: String,
    },
    #[error("failed to launch `{tool}`: {source}")]
    Launch {
        tool: &'static str,
        #[source]
        source: std::io::Error,
    },
}

/// The per-user directory where xpress keeps (or extracts) bundled binaries.
///
/// * macOS: `~/Library/Application Support/xpress/bin`
/// * Linux: `$XDG_DATA_HOME/xpress/bin` (or `~/.local/share/xpress/bin`)
/// * Windows: `%LOCALAPPDATA%\xpress\bin`
pub fn bundle_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join("Library/Application Support/xpress/bin"))
    } else if cfg!(target_os = "windows") {
        std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("xpress\\bin"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .map(|p| p.join("xpress/bin"))
    }
}

/// A process-wide override for the bin directory, checked before everything else.
/// Used by tests (and embeddable hosts) to avoid relying on environment variables.
static BIN_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

/// Set the highest-priority directory to resolve tools from. Idempotent: the
/// first value set wins for the lifetime of the process.
pub fn set_bin_dir_override(dir: PathBuf) {
    let _ = BIN_OVERRIDE.set(dir);
}

/// Candidate bin directories, in resolution order (excluding `PATH`).
fn bin_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = BIN_OVERRIDE.get() {
        dirs.push(dir.clone());
    }
    if let Ok(dir) = std::env::var("XPRESS_BIN_DIR") {
        dirs.push(PathBuf::from(dir));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            dirs.push(parent.join("bin"));
        }
    }
    if let Some(dir) = bundle_dir() {
        dirs.push(dir);
    }
    dirs
}

/// Resolve the absolute path to a tool.
pub fn resolve(tool: Tool) -> Result<PathBuf, ToolError> {
    let name = tool.binary_name();
    for dir in bin_dirs() {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    which::which(name).map_err(|_| ToolError::NotFound(name))
}

/// Whether a tool is available without erroring.
pub fn is_available(tool: Tool) -> bool {
    resolve(tool).is_ok()
}

/// Number of logical CPUs, cached. Used for `--threads`.
pub fn cpu_count() -> usize {
    static N: OnceLock<usize> = OnceLock::new();
    *N.get_or_init(num_cpus::get)
}

/// Whether we are running on Apple Silicon (drives the VideoToolbox path).
pub fn is_arm64() -> bool {
    cfg!(target_arch = "aarch64")
}

/// Run a tool to completion, capturing stdout/stderr. Errors on non-zero exit.
pub fn run<I, S>(tool: Tool, args: I) -> Result<Output, ToolError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let path = resolve(tool)?;
    let name = tool.binary_name();
    let output = Command::new(&path)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|source| ToolError::Launch { tool: name, source })?;

    if !output.status.success() {
        return Err(ToolError::Failed {
            tool: name,
            code: output.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(output)
}

/// Run, retrying up to `tries` times before giving up.
pub fn run_with_retries<I, S>(tool: Tool, args: I, tries: u32) -> Result<Output, ToolError>
where
    I: IntoIterator<Item = S> + Clone,
    S: AsRef<std::ffi::OsStr>,
{
    let mut last_err = None;
    for _ in 0..tries.max(1) {
        match run(tool, args.clone()) {
            Ok(out) => return Ok(out),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap())
}

/// Copy EXIF metadata from `src` to `dst` using exiftool, optionally stripping
/// all metadata first. Best-effort: a missing exiftool is not fatal.
pub fn copy_exif(src: &Path, dst: &Path, strip_metadata: bool, exclude_tags: &[&str]) {
    if !is_available(Tool::Exiftool) {
        return;
    }
    let mut args: Vec<String> = vec![
        "-overwrite_original".into(),
        "-TagsFromFile".into(),
        src.display().to_string(),
    ];
    if strip_metadata {
        // Keep only orientation/colour-critical tags when stripping.
        args.push("-all=".into());
        args.push("-icc_profile".into());
        args.push("-Orientation".into());
    } else {
        args.push("-all:all".into());
    }
    for tag in exclude_tags {
        args.push(format!("--{tag}"));
    }
    args.push(dst.display().to_string());
    let _ = run(Tool::Exiftool, &args);
}
