//! Audio optimisation/conversion via ffmpeg.
//!
//! Encoder arguments are derived from ffmpeg's documented options for each codec
//! (aac_at, libmp3lame VBR, libopus, pcm, flac).

use std::path::Path;

use tempfile::TempDir;

use crate::compression::CompressionQuality;
use crate::filetype::{extension_lower, MediaKind};
use crate::result::{
    file_size, finish, OptimisationResult, OptimiseError, OptimiseOptions, Placement,
};
use crate::tools::{self, Tool};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    SameAsInput,
    Aac,
    Mp3,
    Opus,
    Wav,
    Flac,
    Aiff,
}

impl AudioFormat {
    pub fn file_extension(&self) -> &'static str {
        match self {
            AudioFormat::SameAsInput => "",
            AudioFormat::Aac => "m4a",
            AudioFormat::Mp3 => "mp3",
            AudioFormat::Opus => "ogg",
            AudioFormat::Wav => "wav",
            AudioFormat::Flac => "flac",
            AudioFormat::Aiff => "aiff",
        }
    }

    pub fn ffmpeg_codec(&self) -> &'static str {
        match self {
            AudioFormat::SameAsInput => "",
            AudioFormat::Aac => "aac_at",
            AudioFormat::Mp3 => "libmp3lame",
            AudioFormat::Opus => "libopus",
            AudioFormat::Wav => "pcm_s16le",
            AudioFormat::Flac => "flac",
            AudioFormat::Aiff => "pcm_s16be",
        }
    }

    pub fn is_lossless(&self) -> bool {
        matches!(
            self,
            AudioFormat::Wav | AudioFormat::Flac | AudioFormat::Aiff
        )
    }

    pub fn from_target(target: &str) -> Option<AudioFormat> {
        match target.to_ascii_lowercase().as_str() {
            "aac" | "m4a" => Some(AudioFormat::Aac),
            "mp3" => Some(AudioFormat::Mp3),
            "opus" | "ogg" | "oga" => Some(AudioFormat::Opus),
            "wav" => Some(AudioFormat::Wav),
            "flac" => Some(AudioFormat::Flac),
            "aiff" | "aif" => Some(AudioFormat::Aiff),
            _ => None,
        }
    }

    /// Resolve `SameAsInput` to a concrete format from the source extension.
    pub fn resolved(&self, input_ext: &str) -> AudioFormat {
        if *self != AudioFormat::SameAsInput {
            return *self;
        }
        AudioFormat::from_target(input_ext).unwrap_or(AudioFormat::Aac)
    }

    pub fn allowed_bitrates(&self) -> &'static [i32] {
        match self {
            AudioFormat::SameAsInput => &[56, 64, 80, 96, 128, 160, 192, 256, 320],
            AudioFormat::Aac => &[56, 64, 80, 96, 128, 160, 192, 256],
            AudioFormat::Mp3 => &[56, 64, 80, 96, 128, 160, 192, 256, 320],
            AudioFormat::Opus => &[32, 48, 64, 80, 96, 128],
            AudioFormat::Wav | AudioFormat::Flac | AudioFormat::Aiff => &[],
        }
    }

    pub fn default_bitrate(&self) -> i32 {
        match self {
            AudioFormat::SameAsInput => -1,
            AudioFormat::Aac => 192,
            AudioFormat::Mp3 => 192,
            AudioFormat::Opus => 128,
            AudioFormat::Wav | AudioFormat::Flac | AudioFormat::Aiff => 0,
        }
    }

    pub fn bitrate_range(&self) -> Option<(i32, i32)> {
        match self {
            AudioFormat::Mp3 => Some((64, 320)),
            AudioFormat::Aac => Some((48, 256)),
            AudioFormat::Opus => Some((32, 160)),
            AudioFormat::SameAsInput => Some((48, 256)),
            AudioFormat::Wav | AudioFormat::Flac | AudioFormat::Aiff => None,
        }
    }

    fn lame_vbr_quality(bitrate: i32) -> i32 {
        match bitrate {
            b if b <= 64 => 9,
            b if b <= 80 => 8,
            b if b <= 96 => 7,
            b if b <= 128 => 5,
            b if b <= 160 => 4,
            b if b <= 192 => 2,
            _ => 0,
        }
    }

    /// Args for ffmpeg's built-in AAC encoder, available in every build (unlike
    /// the macOS-only `aac_at` that [`AudioFormat::Aac`] prefers).
    pub fn native_aac_args(bitrate: i32) -> Vec<String> {
        vec![
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            format!("{bitrate}k"),
        ]
    }

    /// ffmpeg encoding args, using VBR where supported.
    pub fn encoding_args(
        &self,
        bitrate: i32,
        aggressive: bool,
        input_sample_rate: Option<f64>,
    ) -> Vec<String> {
        let codec = self.ffmpeg_codec().to_string();
        match self {
            AudioFormat::Aac => vec![
                "-c:a".into(),
                codec,
                "-b:a".into(),
                format!("{bitrate}k"),
                "-aac_at_mode".into(),
                "cvbr".into(),
            ],
            AudioFormat::Mp3 => vec![
                "-c:a".into(),
                codec,
                "-q:a".into(),
                Self::lame_vbr_quality(bitrate).to_string(),
            ],
            AudioFormat::Opus => vec![
                "-c:a".into(),
                codec,
                "-b:a".into(),
                format!("{bitrate}k"),
                "-vbr".into(),
                "on".into(),
            ],
            AudioFormat::Wav => {
                if aggressive {
                    vec!["-c:a".into(), "adpcm_ima_wav".into()]
                } else {
                    let mut args = vec!["-c:a".into(), codec];
                    if let Some(sr) = input_sample_rate {
                        if sr > 48000.0 {
                            args.extend(["-ar".into(), "48000".into()]);
                        }
                    }
                    args
                }
            }
            AudioFormat::Flac => vec![
                "-c:a".into(),
                codec,
                "-compression_level".into(),
                if aggressive { "12".into() } else { "8".into() },
            ],
            AudioFormat::Aiff => {
                let mut args = vec!["-c:a".into(), codec];
                if let Some(sr) = input_sample_rate {
                    if sr > 48000.0 {
                        args.extend(["-ar".into(), "48000".into()]);
                    }
                }
                args
            }
            AudioFormat::SameAsInput => vec!["-c:a".into(), codec],
        }
    }
}

/// Loudness-normalise an audio file to `lufs` integrated loudness (EBU R128)
/// using ffmpeg's `loudnorm` filter. Writes to `dst` (re-encodes).
pub fn normalize(src: &Path, dst: &Path, lufs: f64) -> Result<(), OptimiseError> {
    let filter = format!("loudnorm=I={lufs:.1}:TP=-1.5:LRA=11");
    tools::run(
        Tool::Ffmpeg,
        [
            "-y",
            "-i",
            &src.display().to_string(),
            "-af",
            &filter,
            "-hide_banner",
            "-nostats",
            &dst.display().to_string(),
        ],
    )?;
    Ok(())
}

/// Target bitrate (kbps) for a format from a compression value.
pub fn audio_bitrate(cq: CompressionQuality, format: AudioFormat) -> Option<i32> {
    let (lo, hi) = format.bitrate_range()?;
    if hi <= lo {
        return None;
    }
    let t = (cq.factor.clamp(5, 100) - 5) as f64 / 95.0;
    let raw = hi as f64 - t * (hi - lo) as f64;
    Some(hi.min(lo.max(rounded_audio_bitrate(raw))))
}

fn rounded_audio_bitrate(raw: f64) -> i32 {
    (8).max(((raw / 16.0).round() as i32) * 16)
}

/// Optimise (or convert) an audio file.
pub fn optimise(
    path: &Path,
    options: &OptimiseOptions,
    target: AudioFormat,
    bitrate_override: Option<i32>,
) -> Result<OptimisationResult, OptimiseError> {
    if !path.is_file() {
        return Err(OptimiseError::NotFound(path.to_path_buf()));
    }
    let input_ext = extension_lower(path).unwrap_or_default();
    let format = target.resolved(&input_ext);
    let old_size = file_size(path);
    let cq = options.compression;
    let aggressive = cq.image_is_aggressive();

    let bitrate = bitrate_override
        .or_else(|| audio_bitrate(cq, format))
        .unwrap_or_else(|| format.default_bitrate().max(0));

    let tmp = TempDir::new()?;
    let out_ext = format.file_extension();
    let temp_out = tmp.path().join(format!(
        "{}.{out_ext}",
        crate::result::file_stem_lossy(path)
    ));

    let encode = |codec_args: Vec<String>| {
        let mut args: Vec<String> = vec!["-y".into(), "-i".into(), path.display().to_string()];
        args.extend(codec_args);
        args.push(temp_out.display().to_string());
        tools::run(Tool::Ffmpeg, &args)
    };
    // An explicit bitrate (a size budget, `lowerBitrate`) is honoured as a
    // bitrate: MP3 switches from quality-based VBR to CBR, whose size is
    // predictable. Otherwise VBR picks the bits the audio needs.
    let codec_args = match (bitrate_override, format) {
        (Some(kbps), AudioFormat::Mp3) => vec![
            "-c:a".into(),
            "libmp3lame".into(),
            "-b:a".into(),
            format!("{kbps}k"),
        ],
        _ => format.encoding_args(bitrate, aggressive, None),
    };
    if let Err(e) = encode(codec_args) {
        // `aac_at` (AudioToolbox) only exists in macOS ffmpeg builds that enable
        // it; everywhere else fall back to ffmpeg's native AAC encoder.
        if format != AudioFormat::Aac {
            return Err(e.into());
        }
        encode(AudioFormat::native_aac_args(bitrate))?;
    }

    // Re-encoding to the same format must shrink the file; an explicit format
    // change is kept regardless. In place, a changed extension replaces the
    // source (backed up first, when enabled).
    let default_dest = if out_ext.is_empty() {
        path.to_path_buf()
    } else {
        path.with_extension(out_ext)
    };
    // Optimising (same format) must shrink the file and may replace it in
    // place, e.g. `.oga` -> `.ogg`. A conversion to another format is written
    // alongside, keeping the source, like image and video conversion.
    let converting = format != AudioFormat::SameAsInput.resolved(&input_ext);
    finish(
        MediaKind::Audio,
        path,
        &temp_out,
        default_dest,
        old_size,
        aggressive,
        options,
        Placement {
            size_guard: !converting,
            backup: !converting,
            replace_source: !converting,
        },
    )
}
