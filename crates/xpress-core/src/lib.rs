//! xpress-core — the optimisation engine.
//!
//! This crate is UI-agnostic: it resolves external tools (ffmpeg, pngquant,
//! jpegoptim, gifsicle, ghostscript, ...) and drives them with the same
//! arguments expected by each tool.

pub mod audio;
pub mod budget;
pub mod bundled;
pub mod clipboard;
pub mod compression;
pub mod config;
pub mod crop;
pub mod effects;
pub mod filetype;
pub mod image;
pub mod pdf;
pub mod pipeline;
pub mod result;
pub mod scale;
pub mod store;
pub mod template;
pub mod tools;
pub mod video;

use std::path::{Path, PathBuf};

use filetype::{classify, MediaKind};
use result::{OptimisationResult, OptimiseError, OptimiseOptions};

/// Optimise a single file, dispatching on its media kind.
///
/// `audio_target` selects the audio output format (defaults to same-as-input).
/// `pdf_dpi` controls PDF downsampling (None = no downsample).
pub fn optimise_file(
    path: &Path,
    options: &OptimiseOptions,
    audio_target: audio::AudioFormat,
    pdf_dpi: Option<i32>,
) -> Result<OptimisationResult, OptimiseError> {
    match classify(path) {
        Some(MediaKind::Image) => image::optimise(path, options),
        Some(MediaKind::Video) => video::optimise(path, options),
        Some(MediaKind::Pdf) => pdf::optimise(path, options, pdf_dpi),
        Some(MediaKind::Audio) => audio::optimise(path, options, audio_target, None),
        None => Err(OptimiseError::Unsupported(path.to_path_buf())),
    }
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
