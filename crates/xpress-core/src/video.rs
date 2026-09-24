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

/// ffmpeg output args for the metadata policy: `--strip-metadata` drops all
/// global metadata; `--strip-location` blanks only where it was recorded
/// (QuickTime/MP4 `location` and Apple's ISO 6709 key).
pub fn metadata_args(options: &OptimiseOptions) -> Vec<String> {
    let s = |v: &str| v.to_string();
    if options.strip_metadata {
        vec![s("-map_metadata"), s("-1")]
    } else if options.strip_location {
        [
            "location",
            "location-eng",
            "com.apple.quicktime.location.ISO6709",
        ]
        .iter()
        .flat_map(|k| [s("-metadata"), format!("{k}=")])
        .collect()
    } else {
        Vec::new()
    }
}

/// Whether ffmpeg's stream banner (its `-i` output) shows an HDR video stream,
/// i.e. a PQ (`smpte2084`) or HLG (`arib-std-b67`) transfer.
pub fn describes_hdr(ffmpeg_stderr: &str) -> bool {
    ffmpeg_stderr
        .lines()
        .any(|l| l.contains("Video:") && (l.contains("smpte2084") || l.contains("arib-std-b67")))
}

/// What ffmpeg's stream banner says about a media file.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MediaInfo {
    /// Duration in seconds (0 if unknown).
    pub duration: f64,
    /// Overall bitrate in kbit/s (0 if unknown).
    pub bitrate_kbps: f64,
    /// Video frame size and rate (0 if unknown / no video).
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub has_audio: bool,
    /// PQ or HLG transfer (see [`describes_hdr`]).
    pub hdr: bool,
}

/// Parse ffmpeg's `-i` banner (stderr) into a [`MediaInfo`].
pub fn parse_banner(stderr: &str) -> MediaInfo {
    use std::sync::OnceLock;
    static RE: OnceLock<[regex::Regex; 4]> = OnceLock::new();
    let [duration, bitrate, size, fps] = RE.get_or_init(|| {
        [
            regex::Regex::new(r"Duration: (\d+):(\d+):(\d+(?:\.\d+)?)").unwrap(),
            regex::Regex::new(r"bitrate: (\d+(?:\.\d+)?) kb/s").unwrap(),
            regex::Regex::new(r"[ ,](\d{2,5})x(\d{2,5})[ ,\[]").unwrap(),
            regex::Regex::new(r"(\d+(?:\.\d+)?) fps").unwrap(),
        ]
    });
    let mut info = MediaInfo {
        hdr: describes_hdr(stderr),
        ..Default::default()
    };
    for line in stderr.lines() {
        if let Some(c) = duration.captures(line) {
            let n = |i: usize| c[i].parse::<f64>().unwrap_or(0.0);
            info.duration = n(1) * 3600.0 + n(2) * 60.0 + n(3);
        }
        if line.contains("Duration:") {
            if let Some(c) = bitrate.captures(line) {
                info.bitrate_kbps = c[1].parse().unwrap_or(0.0);
            }
        }
        if line.contains("Video:") && info.width == 0 {
            if let Some(c) = size.captures(line) {
                info.width = c[1].parse().unwrap_or(0);
                info.height = c[2].parse().unwrap_or(0);
            }
            if let Some(c) = fps.captures(line) {
                info.fps = c[1].parse().unwrap_or(0.0);
            }
        }
        info.has_audio |= line.contains("Audio:");
    }
    info
}

/// Probe a media file with ffmpeg. `None` if ffmpeg can't be run.
pub fn probe(path: &Path) -> Option<MediaInfo> {
    // With no output file ffmpeg prints the stream info and exits non-zero.
    let stderr = match tools::run(
        Tool::Ffmpeg,
        ["-hide_banner", "-i", &path.display().to_string()],
    ) {
        Err(tools::ToolError::Failed { stderr, .. }) => stderr,
        Ok(out) => String::from_utf8_lossy(&out.stderr).into_owned(),
        Err(_) => return None,
    };
    Some(parse_banner(&stderr))
}

/// Probe `path` for an HDR video stream. Best-effort: false if unsure.
pub fn is_hdr(path: &Path) -> bool {
    probe(path).is_some_and(|i| i.hdr)
}

/// Bitrates (and, if needed, a smaller frame size) for hitting a file size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BitratePlan {
    pub video_kbps: u32,
    /// AAC bitrate, or `None` for no audio track.
    pub audio_kbps: Option<u32>,
    /// Downscale to this size when the bitrate is too thin for the original.
    pub scale: Option<(u32, u32)>,
}

/// Below this many bits per pixel per frame H.264 turns to mush, so a smaller
/// frame size looks better than the full one at the same bitrate.
const MIN_BITS_PER_PIXEL: f64 = 0.05;

/// Plan the bitrates for a `max_bytes` file: the budget over the duration,
/// minus ~4% container overhead, split between AAC audio (10%, 32–128 kbit/s)
/// and video; downscaled (keeping aspect, not below 240 lines) when the video
/// bitrate is too low for the frame size. `None` without a known duration.
pub fn plan_bitrate(max_bytes: u64, info: &MediaInfo) -> Option<BitratePlan> {
    if info.duration <= 0.0 {
        return None;
    }
    let total_kbps = max_bytes as f64 * 8.0 / info.duration / 1000.0 * 0.96;
    let audio = info
        .has_audio
        .then(|| (total_kbps * 0.1).clamp(32.0, 128.0));
    let video = (total_kbps - audio.unwrap_or(0.0)).max(20.0);

    let even = |n: f64| ((n / 2.0).round() as u32 * 2).max(2);
    let scale = if info.width > 0 && info.height > 0 && info.fps > 0.0 {
        let bpp = video * 1000.0 / (info.width as f64 * info.height as f64 * info.fps);
        let min_h = 240.0_f64.min(info.height as f64);
        let k = (bpp / MIN_BITS_PER_PIXEL).sqrt();
        (k < 0.9).then(|| {
            let h = (info.height as f64 * k).max(min_h);
            let w = h * info.width as f64 / info.height as f64;
            (even(w), even(h))
        })
    } else {
        None
    };
    Some(BitratePlan {
        video_kbps: video.round() as u32,
        audio_kbps: audio.map(|a| a.round() as u32),
        scale,
    })
}

/// Two-pass libx264 encode of `path` to `out` at the planned bitrates (with the
/// HDR tone-map when needed). Two-pass lands close to the requested size.
pub fn encode_to_bitrate(
    path: &Path,
    out: &Path,
    plan: &BitratePlan,
    hdr: bool,
    options: &OptimiseOptions,
) -> Result<(), OptimiseError> {
    let tmp = TempDir::new()?;
    let log = tmp.path().join("x264");
    let scale = plan
        .scale
        .map(|(w, h)| format!("scale={w}:{h}:flags=lanczos,setsar=1"));
    let mut filters: Vec<Option<String>> = Vec::new();
    if hdr {
        filters.push(Some(match &scale {
            Some(s) => format!("{s},{HDR_TO_SDR}"),
            None => HDR_TO_SDR.to_string(),
        }));
    }
    filters.push(scale);

    let s = |v: &str| v.to_string();
    let mut last = None;
    for vf in &filters {
        let common = |pass: &str| -> Vec<String> {
            let mut a = vec![s("-y"), s("-i"), path.display().to_string()];
            if let Some(f) = vf {
                a.extend([s("-vf"), f.clone()]);
            }
            a.extend([
                s("-c:v"),
                s("libx264"),
                s("-preset"),
                s("medium"),
                s("-b:v"),
                format!("{}k", plan.video_kbps),
                s("-pix_fmt"),
                s("yuv420p"),
                s("-pass"),
                s(pass),
                s("-passlogfile"),
                log.display().to_string(),
                s("-hide_banner"),
                s("-nostats"),
            ]);
            a
        };
        let mut pass1 = common("1");
        pass1.extend([s("-an"), s("-f"), s("mp4")]);
        pass1.push(tmp.path().join("pass1.mp4").display().to_string());
        let mut pass2 = common("2");
        match plan.audio_kbps {
            Some(a) => pass2.extend([s("-c:a"), s("aac"), s("-b:a"), format!("{a}k")]),
            None => pass2.push(s("-an")),
        }
        pass2.extend(metadata_args(options));
        pass2.extend([s("-movflags"), s("+faststart")]);
        pass2.push(out.display().to_string());

        match tools::run(Tool::Ffmpeg, &pass1).and_then(|_| tools::run(Tool::Ffmpeg, &pass2)) {
            Ok(_) => return Ok(()),
            Err(e @ tools::ToolError::Timeout { .. }) => return Err(e.into()),
            Err(e) => last = Some(e),
        }
    }
    Err(last
        .map(Into::into)
        .unwrap_or_else(|| OptimiseError::Other("ffmpeg failed".into())))
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
        args.extend(metadata_args(options));
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

    // Like image conversion, the result is written alongside and the source is
    // kept — unless the target has the same path, which is backed up first.
    let default_dest = path.with_extension(ext);
    let overwrites_source = default_dest == path;
    finish(
        MediaKind::Video,
        path,
        &temp_out,
        default_dest,
        old_size,
        cq.image_is_aggressive(),
        options,
        Placement {
            size_guard: false,
            backup: overwrites_source,
            replace_source: false,
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
        args.extend(metadata_args(options));
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

    const BANNER: &str = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'clip.mov':
  Duration: 00:01:02.50, start: 0.000000, bitrate: 6400 kb/s
  Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), yuv420p(tv, bt709, progressive), 1920x1080 [SAR 1:1 DAR 16:9], 6200 kb/s, 29.97 fps, 29.97 tbr
  Stream #0:1[0x2](und): Audio: aac (LC) (mp4a / 0x6134706D), 48000 Hz, stereo, fltp, 128 kb/s";

    #[test]
    fn parses_the_ffmpeg_banner() {
        let i = parse_banner(BANNER);
        assert!((i.duration - 62.5).abs() < 1e-9);
        assert_eq!(i.bitrate_kbps, 6400.0);
        assert_eq!((i.width, i.height), (1920, 1080));
        assert!((i.fps - 29.97).abs() < 1e-9);
        assert!(i.has_audio);
        assert!(!i.hdr);
        assert_eq!(parse_banner("garbage"), MediaInfo::default());
    }

    #[test]
    fn plans_bitrate_from_the_budget() {
        let info = parse_banner(BANNER);
        // 25 MB over 62.5 s = 3200 kbit/s, minus 4% overhead = 3072; audio 10%
        // capped at 128.
        let p = plan_bitrate(25_000_000, &info).unwrap();
        assert_eq!(p.audio_kbps, Some(128));
        assert_eq!(p.video_kbps, 2944);
        assert_eq!(p.scale, None, "enough bits for 1080p");
    }

    #[test]
    fn thin_budgets_downscale_keeping_aspect() {
        let info = parse_banner(BANNER);
        // 8 MB for a minute of 1080p: too thin, so shrink the frame.
        let p = plan_bitrate(8_000_000, &info).unwrap();
        let (w, h) = p.scale.expect("downscaled");
        assert!((240..1080).contains(&h) && h % 2 == 0 && w % 2 == 0);
        assert!(((w as f64 / h as f64) - 16.0 / 9.0).abs() < 0.02);
        // Never below 240 lines, even for absurd budgets.
        let tiny = plan_bitrate(200_000, &info).unwrap();
        assert_eq!(tiny.scale.unwrap().1, 240);
    }

    #[test]
    fn no_duration_no_plan() {
        assert!(plan_bitrate(1_000_000, &MediaInfo::default()).is_none());
    }

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
