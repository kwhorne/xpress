//! Image/video effects: watermark overlay via ffmpeg.

use std::path::Path;

use crate::result::OptimiseError;
use crate::tools::{self, Tool};

/// Where to place the watermark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Center,
}

impl Position {
    pub fn parse(s: &str) -> Position {
        match s.to_ascii_lowercase().replace(['-', '_'], "").as_str() {
            "topleft" => Position::TopLeft,
            "topright" => Position::TopRight,
            "bottomleft" => Position::BottomLeft,
            "center" | "centre" => Position::Center,
            _ => Position::BottomRight,
        }
    }

    /// ffmpeg `overlay=` coordinates with a 10px margin.
    fn overlay_xy(&self) -> &'static str {
        match self {
            Position::TopLeft => "10:10",
            Position::TopRight => "W-w-10:10",
            Position::BottomLeft => "10:H-h-10",
            Position::BottomRight => "W-w-10:H-h-10",
            Position::Center => "(W-w)/2:(H-h)/2",
        }
    }
}

/// Overlay `overlay` onto `src`, writing `dst`. `scale` is the watermark width as
/// a fraction of the main width; `opacity` is 0.0–1.0. Works for images and video.
pub fn watermark(
    src: &Path,
    dst: &Path,
    overlay: &Path,
    position: Position,
    opacity: f64,
    scale: f64,
) -> Result<(), OptimiseError> {
    if !overlay.is_file() {
        return Err(OptimiseError::NotFound(overlay.to_path_buf()));
    }
    let opacity = opacity.clamp(0.0, 1.0);
    let scale = scale.clamp(0.01, 1.0);
    let filter = format!(
        "[1:v]format=rgba,colorchannelmixer=aa={opacity:.3}[wm];\
         [wm][0:v]scale2ref=w=ow*{scale:.4}:h=ow*{scale:.4}/mdar[wm2][base];\
         [base][wm2]overlay={pos}",
        pos = position.overlay_xy()
    );
    tools::run(
        Tool::Ffmpeg,
        [
            "-y",
            "-i",
            &src.display().to_string(),
            "-i",
            &overlay.display().to_string(),
            "-filter_complex",
            &filter,
            "-hide_banner",
            "-nostats",
            &dst.display().to_string(),
        ],
    )?;
    Ok(())
}
