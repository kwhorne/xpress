//! xpress-core — the optimisation engine.
//!
//! This crate is UI-agnostic: images and PDFs are handled in pure Rust, video
//! and audio by driving `ffmpeg` (resolved from a bundle dir or `PATH`).

pub mod audio;
pub mod budget;
pub mod bundled;
pub mod cache;
pub mod clipboard;
pub mod compression;
pub mod config;
pub mod crop;
pub mod effects;
pub mod filetype;
pub mod history;
pub mod image;
pub mod intelligence;
pub mod ocr;
pub mod pdf;
pub mod pipeline;
pub mod privacy;
pub mod quality;
pub mod result;
pub mod scale;
pub mod share;
pub mod store;
pub mod sync;
pub mod template;
pub mod tools;
pub mod update;
pub mod video;
pub mod web;

use std::path::{Path, PathBuf};

use filetype::{classify, MediaKind};
use result::{OptimisationResult, OptimiseError, OptimiseOptions};

/// The user's home folder: `$HOME`, or `%USERPROFILE%` on Windows (where `HOME`
/// usually isn't set).
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| {
            if cfg!(windows) {
                std::env::var_os("USERPROFILE")
            } else {
                None
            }
        })
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// Expand a leading `~/` (or `~\` on Windows) to the home folder.
pub fn expand_home(s: &str) -> String {
    let rest = s
        .strip_prefix("~/")
        .or_else(|| s.strip_prefix("~\\").filter(|_| cfg!(windows)));
    match (rest, home_dir()) {
        (Some(rest), Some(home)) => home.join(rest).to_string_lossy().into_owned(),
        _ => s.to_string(),
    }
}

/// Optimise a single file, dispatching on its media kind.
///
/// `audio_target` selects the audio output format (defaults to same-as-input).
/// `pdf_dpi` controls PDF downsampling (None = no downsample).
///
/// With [`OptimiseOptions::use_cache`], a file already optimised with these
/// settings is skipped (`cached` in the result), and every result is marked so
/// the next run can skip it.
pub fn optimise_file(
    path: &Path,
    options: &OptimiseOptions,
    audio_target: audio::AudioFormat,
    pdf_dpi: Option<i32>,
) -> Result<OptimisationResult, OptimiseError> {
    let kind = classify(path).ok_or_else(|| OptimiseError::Unsupported(path.to_path_buf()))?;
    // Only plain same-format optimisation is cached; conversions always run.
    let cacheable = options.use_cache && audio_target == audio::AudioFormat::SameAsInput;
    if cacheable && cache::is_optimised(path, options, pdf_dpi) {
        let size = result::file_size(path);
        let mut r = result::unchanged(kind, path, size, options.compression.image_is_aggressive());
        r.cached = true;
        // An explicit output still has to exist (e.g. a pipeline's next step).
        if let Some(out) = options.output.as_ref().filter(|o| o.as_path() != path) {
            result::place_file(path, out)?;
            r.output = out.clone();
        }
        return Ok(r);
    }
    let r = match kind {
        MediaKind::Image => image::optimise(path, options),
        MediaKind::Video => video::optimise(path, options),
        MediaKind::Pdf => pdf::optimise(path, options, pdf_dpi),
        MediaKind::Audio => audio::optimise(path, options, audio_target, None),
    }?;
    if cacheable {
        cache::mark(&r.output, options, pdf_dpi);
    }
    Ok(r)
}

/// Recursively collect optimisable files from a list of paths (files or folders).
///
/// `recursive` controls folder descent; `kinds` filters by media kind (empty =
/// all supported kinds).
pub fn collect_files(inputs: &[PathBuf], recursive: bool, kinds: &[MediaKind]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let allow = |k: MediaKind| kinds.is_empty() || kinds.contains(&k);

    for input in inputs {
        if input.is_file() {
            if let Some(k) = classify(input) {
                if allow(k) {
                    out.push(input.clone());
                }
            }
        } else if input.is_dir() {
            let walker =
                walkdir::WalkDir::new(input).max_depth(if recursive { usize::MAX } else { 1 });
            for entry in walker.into_iter().filter_map(Result::ok) {
                let p = entry.path();
                if p.is_file() {
                    if let Some(k) = classify(p) {
                        if allow(k) {
                            out.push(p.to_path_buf());
                        }
                    }
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Optimise many files in parallel, returning per-file results in input order.
pub fn optimise_many(
    files: &[PathBuf],
    options: &OptimiseOptions,
    audio_target: audio::AudioFormat,
    pdf_dpi: Option<i32>,
) -> Vec<(PathBuf, Result<OptimisationResult, OptimiseError>)> {
    use rayon::prelude::*;
    files
        .par_iter()
        .map(|f| (f.clone(), optimise_file(f, options, audio_target, pdf_dpi)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_leading_tilde_is_the_home_folder() {
        let home = home_dir().expect("a home folder");
        assert_eq!(
            PathBuf::from(expand_home("~/Pictures/xpress")),
            home.join("Pictures/xpress")
        );
        assert_eq!(expand_home("/tmp/~/x"), "/tmp/~/x");
        assert_eq!(expand_home("~user/x"), "~user/x");
    }
}
