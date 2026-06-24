//! File-type classification by extension.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
    Pdf,
}

/// Recognised image extensions.
pub const IMAGE_EXTENSIONS: &[&str] = &[
    "webp", "avif", "heic", "heif", "jxl", "bmp", "tiff", "tif", "png", "jpeg", "jpg", "gif",
];

/// Video extensions (quickTime, mp4, webm, mkv, mpeg2, avi, m4v, mpeg).
pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mov", "mp4", "webm", "mkv", "m2v", "avi", "m4v", "mpeg", "mpg",
];

/// Audio extensions (wav, aiff, mp3, flac, m4a, ogg).
pub const AUDIO_EXTENSIONS: &[&str] = &["wav", "aiff", "aif", "mp3", "flac", "m4a", "ogg", "oga"];

pub const PDF_EXTENSIONS: &[&str] = &["pdf"];

pub fn extension_lower(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
}

/// Classify a path by its extension.
pub fn classify(path: &Path) -> Option<MediaKind> {
    let ext = extension_lower(path)?;
    if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
        Some(MediaKind::Image)
    } else if VIDEO_EXTENSIONS.contains(&ext.as_str()) {
        Some(MediaKind::Video)
    } else if AUDIO_EXTENSIONS.contains(&ext.as_str()) {
        Some(MediaKind::Audio)
    } else if PDF_EXTENSIONS.contains(&ext.as_str()) {
        Some(MediaKind::Pdf)
    } else {
        None
    }
}

impl MediaKind {
    pub fn extensions(&self) -> &'static [&'static str] {
        match self {
            MediaKind::Image => IMAGE_EXTENSIONS,
            MediaKind::Video => VIDEO_EXTENSIONS,
            MediaKind::Audio => AUDIO_EXTENSIONS,
            MediaKind::Pdf => PDF_EXTENSIONS,
        }
    }
}
