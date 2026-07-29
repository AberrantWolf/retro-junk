//! Collection-profile resolution from the shared settings file.
//!
//! Profiles are authored by the GUI into `settings.toml` (`[[library.profiles]]`);
//! the CLI and daemon resolve them here instead of growing a parallel
//! configuration surface.

use std::path::Path;

use retro_junk_archive::CollectionProfile;
use serde::Deserialize;

#[derive(Deserialize, Default)]
struct SettingsDoc {
    #[serde(default)]
    library: LibrarySection,
    #[serde(default)]
    general: GeneralSection,
}

/// The GUI-authored `[general]` keys the daemon shares: tool locations and
/// frontend directory conventions.
#[derive(Deserialize, Default, Clone)]
pub struct GeneralSection {
    #[serde(default)]
    pub chdman_path: String,
    #[serde(default, alias = "media_dir")]
    pub assets_dir: String,
    #[serde(default)]
    pub metadata_dir: String,
}

/// Load the shared `[general]` settings section.
#[must_use]
pub fn load_general() -> GeneralSection {
    read(&retro_junk_lib::settings::settings_path()).general
}

#[derive(Deserialize, Default)]
struct LibrarySection {
    #[serde(default)]
    current_profile: Option<retro_junk_archive::ArchiveProfileId>,
    #[serde(default)]
    profiles: Vec<CollectionProfile>,
}

/// All configured profiles.
#[must_use]
pub fn load_profiles() -> Vec<CollectionProfile> {
    load_profiles_from(&retro_junk_lib::settings::settings_path())
}

#[must_use]
pub fn load_profiles_from(path: &Path) -> Vec<CollectionProfile> {
    read(path).library.profiles
}

/// Resolve a profile by id or display name; `None` selects the active
/// profile (or the only one).
#[must_use]
pub fn resolve_profile(selector: Option<&str>) -> Option<CollectionProfile> {
    resolve_profile_from(&retro_junk_lib::settings::settings_path(), selector)
}

#[must_use]
pub fn resolve_profile_from(path: &Path, selector: Option<&str>) -> Option<CollectionProfile> {
    let section = read(path).library;
    if let Some(wanted) = selector {
        return section.profiles.into_iter().find(|profile| {
            profile.profile_id.to_string() == wanted
                || profile.display_name.eq_ignore_ascii_case(wanted)
        });
    }
    if let Some(current) = &section.current_profile
        && let Some(profile) = section
            .profiles
            .iter()
            .find(|profile| profile.profile_id == *current)
    {
        return Some(profile.clone());
    }
    let mut profiles = section.profiles;
    (profiles.len() == 1).then(|| profiles.remove(0))
}

fn read(path: &Path) -> SettingsDoc {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|contents| toml::from_str(&contents).ok())
        .unwrap_or_default()
}
