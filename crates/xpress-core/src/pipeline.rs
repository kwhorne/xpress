//! A small pipeline DSL: a chain of steps joined by `->`, e.g.
//!
//! ```text
//! crop(width: 1600) -> convert(to: webp) -> downscale(factor: 0.5)
//! ```
//!
//! Each step transforms the working file and feeds the next. The final artifact
//! is placed at the pipeline output (or back next to the source).

use std::path::Path;

use tempfile::TempDir;

use crate::audio::{self, AudioFormat};
use crate::crop::{self, CropSpec};
use crate::filetype::{classify, extension_lower, MediaKind};
use crate::image::{self, ImageFormat};
use crate::result::{
    backup_file, copy_dates, file_size, OptimisationResult, OptimiseError, OptimiseOptions,
};
use crate::tools;
use crate::{scale, video};

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Optimise,
    Downscale { factor: f64 },
    Crop(CropArgs),
    Convert { to: String },
    StripExif,
    RemoveAudio,
    ChangeSpeed { factor: f64 },
    CapFps { fps: i32 },
    LowerBitrate { kbps: i32 },
    TargetSize { bytes: u64 },
    Adaptive,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CropArgs {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub long_edge: Option<u32>,
    pub aspect: Option<(u32, u32)>,
    pub smart: bool,
}

impl CropArgs {
    fn to_spec(&self) -> CropSpec {
        CropSpec {
            width: self.width,
            height: self.height,
            long_edge: self.long_edge,
            aspect: self.aspect,
            smart: self.smart,
        }
    }
}

/// Parse a full pipeline string into steps.
pub fn parse(dsl: &str) -> Result<Vec<Step>, String> {
    dsl.split("->")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(parse_step)
        .collect()
}

fn parse_step(s: &str) -> Result<Step, String> {
    let (name, args_str) = match s.split_once('(') {
        Some((n, rest)) => {
            let rest = rest.trim_end();
            let inner = rest
                .strip_suffix(')')
                .ok_or_else(|| format!("missing ')' in step '{s}'"))?;
            (n.trim(), inner)
        }
        None => (s.trim(), ""),
    };
    let args = parse_args(args_str)?;
    let get = |k: &str| {
        args.iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.clone())
    };

    match name {
        "optimise" | "optimize" => Ok(Step::Optimise),
        "downscale" => {
            let factor = get("factor")
                .map(|v| parse_factor(&v))
                .transpose()?
                .ok_or("downscale requires factor:")?;
            Ok(Step::Downscale { factor })
        }
        "crop" => {
            let width = get("width")
                .map(|v| v.parse::<u32>().map_err(|_| "bad width"))
                .transpose()?;
            let height = get("height")
                .map(|v| v.parse::<u32>().map_err(|_| "bad height"))
                .transpose()?;
            let long_edge = get("longEdge")
                .or_else(|| get("long_edge"))
                .map(|v| v.parse::<u32>().map_err(|_| "bad longEdge"))
                .transpose()?;
            let aspect = get("ratio")
                .or_else(|| get("aspect"))
                .map(|v| parse_ratio(&v))
                .transpose()?;
            let smart = get("smart").map(|v| v == "true").unwrap_or(false);
            if width.is_none() && height.is_none() && long_edge.is_none() && aspect.is_none() {
                return Err("crop requires width:, height:, longEdge: or ratio:".into());
            }
            Ok(Step::Crop(CropArgs {
                width,
                height,
                long_edge,
                aspect,
                smart,
            }))
        }
        "convert" => {
            let to = get("to").ok_or("convert requires to:")?;
            Ok(Step::Convert { to })
        }
        "stripExif" | "stripexif" | "strip_exif" => Ok(Step::StripExif),
        "removeAudio" | "removeaudio" | "remove_audio" => Ok(Step::RemoveAudio),
        "changeSpeed" | "change_speed" => {
            let factor = get("factor")
                .map(|v| parse_factor(&v))
                .transpose()?
                .ok_or("changeSpeed requires factor:")?;
            Ok(Step::ChangeSpeed { factor })
        }
        "capFps" | "cap_fps" => {
            let fps = get("fps")
                .ok_or("capFps requires fps:")?
                .parse::<i32>()
                .map_err(|_| "bad fps")?;
            Ok(Step::CapFps { fps })
        }
        "lowerBitrate" | "lower_bitrate" => {
            let kbps = get("kbps")
                .ok_or("lowerBitrate requires kbps:")?
                .parse::<i32>()
                .map_err(|_| "bad kbps")?;
            Ok(Step::LowerBitrate { kbps })
        }
        "targetSize" | "target_size" => {
            let raw = get("bytes")
                .or_else(|| get("kb"))
                .ok_or("targetSize requires bytes: or kb:")?;
            Ok(Step::TargetSize {
                bytes: parse_size(&raw)?,
            })
        }
        "adaptive" => Ok(Step::Adaptive),
        other => Err(format!("unknown step '{other}'")),
    }
}

fn parse_args(s: &str) -> Result<Vec<(String, String)>, String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(vec![]);
    }
    let mut out = Vec::new();
    for part in s.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (k, v) = part
            .split_once(':')
            .ok_or_else(|| format!("expected 'key: value' in '{part}'"))?;
        let v = v.trim().trim_matches('"').trim_matches('\'').to_string();
        out.push((k.trim().to_string(), v));
    }
    Ok(out)
}

fn parse_factor(v: &str) -> Result<f64, String> {
    if let Some(pct) = v.strip_suffix('%') {
        let n: f64 = pct
            .trim()
            .parse()
            .map_err(|_| format!("bad percent '{v}'"))?;
        Ok(n / 100.0)
    } else {
        v.parse::<f64>().map_err(|_| format!("bad factor '{v}'"))
    }
}

fn parse_ratio(v: &str) -> Result<(u32, u32), String> {
    let (a, b) = v
        .split_once(':')
        .ok_or_else(|| format!("bad ratio '{v}'"))?;
    Ok((
        a.trim().parse().map_err(|_| "bad ratio")?,
        b.trim().parse().map_err(|_| "bad ratio")?,
    ))
}

/// Run a parsed pipeline on one file. `options.output` (when set) is the final
/// destination; otherwise the result replaces / sits next to the source.
pub fn run(
    source: &Path,
    steps: &[Step],
    options: &OptimiseOptions,
) -> Result<OptimisationResult, OptimiseError> {
    if !source.is_file() {
        return Err(OptimiseError::NotFound(source.to_path_buf()));
    }
    let old_size = file_size(source);
    let work = TempDir::new()?;

    // The working file starts as a copy of the source.
    let mut current = work.path().join(crate::result::file_name_lossy(source));
    std::fs::copy(source, &current)?;

    let mut counter = 0usize;
    for step in steps {
        counter += 1;
        let next_ext = step_output_ext(step, &current);
        let target = work.path().join(format!("s{counter}.{next_ext}"));
        apply_step(step, &current, &target, options)?;
        current = target;
    }

    // Place the final artifact.
    let final_ext = extension_lower(&current).unwrap_or_default();
    let src_ext = extension_lower(source).unwrap_or_default();
    let new_size = file_size(&current);

    let (dest, backup) = if let Some(out) = &options.output {
        (out.clone(), None)
    } else if final_ext == src_ext {
        // Same type: replace in place with a backup.
        let b = if options.backup {
            Some(backup_file(source)?)
        } else {
            None
        };
        (source.to_path_buf(), b)
    } else {
        // Type changed (e.g. convert): write alongside, keep the original.
        (source.with_extension(&final_ext), None)
    };

    std::fs::copy(&current, &dest)?;
    if options.preserve_dates {
        copy_dates(source, &dest);
    }

    Ok(OptimisationResult {
        kind: classify(&dest).unwrap_or(MediaKind::Image),
        source: source.to_path_buf(),
        output: dest,
        backup,
        old_size,
        new_size,
        aggressive: options.compression.image_is_aggressive(),
    })
}

/// What extension the step's output will carry.
fn step_output_ext(step: &Step, current: &Path) -> String {
    let cur = extension_lower(current).unwrap_or_else(|| "bin".into());
    match step {
        Step::Convert { to } => {
            if to.eq_ignore_ascii_case("gif") {
                "gif".to_string()
            } else if let Some(f) = ImageFormat::from_str(to) {
                f.extension().to_string()
            } else if let Some(a) = AudioFormat::from_target(to) {
                a.file_extension().to_string()
            } else {
                cur
            }
        }
        // Video transforms normalise to mp4.
        Step::RemoveAudio | Step::ChangeSpeed { .. } | Step::CapFps { .. }
            if classify(current) == Some(MediaKind::Video) =>
        {
            "mp4".into()
        }
        Step::Optimise | Step::Downscale { .. } if classify(current) == Some(MediaKind::Video) => {
            "mp4".into()
        }
        _ => cur,
    }
}

fn apply_step(
    step: &Step,
    current: &Path,
    target: &Path,
    base: &OptimiseOptions,
) -> Result<(), OptimiseError> {
    // Per-step options: write to `target`, never back up intermediates.
    let opts = OptimiseOptions {
        output: Some(target.to_path_buf()),
        backup: false,
        allow_larger: true,
        ..base.clone()
    };
    match step {
        Step::Optimise => {
            crate::optimise_file(current, &opts, AudioFormat::SameAsInput, None)?;
        }
        Step::Downscale { factor } => {
            scale::downscale_file(current, *factor, &opts)?;
        }
        Step::Crop(args) => {
            crop::crop_file(current, &args.to_spec(), &opts)?;
        }
        Step::Convert { to } => {
            if to.eq_ignore_ascii_case("gif") && classify(current) == Some(MediaKind::Video) {
                video::to_gif(current, &opts, 15, None)?;
            } else if let Some(f) = ImageFormat::from_str(to) {
                image::convert(current, f, &opts)?;
            } else if let Some(a) = AudioFormat::from_target(to) {
                audio::optimise(current, &opts, a, None)?;
            } else {
                return Err(OptimiseError::Other(format!(
                    "convert: unknown format '{to}'"
                )));
            }
        }
        Step::StripExif => {
            std::fs::copy(current, target)?;
            tools::copy_exif(current, target, true, &[]);
        }
        Step::RemoveAudio => {
            video::remove_audio(current, target)?;
        }
        Step::ChangeSpeed { factor } => {
            video::change_speed(current, target, *factor, &opts)?;
        }
        Step::CapFps { fps } => {
            video::cap_fps(current, target, *fps, &opts)?;
        }
        Step::LowerBitrate { kbps } => {
            let fmt = AudioFormat::SameAsInput;
            audio::optimise(current, &opts, fmt, Some(*kbps))?;
        }
        Step::TargetSize { bytes } => {
            crate::budget::optimise_to_budget(current, *bytes, &opts)?;
        }
        Step::Adaptive => {
            if classify(current) == Some(MediaKind::Image) {
                image::optimise_adaptive(current, &opts)?;
            } else {
                crate::optimise_file(current, &opts, AudioFormat::SameAsInput, None)?;
            }
        }
    }
    if !target.exists() {
        return Err(OptimiseError::Other(format!(
            "step produced no output: {step:?}"
        )));
    }
    Ok(())
}

/// Render steps back to canonical DSL (for `pipeline show`).
pub fn to_dsl(steps: &[Step]) -> String {
    steps
        .iter()
        .map(step_to_string)
        .collect::<Vec<_>>()
        .join(" -> ")
}

fn step_to_string(step: &Step) -> String {
    match step {
        Step::Optimise => "optimise".into(),
        Step::Downscale { factor } => format!("downscale(factor: {factor})"),
        Step::Crop(a) => {
            let mut parts = Vec::new();
            if let Some(w) = a.width {
                parts.push(format!("width: {w}"));
            }
            if let Some(h) = a.height {
                parts.push(format!("height: {h}"));
            }
            if let Some(l) = a.long_edge {
                parts.push(format!("longEdge: {l}"));
            }
            if let Some((x, y)) = a.aspect {
                parts.push(format!("ratio: {x}:{y}"));
            }
            if a.smart {
                parts.push("smart: true".into());
            }
            format!("crop({})", parts.join(", "))
        }
        Step::Convert { to } => format!("convert(to: {to})"),
        Step::StripExif => "stripExif".into(),
        Step::RemoveAudio => "removeAudio".into(),
        Step::ChangeSpeed { factor } => format!("changeSpeed(factor: {factor})"),
        Step::CapFps { fps } => format!("capFps(fps: {fps})"),
        Step::LowerBitrate { kbps } => format!("lowerBitrate(kbps: {kbps})"),
        Step::TargetSize { bytes } => format!("targetSize(bytes: {bytes})"),
        Step::Adaptive => "adaptive".into(),
    }
}

/// Parse a size like `500000`, `500kb`, `1.5mb`.
fn parse_size(v: &str) -> Result<u64, String> {
    let s = v.trim().to_ascii_lowercase();
    let (num, mult) = if let Some(n) = s.strip_suffix("mb") {
        (n.trim(), 1_000_000.0)
    } else if let Some(n) = s.strip_suffix("kb") {
        (n.trim(), 1_000.0)
    } else if let Some(n) = s.strip_suffix('b') {
        (n.trim(), 1.0)
    } else {
        (s.as_str(), 1.0)
    };
    let value: f64 = num.parse().map_err(|_| format!("bad size '{v}'"))?;
    Ok((value * mult) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_chain() {
        let steps =
            parse("crop(width: 1600) -> convert(to: webp) -> downscale(factor: 50%)").unwrap();
        assert_eq!(steps.len(), 3);
        assert!(matches!(
            steps[0],
            Step::Crop(CropArgs {
                width: Some(1600),
                ..
            })
        ));
        assert!(matches!(&steps[1], Step::Convert { to } if to == "webp"));
        assert!(matches!(steps[2], Step::Downscale { factor } if (factor - 0.5).abs() < 1e-9));
    }

    #[test]
    fn roundtrip_dsl() {
        let dsl = "crop(width: 1200, height: 630) -> optimise";
        let steps = parse(dsl).unwrap();
        let back = to_dsl(&steps);
        assert_eq!(parse(&back).unwrap(), steps);
    }

    #[test]
    fn ratio_and_longedge() {
        let steps = parse("crop(ratio: 16:9) -> crop(longEdge: 1920)").unwrap();
        assert!(matches!(
            steps[0],
            Step::Crop(CropArgs {
                aspect: Some((16, 9)),
                ..
            })
        ));
        assert!(matches!(
            steps[1],
            Step::Crop(CropArgs {
                long_edge: Some(1920),
                ..
            })
        ));
    }
}
