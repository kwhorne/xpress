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

/// The backup path (`.<name>.orig`) for a given file.
pub fn backup_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    path.with_file_name(format!(".{name}.orig"))
}

/// If `backup` is a `.<name>.orig` file, return the original path it restores to.
pub fn original_for_backup(backup: &Path) -> Option<PathBuf> {
    let name = backup.file_name()?.to_string_lossy();
    let inner = name.strip_prefix('.')?.strip_suffix(".orig")?;
    if inner.is_empty() {
        return None;
    }
    Some(backup.with_file_name(inner))
}

/// Backup `path` to a hidden sibling `.<name>.orig`. Returns the backup path.
pub fn backup_file(path: &Path) -> std::io::Result<PathBuf> {
    let backup = backup_path(path);
    if !backup.exists() {
        fs::copy(path, &backup)?;
    }
    Ok(backup)
}

/// Find `.<name>.orig` backups under the given inputs (files or folders).
/// Returns `(backup, original)` pairs.
pub fn find_backups(inputs: &[PathBuf], recursive: bool) -> Vec<(PathBuf, PathBuf)> {
    let mut out = Vec::new();
    for input in inputs {
        if input.is_file() {
            if let Some(orig) = original_for_backup(input) {
                out.push((input.clone(), orig));
            } else {
                // A normal file: offer its backup if present.
                let b = backup_path(input);
                if b.exists() {
                    out.push((b, input.clone()));
                }
            }
        } else if input.is_dir() {
            let depth = if recursive { usize::MAX } else { 1 };
            for entry in walkdir::WalkDir::new(input)
                .max_depth(depth)
                .into_iter()
                .filter_map(Result::ok)
            {
                let p = entry.path();
                if p.is_file() {
                    if let Some(orig) = original_for_backup(p) {
                        out.push((p.to_path_buf(), orig));
                    }
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Atomically put the contents of `staged` at `dest`: copy into a hidden temp
/// file next to `dest`, flush it to disk, then `rename` it over `dest`. A crash
/// or full disk mid-write can never leave a truncated `dest` behind. The
/// existing file's permissions (and on macOS its xattrs, e.g. Finder tags) are
/// carried over.
pub fn place_file(staged: &Path, dest: &Path) -> std::io::Result<()> {
    let dir = match dest.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let mut tmp = tempfile::Builder::new()
        .prefix(".xpress-")
        .suffix(".tmp")
        .tempfile_in(dir)?;
    std::io::copy(&mut fs::File::open(staged)?, tmp.as_file_mut())?;
    tmp.as_file().sync_all()?;

    let perms_from = if dest.exists() { dest } else { staged };
    if let Ok(meta) = fs::metadata(perms_from) {
        let _ = fs::set_permissions(tmp.path(), meta.permissions());
    }
    #[cfg(target_os = "macos")]
    if dest.exists() {
        copy_xattrs(dest, tmp.path());
    }

    tmp.persist(dest).map_err(|e| e.error)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn copy_xattrs(from: &Path, to: &Path) {
    use std::os::unix::ffi::OsStrExt;
    let (Ok(from), Ok(to)) = (
        std::ffi::CString::new(from.as_os_str().as_bytes()),
        std::ffi::CString::new(to.as_os_str().as_bytes()),
    ) else {
        return;
    };
    // SAFETY: both paths are valid NUL-terminated strings; a null state is allowed.
    unsafe {
        libc::copyfile(
            from.as_ptr(),
            to.as_ptr(),
            std::ptr::null_mut(),
            libc::COPYFILE_XATTR,
        );
    }
}

/// How [`finish`] places an optimiser's output.
#[derive(Debug, Clone, Copy)]
pub struct Placement {
    /// Keep the original (report "no change") unless the result is smaller.
    /// Ignored when [`OptimiseOptions::allow_larger`] is set.
    pub size_guard: bool,
    /// Back the source up first (only for in-place writes with backups on).
    pub backup: bool,
    /// After an in-place write to a different path (e.g. `clip.mov` ->
    /// `clip.mp4`), delete the source.
    pub replace_source: bool,
}

/// The result for "nothing written; the original stays as it was".
pub fn unchanged(
    kind: MediaKind,
    src: &Path,
    old_size: u64,
    aggressive: bool,
) -> OptimisationResult {
    OptimisationResult {
        kind,
        source: src.to_path_buf(),
        output: src.to_path_buf(),
        backup: None,
        old_size,
        new_size: old_size,
        aggressive,
    }
}

/// The final step shared by every optimiser: apply the size guard, back up the
/// original *before* anything is overwritten, atomically place `staged` at
/// `options.output` (or `default_dest`), carry the timestamps over and, for
/// in-place format changes, remove the old source.
#[allow(clippy::too_many_arguments)]
pub fn finish(
    kind: MediaKind,
    src: &Path,
    staged: &Path,
    default_dest: PathBuf,
    old_size: u64,
    aggressive: bool,
    options: &OptimiseOptions,
    placement: Placement,
) -> Result<OptimisationResult, OptimiseError> {
    let new_size = file_size(staged);
    if placement.size_guard && !options.allow_larger && (new_size == 0 || new_size >= old_size) {
        return Ok(unchanged(kind, src, old_size, aggressive));
    }

    let in_place = options.output.is_none();
    let dest = options.output.clone().unwrap_or(default_dest);
    let backup = if placement.backup && options.backup && in_place {
        Some(backup_file(src)?)
    } else {
        None
    };

    place_file(staged, &dest)?;
    if options.preserve_dates {
        copy_dates(src, &dest);
    }
    if placement.replace_source && in_place && dest != src && src.exists() {
        let _ = fs::remove_file(src);
    }

    Ok(OptimisationResult {
        kind,
        source: src.to_path_buf(),
        output: dest,
        backup,
        old_size,
        new_size,
        aggressive,
    })
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
