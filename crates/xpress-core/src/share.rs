//! Share presets: make a file fit where it's going (`--for discord`), with
//! the destination's size limit and the formats it can show.
//!
//! The limits are hard-coded from the services' published numbers, checked in
//! September 2026:
//! * Discord — 20 MB per file for free accounts (raised from 10 MB on
//!   2026-08-13; announced by Discord on X).
//! * GitHub — issues, pull requests and comments: 10 MB for images and GIFs,
//!   10 MB for videos on free plans (100 MB on paid), 25 MB for other files;
//!   images must be PNG, GIF, JPEG or SVG
//!   (docs.github.com, "Attaching files").
//! * Email — Gmail accepts 25 MB and Outlook 20 MB per message, but base64
//!   encoding inflates attachments by about a third, so a file must stay near
//!   14 MB to get through both.
//!
//! Targets sit a little under each limit to absorb MB/MiB differences and
//! container overhead.

use std::path::Path;

use crate::filetype::{classify, extension_lower, MediaKind};
use crate::image::ImageFormat;
use crate::result::{OptimisationResult, OptimiseError, OptimiseOptions};

/// Where a file is going.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Discord,
    Github,
    Email,
}

impl Target {
    pub const ALL: [Target; 3] = [Target::Discord, Target::Github, Target::Email];

    pub fn parse(s: &str) -> Result<Target, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "discord" => Ok(Target::Discord),
            "github" | "gh" => Ok(Target::Github),
            "email" | "mail" | "gmail" | "outlook" => Ok(Target::Email),
            other => Err(format!(
                "unknown share target '{other}': use discord, github or email"
            )),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Target::Discord => "discord",
            Target::Github => "github",
            Target::Email => "email",
        }
    }

    /// The byte budget for a file of `kind` (already below the published
    /// limit, see the module docs).
    pub fn max_bytes(&self, kind: MediaKind) -> u64 {
        const MB: u64 = 1_000_000;
        match (self, kind) {
            (Target::Discord, _) => 19 * MB,
            (Target::Github, MediaKind::Image | MediaKind::Video) => 9_500_000,
            (Target::Github, _) => 24 * MB,
            (Target::Email, _) => 14 * MB,
        }
    }

    /// The image format a file must be converted to first, if the target
    /// can't show its current one (GitHub: only PNG/GIF/JPEG/SVG; email: HEIC
    /// is not reliably viewable by recipients).
    pub fn image_conversion(&self, path: &Path, has_alpha: bool) -> Option<ImageFormat> {
        let ext = extension_lower(path)?;
        let fallback = if has_alpha {
            ImageFormat::Png
        } else {
            ImageFormat::Jpeg
        };
        match (self, ext.as_str()) {
            (
                Target::Github,
                "webp" | "avif" | "heic" | "heif" | "jxl" | "bmp" | "tiff" | "tif",
            ) => Some(fallback),
            (Target::Email, "heic" | "heif" | "jxl" | "avif") => Some(fallback),
            _ => None,
        }
    }
}

/// Make `path` fit `target`: convert it to a format the destination shows (a
/// new file next to the original) if needed, then compress it to the
/// destination's size limit (in place, with a backup, unless `base.output`).
pub fn prepare(
    path: &Path,
    target: Target,
    base: &OptimiseOptions,
) -> Result<OptimisationResult, OptimiseError> {
    let kind = classify(path).ok_or_else(|| OptimiseError::Unsupported(path.to_path_buf()))?;
    let old_size = crate::result::file_size(path);

    let mut work = path.to_path_buf();
    let mut converted = false;
    if kind == MediaKind::Image {
        let alpha = crate::image::load(path)
            .map(|(img, _)| img.color().has_alpha())
            .unwrap_or(false);
        if let Some(format) = target.image_conversion(path, alpha) {
            let r = crate::image::convert(path, format, base)?;
            work = r.output;
            converted = true;
        }
    }

    // The converted copy is new, so it needs no backup and is shrunk in place.
    let opts = if converted {
        OptimiseOptions {
            backup: false,
            output: None,
            ..base.clone()
        }
    } else {
        base.clone()
    };
    let mut r = crate::budget::optimise_to_budget(&work, target.max_bytes(kind), &opts)?;
    if converted {
        r.source = path.to_path_buf();
        r.old_size = old_size;
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_targets() {
        assert_eq!(Target::parse("Discord"), Ok(Target::Discord));
        assert_eq!(Target::parse("gmail"), Ok(Target::Email));
        assert!(Target::parse("myspace").is_err());
    }

    #[test]
    fn limits_sit_under_the_published_ones() {
        assert!(Target::Discord.max_bytes(MediaKind::Video) < 20_000_000);
        assert!(Target::Github.max_bytes(MediaKind::Image) < 10_000_000);
        assert!(Target::Github.max_bytes(MediaKind::Pdf) < 25_000_000);
        // 14 MB * 4/3 (base64) stays under Outlook's 20 MB.
        assert!(Target::Email.max_bytes(MediaKind::Image) * 4 / 3 < 20_000_000);
    }

    #[test]
    fn converts_only_what_the_target_cant_show() {
        let p = |s: &str| std::path::PathBuf::from(s);
        assert_eq!(
            Target::Github.image_conversion(&p("a.webp"), false),
            Some(ImageFormat::Jpeg)
        );
        assert_eq!(
            Target::Github.image_conversion(&p("logo.webp"), true),
            Some(ImageFormat::Png)
        );
        assert_eq!(Target::Github.image_conversion(&p("a.jpg"), false), None);
        assert_eq!(
            Target::Email.image_conversion(&p("IMG_1.HEIC"), false),
            Some(ImageFormat::Jpeg)
        );
        assert_eq!(Target::Email.image_conversion(&p("a.webp"), false), None);
        assert_eq!(Target::Discord.image_conversion(&p("a.heic"), false), None);
    }
}
