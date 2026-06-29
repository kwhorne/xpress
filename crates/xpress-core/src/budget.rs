//! Compress a file to fit within a target byte budget by ramping up the
//! compression factor until the result is small enough (or we run out of room).

use std::path::Path;

use crate::audio::AudioFormat;
use crate::compression::{CompressionQuality, CompressionTier};
use crate::filetype::{classify, MediaKind};
use crate::result::{file_size, OptimisationResult, OptimiseError, OptimiseOptions};

/// Optimise `path` so the result is at most `max_bytes`, trying progressively
/// harder compression. Returns the best result achieved (which may still exceed
/// the budget if the format can't compress further).
pub fn optimise_to_budget(
    path: &Path,
    max_bytes: u64,
    base: &OptimiseOptions,
) -> Result<OptimisationResult, OptimiseError> {
    if !path.is_file() {
        return Err(OptimiseError::NotFound(path.to_path_buf()));
    }
    let kind = classify(path).ok_or_else(|| OptimiseError::Unsupported(path.to_path_buf()))?;

    // Factors to try, from gentle to aggressive.
    let factors = [30, 50, 64, 80, 90, 100];
    let mut best: Option<OptimisationResult> = None;

    // Always write to a temp candidate so we don't clobber the source between tries.
    let tmp = tempfile::TempDir::new()?;

    for (i, &factor) in factors.iter().enumerate() {
        let candidate = tmp.path().join(format!(
            "try{i}.{}",
            crate::filetype::extension_lower(path).unwrap_or_else(|| "bin".into())
        ));
        let opts = OptimiseOptions {
            compression: CompressionQuality::new(CompressionTier::Custom, factor),
            output: Some(candidate.clone()),
            backup: false,
            allow_larger: true,
            ..base.clone()
        };
        let r = run_one(path, kind, &opts)?;
        let size = file_size(&r.output);

        let within = size <= max_bytes;
        let better = best.as_ref().map(|b| size < b.new_size).unwrap_or(true);
        if better {
            best = Some(r);
        }
        if within {
            break;
        }
    }

    let best = best.ok_or_else(|| OptimiseError::Other("budget: no candidate produced".into()))?;

    // Place the best candidate at the final destination.
    let old_size = file_size(path);
    let new_size = file_size(&best.output);
    let dest = base.output.clone().unwrap_or_else(|| path.to_path_buf());
    let backup = if base.backup && base.output.is_none() {
        Some(crate::result::backup_file(path)?)
    } else {
        None
    };
    std::fs::copy(&best.output, &dest)?;
    if base.preserve_dates {
        crate::result::copy_dates(path, &dest);
    }

    Ok(OptimisationResult {
        kind,
        source: path.to_path_buf(),
        output: dest,
        backup,
        old_size,
        new_size,
        aggressive: true,
    })
}

fn run_one(
    path: &Path,
    kind: MediaKind,
    opts: &OptimiseOptions,
) -> Result<OptimisationResult, OptimiseError> {
    match kind {
        MediaKind::Image => crate::image::optimise(path, opts),
        MediaKind::Video => crate::video::optimise(path, opts),
        MediaKind::Pdf => crate::pdf::optimise(path, opts, None),
        MediaKind::Audio => crate::audio::optimise(path, opts, AudioFormat::SameAsInput, None),
    }
}
