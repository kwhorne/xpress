//! Resolution scaling (downscale) for images and videos.
//!
//! Images are scaled in pure Rust (the `image` crate); videos via an ffmpeg
//! `scale=` filter folded into the optimise encode.

use std::path::{Path, PathBuf};

use tempfile::TempDir;

use crate::filetype::{classify, MediaKind};
use crate::result::{
    backup_file, copy_dates, file_size, OptimisationResult, OptimiseError, OptimiseOptions,
};
use crate::{image, video};

/// Read the pixel dimensions of an image file, if possible.
pub fn image_dimensions(path: &Path) -> Option<(u32, u32)> {
    imagesize::size(path)
        .ok()
        .map(|s| (s.width as u32, s.height as u32))
}

/// Downscale a single file by `factor` (0.0–1.0), then optimise it.
///
/// `factor` of 1.0 is a no-op scale (pure optimise). The output goes to
/// `options.output` or replaces the original (with a backup, when enabled).
pub fn downscale_file(
    path: &Path,
    factor: f64,
    options: &OptimiseOptions,
) -> Result<OptimisationResult, OptimiseError> {
    if !path.is_file() {
        return Err(OptimiseError::NotFound(path.to_path_buf()));
    }
    let factor = factor.clamp(0.05, 1.0);
    let old_size = file_size(path);

    match classify(path) {
        Some(MediaKind::Image) => downscale_image(path, factor, old_size, options),
        Some(MediaKind::Video) => {
            // Fold the scale into the encode: scale=trunc(iw*f/2)*2:trunc(ih*f/2)*2
            let vf = format!(
                "scale=trunc(iw*{f:.4}/2)*2:trunc(ih*{f:.4}/2)*2",
                f = factor
            );
            let mut r = video::optimise_with_filter(path, options, Some(&vf))?;
            r.old_size = old_size;
            Ok(r)
        }
        _ => Err(OptimiseError::Unsupported(path.to_path_buf())),
    }
}

fn downscale_image(
    path: &Path,
    factor: f64,
    old_size: u64,
    options: &OptimiseOptions,
) -> Result<OptimisationResult, OptimiseError> {
    let tmp = TempDir::new()?;
    let scaled = tmp.path().join(crate::result::file_name_lossy(path));

    // Pure-Rust resize for all raster image formats.
    image::scale_image(path, &scaled, factor)?;

    // Optimise the scaled temp, writing to the final destination. We always keep
    // the scaled result (allow_larger) since the user explicitly asked to shrink.
    let dest: PathBuf = options.output.clone().unwrap_or_else(|| path.to_path_buf());
    let opt_options = OptimiseOptions {
        output: Some(dest.clone()),
        backup: false,
        allow_larger: true,
        ..options.clone()
    };
    let mut result = image::optimise(&scaled, &opt_options)?;

    // Restore the true source identity + original size for accurate reporting.
    if options.backup && options.output.is_none() {
        result.backup = Some(backup_file(path)?);
    }
    if options.preserve_dates {
        copy_dates(path, &dest);
    }
    result.source = path.to_path_buf();
    result.output = dest;
    result.old_size = old_size;
    Ok(result)
}
