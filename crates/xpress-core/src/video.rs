//! Video optimisation via ffmpeg (default H.264 path).

use std::path::Path;

use tempfile::TempDir;

use crate::filetype::MediaKind;
use crate::result::{
    file_size, finish, OptimisationResult, OptimiseError, OptimiseOptions, Placement,
};
use crate::tools::{self, Tool};

/// Tone-maps HDR (PQ / HLG, BT.2020) to SDR BT.709 in 8 bits. H.264 output is
/// always 8-bit 4:2:0, and squeezing an iPhone HDR clip into that without
/// tone-mapping leaves it grey and washed out (or wrongly tagged as HDR).
///
/// Mobius keeps tones up to the knee (`param`) linear, so normally exposed
/// footage keeps its brightness, and rolls highlights off smoothly above it.
pub const HDR_TO_SDR: &str = "zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,\
     tonemap=tonemap=mobius:param=0.5:desat=0,zscale=t=bt709:m=bt709:r=tv,format=yuv420p";

/// Whether ffmpeg's stream banner (its `-i` output) shows an HDR video stream,
/// i.e. a PQ (`smpte2084`) or HLG (`arib-std-b67`) transfer.
pub fn describes_hdr(ffmpeg_stderr: &str) -> bool {
    ffmpeg_stderr
        .lines()
        .any(|l| l.contains("Video:") && (l.contains("smpte2084") || l.contains("arib-std-b67")))
}

/// Probe `path` for an HDR video stream. Best-effort: false if unsure.
pub fn is_hdr(path: &Path) -> bool {
    // With no output file ffmpeg prints the stream info and exits non-zero.
    let stderr = match tools::run(
        Tool::Ffmpeg,
        ["-hide_banner", "-i", &path.display().to_string()],
    ) {
        Err(tools::ToolError::Failed { stderr, .. }) => stderr,
        Ok(out) => String::from_utf8_lossy(&out.stderr).into_owned(),
        Err(_) => return false,
    };
    describes_hdr(&stderr)
}

/// Run an H.264 encode built by `build(filter, reencode_audio)`: tone-mapping
/// HDR input first (falling back to plain if this ffmpeg lacks `zscale`), and
/// copying the audio before re-encoding it.
fn run_h264_encode(
    path: &Path,
    vf: Option<&str>,
    build: impl Fn(Option<&str>, bool) -> Vec<String>,
) -> Result<(), OptimiseError> {
    let mut filters: Vec<Option<String>> = Vec::new();
    if is_hdr(path) {
        filters.push(Some(match vf {
            Some(f) => format!("{f},{HDR_TO_SDR}"),
            None => HDR_TO_SDR.to_string(),
        }));
    }
    filters.push(vf.map(String::from));
    let mut last = None;
    for filter in &filters {
        for reencode_audio in [false, true] {
            match tools::run(Tool::Ffmpeg, build(filter.as_deref(), reencode_audio)) {
                Ok(_) => return Ok(()),
                // A timeout won't get better with another attempt.
                Err(e @ tools::ToolError::Timeout { .. }) => return Err(e.into()),
                Err(e) => last = Some(e),
            }
        }
    }
    Err(last
        .map(Into::into)
        .unwrap_or_else(|| OptimiseError::Other("ffmpeg failed".into())))
}

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

    let build = |vf: Option<&str>, reencode_audio: bool| -> Vec<String> {
        let mut args: Vec<String> = vec![s("-y"), s("-i"), path.display().to_string()];
        if let Some(f) = vf {
            args.extend([s("-vf"), f.to_string()]);
        }
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

    if codec == VideoCodec::H264 {
        // 8-bit H.264: tone-map HDR sources. HEVC/AV1/VP9 keep 10-bit HDR as is.
        run_h264_encode(path, None, build)?;
    } else if tools::run(Tool::Ffmpeg, build(None, false)).is_err() {
        tools::run(Tool::Ffmpeg, build(None, true))?;
    }

    // Converting in place replaces the source (backed up first, when enabled).
    finish(
        MediaKind::Video,
        path,
        &temp_out,
        path.with_extension(ext),
        old_size,
        cq.image_is_aggressive(),
        options,
        Placement {
            size_guard: false,
            backup: true,
            replace_source: true,
        },
    )
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

    // The GIF is written alongside the video, which is kept.
    finish(
        MediaKind::Image,
        path,
        &out,
        path.with_extension("gif"),
        old_size,
        options.compression.image_is_aggressive(),
        options,
        Placement {
            size_guard: false,
            backup: false,
            replace_source: false,
        },
    )
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

    let build = |vf: Option<&str>, reencode_audio: bool| -> Vec<String> {
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

    run_h264_encode(path, vf, build)?;

    // A plain optimise must never replace a video with a bigger one — including
    // a `.mov` that would become a larger `.mp4` (e.g. an iPhone HEVC clip
    // re-encoded to H.264). Crops/scales (`vf`) were asked for explicitly, so
    // they are kept regardless of size. An in-place `.mov` -> `.mp4` replaces
    // the source (backed up first, when enabled).
    finish(
        MediaKind::Video,
        path,
        &temp_out,
        path.with_extension("mp4"),
        old_size,
        cq.image_is_aggressive(),
        options,
        Placement {
            size_guard: vf.is_none(),
            backup: true,
            replace_source: true,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_hdr_transfers() {
        let hlg = "  Stream #0:0[0x1]: Video: hevc (Main 10) (hvc1 / 0x31637668), \
                   yuv420p10le(tv, bt2020nc/bt2020/arib-std-b67, progressive), 640x360";
        let pq = "  Stream #0:0: Video: hevc (Main 10), yuv420p10le(tv, bt2020nc/bt2020/smpte2084)";
        let sdr = "  Stream #0:0: Video: h264 (High), yuv420p(tv, bt709, progressive)";
        let audio_only = "  Stream #0:1: Audio: aac, 48000 Hz, stereo (arib-std-b67 in a tag)";
        assert!(describes_hdr(hlg));
        assert!(describes_hdr(pq));
        assert!(!describes_hdr(sdr));
        assert!(!describes_hdr(audio_only));
    }
}
