//! Video optimisation via ffmpeg (default H.264 path).

use std::path::Path;

use tempfile::TempDir;

use crate::filetype::MediaKind;
use crate::result::{
    backup_file, copy_dates, file_size, OptimisationResult, OptimiseError, OptimiseOptions,
};
use crate::tools::{self, Tool};

/// Optimise a video in place (or to `options.output`), re-encoding to H.264/mp4.
pub fn optimise(
    path: &Path,
    options: &OptimiseOptions,
) -> Result<OptimisationResult, OptimiseError> {
    optimise_with_filter(path, options, None)
}

/// Strip the audio track. Stream-copies video, no re-encode. Writes to `dst`.
pub fn remove_audio(src: &Path, dst: &Path) -> Result<(), OptimiseError> {
    tools::run(
        Tool::Ffmpeg,
        [
            "-y",
            "-i",
            &src.display().to_string(),
            "-c",
            "copy",
            "-an",
            "-hide_banner",
            "-nostats",
            &dst.display().to_string(),
        ],
    )?;
    Ok(())
}

/// Change playback speed by `factor` (e.g. 2.0 = twice as fast). Re-encodes via
/// setpts (video) and atempo (audio). Writes to `dst`.
pub fn change_speed(
    src: &Path,
    dst: &Path,
    factor: f64,
    options: &OptimiseOptions,
) -> Result<(), OptimiseError> {
    let f = factor.clamp(0.25, 8.0);
    let pts = 1.0 / f;
    let mut args: Vec<String> = vec![
        "-y".into(),
        "-i".into(),
        src.display().to_string(),
        "-filter:v".into(),
        format!("setpts={pts:.5}*PTS"),
        "-filter:a".into(),
        format!("atempo={f:.5}"),
    ];
    args.extend(options.compression.video_h264_args(tools::is_arm64()));
    args.extend(["-hide_banner", "-nostats"].map(String::from));
    args.push(dst.display().to_string());
    tools::run(Tool::Ffmpeg, &args)?;
    Ok(())
}

/// Target codecs for explicit video conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
    Hevc,
    Av1,
    Vp9,
}

impl VideoCodec {
    pub fn from_target(s: &str) -> Option<VideoCodec> {
        match s.to_ascii_lowercase().as_str() {
            "mp4" | "h264" | "avc" => Some(VideoCodec::H264),
            "hevc" | "h265" | "x265" => Some(VideoCodec::Hevc),
            "av1" => Some(VideoCodec::Av1),
            "webm" | "vp9" => Some(VideoCodec::Vp9),
            _ => None,
        }
    }

    pub fn container_ext(&self) -> &'static str {
        match self {
            VideoCodec::H264 | VideoCodec::Hevc | VideoCodec::Av1 => "mp4",
            VideoCodec::Vp9 => "webm",
        }
    }
}

/// Convert a video to a specific codec/container. `hw` requests a hardware
/// encoder (VideoToolbox) on Apple Silicon where applicable.
pub fn convert_codec(
    path: &Path,
    codec: VideoCodec,
    options: &OptimiseOptions,
    hw: bool,
) -> Result<OptimisationResult, OptimiseError> {
    if !path.is_file() {
        return Err(OptimiseError::NotFound(path.to_path_buf()));
    }
    let old_size = file_size(path);
    let cq = options.compression;
    let crf = cq.video_h264_crf(); // 17..32 baseline
    let arm = tools::is_arm64();
    let s = |v: &str| v.to_string();

    let ext = codec.container_ext();
    let webm = codec == VideoCodec::Vp9;

    let vcodec_args: Vec<String> = match codec {
        VideoCodec::H264 => cq.video_h264_args(arm && hw),
        VideoCodec::Hevc => {
            if arm && hw {
                vec![s("-vcodec"), s("hevc_videotoolbox"), s("-tag:v"), s("hvc1")]
            } else {
                vec![
                    s("-vcodec"),
                    s("libx265"),
                    s("-tag:v"),
                    s("hvc1"),
                    s("-preset"),
                    s(cq.video_h264_preset()),
                    s("-crf"),
                    crf.to_string(),
                ]
            }
        }
        VideoCodec::Av1 => {
            // SVT-AV1 CRF scale ~0..63. Map 5..100 -> 24..50.
            let av1_crf = 24 + ((cq.factor.max(5) - 5) as f64 / 95.0 * 26.0).round() as i32;
            vec![
                s("-vcodec"),
                s("libsvtav1"),
                s("-crf"),
                av1_crf.to_string(),
                s("-preset"),
                s("6"),
            ]
        }
        VideoCodec::Vp9 => {
            let vp9_crf = 24 + ((cq.factor.max(5) - 5) as f64 / 95.0 * 24.0).round() as i32;
            vec![
                s("-vcodec"),
                s("libvpx-vp9"),
                s("-crf"),
                vp9_crf.to_string(),
                s("-b:v"),
                s("0"),
            ]
        }
    };

    let tmp = TempDir::new()?;
    let temp_out = tmp
        .path()
        .join(format!("{}.{ext}", crate::result::file_stem_lossy(path)));

    let build = |reencode_audio: bool| -> Vec<String> {
        let mut args: Vec<String> = vec![s("-y"), s("-i"), path.display().to_string()];
        args.extend(vcodec_args.clone());
        if webm {
            args.extend(
                [
                    "-c:a", "libopus", "-b:a", "128k", "-map", "0:v", "-map", "0:a?",
                ]
                .map(String::from),
            );
        } else if reencode_audio {
            args.extend(["-c:a", "aac", "-b:a", "192k"].map(String::from));
        } else {
            args.extend(["-c:a", "copy", "-map", "0:v", "-map", "0:a?"].map(String::from));
        }
        if !webm {
            args.extend(["-movflags", "+faststart"].map(String::from));
        }
        args.extend(["-hide_banner", "-nostats"].map(String::from));
        args.push(temp_out.display().to_string());
        args
    };

    if tools::run(Tool::Ffmpeg, build(false)).is_err() {
        tools::run(Tool::Ffmpeg, build(true))?;
    }

    let new_size = file_size(&temp_out);
    let dest = options
        .output
        .clone()
        .unwrap_or_else(|| path.with_extension(ext));
    let backup = if options.backup && options.output.is_none() && dest == path {
        Some(backup_file(path)?)
    } else {
        None
    };
    std::fs::copy(&temp_out, &dest)?;
    if options.preserve_dates {
        copy_dates(path, &dest);
    }
    if options.output.is_none() && dest != path && path.exists() {
        let _ = std::fs::remove_file(path);
    }

    Ok(OptimisationResult {
        kind: MediaKind::Video,
        source: path.to_path_buf(),
        output: dest,
        backup,
        old_size,
        new_size,
        aggressive: cq.image_is_aggressive(),
    })
}

/// Convert a video to an animated GIF.
///
/// Uses `gifski` (best quality) when available, extracting frames with ffmpeg;
/// otherwise falls back to a single ffmpeg pass. `fps` defaults to 15 and
/// `max_width` optionally caps the width (keeping aspect).
pub fn to_gif(
    path: &Path,
    options: &OptimiseOptions,
    fps: u32,
    max_width: Option<u32>,
) -> Result<OptimisationResult, OptimiseError> {
    if !path.is_file() {
        return Err(OptimiseError::NotFound(path.to_path_buf()));
    }
    let old_size = file_size(path);
    let fps = fps.max(1);
    let quality = if options.compression.image_is_aggressive() {
        60
    } else {
        90
    };

    let tmp = TempDir::new()?;
    let out = tmp
        .path()
        .join(format!("{}.gif", crate::result::file_stem_lossy(path)));

    if tools::is_available(Tool::Gifski) {
        // Extract frames, then assemble with gifski.
        let frames = tmp.path().join("frames");
        std::fs::create_dir_all(&frames)?;
        let mut vf = format!("fps={fps}");
        if let Some(w) = max_width {
            vf.push_str(&format!(",scale={w}:-1:flags=lanczos"));
        }
        tools::run(
            Tool::Ffmpeg,
            [
                "-y",
                "-i",
                &path.display().to_string(),
                "-vf",
                &vf,
                &frames.join("f%05d.png").display().to_string(),
            ],
        )?;
        let mut args: Vec<String> = vec![
            "-o".into(),
            out.display().to_string(),
            "--fps".into(),
            fps.to_string(),
            "--quality".into(),
            quality.to_string(),
        ];
        if let Some(w) = max_width {
            args.push("--width".into());
            args.push(w.to_string());
        }
        // Collect frame paths.
        let mut pngs: Vec<String> = std::fs::read_dir(&frames)?
            .filter_map(|e| e.ok().map(|e| e.path().display().to_string()))
            .collect();
        pngs.sort();
        if pngs.is_empty() {
            return Err(OptimiseError::Other("no frames extracted for GIF".into()));
        }
        args.extend(pngs);
        tools::run(Tool::Gifski, &args)?;
    } else {
        // Single ffmpeg pass.
        let mut vf = format!("fps={fps}");
        if let Some(w) = max_width {
            vf.push_str(&format!(",scale={w}:-1:flags=lanczos"));
        }
        tools::run(
            Tool::Ffmpeg,
            [
                "-y",
                "-i",
                &path.display().to_string(),
                "-vf",
                &vf,
                "-hide_banner",
                "-nostats",
                &out.display().to_string(),
            ],
        )?;
    }

    let new_size = file_size(&out);
    let dest = options
        .output
        .clone()
        .unwrap_or_else(|| path.with_extension("gif"));
    std::fs::copy(&out, &dest)?;
    if options.preserve_dates {
        copy_dates(path, &dest);
    }

    Ok(OptimisationResult {
        kind: MediaKind::Image,
        source: path.to_path_buf(),
        output: dest,
        backup: None,
        old_size,
        new_size,
        aggressive: options.compression.image_is_aggressive(),
    })
}

/// Cap the frame rate at `fps`. Writes to `dst`.
pub fn cap_fps(
    src: &Path,
    dst: &Path,
    fps: i32,
    options: &OptimiseOptions,
) -> Result<(), OptimiseError> {
    let mut args: Vec<String> = vec![
        "-y".into(),
        "-i".into(),
        src.display().to_string(),
        "-vf".into(),
        format!("fps=fps={}", fps.max(1)),
    ];
    args.extend(options.compression.video_h264_args(tools::is_arm64()));
    args.extend(["-hide_banner", "-nostats"].map(String::from));
    args.push(dst.display().to_string());
    tools::run(Tool::Ffmpeg, &args)?;
    Ok(())
}

/// Like [`optimise`], but applies an optional ffmpeg `-vf` filter chain (e.g. a
/// `scale=` or `crop=` expression) during the encode.
pub fn optimise_with_filter(
    path: &Path,
    options: &OptimiseOptions,
    vf: Option<&str>,
) -> Result<OptimisationResult, OptimiseError> {
    if !path.is_file() {
        return Err(OptimiseError::NotFound(path.to_path_buf()));
    }
    let old_size = file_size(path);
    let cq = options.compression;

    let tmp = TempDir::new()?;
    let temp_out = tmp
        .path()
        .join(crate::result::file_name_lossy(&path.with_extension("mp4")));

    let build = |reencode_audio: bool| -> Vec<String> {
        // ffmpeg -y -i <in> [-vf <filter>] <encoderArgs> [-c:a copy -map ...] -movflags +faststart <out>
        let mut args: Vec<String> = vec!["-y".into(), "-i".into(), path.display().to_string()];
        if let Some(f) = vf {
            args.extend(["-vf".to_string(), f.to_string()]);
        }
        args.extend(cq.video_h264_args(tools::is_arm64()));
        if !reencode_audio {
            args.extend(["-c:a", "copy", "-map", "0:v", "-map", "0:a?"].map(String::from));
        }
        args.extend(["-movflags", "+faststart", "-hide_banner", "-nostats"].map(String::from));
        args.push(temp_out.display().to_string());
        args
    };

    // Try the audio-copy variant; on failure, retry re-encoding audio.
    if tools::run(Tool::Ffmpeg, build(false)).is_err() {
        tools::run(Tool::Ffmpeg, build(true))?;
    }

    let new_size = file_size(&temp_out);

    let same_ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("mp4"))
        .unwrap_or(false);

    if !options.allow_larger && same_ext && new_size >= old_size {
        return Ok(OptimisationResult {
            kind: MediaKind::Video,
            source: path.to_path_buf(),
            output: path.to_path_buf(),
            backup: None,
            old_size,
            new_size: old_size,
            aggressive: cq.image_is_aggressive(),
        });
    }

    let dest = options
        .output
        .clone()
        .unwrap_or_else(|| path.with_extension("mp4"));

    let backup = if options.backup && options.output.is_none() {
        Some(backup_file(path)?)
    } else {
        None
    };

    std::fs::copy(&temp_out, &dest)?;
    if options.preserve_dates {
        copy_dates(path, &dest);
    }
    // If we changed the extension (e.g. mov -> mp4) and replaced in place, remove the original.
    if options.output.is_none() && dest != path && path.exists() {
        let _ = std::fs::remove_file(path);
    }

    Ok(OptimisationResult {
        kind: MediaKind::Video,
        source: path.to_path_buf(),
        output: dest,
        backup,
        old_size,
        new_size,
        aggressive: cq.image_is_aggressive(),
    })
}
