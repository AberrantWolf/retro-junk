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

/// Where a profile's frontend files go: playables, scraped media, gamelists.
///
/// One answer for every surface. The CLI used to build these itself from empty
/// strings and then substitute its own sibling `-metadata` default, so it
/// maintained `roms-metadata/psx/gamelist.xml` while the user's setting put
/// the gamelist the frontend actually reads at `roms/psx/gamelist.xml`. Both
/// files existed, both looked healthy, and every projection updated the one
/// nobody read — a renamed game kept its old entry in the frontend forever.
///
/// `media_override` and `metadata_override` are the explicit `--media-root`
/// and `--metadata-root` flags: a caller may still say exactly where it wants
/// output, but silence means the user's settings, never a second default.
#[must_use]
pub fn frontend_roots(
    playable_root: &Path,
    media_override: Option<std::path::PathBuf>,
    metadata_override: Option<std::path::PathBuf>,
) -> retro_junk_lib::archive_ops::FrontendRoots {
    frontend_roots_from(
        &retro_junk_lib::settings::settings_path(),
        playable_root,
        media_override,
        metadata_override,
    )
}

/// [`frontend_roots`] against a named settings file.
#[must_use]
pub fn frontend_roots_from(
    settings_path: &Path,
    playable_root: &Path,
    media_override: Option<std::path::PathBuf>,
    metadata_override: Option<std::path::PathBuf>,
) -> retro_junk_lib::archive_ops::FrontendRoots {
    let general = read(settings_path).general;
    let mut roots = retro_junk_lib::archive_ops::FrontendRoots::from_settings(
        playable_root,
        &general.assets_dir,
        &general.metadata_dir,
    );
    if let Some(media_root) = media_override {
        roots.media_root = media_root;
    }
    if let Some(metadata_root) = metadata_override {
        roots.metadata_root = metadata_root;
    }
    roots
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
    // Selection happens on the ids as written, then the winner re-adopts the
    // identity of the archive it points at. Settings authored before profiles
    // took their identity from the archive can hold an id the archive never
    // knew, and the projection is keyed on the archive's id — the CLI and
    // daemon must not have to wait for the GUI to rewrite the file.
    let mut profiles = section.profiles;
    let selected = match selector {
        Some(wanted) => profiles.iter().position(|profile| {
            profile.profile_id.to_string() == wanted
                || profile.display_name.eq_ignore_ascii_case(wanted)
        }),
        None => section
            .current_profile
            .and_then(|current| {
                profiles
                    .iter()
                    .position(|profile| profile.profile_id == current)
            })
            .or_else(|| (profiles.len() == 1).then_some(0)),
    };
    let mut resolved = profiles.remove(selected?);
    resolved.adopt_archive_identity();
    Some(resolved)
}

fn read(path: &Path) -> SettingsDoc {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|contents| toml::from_str(&contents).ok())
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "tests/profiles_tests.rs"]
mod tests;
