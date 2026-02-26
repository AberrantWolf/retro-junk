use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppSettings {
    #[serde(default)]
    pub library: LibrarySettings,
    #[serde(default)]
    pub general: GeneralSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LibrarySettings {
    pub current_root: Option<PathBuf>,
    #[serde(default)]
    pub recent_roots: Vec<RecentRoot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentRoot {
    pub path: PathBuf,
    pub last_opened: String,
    pub console_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralSettings {
    #[serde(default = "default_true")]
    pub auto_scan_on_open: bool,
    #[serde(default = "default_true")]
    pub warn_on_region_override: bool,
    /// Metadata output directory. Relative paths resolve from the ROM root.
    /// Default: `"."` (inline with ROMs, ES-DE legacy mode compatible).
    #[serde(default = "default_metadata_dir")]
    pub metadata_dir: String,
    /// Media output directory. If empty, uses `"{root}-media"` sibling convention.
    /// Relative paths resolve from the ROM root.
    #[serde(default)]
    pub media_dir: String,
}

fn default_true() -> bool {
    true
}

fn default_metadata_dir() -> String {
    ".".to_string()
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            auto_scan_on_open: true,
            warn_on_region_override: true,
            metadata_dir: default_metadata_dir(),
            media_dir: String::new(),
        }
    }
}

/// Returns `~/.config/retro-junk/settings.toml`.
pub fn settings_path() -> PathBuf {
    let config = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    config.join("retro-junk").join("settings.toml")
}

/// Load settings from disk, returning defaults if missing or corrupt.
pub fn load_settings() -> AppSettings {
    let path = settings_path();
    match std::fs::read_to_string(&path) {
        Ok(contents) => toml::from_str(&contents).unwrap_or_else(|e| {
            log::warn!("Failed to parse settings at {}: {}", path.display(), e);
            AppSettings::default()
        }),
        Err(_) => AppSettings::default(),
    }
}

/// Save settings to disk atomically (write to temp, then rename).
pub fn save_settings(settings: &AppSettings) -> std::io::Result<()> {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let contents = toml::to_string_pretty(settings).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}
