//! Common result + backup/output helpers shared by all optimisers.

use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::filetype::MediaKind;

#[derive(Debug, Error)]
pub enum OptimiseError {
    #[error("file not found: {0}")]
    NotFound(PathBuf),
    #[error("unsupported file type: {0}")]
    Unsupported(PathBuf),
    #[error(transparent)]
    Tool(#[from] crate::tools::ToolError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

/// The outcome of optimising a single file.
#[derive(Debug, Clone)]
pub struct OptimisationResult {
    pub kind: MediaKind,
    /// Original file path.
    pub source: PathBuf,
    /// Where the optimised output ended up.
    pub output: PathBuf,
    /// Backup of the original, if one was made.
    pub backup: Option<PathBuf>,
    pub old_size: u64,
    pub new_size: u64,
    /// Whether the aggressive compression preset was used.
    pub aggressive: bool,
}

impl OptimisationResult {
    pub fn saved_bytes(&self) -> i64 {
        self.old_size as i64 - self.new_size as i64
    }

    pub fn saved_percent(&self) -> f64 {
        if self.old_size == 0 {
            return 0.0;
        }
        (self.saved_bytes() as f64 / self.old_size as f64) * 100.0
    }

    /// Whether optimisation actually reduced the size.
    pub fn improved(&self) -> bool {
        self.new_size > 0 && self.new_size < self.old_size
    }
}

/// Controls how outputs and backups are placed.
#[derive(Debug, Clone)]
pub struct OptimiseOptions {
    /// Compression value to use.
    pub compression: crate::compression::CompressionQuality,
    /// Make a `.<name>.orig` backup of the original before overwriting.
    pub backup: bool,
    /// Strip non-essential metadata.
    pub strip_metadata: bool,
    /// Preserve original creation/modification timestamps on the output.
    pub preserve_dates: bool,
    /// Optional explicit output path. When `None`, the file is optimised in place.
    pub output: Option<PathBuf>,
    /// Allow the result to be written even if it is larger than the original.
    pub allow_larger: bool,
}

impl Default for OptimiseOptions {
    fn default() -> Self {
        Self {
            compression: crate::compression::CompressionQuality::normal(),
            backup: true,
            strip_metadata: false,
            preserve_dates: true,
            output: None,
            allow_larger: false,
        }
    }
}

/// The file name of `path`, or a safe fallback when it has none (e.g. `/` or `..`).
pub fn file_name_lossy(path: &Path) -> std::ffi::OsString {
    path.file_name()
        .map(|n| n.to_owned())
        .unwrap_or_else(|| std::ffi::OsString::from("file"))
}

/// The file stem of `path`, or `"file"` when it has none.
pub fn file_stem_lossy(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string())
}

pub fn file_size(path: &Path) -> u64 {
    fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// Backup `path` to a hidden sibling `.<name>.orig`. Returns the backup path.
pub fn backup_file(path: &Path) -> std::io::Result<PathBuf> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    let backup = path.with_file_name(format!(".{name}.orig"));
    if !backup.exists() {
        fs::copy(path, &backup)?;
    }
    Ok(backup)
}

/// Copy creation/modification times from `src` to `dst` (mtime only, portably).
pub fn copy_dates(src: &Path, dst: &Path) {
    if let Ok(meta) = fs::metadata(src) {
        if let Ok(mtime) = meta.modified() {
            let _ = filetime_set(dst, mtime);
        }
    }
}

fn filetime_set(path: &Path, mtime: std::time::SystemTime) -> std::io::Result<()> {
    // Use utimes via std by opening and setting; fall back to no-op if unsupported.
    let file = fs::File::options().write(true).open(path)?;
    file.set_modified(mtime)?;
    Ok(())
}
