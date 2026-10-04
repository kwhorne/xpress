//! The app's settings, remembered between launches in `gui.json` next to the
//! CLI's `config.json` (see [`xpress_core::config`]). The first launch takes
//! its defaults from `config.json`; after that the two are independent, so
//! changing the slider in the app doesn't change what `xpress optimise` does.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Everything the app remembers. Unknown or missing fields fall back to the
/// defaults, so older and newer files both load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub compression: i32,
    pub aggressive: bool,
    pub backup: bool,
    pub strip_metadata: bool,
    pub strip_location: bool,
    /// `off`, `visually-lossless`, `high`, `medium` or `low`.
    pub quality_target: String,
    /// Image format to convert to (its extension), or none to optimise.
    pub convert_to: Option<String>,
    pub skip_optimised: bool,
    pub always_on_top: bool,
    pub pipeline: String,
    pub use_pipeline: bool,
    /// Record clipboard history (off until turned on).
    pub history_enabled: bool,
    /// Also record new screenshots.
    pub history_screenshots: bool,
    /// Recognise the text in recorded images.
    pub history_ocr: bool,
    /// Forget unpinned clips after this many days (0 = keep).
    pub history_days: u32,
    /// Paste a chosen clip into the previous app (needs Accessibility).
    pub paste_directly: bool,
    /// Sync the history between Macs through iCloud Drive.
    pub history_sync: bool,
    /// Global shortcuts (`global_hotkey` strings; empty = off).
    pub shortcut_clipboard: String,
    pub shortcut_show: String,
    pub shortcut_history: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self::from_config(&xpress_core::config::Config::default())
    }
}

impl Settings {
    /// First-launch settings: the CLI's defaults where both have the setting.
    pub fn from_config(config: &xpress_core::config::Config) -> Self {
        Self {
            compression: config.compression,
            aggressive: config.aggressive,
            backup: config.backup,
            strip_metadata: config.strip_metadata,
            strip_location: false,
            quality_target: "off".into(),
            convert_to: None,
            skip_optimised: true,
            always_on_top: false,
            pipeline: "crop(longEdge: 2000) -> convert(to: webp)".into(),
            use_pipeline: false,
            history_enabled: false,
            history_screenshots: true,
            history_ocr: true,
            history_days: 30,
            paste_directly: false,
            history_sync: false,
            shortcut_clipboard: crate::shortcuts::Action::Clipboard
                .default_shortcut()
                .into(),
            shortcut_show: crate::shortcuts::Action::Show.default_shortcut().into(),
            shortcut_history: crate::shortcuts::Action::History.default_shortcut().into(),
        }
    }

    /// Where the settings live: `gui.json` in the config folder.
    pub fn path() -> Option<PathBuf> {
        xpress_core::config::Config::path().map(|p| p.with_file_name("gui.json"))
    }

    /// Load from `path`; a missing or unreadable file gives `fallback()`.
    pub fn load(path: &Path, fallback: impl FnOnce() -> Settings) -> Settings {
        match std::fs::read_to_string(path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|_| fallback()),
            Err(_) => fallback(),
        }
    }

    /// Write to `path` atomically (a temp file renamed over the old one).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let dir = path.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir)?;
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/gui.json");
        let settings = Settings {
            compression: 64,
            strip_location: true,
            quality_target: "high".into(),
            convert_to: Some("webp".into()),
            always_on_top: true,
            pipeline: "convert(to: avif)".into(),
            use_pipeline: true,
            ..Settings::default()
        };
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path, Settings::default), settings);
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn missing_or_broken_files_use_the_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui.json");
        let fallback = || Settings {
            compression: 42,
            ..Settings::default()
        };
        assert_eq!(Settings::load(&path, fallback).compression, 42);
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(Settings::load(&path, fallback).compression, 42);
    }

    #[test]
    fn partial_files_keep_the_defaults_for_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gui.json");
        std::fs::write(&path, r#"{ "compression": 80, "somethingNew": 1 }"#).unwrap();
        let s = Settings::load(&path, Settings::default);
        assert_eq!(s.compression, 80);
        assert!(s.backup);
        assert_eq!(s.quality_target, "off");
    }

    #[test]
    fn first_launch_follows_the_cli_config() {
        let config = xpress_core::config::Config {
            compression: 70,
            backup: false,
            strip_metadata: true,
            ..Default::default()
        };
        let s = Settings::from_config(&config);
        assert_eq!(s.compression, 70);
        assert!(!s.backup);
        assert!(s.strip_metadata);
    }
}
