//! Miximage regeneration from existing on-disk media.
//!
//! Composites miximages using already-scraped component images (screenshot,
//! box art, etc.) without contacting `ScreenScraper`. Archived component
//! images are first restored into the target media directory; a generated
//! miximage for an archived release is published back into the archive under
//! its own lock. Runs synchronously — no async runtime is needed.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use retro_junk_frontend::AssetType;
use retro_junk_io::ProgressUnit;

use super::OpCtx;

#[cfg(test)]
#[path = "miximage_tests.rs"]
mod tests;

/// One entry (or archive-only release) to regenerate a miximage for.
pub struct MiximageWorkItem {
    /// The playable library row this item belongs to. `None` for a release
    /// that exists only in the archive, whose output composes in a scratch
    /// directory and lives on only as an archived supporting file.
    pub entry_id: Option<retro_junk_db::LibraryEntryId>,
    pub entry_name: String,
    pub rom_stem: String,
    pub archive_release_id: Option<retro_junk_archive::ArchiveReleaseId>,
    /// Archived component images restored into the target before composing.
    pub archived_assets: HashMap<AssetType, PathBuf>,
}

/// Build miximage work from the same logical selection and archive ownership
/// used by artwork scraping.
#[must_use]
pub fn plan_miximage_work<S1: std::hash::BuildHasher, S2: std::hash::BuildHasher>(
    entries: &[crate::library::LibraryEntry],
    selected_entry_ids: &HashSet<retro_junk_db::LibraryEntryId, S1>,
    archive_releases: &[retro_junk_db::ArchivedLibraryListItem],
    selected_archive_release_ids: &HashSet<String, S2>,
    focused_archive_release_id: Option<&str>,
) -> Vec<MiximageWorkItem> {
    let mut work = selected_entry_ids
        .iter()
        .filter_map(|entry_id| {
            let entry = entries.iter().find(|entry| entry.id == Some(*entry_id))?;
            let release = archive_releases.iter().find(|release| {
                release
                    .playable_library_entries
                    .iter()
                    .any(|playable| playable.id == *entry_id)
            });
            Some(MiximageWorkItem {
                entry_id: Some(*entry_id),
                entry_name: entry.game_entry.display_name().to_owned(),
                rom_stem: entry.game_entry.rom_stem().to_owned(),
                archive_release_id: release
                    .and_then(|release| release.summary.archive_release_id.parse().ok()),
                archived_assets: release
                    .map(crate::assets::archived_asset_paths)
                    .unwrap_or_default(),
            })
        })
        .collect::<Vec<_>>();
    let represented = work
        .iter()
        .filter_map(|item| item.archive_release_id.map(|id| id.to_string()))
        .collect::<HashSet<_>>();
    for release in archive_releases.iter().filter(|release| {
        (selected_archive_release_ids.contains(&release.summary.archive_release_id)
            || (selected_archive_release_ids.is_empty()
                && focused_archive_release_id == Some(release.summary.archive_release_id.as_str())))
            && !represented.contains(&release.summary.archive_release_id)
    }) {
        let Ok(archive_release_id) = release.summary.archive_release_id.parse() else {
            continue;
        };
        work.push(MiximageWorkItem {
            entry_id: None,
            entry_name: release.summary.title.clone(),
            rom_stem: format!("archive-{archive_release_id}"),
            archive_release_id: Some(archive_release_id),
            archived_assets: crate::assets::archived_asset_paths(release),
        });
    }
    work
}

/// Everything a miximage run needs beyond the work items themselves.
pub struct MiximageRequest {
    /// Playable root the console folder lives under.
    pub root_path: PathBuf,
    pub folder_name: String,
    /// The user's media-directory setting, resolved per console.
    pub media_dir_setting: String,
    pub work: Vec<MiximageWorkItem>,
    /// Active collection profile; enables archive publication and provides
    /// the scratch workspace for archive-only rows.
    pub archive_profile: Option<retro_junk_archive::CollectionProfile>,
    /// Unique tag for this run's scratch directory (e.g. the operation id).
    pub scratch_tag: String,
}

/// What a miximage run hands back to the frontend.
#[derive(Default)]
pub struct MiximageReport {
    /// Number of miximages successfully composed.
    pub generated: usize,
    /// User-facing failure lines, one per item or publication problem.
    pub failures: Vec<String>,
    /// Per entry: the media now on disk in its media directory.
    pub entry_assets: Vec<(retro_junk_db::LibraryEntryId, HashMap<AssetType, PathBuf>)>,
    /// Image paths whose bytes may have changed; a frontend should evict any
    /// cached decodes of these.
    pub invalidated_paths: Vec<PathBuf>,
    /// The archive gained supporting files; projections should refresh.
    pub archive_assets_changed: bool,
}

/// Re-generate miximages from existing on-disk media.
///
/// Returns `Err` only for run-level setup failures (no media directory, no
/// layout); per-item problems land in the report's `failures`.
pub fn regenerate_miximages(
    request: MiximageRequest,
    ctx: &OpCtx,
) -> Result<MiximageReport, String> {
    let MiximageRequest {
        root_path,
        folder_name,
        media_dir_setting,
        work,
        archive_profile,
        scratch_tag,
    } = request;

    let media_dir =
        retro_junk_lib::util::asset_dir_for_console(&root_path, &folder_name, &media_dir_setting)
            .ok_or_else(|| format!("Cannot determine media directory for {folder_name}"))?;
    let layout = retro_junk_frontend::miximage_layout::MiximageLayout::load_or_create()
        .map_err(|error| format!("Failed to load miximage layout: {error}"))?;

    let scratch_root = archive_profile
        .as_ref()
        .map(|profile| profile.workspace_root.join("miximages").join(&scratch_tag));

    let mut report = MiximageReport::default();
    let mut generated = Vec::new();
    for (file_num, item) in work.iter().enumerate() {
        if ctx.cancelled() {
            break;
        }
        (ctx.progress)(
            "Re-generating miximages",
            ProgressUnit::Items,
            file_num as u64,
            work.len() as u64,
        );

        let target = if item.entry_id.is_some() {
            media_dir.clone()
        } else if let Some(scratch_root) = scratch_root.as_ref() {
            scratch_root.join(file_num.to_string())
        } else {
            report.failures.push(format!(
                "{} has no playable target or archive workspace",
                item.entry_name
            ));
            continue;
        };
        match generate_miximage_with_archived_assets(
            &item.archived_assets,
            &target,
            &item.rom_stem,
            &layout,
        ) {
            Ok(output_path) => {
                report.invalidated_paths.push(output_path.clone());
                generated.push((item.archive_release_id, output_path));
            }
            Err(error) => report
                .failures
                .push(format!("{}: {error}", item.entry_name)),
        }
        let updated_media = crate::assets::collect_existing_assets(&target, &item.rom_stem);

        // Invalidate any currently displayed component images without
        // loading bulk-operation results into memory.
        report.invalidated_paths.extend(
            updated_media
                .iter()
                .filter(|(mt, _)| **mt != AssetType::Miximage)
                .map(|(_, path)| path.clone()),
        );
        if let Some(entry_id) = item.entry_id {
            report.entry_assets.push((entry_id, updated_media));
        }
    }

    let archived_miximages = generated
        .iter()
        .filter_map(|(release_id, path)| release_id.map(|release_id| (release_id, path)))
        .collect::<Vec<_>>();
    if let Some(profile) = archive_profile.as_ref()
        && !archived_miximages.is_empty()
    {
        let requests = archived_miximages
            .iter()
            .map(|(release_id, path)| retro_junk_archive::NewReleaseFile {
                release_id: *release_id,
                source_file: path,
                category: retro_junk_archive::ReleaseFileCategory::Artwork,
                asset_type: "miximage",
                source: "retro-junk miximage",
                source_url: "",
                caption: "",
            })
            .collect::<Vec<_>>();
        match retro_junk_archive::ArchiveLock::acquire_wait(&profile.archive_root, ctx.cancel)
            .map_err(|error| error.to_string())
            .and_then(|lock| {
                let _lock = lock.ok_or_else(|| "archive publication cancelled".to_owned())?;
                retro_junk_archive::add_release_files(&profile.archive_root, &requests, ctx.cancel)
                    .map_err(|error| error.to_string())
            }) {
            Ok(results) => {
                if results.iter().any(|result| result.added) {
                    report.archive_assets_changed = true;
                }
            }
            Err(error) => report.failures.push(format!(
                "Generated miximage could not be stored in the archive: {error}"
            )),
        }
    }
    if let Some(scratch_root) = scratch_root.as_ref()
        && let Err(error) = std::fs::remove_dir_all(scratch_root)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!(
            "Could not remove miximage workspace {}: {error}",
            scratch_root.display()
        );
    }
    report.generated = generated.len();
    Ok(report)
}

/// Restore archived component images into `media_dir`, then compose the
/// miximage there. A screenshot must exist (restored or already on disk)
/// before anything can be composed.
fn generate_miximage_with_archived_assets(
    archived_assets: &HashMap<AssetType, PathBuf>,
    media_dir: &Path,
    rom_stem: &str,
    layout: &retro_junk_frontend::miximage_layout::MiximageLayout,
) -> Result<PathBuf, String> {
    for (asset_type, source) in archived_assets {
        retro_junk_lib::archive_assets::project_asset_file(
            source,
            media_dir,
            rom_stem,
            *asset_type,
        )
        .map_err(|error| format!("could not restore {asset_type}: {error}"))?;
    }
    retro_junk_lib::archive_assets::generate_frontend_miximage(media_dir, rom_stem, layout)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "a screenshot is required before a miximage can be generated".to_owned())
}
