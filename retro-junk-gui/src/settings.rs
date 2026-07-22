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
    /// Active preservation/playable collection profile.
    #[serde(default)]
    pub current_profile: Option<retro_junk_archive::ArchiveProfileId>,
    /// Device-local root mappings for portable collection identities.
    #[serde(default)]
    pub profiles: Vec<retro_junk_archive::CollectionProfile>,
    /// Legacy 0.3 playable root, retained while callers migrate to profiles.
    #[serde(default)]
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
    /// Asset output directory. If empty, uses `"{root}-media"` sibling convention.
    /// Relative paths resolve from the ROM root.
    #[serde(default, alias = "media_dir")]
    pub assets_dir: String,
    /// Catalog YAML seed-data directory (platforms/companies/overrides), used
    /// when importing DATs into the catalog DB. If empty, falls back to
    /// `./catalog` relative to the working directory (matching the CLI default).
    #[serde(default)]
    pub catalog_data_dir: String,
    /// Path to the chdman executable (MAME tools), used for CHD compression.
    /// If empty, `chdman` is looked up on PATH.
    #[serde(default)]
    pub chdman_path: String,
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
            assets_dir: String::new(),
            catalog_data_dir: String::new(),
            chdman_path: String::new(),
        }
    }
}

/// Returns `~/.config/retro-junk/settings.toml`.
///
/// Delegates to the shared implementation in `retro-junk-lib`.
pub fn settings_path() -> PathBuf {
    retro_junk_lib::settings::settings_path()
}

/// Load settings from disk, returning defaults if missing or corrupt.
pub fn load_settings() -> AppSettings {
    let path = settings_path();
    match std::fs::read_to_string(&path) {
        Ok(contents) => toml::from_str(&contents).map_or_else(
            |e| {
                log::warn!("Failed to parse settings at {}: {}", path.display(), e);
                AppSettings::default()
            },
            migrate_legacy_profile,
        ),
        Err(_) => AppSettings::default(),
    }
}

fn migrate_legacy_profile(mut settings: AppSettings) -> AppSettings {
    if settings.library.profiles.is_empty()
        && let Some(root) = settings.library.current_root.as_deref()
    {
        let profile = retro_junk_archive::CollectionProfile::from_legacy_playable_root(root);
        settings.library.current_profile = Some(profile.profile_id);
        settings.library.profiles.push(profile);
    }
    for profile in &mut settings.library.profiles {
        let legacy_workspace = profile.archive_root.join(".retro-junk").join("work");
        if profile.workspace_root == legacy_workspace {
            profile.workspace_root = retro_junk_io::default_profile_workspace(profile.profile_id.0);
        }
    }
    settings
}

impl LibrarySettings {
    #[must_use]
    pub fn active_profile(&self) -> Option<&retro_junk_archive::CollectionProfile> {
        let id = self.current_profile?;
        self.profiles
            .iter()
            .find(|profile| profile.profile_id == id)
    }

    pub fn active_profile_mut(&mut self) -> Option<&mut retro_junk_archive::CollectionProfile> {
        let id = self.current_profile?;
        self.profiles
            .iter_mut()
            .find(|profile| profile.profile_id == id)
    }

    pub fn ensure_profile_for_root(
        &mut self,
        playable_root: &std::path::Path,
    ) -> retro_junk_archive::ArchiveProfileId {
        if let Some(profile) = self
            .profiles
            .iter()
            .find(|profile| profile.playable_root == playable_root)
        {
            self.current_profile = Some(profile.profile_id);
            self.current_root = Some(playable_root.to_path_buf());
            return profile.profile_id;
        }
        let profile =
            retro_junk_archive::CollectionProfile::from_legacy_playable_root(playable_root);
        let id = profile.profile_id;
        self.profiles.push(profile);
        self.current_profile = Some(id);
        self.current_root = Some(playable_root.to_path_buf());
        id
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_archive_relative_workspace_moves_to_device_local_cache() {
        let playable = PathBuf::from("/collections/roms");
        let mut profile =
            retro_junk_archive::CollectionProfile::from_legacy_playable_root(&playable);
        profile.workspace_root = profile.archive_root.join(".retro-junk/work");
        let expected = retro_junk_io::default_profile_workspace(profile.profile_id.0);
        let settings = migrate_legacy_profile(AppSettings {
            library: LibrarySettings {
                current_profile: Some(profile.profile_id),
                profiles: vec![profile],
                current_root: Some(playable),
                recent_roots: Vec::new(),
            },
            general: GeneralSettings::default(),
        });
        assert_eq!(settings.library.profiles[0].workspace_root, expected);
    }

    #[test]
    fn explicitly_configured_workspace_is_preserved() {
        let playable = PathBuf::from("/collections/roms");
        let mut profile =
            retro_junk_archive::CollectionProfile::from_legacy_playable_root(&playable);
        profile.workspace_root = PathBuf::from("/fast-scratch/retro-junk");
        let settings = migrate_legacy_profile(AppSettings {
            library: LibrarySettings {
                current_profile: Some(profile.profile_id),
                profiles: vec![profile],
                current_root: Some(playable),
                recent_roots: Vec::new(),
            },
            general: GeneralSettings::default(),
        });
        assert_eq!(
            settings.library.profiles[0].workspace_root,
            PathBuf::from("/fast-scratch/retro-junk")
        );
    }
}
