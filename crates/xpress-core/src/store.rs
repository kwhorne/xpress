//! Persistent storage for saved pipelines and folder automations.
//!
//! Stored as JSON at the per-user config dir:
//!   * macOS:  `~/Library/Application Support/xpress/pipelines.json`
//!   * Linux:  `$XDG_CONFIG_HOME/xpress/pipelines.json` (or `~/.config/...`)
//!   * Windows:`%APPDATA%\xpress\pipelines.json`

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Store {
    /// name -> pipeline DSL string
    #[serde(default)]
    pub pipelines: BTreeMap<String, String>,
    /// source (folder path / "clipboard" / "dropZone") -> attachment
    #[serde(default)]
    pub automations: Vec<Automation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Automation {
    /// A folder path, or a special source name.
    pub source: String,
    /// File type this applies to: "image" | "video" | "audio" | "pdf" | "all".
    #[serde(default = "all_type")]
    pub file_type: String,
    /// Pipeline name or inline DSL.
    pub pipeline: String,
}

fn all_type() -> String {
    "all".into()
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
    dir.map(|d| d.join("pipelines.json"))
}

impl Store {
    pub fn load() -> Store {
        let Some(path) = config_path() else {
            return Store::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Store::default(),
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path =
            config_path().ok_or_else(|| std::io::Error::other("could not determine config dir"))?;
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
