//! Thin dispatch to the backend's asset operations.
//!
//! Everything here reads the selection, schedules a backend call, and turns
//! what comes back into messages. Asset discovery, scraping, and miximage
//! composition live in `retro_junk_backend::ops::{assets, scrape_media,
//! miximage}`.

use std::collections::HashMap;
use std::path::PathBuf;

use retro_junk_backend::ops::OpCtx;
use retro_junk_backend::ops::miximage::{MiximageRequest, MiximageWorkItem};
use retro_junk_backend::ops::scrape_media::{ScrapeMediaRequest, ScrapeWorkItem};
use retro_junk_frontend::AssetType;

use crate::app::RetroJunkApp;
use crate::backend::worker::{forward_phases, spawn_background_op};
use crate::state::{self, AppMessage, OperationKind, ProgressDisplay};

/// Load media files for an entry on a background thread.
///
/// The detail panel loads them through egui's file loader, so this path never
/// creates a permanent in-memory byte cache.
pub fn load_assets_for_entry(
    tx: crate::state::AppMessageSender,
    ctx: egui::Context,
    root_path: PathBuf,
    folder_name: String,
    entry_id: retro_junk_db::LibraryEntryId,
    _entry_name: String,
    rom_stem: String,
    media_dir_setting: String,
) {
    std::thread::spawn(move || {
        let assets = retro_junk_backend::ops::assets::load_entry_assets(
            &root_path,
            &folder_name,
            &media_dir_setting,
            &rom_stem,
        );
        let _ = tx.send(AppMessage::AssetsLoaded {
            folder_name,
            entry_id,
            assets,
        });
        ctx.request_repaint();
    });
}

/// Query media availability for every row in a newly-loaded page without
/// reading or retaining any image data.
pub fn load_asset_statuses_for_page(
    tx: crate::state::AppMessageSender,
    ctx: egui::Context,
    root_path: PathBuf,
    console_id: retro_junk_db::LibraryConsoleId,
    folder_name: String,
    entries: Vec<(retro_junk_db::LibraryEntryId, String)>,
    media_dir_setting: String,
) {
    std::thread::spawn(move || {
        let statuses = retro_junk_backend::ops::assets::load_page_asset_statuses(
            &root_path,
            &folder_name,
            &media_dir_setting,
            entries,
        );
        let _ = tx.send(AppMessage::AssetStatusesLoaded {
            console_id,
            statuses,
        });
        ctx.request_repaint();
    });
}

/// Re-scrape media (force redownload of all media types).
pub fn rescrape_media_for_selection(
    app: &mut RetroJunkApp,
    console_idx: usize,
    ctx: &egui::Context,
) {
    scrape_media_for_selection(app, console_idx, ctx, true, false, None);
}

/// Scrape only missing media (skip types that already exist on disk).
pub fn scrape_missing_media_for_selection(
    app: &mut RetroJunkApp,
    console_idx: usize,
    ctx: &egui::Context,
) {
    scrape_media_for_selection(app, console_idx, ctx, false, false, None);
}

/// Scrape only missing image artwork for selected rows, excluding videos.
pub fn scrape_missing_artwork_for_selection(
    app: &mut RetroJunkApp,
    console_idx: usize,
    ctx: &egui::Context,
) {
    scrape_media_for_selection(app, console_idx, ctx, false, true, None);
}

/// Re-scrape a whole console using unpaginated archive-release details.
pub fn rescrape_media_for_console(
    app: &mut RetroJunkApp,
    console_idx: usize,
    ctx: &egui::Context,
    archived_releases: Vec<retro_junk_db::ArchivedLibraryListItem>,
) {
    scrape_media_for_selection(app, console_idx, ctx, true, false, Some(archived_releases));
}

/// Scrape only missing media for a whole console using unpaginated
/// archive-release details.
pub fn scrape_missing_artwork_for_console(
    app: &mut RetroJunkApp,
    console_idx: usize,
    ctx: &egui::Context,
    archived_releases: Vec<retro_junk_db::ArchivedLibraryListItem>,
) {
    scrape_media_for_selection(app, console_idx, ctx, false, true, Some(archived_releases));
}

/// Restore archived originals to the active frontend layout without network
/// access. This is deliberately separate from scraping: a cleaned or newly
/// synced device can reconstruct its media tree while offline.
pub fn restore_archived_media_for_release(
    app: &mut RetroJunkApp,
    release_id: String,
    folder_name: String,
    frontend_stems: Vec<String>,
) {
    let Some(profile) = app.settings.library.active_profile().cloned() else {
        app.push_error("Restore archived media", "No active collection profile");
        return;
    };
    let Some(media_directory) = retro_junk_lib::util::asset_dir_for_console(
        &profile.playable_root,
        &folder_name,
        &app.settings.general.assets_dir,
    ) else {
        app.push_error(
            "Restore archived media",
            "Cannot determine the frontend media directory",
        );
        return;
    };
    spawn_background_op(
        app,
        "Restoring archived media".to_owned(),
        OperationKind::Other,
        folder_name,
        ProgressDisplay::Count,
        move |_op_id, cancel, tx| {
            let result = retro_junk_backend::ops::assets::restore_archived_release_assets(
                &profile.archive_root,
                &release_id,
                &media_directory,
                frontend_stems,
                &cancel,
            );
            crate::backend::worker::deliver_result(&tx, result, |result| {
                AppMessage::AssetProjectionComplete { result }
            })
        },
    );
}

/// Collect the archived artwork a release already holds, keyed by type.
fn archived_asset_paths(
    release: &retro_junk_db::ArchivedLibraryListItem,
) -> HashMap<AssetType, PathBuf> {
    release
        .archived_assets
        .iter()
        .filter_map(|asset| {
            AssetType::from_archive_name(&asset.asset_type)
                .map(|asset_type| (asset_type, PathBuf::from(&asset.absolute_path)))
        })
        .collect()
}

/// The archive releases the user is acting on: an explicit selection, the
/// focused row, or every row when the caller asked for the whole console.
fn selected_archive_ids(
    app: &RetroJunkApp,
    whole_console: bool,
    rows: &[retro_junk_db::ArchivedLibraryListItem],
) -> Vec<String> {
    if whole_console {
        rows.iter()
            .map(|release| release.summary.archive_release_id.clone())
            .collect()
    } else if app.ui_state.selected_archive_releases.is_empty() {
        app.ui_state
            .focused_archive_release
            .iter()
            .cloned()
            .collect()
    } else {
        app.ui_state
            .selected_archive_releases
            .iter()
            .cloned()
            .collect()
    }
}

/// Scrape media from `ScreenScraper` for selected entries.
fn scrape_media_for_selection(
    app: &mut RetroJunkApp,
    console_idx: usize,
    ctx: &egui::Context,
    force_redownload: bool,
    artwork_only: bool,
    archive_rows_override: Option<Vec<retro_junk_db::ArchivedLibraryListItem>>,
) {
    let console = &app.browser.consoles[console_idx];
    let platform = console.platform;
    let folder_name = console.folder_name.clone();

    let Some(root_path) = app.root_path.clone() else {
        return;
    };

    // The analyzer is borrowed here because serial adaptation needs it and it
    // is not `Send`; everything it produces is plain data.
    let analyzer = app.context.get_by_platform(platform);
    let archive_profile = app.settings.library.active_profile().cloned();
    let whole_console = archive_rows_override.is_some();
    let archive_rows = archive_rows_override.unwrap_or_else(|| {
        app.browser
            .active_page
            .as_ref()
            .map(|page| page.archived_releases.clone())
            .unwrap_or_default()
    });

    let mut work: Vec<ScrapeWorkItem> = app
        .ui_state
        .selected_entries
        .iter()
        .copied()
        .filter_map(|i| {
            let entry = console.entry_by_id(i)?;
            let analysis_path = entry.game_entry.analysis_path();
            let filename = analysis_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();

            let serial = entry
                .identification
                .as_ref()
                .map(|id| id.serial_number.clone())
                .unwrap_or_default();
            let scraper_serial = (!serial.is_empty())
                .then(|| analyzer.and_then(|a| a.analyzer.extract_scraper_serial(&serial)))
                .flatten()
                .unwrap_or_default();

            let regions = entry.effective_regions();
            let preferred_region = regions.first().map_or_else(
                || "us".to_string(),
                |r| retro_junk_scraper::region_to_ss_code(r).to_string(),
            );
            let archived_release = archive_rows.iter().find(|release| {
                release
                    .playable_library_entries
                    .iter()
                    .any(|playable| playable.id == i)
            });

            Some(ScrapeWorkItem {
                entry_id: Some(i),
                entry_name: entry.game_entry.display_name().to_string(),
                rom_stem: entry.game_entry.rom_stem().to_string(),
                filename,
                analysis_path: analysis_path.to_path_buf(),
                file_size: 0,
                serial,
                scraper_serial,
                hashes: entry.hashes.as_ref().and_then(|h| {
                    retro_junk_scraper::lookup::RomHashes::complete(
                        &h.crc32,
                        h.md5.as_deref().unwrap_or_default(),
                        h.sha1.as_deref().unwrap_or_default(),
                    )
                }),
                // Resolved from the catalog once the op is off the UI thread.
                derivation: retro_junk_scraper::Derivation::Own,
                preferred_region,
                platform,
                archive_release_id: archived_release
                    .and_then(|release| release.summary.archive_release_id.parse().ok()),
                archived_assets: archived_release.map_or_else(HashMap::new, archived_asset_paths),
            })
        })
        .collect();

    let selected_ids = selected_archive_ids(app, whole_console, &archive_rows);
    let represented_archive_ids = work
        .iter()
        .filter_map(|item| item.archive_release_id.map(|id| id.to_string()))
        .collect::<std::collections::HashSet<_>>();
    for release in archive_rows.iter().filter(|release| {
        selected_ids.contains(&release.summary.archive_release_id)
            && !represented_archive_ids.contains(&release.summary.archive_release_id)
    }) {
        let Some(identity) = release.scrape_identity.as_ref() else {
            continue;
        };
        let Ok(archive_release_id) = release.summary.archive_release_id.parse() else {
            continue;
        };
        let serial = identity.serial.clone();
        let scraper_serial = analyzer
            .and_then(|analyzer| analyzer.analyzer.extract_scraper_serial(&serial))
            .unwrap_or_default();
        work.push(ScrapeWorkItem {
            entry_id: None,
            entry_name: release.summary.title.clone(),
            rom_stem: format!("archive-{archive_release_id}"),
            filename: identity.filename.clone(),
            analysis_path: PathBuf::new(),
            file_size: identity.file_size,
            serial,
            scraper_serial,
            hashes: retro_junk_scraper::lookup::RomHashes::complete(
                &identity.crc32,
                &identity.md5,
                &identity.sha1,
            ),
            // The projection already resolved this release's derivation; an
            // archive-only row has no library entry to look it up from.
            derivation: retro_junk_backend::scrape::scrape_derivation(&identity.derivation),
            preferred_region: retro_junk_scraper::systems::region_slug_to_ss_code(
                &release.summary.region,
            )
            .to_owned(),
            platform,
            archive_release_id: Some(archive_release_id),
            archived_assets: archived_asset_paths(release),
        });
    }

    if work.is_empty() {
        return;
    }

    // Force-redownload clears cached paths so the rows read as loading;
    // scraping only what is missing leaves existing artwork visible.
    if force_redownload {
        for item in &work {
            if let Some(entry_id) = item.entry_id
                && let Some(entry) = app.browser.consoles[console_idx].entry_by_id_mut(entry_id)
            {
                entry.asset_paths = None;
            }
        }
    }

    let media_dir_setting = app.settings.general.assets_dir.clone();
    let db_path = app.db_path.clone();
    let ui_ctx = ctx.clone();
    let verb = if force_redownload {
        "Scraping media"
    } else if artwork_only {
        "Scraping missing artwork"
    } else {
        "Scraping missing media"
    };
    let description = format!("{} ({} entries)", verb, work.len());

    spawn_background_op(
        app,
        description,
        OperationKind::Other,
        folder_name.clone(),
        ProgressDisplay::Count,
        move |op_id, cancel, tx| {
            let progress = forward_phases(op_id, tx.clone());
            let report = retro_junk_backend::ops::scrape_media::scrape_media(
                db_path.as_deref(),
                ScrapeMediaRequest {
                    platform,
                    root_path,
                    folder_name: folder_name.clone(),
                    media_dir_setting,
                    work,
                    force_redownload,
                    artwork_only,
                    archive_profile,
                    scratch_tag: op_id.to_string(),
                },
                &OpCtx::new(&cancel, &progress),
            );
            deliver_media_updates(
                &tx,
                &ui_ctx,
                &folder_name,
                &report.entry_assets,
                &report.invalidated_paths,
            );
            for failure in report.entry_failures {
                let _ = tx.send(AppMessage::ScrapeEntryFailed {
                    folder_name: folder_name.clone(),
                    entry_id: failure.entry_id,
                    entry_name: failure.entry_name,
                    error: failure.error,
                });
            }
            let fatal_error =
                (!report.fatal_errors.is_empty()).then(|| report.fatal_errors.join("; "));
            for message in report.fatal_errors {
                let _ = tx.send(AppMessage::ScrapeFatalError { message, op_id });
            }
            if report.archive_assets_changed {
                let _ = tx.send(AppMessage::ArchiveAssetsChanged);
            }
            ui_ctx.request_repaint();
            fatal_error.map_or(Ok(()), Err)
        },
    );
}

/// Re-generate miximages from existing on-disk media for selected entries.
pub fn regenerate_miximages_for_selection(
    app: &mut RetroJunkApp,
    console_idx: usize,
    ctx: &egui::Context,
) {
    let console = &app.browser.consoles[console_idx];
    let folder_name = console.folder_name.clone();

    let Some(root_path) = app.root_path.clone() else {
        return;
    };
    let archive_profile = app.settings.library.active_profile().cloned();
    let archive_rows = app
        .browser
        .active_page
        .as_ref()
        .map(|page| page.archived_releases.clone())
        .unwrap_or_default();

    let mut work: Vec<MiximageWorkItem> = app
        .ui_state
        .selected_entries
        .iter()
        .copied()
        .filter_map(|i| {
            let entry = console.entry_by_id(i)?;
            let archived_release = archive_rows.iter().find(|release| {
                release
                    .playable_library_entries
                    .iter()
                    .any(|playable| playable.id == i)
            });
            Some(MiximageWorkItem {
                entry_id: Some(i),
                entry_name: entry.game_entry.display_name().to_string(),
                rom_stem: entry.game_entry.rom_stem().to_string(),
                archive_release_id: archived_release
                    .and_then(|release| release.summary.archive_release_id.parse().ok()),
                archived_assets: archived_release.map_or_else(HashMap::new, archived_asset_paths),
            })
        })
        .collect();

    let selected_ids = selected_archive_ids(app, false, &archive_rows);
    let represented_archive_ids = work
        .iter()
        .filter_map(|item| item.archive_release_id.map(|id| id.to_string()))
        .collect::<std::collections::HashSet<_>>();
    for release in archive_rows.iter().filter(|release| {
        selected_ids.contains(&release.summary.archive_release_id)
            && !represented_archive_ids.contains(&release.summary.archive_release_id)
    }) {
        let Ok(archive_release_id) = release.summary.archive_release_id.parse() else {
            continue;
        };
        work.push(MiximageWorkItem {
            entry_id: None,
            entry_name: release.summary.title.clone(),
            rom_stem: format!("archive-{archive_release_id}"),
            archive_release_id: Some(archive_release_id),
            archived_assets: archived_asset_paths(release),
        });
    }

    if work.is_empty() {
        app.push_error(
            "Generate miximage",
            "No playable entry or archived release is selected",
        );
        return;
    }

    let media_dir_setting = app.settings.general.assets_dir.clone();
    let ui_ctx = ctx.clone();
    let description = format!("Re-generating miximages ({} entries)", work.len());

    spawn_background_op(
        app,
        description,
        OperationKind::Other,
        folder_name.clone(),
        ProgressDisplay::Count,
        move |op_id, cancel, tx| {
            let progress = forward_phases(op_id, tx.clone());
            let outcome = retro_junk_backend::ops::miximage::regenerate_miximages(
                MiximageRequest {
                    root_path,
                    folder_name: folder_name.clone(),
                    media_dir_setting,
                    work,
                    archive_profile,
                    scratch_tag: op_id.to_string(),
                },
                &OpCtx::new(&cancel, &progress),
            );
            let lifecycle = match outcome {
                Ok(report) => {
                    deliver_media_updates(
                        &tx,
                        &ui_ctx,
                        &folder_name,
                        &report.entry_assets,
                        &report.invalidated_paths,
                    );
                    if report.archive_assets_changed {
                        let _ = tx.send(AppMessage::ArchiveAssetsChanged);
                    }
                    let _ = tx.send(AppMessage::MiximageComplete {
                        generated: report.generated,
                        failures: report.failures,
                    });
                    Ok(())
                }
                Err(error) => {
                    let lifecycle_error = error.clone();
                    let _ = tx.send(AppMessage::MiximageComplete {
                        generated: 0,
                        failures: vec![error],
                    });
                    Err(lifecycle_error)
                }
            };
            ui_ctx.request_repaint();
            lifecycle
        },
    );
}

/// Drop egui's cached decodes for images whose bytes changed, then hand each
/// entry its refreshed media set. Both scraping and miximage composition end
/// this way, so the cache can never keep showing superseded artwork.
fn deliver_media_updates(
    tx: &crate::state::AppMessageSender,
    ctx: &egui::Context,
    folder_name: &str,
    entry_assets: &[(retro_junk_db::LibraryEntryId, HashMap<AssetType, PathBuf>)],
    invalidated_paths: &[PathBuf],
) {
    for path in invalidated_paths {
        ctx.forget_image(&state::asset_image_uri(path));
    }
    for (entry_id, assets) in entry_assets {
        for path in assets.values() {
            ctx.forget_image(&state::asset_image_uri(path));
        }
        let _ = tx.send(AppMessage::AssetsLoaded {
            folder_name: folder_name.to_owned(),
            entry_id: *entry_id,
            assets: assets.clone(),
        });
    }
    ctx.request_repaint();
}
