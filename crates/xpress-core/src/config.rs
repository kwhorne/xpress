//! User defaults, read from a JSON config file. Flags override these; these
//! override the built-in defaults.
//!
//! Location: the per-user config dir
//!   * macOS:  `~/Library/Application Support/xpress/config.json`
//!   * Linux:  `$XDG_CONFIG_HOME/xpress/config.json` (or `~/.config/...`)
//!   * Windows:`%APPDATA%\xpress\config.json`

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Default compression factor (5–100).
    pub compression: i32,
    /// Use the aggressive preset by default.
    pub aggressive: bool,
    /// Write `.orig` backups by default.
    pub backup: bool,
    /// Strip non-essential metadata by default.
    pub strip_metadata: bool,
    /// Preserve original timestamps by default.
    pub preserve_dates: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            compression: crate::compression::COMPRESSION_FACTOR_NORMAL,
            aggressive: false,
            backup: true,
            strip_metadata: false,
            preserve_dates: true,
        }
    }
}

fn config_path() -> Option<PathBuf> {
    let dir = if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join("Library/Application Support/xpress"))
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join("xpress"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|p| p.join("xpress"))
    };
    dir.map(|d| d.join("config.json"))
}

impl Config {
    /// Load the config, or defaults if it is missing/unreadable.
    pub fn load() -> Config {
        let Some(path) = config_path() else {
            return Config::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Config::default(),
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = config_path().ok_or_else(|| std::io::Error::other("no config dir"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }

    pub fn path() -> Option<PathBuf> {
        config_path()
    }
}
