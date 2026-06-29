//! Resolution scaling (downscale) for images and videos.
//!
//! Images are scaled with `vips` (preferred) or `ffmpeg`, GIFs with `gifsicle`,
//! and videos via an ffmpeg `scale=` filter folded into the optimise encode.

use std::path::{Path, PathBuf};

use tempfile::TempDir;

use crate::filetype::{classify, extension_lower, MediaKind};
use crate::result::{
    backup_file, copy_dates, file_size, OptimisationResult, OptimiseError, OptimiseOptions,
};
use crate::tools::{self, Tool};
use crate::{image, video};

/// Read the pixel dimensions of an image file, if possible.
pub fn image_dimensions(path: &Path) -> Option<(u32, u32)> {
    imagesize::size(path)
        .ok()
        .map(|s| (s.width as u32, s.height as u32))
}

fn even(n: u32) -> u32 {
    if n.is_multiple_of(2) {
        n
    } else {
        n.saturating_sub(1).max(2)
    }
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
    let ext = extension_lower(path).unwrap_or_default();
    let tmp = TempDir::new()?;
    let scaled = tmp.path().join(crate::result::file_name_lossy(path));

    if ext == "gif" {
        // gifsicle --scale <factor> --output <out> <src>
        tools::run_with_retries(
            Tool::Gifsicle,
            [
                "--scale",
                &format!("{factor:.4}"),
                "--output",
                &scaled.display().to_string(),
                &path.display().to_string(),
            ],
            3,
        )?;
    } else if tools::is_available(Tool::Vips) {
        // vips resize <in> <out> <factor>
        tools::run_with_retries(
            Tool::Vips,
            [
                "resize",
                &path.display().to_string(),
                &scaled.display().to_string(),
                &format!("{factor:.4}"),
            ],
            2,
        )?;
    } else {
        // ffmpeg fallback (relative scale, keep even dims for broad codec support).
        let (tw, th) = match image_dimensions(path) {
            Some((w, h)) => (
                even((w as f64 * factor).round() as u32),
                even((h as f64 * factor).round() as u32),
            ),
            None => (0, 0),
        };
        let vf = if tw > 0 && th > 0 {
            format!("scale={tw}:{th}")
        } else {
            format!("scale=trunc(iw*{factor:.4}):trunc(ih*{factor:.4})")
        };
        tools::run(
            Tool::Ffmpeg,
            [
                "-y",
                "-i",
                &path.display().to_string(),
                "-vf",
                &vf,
                &scaled.display().to_string(),
            ],
        )?;
    }

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
