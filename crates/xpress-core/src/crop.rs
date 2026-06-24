//! Crop / resize to a target size or aspect ratio.
//!
//! Images go through `vips`/`vipsthumbnail` (with a centre/attention smart crop);
//! videos use ffmpeg `scale=`/`crop=` expressions. The result is then optimised.

use std::path::Path;
use std::path::PathBuf;

use tempfile::TempDir;

use crate::filetype::{classify, MediaKind};
use crate::result::{
    backup_file, copy_dates, file_size, OptimisationResult, OptimiseError, OptimiseOptions,
};
use crate::scale::image_dimensions;
use crate::tools::{self, Tool};
use crate::{image, video};

/// What the user asked for. Exactly one of the size axes is typically meaningful.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CropSpec {
    /// Target width in pixels (0 / None = auto).
    pub width: Option<u32>,
    /// Target height in pixels (0 / None = auto).
    pub height: Option<u32>,
    /// Single number applied to the longer edge (keeps aspect, no crop).
    pub long_edge: Option<u32>,
    /// Aspect ratio `(w, h)` — crops to ratio without scaling.
    pub aspect: Option<(u32, u32)>,
    /// Use feature-aware crop (vips `attention`) instead of centre.
    pub smart: bool,
}

impl CropSpec {
    pub fn size(width: u32, height: u32) -> Self {
        Self {
            width: (width > 0).then_some(width),
            height: (height > 0).then_some(height),
            long_edge: None,
            aspect: None,
            smart: false,
        }
    }

    /// Parse a `--size` argument: `1200x630`, `1200x0`, `0x720`, `16:9`, or `1920`.
    pub fn parse(s: &str) -> Result<CropSpec, String> {
        let s = s.trim();
        if let Some((a, b)) = s.split_once(':') {
            let aw: u32 = a.trim().parse().map_err(|_| format!("bad ratio '{s}'"))?;
            let ah: u32 = b.trim().parse().map_err(|_| format!("bad ratio '{s}'"))?;
            if aw == 0 || ah == 0 {
                return Err(format!("ratio parts must be > 0 in '{s}'"));
            }
            return Ok(CropSpec {
                width: None,
                height: None,
                long_edge: None,
                aspect: Some((aw, ah)),
                smart: false,
            });
        }
        if let Some((a, b)) = s.split_once('x') {
            let w: u32 = a
                .trim()
                .parse()
                .map_err(|_| format!("bad width in '{s}'"))?;
            let h: u32 = b
                .trim()
                .parse()
                .map_err(|_| format!("bad height in '{s}'"))?;
            return Ok(CropSpec::size(w, h));
        }
        let n: u32 = s.parse().map_err(|_| format!("bad size '{s}'"))?;
        Ok(CropSpec {
            width: Some(n),
            height: Some(n),
            long_edge: None,
            aspect: None,
            smart: false,
        })
    }

    pub fn with_long_edge(mut self, on: bool) -> Self {
        if on {
            // A single number was given as width==height; treat it as the long edge.
            self.long_edge = self.width.or(self.height);
            self.width = None;
            self.height = None;
        }
        self
    }

    pub fn with_smart(mut self, on: bool) -> Self {
        self.smart = on;
        self
    }
}

fn even(n: u32) -> u32 {
    if n.is_multiple_of(2) {
        n
    } else {
        n.saturating_sub(1).max(2)
    }
}

/// How the source maps to the target.
enum Plan {
    /// Scale keeping aspect to exactly these dims (no crop).
    Resize(u32, u32),
    /// Scale to cover then crop to exactly these dims.
    Cover(u32, u32),
    /// Crop centred to these dims, no scaling.
    CropOnly(u32, u32),
}

/// Resolve the crop plan for an image whose dimensions are known.
fn plan_for_image(sw: u32, sh: u32, spec: &CropSpec) -> Plan {
    if let Some(n) = spec.long_edge {
        return if sw >= sh {
            Plan::Resize(n, even(((n as f64) * sh as f64 / sw as f64).round() as u32))
        } else {
            Plan::Resize(even(((n as f64) * sw as f64 / sh as f64).round() as u32), n)
        };
    }
    if let Some((aw, ah)) = spec.aspect {
        let r = aw as f64 / ah as f64;
        let (mut tw, mut th) = if sw as f64 / sh as f64 > r {
            (((sh as f64) * r).round() as u32, sh)
        } else {
            (sw, ((sw as f64) / r).round() as u32)
        };
        tw = tw.min(sw).max(2);
        th = th.min(sh).max(2);
        return Plan::CropOnly(tw, th);
    }
    match (spec.width, spec.height) {
        (Some(w), Some(h)) => Plan::Cover(w, h),
        (Some(w), None) => {
            Plan::Resize(w, even(((w as f64) * sh as f64 / sw as f64).round() as u32))
        }
        (None, Some(h)) => {
            Plan::Resize(even(((h as f64) * sw as f64 / sh as f64).round() as u32), h)
        }
        (None, None) => Plan::Resize(sw, sh),
    }
}

/// Crop/resize a file then optimise it.
pub fn crop_file(
    path: &Path,
    spec: &CropSpec,
    options: &OptimiseOptions,
) -> Result<OptimisationResult, OptimiseError> {
    if !path.is_file() {
        return Err(OptimiseError::NotFound(path.to_path_buf()));
    }
    let old_size = file_size(path);
    match classify(path) {
        Some(MediaKind::Image) => crop_image(path, spec, old_size, options),
        Some(MediaKind::Video) => crop_video(path, spec, old_size, options),
        _ => Err(OptimiseError::Unsupported(path.to_path_buf())),
    }
}

fn crop_image(
    path: &Path,
    spec: &CropSpec,
    old_size: u64,
    options: &OptimiseOptions,
) -> Result<OptimisationResult, OptimiseError> {
    let (sw, sh) = image_dimensions(path).ok_or_else(|| {
        OptimiseError::Other(format!("could not read dimensions of {}", path.display()))
    })?;
    let plan = plan_for_image(sw, sh, spec);

    let tmp = TempDir::new()?;
    let cropped = tmp.path().join(path.file_name().unwrap());
    let src = path.display().to_string();
    let dst = cropped.display().to_string();

    let use_vips = tools::is_available(Tool::Vipsthumbnail) || tools::is_available(Tool::Vips);
    match plan {
        Plan::Resize(w, h) => {
            if tools::is_available(Tool::Vipsthumbnail) {
                tools::run_with_retries(
                    Tool::Vipsthumbnail,
                    ["-s", &format!("{w}x{h}"), "-o", &dst, &src],
                    2,
                )?;
            } else if tools::is_available(Tool::Vips) {
                let f = (w as f64 / sw as f64).min(h as f64 / sh as f64);
                tools::run_with_retries(Tool::Vips, ["resize", &src, &dst, &format!("{f:.5}")], 2)?;
            } else {
                ffmpeg_vf(&src, &dst, &format!("scale={w}:{h}"))?;
            }
        }
        Plan::Cover(w, h) => {
            if tools::is_available(Tool::Vipsthumbnail) {
                let crop = if spec.smart { "attention" } else { "centre" };
                tools::run_with_retries(
                    Tool::Vipsthumbnail,
                    [
                        "-s",
                        &format!("{w}x{h}"),
                        "--smartcrop",
                        crop,
                        "-o",
                        &dst,
                        &src,
                    ],
                    2,
                )?;
            } else {
                ffmpeg_vf(
                    &src,
                    &dst,
                    &format!("scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h}"),
                )?;
            }
        }
        Plan::CropOnly(w, h) => {
            if tools::is_available(Tool::Vips) {
                let x = (sw - w) / 2;
                let y = (sh - h) / 2;
                tools::run_with_retries(
                    Tool::Vips,
                    [
                        "crop",
                        &src,
                        &dst,
                        &x.to_string(),
                        &y.to_string(),
                        &w.to_string(),
                        &h.to_string(),
                    ],
                    2,
                )?;
            } else if use_vips {
                let crop = if spec.smart { "attention" } else { "centre" };
                tools::run_with_retries(
                    Tool::Vipsthumbnail,
                    [
                        "-s",
                        &format!("{w}x{h}"),
                        "--smartcrop",
                        crop,
                        "-o",
                        &dst,
                        &src,
                    ],
                    2,
                )?;
            } else {
                ffmpeg_vf(&src, &dst, &format!("crop={w}:{h}:(iw-{w})/2:(ih-{h})/2"))?;
            }
        }
    }

    finalise_image(path, &cropped, old_size, options)
}

fn crop_video(
    path: &Path,
    spec: &CropSpec,
    old_size: u64,
    options: &OptimiseOptions,
) -> Result<OptimisationResult, OptimiseError> {
    // Build an ffmpeg filter purely from expressions, so we never need ffprobe.
    let vf = if let Some(n) = spec.long_edge {
        format!("scale='if(gt(iw,ih),{n},-2)':'if(gt(iw,ih),-2,{n})'")
    } else if let Some((aw, ah)) = spec.aspect {
        // largest centred rect of the ratio
        format!("crop='min(iw,ih*{aw}/{ah})':'min(ih,iw*{ah}/{aw})'",)
    } else {
        match (spec.width, spec.height) {
            (Some(w), Some(h)) => {
                format!("scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h}")
            }
            (Some(w), None) => format!("scale={w}:-2"),
            (None, Some(h)) => format!("scale=-2:{h}"),
            (None, None) => return Err(OptimiseError::Other("crop: no target size given".into())),
        }
    };
    let mut r = video::optimise_with_filter(path, options, Some(&vf))?;
    r.old_size = old_size;
    Ok(r)
}

fn ffmpeg_vf(src: &str, dst: &str, vf: &str) -> Result<(), OptimiseError> {
    tools::run(Tool::Ffmpeg, ["-y", "-i", src, "-vf", vf, dst])?;
    Ok(())
}

fn finalise_image(
    path: &Path,
    cropped: &Path,
    old_size: u64,
    options: &OptimiseOptions,
) -> Result<OptimisationResult, OptimiseError> {
    let dest: PathBuf = options.output.clone().unwrap_or_else(|| path.to_path_buf());
    let opt_options = OptimiseOptions {
        output: Some(dest.clone()),
        backup: false,
        allow_larger: true,
        ..options.clone()
    };
    let mut result = image::optimise(cropped, &opt_options)?;
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
