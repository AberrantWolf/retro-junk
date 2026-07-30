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
    adopt_archive_identities(&mut settings.library);
    settings
}

/// Re-key every profile onto the identity of the archive it points at, then
/// collapse profiles that resolved to the same archive.
///
/// Settings written before profiles adopted archive identity can hold an id
/// the archive never knew, which makes its indexed releases invisible. Two
/// entries for one archive on different mounts are the same collection, and
/// the projection stores one root pair per identity, so the most recently
/// selected mount wins and the stale duplicate is dropped.
fn adopt_archive_identities(library: &mut LibrarySettings) {
    let previous_current = library.current_profile;
    let mut current = previous_current;
    for profile in &mut library.profiles {
        let was_current = Some(profile.profile_id) == previous_current;
        if profile.adopt_archive_identity() && was_current {
            current = Some(profile.profile_id);
        }
    }
    library.current_profile = current;

    let selected_root = library.current_root.clone();
    let mut kept: Vec<retro_junk_archive::CollectionProfile> = Vec::new();
    for profile in std::mem::take(&mut library.profiles) {
        let selected = selected_root
            .as_deref()
            .is_some_and(|root| root == profile.playable_root);
        match kept
            .iter_mut()
            .find(|existing| existing.profile_id == profile.profile_id)
        {
            Some(existing) if selected => *existing = profile,
            Some(_) => {}
            None => kept.push(profile),
        }
    }
    library.profiles = kept;

    if library
        .current_profile
        .is_some_and(|id| !library.profiles.iter().any(|p| p.profile_id == id))
    {
        library.current_profile = library.profiles.first().map(|p| p.profile_id);
    }
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

    /// Select (creating if needed) the profile for a playable root.
    ///
    /// A candidate is built first so its archive identity is known: opening a
    /// copy of an already-configured archive at a new mount must re-point the
    /// existing profile rather than add a second one under a fresh id, which
    /// would leave the copy's indexed releases unreachable.
    pub fn ensure_profile_for_root(
        &mut self,
        playable_root: &std::path::Path,
    ) -> retro_junk_archive::ArchiveProfileId {
        let candidate =
            retro_junk_archive::CollectionProfile::from_legacy_playable_root(playable_root);
        let existing = self.profiles.iter_mut().find(|profile| {
            profile.playable_root == playable_root || profile.profile_id == candidate.profile_id
        });
        let id = if let Some(profile) = existing {
            profile.archive_root = candidate.archive_root;
            profile.playable_root = candidate.playable_root;
            profile.profile_id
        } else {
            let id = candidate.profile_id;
            self.profiles.push(candidate);
            id
        };
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

    /// Initialize a collection at `<root>/archive` and return its identity.
    fn init_collection(root: &std::path::Path, name: &str) -> retro_junk_archive::ArchiveProfileId {
        std::fs::create_dir_all(root.join("roms")).unwrap();
        let manifest = retro_junk_archive::ArchiveRootManifest::new(name);
        retro_junk_archive::initialize_archive(&root.join("archive"), &manifest).unwrap();
        manifest.profile_id
    }

    fn settings_for(
        profiles: Vec<retro_junk_archive::CollectionProfile>,
        current: retro_junk_archive::ArchiveProfileId,
        current_root: PathBuf,
    ) -> AppSettings {
        AppSettings {
            library: LibrarySettings {
                current_profile: Some(current),
                profiles,
                current_root: Some(current_root),
                recent_roots: Vec::new(),
            },
            general: GeneralSettings::default(),
        }
    }

    #[test]
    fn a_profile_whose_id_drifted_from_its_archive_is_repaired_on_load() {
        let temp = tempfile::tempdir().unwrap();
        let archived = init_collection(temp.path(), "Collection");
        let mut profile = retro_junk_archive::CollectionProfile::for_roots(
            temp.path().join("archive"),
            temp.path().join("roms"),
        );
        let stale = retro_junk_archive::ArchiveProfileId::new();
        profile.profile_id = stale;
        let settings =
            migrate_legacy_profile(settings_for(vec![profile], stale, temp.path().join("roms")));

        assert_eq!(settings.library.profiles[0].profile_id, archived);
        // The selection has to follow the rekey, or nothing is active.
        assert_eq!(settings.library.current_profile, Some(archived));
    }

    #[test]
    fn duplicate_profiles_for_one_archive_collapse_onto_the_selected_mount() {
        let network = tempfile::tempdir().unwrap();
        let archived = init_collection(network.path(), "Collection");
        // The same collection rsynced to a second drive keeps its manifest.
        let local = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(local.path().join("archive")).unwrap();
        std::fs::create_dir_all(local.path().join("roms")).unwrap();
        std::fs::copy(
            retro_junk_archive::root_manifest_path(&network.path().join("archive")),
            retro_junk_archive::root_manifest_path(&local.path().join("archive")),
        )
        .unwrap();

        let network_profile = retro_junk_archive::CollectionProfile::for_roots(
            network.path().join("archive"),
            network.path().join("roms"),
        );
        let mut local_profile = retro_junk_archive::CollectionProfile::for_roots(
            local.path().join("archive"),
            local.path().join("roms"),
        );
        // A second entry added before identity adoption carried a fresh id.
        let stale = retro_junk_archive::ArchiveProfileId::new();
        local_profile.profile_id = stale;

        let settings = migrate_legacy_profile(settings_for(
            vec![network_profile, local_profile],
            stale,
            local.path().join("roms"),
        ));

        // One archive identity, and the projection tracks one root pair, so
        // the mount that is actually selected must be the surviving one.
        assert_eq!(settings.library.profiles.len(), 1);
        assert_eq!(settings.library.profiles[0].profile_id, archived);
        assert_eq!(
            settings.library.profiles[0].playable_root,
            local.path().join("roms")
        );
        assert_eq!(settings.library.current_profile, Some(archived));
    }

    #[test]
    fn opening_a_copied_archive_repoints_the_existing_profile() {
        let network = tempfile::tempdir().unwrap();
        let archived = init_collection(network.path(), "Collection");
        let local = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(local.path().join("archive")).unwrap();
        std::fs::create_dir_all(local.path().join("roms")).unwrap();
        std::fs::copy(
            retro_junk_archive::root_manifest_path(&network.path().join("archive")),
            retro_junk_archive::root_manifest_path(&local.path().join("archive")),
        )
        .unwrap();
        let mut library = LibrarySettings {
            current_profile: Some(archived),
            profiles: vec![retro_junk_archive::CollectionProfile::for_roots(
                network.path().join("archive"),
                network.path().join("roms"),
            )],
            current_root: Some(network.path().join("roms")),
            recent_roots: Vec::new(),
        };

        let id = library.ensure_profile_for_root(&local.path().join("roms"));

        assert_eq!(id, archived);
        assert_eq!(library.profiles.len(), 1);
        assert_eq!(
            library.profiles[0].archive_root,
            local.path().join("archive")
        );
    }

    #[test]
    fn distinct_archives_keep_distinct_profiles() {
        let first = tempfile::tempdir().unwrap();
        let first_id = init_collection(first.path(), "First");
        let second = tempfile::tempdir().unwrap();
        let second_id = init_collection(second.path(), "Second");
        let mut library = LibrarySettings {
            current_profile: Some(first_id),
            profiles: vec![retro_junk_archive::CollectionProfile::for_roots(
                first.path().join("archive"),
                first.path().join("roms"),
            )],
            current_root: Some(first.path().join("roms")),
            recent_roots: Vec::new(),
        };

        let id = library.ensure_profile_for_root(&second.path().join("roms"));

        assert_eq!(id, second_id);
        assert_eq!(library.profiles.len(), 2);
    }

    #[test]
    fn legacy_collection_profiles_default_to_network_mode() {
        let profile = retro_junk_archive::CollectionProfile::from_legacy_playable_root(
            std::path::Path::new("/collections/roms"),
        );
        let serialized = toml::to_string(&profile).unwrap();
        let legacy = serialized
            .lines()
            .filter(|line| !line.starts_with("network_mode"))
            .collect::<Vec<_>>()
            .join("\n");
        let loaded: retro_junk_archive::CollectionProfile = toml::from_str(&legacy).unwrap();

        assert!(loaded.network_mode);
    }

    #[test]
    fn collection_network_mode_round_trips_when_disabled() {
        let mut profile = retro_junk_archive::CollectionProfile::from_legacy_playable_root(
            std::path::Path::new("/collections/roms"),
        );
        profile.network_mode = false;
        let loaded: retro_junk_archive::CollectionProfile =
            toml::from_str(&toml::to_string(&profile).unwrap()).unwrap();

        assert!(!loaded.network_mode);
    }
}
