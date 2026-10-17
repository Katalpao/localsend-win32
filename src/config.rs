//! Persistent application settings, stored as JSON next to the executable
//! (or in the user's config dir). Kept intentionally small and flat so the
//! settings panel maps 1:1 onto fields.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Human readable alias shown to peers.
    pub alias: String,
    /// Local API/HTTP port (defaults to 53317).
    pub port: u16,
    /// Multicast discovery port (defaults to 53317).
    pub multicast_port: u16,
    /// Require HTTPS (TLS). Matches official "require encryption".
    pub require_encryption: bool,
    /// Folder received files are written to.
    pub download_dir: PathBuf,
    /// Optional PIN required from senders.
    pub pin: String,
    /// Device model string advertised during discovery.
    pub device_model: String,
    /// Device type advertised during discovery.
    pub device_type: String,
    /// Last device we successfully sent to (used by "quick send").
    pub last_target_id: Option<String>,
    /// Active UI language pack code (filename stem under `lang/`, e.g. `zh-CN`).
    pub language: String,
}

impl Default for Config {
    fn default() -> Self {
        let download_dir = directories::UserDirs::new()
            .map(|u| u.download_dir().unwrap_or(u.home_dir()).to_path_buf())
            .unwrap_or_else(|| PathBuf::from("C:\\Downloads"));

        Config {
            alias: whoami_alias(),
            port: 53317,
            multicast_port: 53317,
            require_encryption: false,
            download_dir,
            pin: String::new(),
            device_model: "Windows".to_string(),
            device_type: "desktop".to_string(),
            last_target_id: None,
            language: crate::i18n::default_language(),
        }
    }
}

fn whoami_alias() -> String {
    // Best-effort friendly machine name.
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "Windows PC".to_string())
}

impl Config {
    pub fn path() -> PathBuf {
        let dir = directories::ProjectDirs::from("", "", "LocalSendWin32")
            .map(|p| p.config_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let _ = std::fs::create_dir_all(&dir);
        dir.join("config.json")
    }

    pub fn load() -> Config {
        let p = Self::path();
        match std::fs::read_to_string(&p) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Config::default(),
        }
    }

    pub fn save(&self) -> Result<()> {
        let p = Self::path();
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&p, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}
