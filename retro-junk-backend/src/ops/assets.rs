//! Media-asset filesystem operations that need no network access: per-entry
//! asset discovery, page-wide availability queries, offline restoration of
//! archived artwork into the frontend media tree, and adoption of existing
//! frontend media into a freshly scanned archive.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use retro_junk_frontend::AssetType;

use crate::assets::{AssetStatus, asset_availability, collect_existing_assets};

#[cfg(test)]
#[path = "assets_tests.rs"]
mod tests;

/// Discover media files on disk for one entry.
///
/// Returns an empty map when the console has no media directory — the caller
/// cannot tell "no directory" from "directory with nothing in it", and does
/// not need to.
pub fn load_entry_assets(
    root_path: &Path,
    folder_name: &str,
    media_dir_setting: &str,
    rom_stem: &str,
) -> HashMap<AssetType, PathBuf> {
    retro_junk_lib::util::asset_dir_for_console(root_path, folder_name, media_dir_setting)
        .map(|media_dir| collect_existing_assets(&media_dir, rom_stem))
        .unwrap_or_default()
}

/// Query media availability for every row in a page without reading or
/// retaining any image data. Each result is (entry, completeness, has a
/// miximage).
pub fn load_page_asset_statuses(
    root_path: &Path,
    folder_name: &str,
    media_dir_setting: &str,
    entries: Vec<(retro_junk_db::LibraryEntryId, String)>,
) -> Vec<(retro_junk_db::LibraryEntryId, AssetStatus, bool)> {
    let media_dir =
        retro_junk_lib::util::asset_dir_for_console(root_path, folder_name, media_dir_setting);
    entries
        .into_iter()
        .map(|(entry_id, display_name)| {
            if let Some(media_dir) = media_dir.as_ref() {
                let rom_stem = media_stem_for_display_name(display_name);
                let found = collect_existing_assets(media_dir, &rom_stem);
                let (status, has_miximage) = asset_availability(&found);
                (entry_id, status, has_miximage)
            } else {
                (entry_id, AssetStatus::None, false)
            }
        })
        .collect()
}

/// Multi-disc entries intentionally keep the `.m3u` suffix in their media
/// stem; ordinary files use the filename stem.
fn media_stem_for_display_name(display_name: String) -> String {
    if display_name.to_ascii_lowercase().ends_with(".m3u") {
        display_name
    } else {
        Path::new(&display_name)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(&display_name)
            .to_owned()
    }
}

/// Restore archived originals to the active frontend layout without network
/// access. This is deliberately separate from scraping: a cleaned or newly
/// synced device can reconstruct its media tree while offline.
///
/// When `frontend_stems` is empty, the stems recorded by the archive's own
/// playable outputs are used.
pub fn restore_archived_release_assets(
    archive_root: &Path,
    release_id: &str,
    media_dir: &Path,
    frontend_stems: Vec<String>,
    cancel: &AtomicBool,
) -> Result<retro_junk_lib::archive_assets::AssetProjectionReport, String> {
    let snapshot =
        retro_junk_archive::scan_archive(archive_root).map_err(|error| error.to_string())?;
    let release = snapshot
        .releases
        .iter()
        .find(|release| release.manifest.archive_release_id.to_string() == release_id)
        .ok_or_else(|| "Archived release is no longer present".to_owned())?;
    let stems = if frontend_stems.is_empty() {
        retro_junk_lib::archive_assets::release_media_stems(release)
    } else {
        frontend_stems.into_iter().collect()
    };
    retro_junk_lib::archive_assets::project_release_assets(release, media_dir, &stems, cancel)
        .map_err(|error| error.to_string())
}

struct CandidateArchiveAsset {
    release_id: retro_junk_archive::ArchiveReleaseId,
    asset_type: AssetType,
    path: PathBuf,
}

/// Adopt already-downloaded frontend media for releases present in a freshly
/// scanned archive. The caller owns the archive write lock.
pub fn adopt_playable_artwork(
    connection: &retro_junk_db::Connection,
    snapshot: &retro_junk_archive::ArchiveIndexSnapshot,
    profile: &retro_junk_archive::CollectionProfile,
    media_dir_setting: &str,
    cancel: &AtomicBool,
) -> Result<usize, String> {
    // Which archive release holds which catalog release is the projection's
    // answer, derived by content from each release's carriers. The manifests
    // themselves name no catalog row.
    let archive_releases = retro_junk_db::archive::archive_releases_by_catalog_release(
        connection,
        &snapshot.manifest.profile_id.to_string(),
    )
    .map_err(|error| error.to_string())?;
    if archive_releases.is_empty() {
        return Ok(0);
    }
    let selection = retro_junk_scraper::AssetSelection::all();
    let mut assets = Vec::new();
    for candidate in
        retro_junk_db::playable_artwork_candidates(connection).map_err(|error| error.to_string())?
    {
        let Some(release_id) = archive_releases
            .get(&candidate.catalog_release_id)
            .and_then(|id| id.parse::<retro_junk_archive::ArchiveReleaseId>().ok())
        else {
            continue;
        };
        let Ok(game_entry) =
            serde_json::from_str::<retro_junk_lib::scanner::GameEntry>(&candidate.game_entry_json)
        else {
            continue;
        };
        let Some(media_dir) = retro_junk_lib::util::asset_dir_for_console(
            &profile.playable_root,
            &candidate.folder_name,
            media_dir_setting,
        ) else {
            continue;
        };
        assets.extend(
            selection
                .collect_existing(&media_dir, game_entry.rom_stem())
                .into_iter()
                .map(|(asset_type, path)| CandidateArchiveAsset {
                    release_id,
                    asset_type,
                    path,
                }),
        );
    }
    if assets.is_empty() {
        return Ok(0);
    }
    let asset_names = assets
        .iter()
        .map(|asset| asset.asset_type.to_string())
        .collect::<Vec<_>>();
    let requests = assets
        .iter()
        .zip(&asset_names)
        .map(|(asset, asset_name)| retro_junk_archive::NewReleaseFile {
            release_id: asset.release_id,
            source_file: &asset.path,
            category: if asset.asset_type == AssetType::Video {
                retro_junk_archive::ReleaseFileCategory::Video
            } else {
                retro_junk_archive::ReleaseFileCategory::Artwork
            },
            asset_type: asset_name,
            source: "existing playable media",
            source_url: "",
            caption: "",
        })
        .collect::<Vec<_>>();
    retro_junk_archive::add_release_files(&profile.archive_root, &requests, cancel)
        .map(|results| results.into_iter().filter(|result| result.added).count())
        .map_err(|error| error.to_string())
}
