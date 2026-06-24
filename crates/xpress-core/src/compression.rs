//! The `CompressionQuality` model.
//!
//! A single per-format "how hard do we compress" value: a named [`CompressionTier`]
//! plus a continuous `factor` from 5 (least compression / best quality) to 100
//! (most compression / smallest file). 0 is the "Auto" sentinel for video.
//!
//! The factor-to-parameter mappings below translate a single percentage into the
//! native quality knobs each tool exposes (jpegoptim --max, pngquant --quality,
//! gifsicle -O/--lossy, libx264 CRF/preset, audio bitrate).

use serde::{Deserialize, Serialize};

/// Named factor anchors for the two built-in presets.
/// factor 30 == "normal", factor 64 == "aggressive".
pub const COMPRESSION_FACTOR_NORMAL: i32 = 30;
pub const COMPRESSION_FACTOR_AGGRESSIVE: i32 = 64;

#[inline]
fn cq_clamp(value: i32, min: i32, max: i32) -> i32 {
    value.clamp(min, max)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum CompressionTier {
    /// Visually lossless.
    Lossless,
    /// Adaptive cross-test between normal and aggressive.
    Adaptive,
    /// Video only: hardware VideoToolbox encoder.
    Fast,
    /// Video only: efficient software encoder.
    Smaller,
    /// Pure-factor mode (no named anchor).
    #[default]
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompressionQuality {
    pub tier: CompressionTier,
    pub factor: i32,
}

impl Default for CompressionQuality {
    fn default() -> Self {
        Self::new(CompressionTier::Custom, 50)
    }
}

impl CompressionQuality {
    pub fn new(tier: CompressionTier, factor: i32) -> Self {
        Self {
            tier,
            // 0 is a valid sentinel for "Auto".
            factor: cq_clamp(factor, 0, 100),
        }
    }

    pub fn factor(factor: i32) -> Self {
        Self::new(CompressionTier::Custom, factor)
    }

    pub fn normal() -> Self {
        Self::factor(COMPRESSION_FACTOR_NORMAL)
    }

    pub fn aggressive() -> Self {
        Self::factor(COMPRESSION_FACTOR_AGGRESSIVE)
    }

    // MARK: Image translation (factor 5..100, higher = more compression)

    /// Whether this resolves to the "aggressive" preset.
    pub fn image_is_aggressive(&self) -> bool {
        self.tier != CompressionTier::Adaptive && self.factor >= 50
    }

    /// jpegoptim --max quality ceiling. factor 30 -> 85, ramping to 30 at max.
    pub fn jpeg_max_quality(&self) -> i32 {
        cq_clamp(
            (85.0 - (self.factor - 30) as f64 * (55.0 / 70.0)).round() as i32,
            25,
            95,
        )
    }

    /// jpegoptim --max for the old-binary fallback / adaptive cross-test.
    pub fn jpeg_secondary_max_quality(&self) -> i32 {
        cq_clamp(
            (90.0 - (self.factor - 30) as f64 * (60.0 / 70.0)).round() as i32,
            25,
            97,
        )
    }

    /// pngquant --quality string "0-MAX".
    pub fn pngquant_quality(&self) -> String {
        let max = cq_clamp(
            (100.0 - (self.factor - 30) as f64 * (75.0 / 70.0)).round() as i32,
            25,
            100,
        );
        format!("0-{max}")
    }

    /// pngquant --speed (1 = slowest/best, 11 = fastest).
    pub fn pngquant_speed(&self) -> i32 {
        match self.factor {
            f if f < 40 => 4,
            f if f < 60 => 3,
            f if f < 85 => 2,
            _ => 1,
        }
    }

    /// gifsicle args. factor 30 -> -O2 --lossy=30; 64 -> -O3 --lossy=80 --colors=N.
    pub fn gifsicle_args(&self) -> Vec<String> {
        let o_level = if self.factor >= 50 {
            3
        } else if self.factor >= 20 {
            2
        } else {
            1
        };
        let lossy = cq_clamp(
            (30.0 + (self.factor - 30) as f64 * (50.0 / 34.0)).round() as i32,
            0,
            200,
        );
        let mut args = vec![format!("-O{o_level}"), format!("--lossy={lossy}")];
        if self.factor >= 50 {
            let colors = cq_clamp(
                (256.0 - (self.factor - 50) as f64 * (192.0 / 50.0)).round() as i32,
                32,
                256,
            );
            args.push(format!("--colors={colors}"));
        }
        args
    }

    /// cwebp / heif-enc -q quality (0-100). factor 30 -> 60.
    pub fn conversion_quality(&self) -> i32 {
        cq_clamp((75.0 - self.factor as f64 * 0.5).round() as i32, 20, 90)
    }

    /// JXLCoder quality (0-100). factor 30 -> 60.
    pub fn jxl_quality(&self) -> i32 {
        cq_clamp((75.0 - self.factor as f64 * 0.5).round() as i32, 20, 95)
    }

    /// JXLCoder effort (1-9).
    pub fn jxl_effort(&self) -> i32 {
        if self.factor >= 70 {
            9
        } else if self.factor >= 50 {
            8
        } else {
            7
        }
    }

    // MARK: Video translation (default H.264 path; factor 5..100, higher = more compression)

    /// libx264 CRF for the software path. factor 5 -> 18, 100 -> 30; 50 ≈ 24.
    pub fn video_h264_crf(&self) -> i32 {
        cq_clamp(
            18 + ((self.factor.max(5) - 5) as f64 / 95.0 * 12.0).round() as i32,
            17,
            32,
        )
    }

    /// Whether the software encoder lets ffmpeg pick the CRF (factor 0 = Auto).
    pub fn video_uses_auto_crf(&self) -> bool {
        self.factor <= 0
    }

    /// libx264 -preset chosen from the compression percentage.
    pub fn video_h264_preset(&self) -> &'static str {
        match self.factor {
            f if f < 20 => "veryfast",
            f if f < 40 => "fast",
            f if f < 60 => "medium",
            f if f < 85 => "slow",
            _ => "slower",
        }
    }

    /// ffmpeg encoder args for the default H.264 encode.
    ///
    /// `arm` toggles the VideoToolbox hardware path used on Apple Silicon for the
    /// `Fast` tier.
    pub fn video_h264_args(&self, arm: bool) -> Vec<String> {
        let s = |v: &str| v.to_string();
        match self.tier {
            CompressionTier::Lossless => {
                vec![s("-vcodec"), s("h264"), s("-tag:v"), s("avc1"), s("-crf"), s("17")]
            }
            CompressionTier::Fast => {
                if arm {
                    let q = cq_clamp(
                        (70.0 - (self.factor.max(5) - 5) as f64 / 95.0 * 45.0).round() as i32,
                        25,
                        75,
                    );
                    vec![
                        s("-vcodec"),
                        s("h264_videotoolbox"),
                        s("-q:v"),
                        q.to_string(),
                        s("-tag:v"),
                        s("avc1"),
                    ]
                } else if self.video_uses_auto_crf() {
                    vec![s("-vcodec"), s("h264"), s("-tag:v"), s("avc1"), s("-preset"), s("veryfast")]
                } else {
                    vec![
                        s("-vcodec"), s("h264"), s("-tag:v"), s("avc1"),
                        s("-preset"), s("veryfast"), s("-crf"), self.video_h264_crf().to_string(),
                    ]
                }
            }
            // .smaller / .custom / .adaptive -> efficient software libx264
            _ => {
                if self.video_uses_auto_crf() {
                    vec![s("-vcodec"), s("h264"), s("-tag:v"), s("avc1"), s("-preset"), s("slower")]
                } else {
                    vec![
                        s("-vcodec"), s("h264"), s("-tag:v"), s("avc1"),
                        s("-preset"), s(self.video_h264_preset()), s("-crf"), self.video_h264_crf().to_string(),
                    ]
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_preset_anchors() {
        let cq = CompressionQuality::normal();
        assert_eq!(cq.jpeg_max_quality(), 85);
        assert_eq!(cq.pngquant_quality(), "0-100");
        assert_eq!(cq.pngquant_speed(), 4);
        assert_eq!(cq.gifsicle_args(), vec!["-O2", "--lossy=30"]);
        assert_eq!(cq.conversion_quality(), 60);
    }

    #[test]
    fn aggressive_preset_anchors() {
        let cq = CompressionQuality::aggressive();
        assert!(cq.image_is_aggressive());
        assert_eq!(cq.gifsicle_args(), vec!["-O3", "--lossy=80", "--colors=202"]);
    }

    #[test]
    fn video_crf_bounds() {
        assert_eq!(CompressionQuality::factor(5).video_h264_crf(), 18);
        assert_eq!(CompressionQuality::factor(100).video_h264_crf(), 30);
    }
}
