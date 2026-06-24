//! Video optimisation via ffmpeg (default H.264 path).

use std::path::Path;

use tempfile::TempDir;

use crate::result::{
    backup_file, copy_dates, file_size, OptimisationResult, OptimiseError, OptimiseOptions,
};
use crate::tools::{self, Tool};
use crate::filetype::MediaKind;

/// Optimise a video in place (or to `options.output`), re-encoding to H.264/mp4.
pub fn optimise(path: &Path, options: &OptimiseOptions) -> Result<OptimisationResult, OptimiseError> {
    optimise_with_filter(path, options, None)
}

/// Strip the audio track. Stream-copies video, no re-encode. Writes to `dst`.
pub fn remove_audio(src: &Path, dst: &Path) -> Result<(), OptimiseError> {
    tools::run(
        Tool::Ffmpeg,
        ["-y", "-i", &src.display().to_string(), "-c", "copy", "-an",
         "-hide_banner", "-nostats", &dst.display().to_string()],
    )?;
    Ok(())
}

/// Change playback speed by `factor` (e.g. 2.0 = twice as fast). Re-encodes via
/// setpts (video) and atempo (audio). Writes to `dst`.
pub fn change_speed(src: &Path, dst: &Path, factor: f64, options: &OptimiseOptions) -> Result<(), OptimiseError> {
    let f = factor.clamp(0.25, 8.0);
    let pts = 1.0 / f;
    let mut args: Vec<String> = vec![
        "-y".into(), "-i".into(), src.display().to_string(),
        "-filter:v".into(), format!("setpts={pts:.5}*PTS"),
        "-filter:a".into(), format!("atempo={f:.5}"),
    ];
    args.extend(options.compression.video_h264_args(tools::is_arm64()));
    args.extend(["-hide_banner", "-nostats"].map(String::from));
    args.push(dst.display().to_string());
    tools::run(Tool::Ffmpeg, &args)?;
    Ok(())
}

/// Cap the frame rate at `fps`. Writes to `dst`.
pub fn cap_fps(src: &Path, dst: &Path, fps: i32, options: &OptimiseOptions) -> Result<(), OptimiseError> {
    let mut args: Vec<String> = vec![
        "-y".into(), "-i".into(), src.display().to_string(),
        "-vf".into(), format!("fps=fps={}", fps.max(1)),
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
    let temp_out = tmp.path().join(
        path.with_extension("mp4")
            .file_name()
            .unwrap()
            .to_owned(),
    );

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
