//! Compress a file to fit within a target byte budget.
//!
//! Video and lossy audio compute the bitrate the budget allows from the
//! duration and encode straight to it (two-pass for video, downscaling when
//! the bitrate is too thin for the frame size), correcting once or twice if the
//! result overshoots. Everything else ramps up the compression factor until
//! the result is small enough (or we run out of room).

use std::path::{Path, PathBuf};

use crate::audio::AudioFormat;
use crate::compression::{CompressionQuality, CompressionTier};
use crate::filetype::{classify, MediaKind};
use crate::result::{
    file_size, finish, OptimisationResult, OptimiseError, OptimiseOptions, Placement,
};

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
    let old_size = file_size(path);

    // Already within budget: a normal optimise is all that's needed.
    if old_size <= max_bytes {
        return crate::optimise_file(path, base, AudioFormat::SameAsInput, None);
    }

    let tmp = tempfile::TempDir::new()?;
    let by_rate = match kind {
        MediaKind::Video => video_to_budget(path, max_bytes, tmp.path())?,
        MediaKind::Audio => audio_to_budget(path, max_bytes, base, tmp.path())?,
        _ => None,
    };
    if let Some(staged) = by_rate {
        // Video is re-encoded to mp4; in place that replaces a `.mov` source.
        let (dest, replace) = if kind == MediaKind::Video {
            (path.with_extension("mp4"), true)
        } else {
            (path.to_path_buf(), false)
        };
        return finish(
            kind,
            path,
            &staged,
            dest,
            old_size,
            false,
            base,
            Placement {
                size_guard: false,
                backup: true,
                replace_source: replace,
            },
        );
    }

    ladder(path, kind, max_bytes, base, tmp.path())
}

/// Encode video straight to the bitrate the budget allows (two-pass), then
/// correct: a big overshoot means the encoder is at its quality floor (too few
/// bits for the frame size), so the frame is shrunk; a small one lowers the
/// bitrate; undershooting far after an overshoot bisects between the two.
/// Returns the largest candidate within budget (else the smallest), or `None`
/// if the duration can't be probed.
fn video_to_budget(
    path: &Path,
    max_bytes: u64,
    tmp: &Path,
) -> Result<Option<PathBuf>, OptimiseError> {
    let Some(info) = crate::video::probe(path) else {
        return Ok(None);
    };
    let Some(mut plan) = crate::video::plan_bitrate(max_bytes, &info) else {
        return Ok(None);
    };
    let max = max_bytes as f64;
    let mut within: Option<(PathBuf, u64)> = None;
    let mut smallest: Option<(PathBuf, u64)> = None;
    let mut last_over_kbps: Option<u32> = None;
    for attempt in 0..4 {
        let out = tmp.join(format!("rate{attempt}.mp4"));
        crate::video::encode_to_bitrate(path, &out, &plan, info.hdr)?;
        let size = file_size(&out);
        if smallest.as_ref().is_none_or(|(_, b)| size < *b) {
            smallest = Some((out.clone(), size));
        }
        let ratio = size as f64 / max;
        if size <= max_bytes {
            if within.as_ref().is_none_or(|(_, b)| size > *b) {
                within = Some((out, size));
            }
            // Close enough — or nothing better to try.
            let Some(over) = last_over_kbps.filter(|_| ratio < 0.8) else {
                break;
            };
            plan.video_kbps = ((plan.video_kbps as f64 * over as f64).sqrt()) as u32;
            continue;
        }
        last_over_kbps = Some(plan.video_kbps);
        let (w, h) = plan.scale.unwrap_or((info.width, info.height));
        if ratio > 1.3 && h > 240 && w > 0 {
            let k = (1.0 / ratio).sqrt();
            let nh = ((h as f64 * k).max(240.0) / 2.0).round() as u32 * 2;
            let nw = ((nh as f64 * w as f64 / h as f64) / 2.0).round() as u32 * 2;
            plan.scale = Some((nw.max(2), nh.max(2)));
        } else {
            plan.video_kbps = ((plan.video_kbps as f64 / ratio * 0.97) as u32).max(10);
        }
    }
    Ok(within.or(smallest).map(|(p, _)| p))
}

/// Lossy audio: encode at the bitrate the budget allows, then correct in
/// proportion if the result lands well off (VBR encoders drift). Returns the
/// largest candidate within budget (else the smallest). `None` for lossless
/// formats or an unknown length.
fn audio_to_budget(
    path: &Path,
    max_bytes: u64,
    base: &OptimiseOptions,
    tmp: &Path,
) -> Result<Option<PathBuf>, OptimiseError> {
    let ext = crate::filetype::extension_lower(path).unwrap_or_default();
    let format = AudioFormat::SameAsInput.resolved(&ext);
    let Some((lo, hi)) = format.bitrate_range() else {
        return Ok(None);
    };
    let Some(info) = crate::video::probe(path).filter(|i| i.duration > 0.0) else {
        return Ok(None);
    };
    let max = max_bytes as f64;
    let mut kbps = ((max * 8.0 / info.duration / 1000.0 * 0.97) as i32).clamp(lo, hi);
    let mut within: Option<(PathBuf, u64)> = None;
    let mut smallest: Option<(PathBuf, u64)> = None;
    for attempt in 0..3 {
        let out = tmp.join(format!("rate{attempt}.{}", format.file_extension()));
        let opts = OptimiseOptions {
            output: Some(out.clone()),
            backup: false,
            allow_larger: true,
            ..base.clone()
        };
        crate::audio::optimise(path, &opts, AudioFormat::SameAsInput, Some(kbps))?;
        let size = file_size(&out);
        if smallest.as_ref().is_none_or(|(_, b)| size < *b) {
            smallest = Some((out.clone(), size));
        }
        let ratio = size as f64 / max;
        let next = ((kbps as f64 / ratio * 0.97) as i32).clamp(lo, hi);
        if size <= max_bytes {
            if within.as_ref().is_none_or(|(_, b)| size > *b) {
                within = Some((out, size));
            }
            if ratio >= 0.85 || next <= kbps {
                break;
            }
        } else if kbps <= lo {
            break;
        }
        kbps = next;
    }
    Ok(within.or(smallest).map(|(p, _)| p))
}

/// Try progressively harder compression factors until the result fits.
fn ladder(
    path: &Path,
    kind: MediaKind,
    max_bytes: u64,
    base: &OptimiseOptions,
    tmp: &Path,
) -> Result<OptimisationResult, OptimiseError> {
    // Factors to try, from gentle to aggressive.
    let factors = [30, 50, 64, 80, 90, 100];
    let mut best: Option<OptimisationResult> = None;

    // Always write to a temp candidate so we don't clobber the source between tries.
    for (i, &factor) in factors.iter().enumerate() {
        let candidate = tmp.join(format!(
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

    // Place the best candidate at the final destination (even if it is still
    // over budget: it is the smallest this format can get).
    finish(
        kind,
        path,
        &best.output,
        path.to_path_buf(),
        file_size(path),
        true,
        base,
        Placement {
            size_guard: false,
            backup: true,
            replace_source: false,
        },
    )
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
