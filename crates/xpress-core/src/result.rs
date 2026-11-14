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
    /// Skipped because the file was already optimised with these settings
    /// (see [`crate::cache`]).
    pub cached: bool,
    /// SSIMULACRA2 score of the output, when a quality target was used
    /// (see [`crate::quality`]).
    pub score: Option<f64>,
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
    /// Remove only where a photo/video was taken (GPS), keeping the rest.
    pub strip_location: bool,
    /// Preserve original creation/modification timestamps on the output.
    pub preserve_dates: bool,
    /// Optional explicit output path. When `None`, the file is optimised in place.
    pub output: Option<PathBuf>,
    /// `output` was derived from the source's name (an output folder or a
    /// `%f.%e` template), so a format change swaps its extension: converting
    /// `photo.png` to WebP with `-o dist/` writes `dist/photo.webp`.
    pub output_follows_format: bool,
    /// Allow the result to be written even if it is larger than the original.
    pub allow_larger: bool,
    /// Skip files already optimised with these settings, and mark new results
    /// (see [`crate::cache`]). Turn off to force a re-run.
    pub use_cache: bool,
}

impl Default for OptimiseOptions {
    fn default() -> Self {
        Self {
            compression: crate::compression::CompressionQuality::normal(),
            backup: true,
            strip_metadata: false,
            strip_location: false,
            preserve_dates: true,
            output: None,
            output_follows_format: false,
            allow_larger: false,
            use_cache: true,
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

/// A human-readable size in decimal units (1 KB = 1000 bytes), matching how
/// sizes are parsed (`--max-size 500kb`) and how Finder displays them.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1000.0 && unit < UNITS.len() - 1 {
        size /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
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
///
/// An existing backup is kept (it holds the true original). A new one is
/// written to a temp file, flushed and only then moved into place without
/// clobbering, so an interrupted copy (full disk, Ctrl-C) never leaves a
/// truncated `.orig` that a later run would trust.
pub fn backup_file(path: &Path) -> std::io::Result<PathBuf> {
    use std::io::{Error, ErrorKind};
    let backup = backup_path(path);
    match fs::symlink_metadata(&backup) {
        Ok(m) if m.is_file() => return Ok(backup),
        // A symlink (possibly dangling) or folder in the way: never write
        // through it, and never overwrite the original without a backup.
        Ok(_) => {
            return Err(Error::new(
                ErrorKind::AlreadyExists,
                format!("{} exists but is not a regular file", backup.display()),
            ))
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let dir = match backup.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    // `fs::copy` to a fresh path keeps APFS clones and the source's
    // permissions/metadata.
    let tmp = tempfile::Builder::new()
        .prefix(".xpress-backup-")
        .suffix(".tmp")
        .make_in(dir, |p| {
            fs::copy(path, p).map(|_| ()).inspect_err(|_| {
                // A half-written copy must not linger.
                let _ = fs::remove_file(p);
            })
        })?;
    // Write access: Windows refuses to flush (FlushFileBuffers) a file
    // opened read-only.
    fs::OpenOptions::new()
        .write(true)
        .open(tmp.path())?
        .sync_all()?;
    match tmp.persist_noclobber(&backup) {
        Ok(()) => Ok(backup),
        // Another job made the backup in the meantime: keep theirs.
        Err(e) if e.error.kind() == ErrorKind::AlreadyExists => Ok(backup),
        Err(e) => Err(e.error),
    }
}

/// Whether `a` and `b` name the same file on disk — not just byte-equal paths.
/// On case-insensitive file systems (macOS APFS, Windows NTFS) `IMG.MP4` and
/// `IMG.mp4` are one file; so are hard links. A path that doesn't exist is
/// only "the same" as a byte-equal one.
pub fn same_file(a: &Path, b: &Path) -> bool {
    a == b || same_file::is_same_file(a, b).unwrap_or(false)
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
    // An output folder (e.g. from a `-o '%P/optimised/%f.%e'` template) may
    // not exist yet.
    fs::create_dir_all(dir)?;
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
        cached: false,
        score: None,
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
        // Nothing smaller to write. An explicit `-o` still gets the file (the
        // original, as is), like a cache hit does, so scripts find it there.
        if let Some(out) = options.output.as_ref().filter(|o| !same_file(o, src)) {
            place_file(src, out)?;
            if options.preserve_dates {
                copy_dates(src, out);
            }
            return Ok(OptimisationResult {
                output: out.clone(),
                ..unchanged(kind, src, old_size, aggressive)
            });
        }
        return Ok(unchanged(kind, src, old_size, aggressive));
    }

    let in_place = options.output.is_none();
    let dest = match &options.output {
        Some(out) if options.output_follows_format => retarget_extension(out, src, &default_dest),
        Some(out) => out.clone(),
        None => default_dest,
    };
    // Decided before anything is written: afterwards both names may resolve
    // to the new file. `IMG_0001.MP4` -> `IMG_0001.mp4` is the source itself
    // on a case-insensitive file system.
    let dest_is_src = same_file(&dest, src);
    if in_place && placement.replace_source && !dest_is_src && dest.exists() {
        // e.g. `clip.mov` -> `clip.mp4` while an unrelated `clip.mp4` exists.
        return Err(OptimiseError::Other(format!(
            "{} already exists; not overwriting it",
            dest.display()
        )));
    }
    // Back up whenever the source itself is about to be overwritten — even if
    // the caller only compared path bytes, or `-o` names the source itself.
    let backup = if options.backup && ((in_place && placement.backup) || dest_is_src) {
        Some(backup_file(src)?)
    } else {
        None
    };

    // Read before placing: in place, `dest` *is* `src` and gets a new mtime.
    let src_mtime = if options.preserve_dates {
        fs::metadata(src).and_then(|m| m.modified()).ok()
    } else {
        None
    };
    place_file(staged, &dest)?;
    if let Some(mtime) = src_mtime {
        let _ = filetime_set(&dest, mtime);
    }
    if placement.replace_source && in_place && !dest_is_src && src.exists() {
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
        cached: false,
        score: None,
    })
}

/// An output named after the source (`-o dist/` gives `dist/photo.png`, as
/// does a `%f.%e` template) takes the extension of a format change, so a
/// conversion to WebP is never written into a `.png`.
fn retarget_extension(out: &Path, src: &Path, default_dest: &Path) -> PathBuf {
    let ext_eq = |a: &Path, b: &Path| {
        a.extension().map(|e| e.to_ascii_lowercase())
            == b.extension().map(|e| e.to_ascii_lowercase())
    };
    match default_dest.extension() {
        Some(ext) if ext_eq(out, src) && !ext_eq(default_dest, src) => out.with_extension(ext),
        _ => out.to_path_buf(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn opts(backup: bool) -> OptimiseOptions {
        OptimiseOptions {
            backup,
            ..OptimiseOptions::default()
        }
    }

    const REPLACE: Placement = Placement {
        size_guard: false,
        backup: true,
        replace_source: true,
    };

    #[test]
    fn upper_case_extension_is_replaced_not_deleted() {
        // `IMG.MP4` -> `IMG.mp4`: one file on APFS/NTFS, two on ext4. Either
        // way the optimised result must survive.
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("IMG.MP4");
        let staged = dir.path().join("staged");
        fs::write(&src, b"original original").unwrap();
        fs::write(&staged, b"optimised").unwrap();
        let r = finish(
            MediaKind::Video,
            &src,
            &staged,
            src.with_extension("mp4"),
            17,
            false,
            &opts(false),
            REPLACE,
        )
        .unwrap();
        assert_eq!(fs::read(&r.output).unwrap(), b"optimised");
    }

    #[test]
    fn case_only_conversion_backs_up_the_source() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("PHOTO.JPG");
        let staged = dir.path().join("staged");
        fs::write(&src, b"original").unwrap();
        fs::write(&staged, b"converted").unwrap();
        let dest = src.with_extension("jpg");
        let case_insensitive = same_file(&src, &dest);
        let r = finish(
            MediaKind::Image,
            &src,
            &staged,
            dest,
            8,
            false,
            &opts(true),
            Placement {
                size_guard: false,
                backup: false, // what a byte-wise `default_dest == path` gives
                replace_source: false,
            },
        )
        .unwrap();
        if case_insensitive {
            let b = r.backup.expect("source overwritten without backup");
            assert_eq!(fs::read(b).unwrap(), b"original");
        }
    }

    #[test]
    fn never_overwrites_an_unrelated_sibling() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("clip.mov");
        let other = dir.path().join("clip.mp4");
        let staged = dir.path().join("staged");
        fs::write(&src, b"mov").unwrap();
        fs::write(&other, b"someone else's mp4").unwrap();
        fs::write(&staged, b"new").unwrap();
        let err = finish(
            MediaKind::Video,
            &src,
            &staged,
            other.clone(),
            3,
            false,
            &opts(false),
            REPLACE,
        );
        assert!(err.is_err());
        assert_eq!(fs::read(&src).unwrap(), b"mov");
        assert_eq!(fs::read(&other).unwrap(), b"someone else's mp4");
    }

    #[test]
    fn output_named_after_the_source_follows_a_format_change() {
        let src = Path::new("a/photo.png");
        let webp = Path::new("a/photo.webp");
        assert_eq!(
            retarget_extension(Path::new("dist/photo.png"), src, webp),
            PathBuf::from("dist/photo.webp")
        );
        // An explicit, different name is kept.
        assert_eq!(
            retarget_extension(Path::new("dist/hero.webp"), src, webp),
            PathBuf::from("dist/hero.webp")
        );
        // No format change: nothing to do.
        assert_eq!(
            retarget_extension(Path::new("dist/photo.png"), src, src),
            PathBuf::from("dist/photo.png")
        );
    }

    #[test]
    fn explicit_output_is_written_even_when_nothing_shrinks() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.png");
        let staged = dir.path().join("staged");
        let out = dir.path().join("dist/a.png");
        fs::write(&src, b"small").unwrap();
        fs::write(&staged, b"not smaller").unwrap();
        let r = finish(
            MediaKind::Image,
            &src,
            &staged,
            src.clone(),
            5,
            false,
            &OptimiseOptions {
                output: Some(out.clone()),
                ..opts(false)
            },
            Placement {
                size_guard: true,
                backup: true,
                replace_source: false,
            },
        )
        .unwrap();
        assert_eq!(r.output, out);
        assert_eq!(fs::read(&out).unwrap(), b"small");
    }

    #[test]
    fn in_place_keeps_the_original_date() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.png");
        let staged = dir.path().join("staged");
        fs::write(&src, b"original!").unwrap();
        fs::write(&staged, b"small").unwrap();
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_500_000_000);
        filetime_set(&src, old).unwrap();
        finish(
            MediaKind::Image,
            &src,
            &staged,
            src.clone(),
            9,
            false,
            &opts(false),
            Placement {
                size_guard: true,
                backup: true,
                replace_source: false,
            },
        )
        .unwrap();
        assert_eq!(fs::read(&src).unwrap(), b"small");
        assert_eq!(fs::metadata(&src).unwrap().modified().unwrap(), old);
    }

    #[test]
    fn backup_is_complete_and_never_written_through_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.png");
        fs::write(&src, b"data").unwrap();
        let b = backup_file(&src).unwrap();
        assert_eq!(fs::read(&b).unwrap(), b"data");
        // An existing backup holds the true original and is kept.
        fs::write(&src, b"changed").unwrap();
        assert_eq!(fs::read(backup_file(&src).unwrap()).unwrap(), b"data");
        // No temp files left behind.
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.iter().all(|n| !n.ends_with(".tmp")), "{names:?}");

        #[cfg(unix)]
        {
            let src = dir.path().join("b.png");
            fs::write(&src, b"data").unwrap();
            let target = dir.path().join("elsewhere");
            std::os::unix::fs::symlink(&target, backup_path(&src)).unwrap();
            assert!(backup_file(&src).is_err());
            assert!(!target.exists());
        }
    }
}
