use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use retro_junk_core::Platform;
use retro_junk_frontend::AssetType;
use retro_junk_lib::async_util::cancellable;
use retro_junk_scraper::ScrapeError;

use crate::app::RetroJunkApp;
use crate::backend::worker::spawn_background_op;
use crate::state::{self, AppMessage, OperationKind, ProgressDisplay};

/// Load media files for an entry on a background thread.
///
/// Discovers media files on disk. The detail panel loads them through egui's
/// file loader, so this path never creates a permanent in-memory byte cache.
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
        let Some(media_dir) =
            state::asset_dir_for_console(&root_path, &folder_name, &media_dir_setting)
        else {
            let _ = tx.send(AppMessage::AssetsLoaded {
                folder_name,
                entry_id,
                assets: HashMap::new(),
            });
            ctx.request_repaint();
            return;
        };

        let found = state::collect_existing_assets(&media_dir, &rom_stem);

        let _ = tx.send(AppMessage::AssetsLoaded {
            folder_name,
            entry_id,
            assets: found,
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
        let media_dir = state::asset_dir_for_console(&root_path, &folder_name, &media_dir_setting);
        let statuses = entries
            .into_iter()
            .map(|(entry_id, display_name)| {
                if let Some(media_dir) = media_dir.as_ref() {
                    // Multi-disc entries intentionally keep the `.m3u` suffix in
                    // their media stem; ordinary files use the filename stem.
                    let rom_stem = if display_name.to_ascii_lowercase().ends_with(".m3u") {
                        display_name
                    } else {
                        Path::new(&display_name)
                            .file_stem()
                            .and_then(|stem| stem.to_str())
                            .unwrap_or(&display_name)
                            .to_owned()
                    };
                    let found = state::collect_existing_assets(media_dir, &rom_stem);
                    let (status, has_miximage) = state::asset_availability(&found);
                    (entry_id, status, has_miximage)
                } else {
                    (entry_id, state::AssetStatus::None, false)
                }
            })
            .collect();
        let _ = tx.send(AppMessage::AssetStatusesLoaded {
            console_id,
            statuses,
        });
        ctx.request_repaint();
    });
}

/// Data collected on the UI thread for each entry to scrape.
struct ScrapeWorkItem {
    entry_id: Option<retro_junk_db::LibraryEntryId>,
    entry_name: String,
    rom_stem: String,
    filename: String,
    analysis_path: PathBuf,
    file_size: u64,
    /// Serial from ROM header analysis. Empty = none.
    serial: String,
    /// Serial adapted for `ScreenScraper` lookups. Empty = none.
    scraper_serial: String,
    /// Hash triple for the `ScreenScraper` hash tier (all-or-nothing).
    hashes: Option<retro_junk_scraper::lookup::RomHashes>,
    preferred_region: String,
    platform: Platform,
    archive_release_id: Option<retro_junk_archive::ArchiveReleaseId>,
    archived_assets: HashMap<AssetType, PathBuf>,
}

struct CandidateArchiveAsset {
    release_id: retro_junk_archive::ArchiveReleaseId,
    asset_type: AssetType,
    path: PathBuf,
}

struct MiximageWorkItem {
    entry_id: Option<retro_junk_db::LibraryEntryId>,
    entry_name: String,
    rom_stem: String,
    archive_release_id: Option<retro_junk_archive::ArchiveReleaseId>,
    archived_assets: HashMap<AssetType, PathBuf>,
}

/// Returns true for scrape errors that should abort the entire operation.
fn is_fatal_scrape_error(err: &ScrapeError) -> bool {
    matches!(
        err,
        ScrapeError::InvalidCredentials(_)
            | ScrapeError::QuotaExceeded { .. }
            | ScrapeError::ServerClosed(_)
    )
}

fn default_asset_selection(artwork_only: bool) -> retro_junk_scraper::AssetSelection {
    let mut selection = retro_junk_scraper::AssetSelection::default();
    if artwork_only {
        selection
            .types
            .retain(|asset_type| *asset_type != AssetType::Video);
    }
    selection
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
    let Some(media_directory) = state::asset_dir_for_console(
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
        move |op_id, cancel, tx| {
            let result = (|| {
                let snapshot = retro_junk_archive::scan_archive(&profile.archive_root)
                    .map_err(|error| error.to_string())?;
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
                retro_junk_lib::archive_assets::project_release_assets(
                    release,
                    &media_directory,
                    &stems,
                    &cancel,
                )
                .map_err(|error| error.to_string())
            })();
            let _ = tx.send(AppMessage::AssetProjectionComplete { op_id, result });
        },
    );
}

/// Scrape media from `ScreenScraper` for selected entries.
///
/// When `force_redownload` is true, re-downloads all media types and clears cached paths.
/// When false, only downloads missing types and leaves existing paths visible during scrape.
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

    // Borrow the analyzer for extract_scraper_serial (UI thread only)
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

    // Collect work items from selected entries
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
            let archive_release_id = archived_release
                .and_then(|release| release.summary.archive_release_id.parse().ok());
            let archived_assets = archived_release.map_or_else(HashMap::new, |release| {
                release
                    .archived_assets
                    .iter()
                    .filter_map(|asset| {
                        asset_type_from_archive_name(&asset.asset_type)
                            .map(|asset_type| (asset_type, PathBuf::from(&asset.absolute_path)))
                    })
                    .collect()
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
                    Some(retro_junk_scraper::lookup::RomHashes {
                        crc32: h.crc32.clone(),
                        md5: h.md5.clone()?,
                        sha1: h.sha1.clone()?,
                    })
                }),
                preferred_region,
                platform,
                archive_release_id,
                archived_assets,
            })
        })
        .collect();

    let selected_archive_ids = if whole_console {
        archive_rows
            .iter()
            .map(|release| release.summary.archive_release_id.clone())
            .collect::<Vec<_>>()
    } else if app.ui_state.selected_archive_releases.is_empty() {
        app.ui_state
            .focused_archive_release
            .iter()
            .cloned()
            .collect::<Vec<_>>()
    } else {
        app.ui_state
            .selected_archive_releases
            .iter()
            .cloned()
            .collect::<Vec<_>>()
    };
    let represented_archive_ids = work
        .iter()
        .filter_map(|item| item.archive_release_id.map(|id| id.to_string()))
        .collect::<std::collections::HashSet<_>>();
    for release in archive_rows.iter().filter(|release| {
        selected_archive_ids.contains(&release.summary.archive_release_id)
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
        let hashes =
            (!identity.crc32.is_empty() && !identity.md5.is_empty() && !identity.sha1.is_empty())
                .then(|| retro_junk_scraper::lookup::RomHashes {
                    crc32: identity.crc32.clone(),
                    md5: identity.md5.clone(),
                    sha1: identity.sha1.clone(),
                });
        let archived_assets = release
            .archived_assets
            .iter()
            .filter_map(|asset| {
                asset_type_from_archive_name(&asset.asset_type)
                    .map(|asset_type| (asset_type, PathBuf::from(&asset.absolute_path)))
            })
            .collect();
        work.push(ScrapeWorkItem {
            entry_id: None,
            entry_name: release.summary.title.clone(),
            rom_stem: format!("archive-{archive_release_id}"),
            filename: identity.filename.clone(),
            analysis_path: PathBuf::new(),
            file_size: identity.file_size,
            serial,
            scraper_serial,
            hashes,
            preferred_region: screenscraper_region(&release.summary.region).to_owned(),
            platform,
            archive_release_id: Some(archive_release_id),
            archived_assets,
        });
    }

    if work.is_empty() {
        return;
    }

    // When force-redownloading, clear cached asset_paths so the UI shows them as loading.
    // When scraping missing only, keep paths visible during the operation.
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
    let ctx = ctx.clone();
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
            let mut work = work;
            for item in &mut work {
                if cancel.load(Ordering::Relaxed) {
                    let _ = tx.send(AppMessage::OperationComplete { op_id });
                    return;
                }
                if !item.analysis_path.as_os_str().is_empty() {
                    item.file_size =
                        std::fs::metadata(&item.analysis_path).map_or(0, |metadata| metadata.len());
                }
            }
            let rt = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(e) => {
                    log::error!("Failed to create tokio runtime: {e}");
                    let _ = tx.send(AppMessage::ScrapeFatalError {
                        message: format!("Failed to create async runtime: {e}"),
                        op_id,
                    });
                    let _ = tx.send(AppMessage::OperationComplete { op_id });
                    return;
                }
            };

            rt.block_on(async {
                // Connect to ScreenScraper (cancel-aware — initial handshake can take ~90s if slow)
                let (client, _max_workers) =
                    match cancellable(retro_junk_scraper::create_client(None), &cancel).await {
                        None => {
                            let _ = tx.send(AppMessage::OperationComplete { op_id });
                            return;
                        }
                        Some(Ok(r)) => r,
                        Some(Err(e)) => {
                            log::error!("Failed to connect to ScreenScraper: {e}");
                            let _ = tx.send(AppMessage::ScrapeFatalError {
                                message: format!("ScreenScraper connection failed: {e}"),
                                op_id,
                            });
                            let _ = tx.send(AppMessage::OperationComplete { op_id });
                            return;
                        }
                    };

                let Some(system_id) = retro_junk_scraper::screenscraper_system_id(platform) else {
                    log::error!("No ScreenScraper system ID for {platform:?}");
                    let _ = tx.send(AppMessage::ScrapeFatalError {
                        message: format!("Platform {platform:?} is not supported by ScreenScraper"),
                        op_id,
                    });
                    let _ = tx.send(AppMessage::OperationComplete { op_id });
                    return;
                };

                let Some(media_dir) =
                    state::asset_dir_for_console(&root_path, &folder_name, &media_dir_setting)
                else {
                    log::error!("Cannot determine media directory for {folder_name}");
                    let _ = tx.send(AppMessage::ScrapeFatalError {
                        message: "Cannot determine media directory".to_string(),
                        op_id,
                    });
                    let _ = tx.send(AppMessage::OperationComplete { op_id });
                    return;
                };

                let selection = default_asset_selection(artwork_only);
                // Event channel for download_game_media (we don't consume events, just log)
                let (event_tx, _event_rx) = tokio::sync::mpsc::unbounded_channel();

                // Load miximage layout once for auto-generation after each entry
                let layout =
                    retro_junk_frontend::miximage_layout::MiximageLayout::load_or_create().ok();

                let mut downloaded_for_archive = Vec::new();
                let mut archive_changed = false;
                for (file_num, item) in work.iter().enumerate() {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }

                    let _ = tx.send(AppMessage::OperationProgress {
                        op_id,
                        current: file_num as u64,
                        total: work.len() as u64,
                    });
                    ctx.request_repaint();

                    // Build RomInfo from pre-collected data
                    let rom_info = retro_junk_scraper::lookup::RomInfo {
                        serial: item.serial.clone(),
                        scraper_serial: item.scraper_serial.clone(),
                        filename: item.filename.clone(),
                        file_size: item.file_size,
                        hashes: item.hashes.clone(),
                        platform: item.platform,
                        expects_serial: retro_junk_scraper::expects_serial(item.platform),
                    };

                    // Look up the game on ScreenScraper
                    let lookup_result = match cancellable(
                        retro_junk_scraper::lookup::lookup_game(&client, system_id, &rom_info),
                        &cancel,
                    )
                    .await
                    {
                        None => break,
                        Some(Ok(result)) => result,
                        Some(Err(e)) => {
                            if is_fatal_scrape_error(&e) {
                                log::error!("Fatal scrape error: {e}");
                                let _ = tx.send(AppMessage::ScrapeFatalError {
                                    message: e.to_string(),
                                    op_id,
                                });
                                let _ = tx.send(AppMessage::OperationComplete { op_id });
                                return;
                            }
                            log::warn!("Lookup failed for {}: {}", item.filename, e);
                            report_scrape_failure(
                                &tx,
                                &folder_name,
                                item,
                                e.to_string(),
                                op_id,
                            );
                            ctx.request_repaint();
                            continue;
                        }
                    };

                    let archived = item.archive_release_id.is_some() && archive_profile.is_some();
                    if archived {
                        for (asset_type, source) in &item.archived_assets {
                            if let Err(error) =
                                project_asset(source, &media_dir, &item.rom_stem, *asset_type)
                            {
                                log::warn!(
                                    "Could not project archived {asset_type} for {}: {error}",
                                    item.entry_name
                                );
                            }
                        }
                    }
                    let item_selection = if archived && !force_redownload {
                        retro_junk_scraper::AssetSelection {
                            types: selection
                                .types
                                .iter()
                                .copied()
                                .filter(|asset_type| {
                                    !item.archived_assets.contains_key(asset_type)
                                })
                                .collect(),
                        }
                    } else {
                        selection.clone()
                    };
                    let download_dir = if archived {
                        archive_profile
                            .as_ref()
                            .expect("archived scrape has profile")
                            .workspace_root
                            .join("archive-scrape")
                            .join(op_id.to_string())
                            .join(file_num.to_string())
                    } else {
                        media_dir.clone()
                    };
                    let source_urls = retro_junk_scraper::assets::asset_source_urls(
                        &lookup_result.game,
                        &item_selection,
                        &item.preferred_region,
                    );

                    // Download media
                    let downloaded = match cancellable(
                        retro_junk_scraper::assets::download_game_assets(
                            &client,
                            &retro_junk_scraper::assets::AssetDownloadRequest {
                                game: &lookup_result.game,
                                selection: &item_selection,
                                media_dir: &download_dir,
                                rom_stem: if archived {
                                    "original"
                                } else {
                                    &item.rom_stem
                                },
                                preferred_region: &item.preferred_region,
                                force_redownload,
                                index: file_num,
                                filename: &item.filename,
                                events: &event_tx,
                            },
                        ),
                        &cancel,
                    )
                    .await
                    {
                        None => break,
                        Some(Ok(media)) => media,
                        Some(Err(e)) => {
                            if is_fatal_scrape_error(&e) {
                                log::error!("Fatal scrape error during download: {e}");
                                let _ = tx.send(AppMessage::ScrapeFatalError {
                                    message: e.to_string(),
                                    op_id,
                                });
                                let _ = tx.send(AppMessage::OperationComplete { op_id });
                                return;
                            }
                            log::warn!("Media download failed for {}: {}", item.filename, e);
                            report_scrape_failure(
                                &tx,
                                &folder_name,
                                item,
                                e.to_string(),
                                op_id,
                            );
                            ctx.request_repaint();
                            continue;
                        }
                    };

                    if let Some(release_id) = item.archive_release_id
                        && archive_profile.is_some()
                    {
                        downloaded_for_archive.extend(downloaded.into_iter().map(
                            |(asset_type, path)| {
                                let source_url =
                                    source_urls.get(&asset_type).cloned().unwrap_or_default();
                                (file_num, release_id, asset_type, path, source_url)
                            },
                        ));
                    } else {
                        finish_playable_asset_update(
                            item,
                            &downloaded,
                            &media_dir,
                            layout.as_ref(),
                            &folder_name,
                            &tx,
                            &ctx,
                        );
                    }
                }

                if !downloaded_for_archive.is_empty()
                    && let Some(profile) = archive_profile.as_ref()
                {
                    let asset_names = downloaded_for_archive
                        .iter()
                        .map(|(_, _, asset_type, _, _)| asset_type.to_string())
                        .collect::<Vec<_>>();
                    let requests = downloaded_for_archive
                        .iter()
                        .zip(&asset_names)
                        .map(|((_, release_id, asset_type, path, source_url), asset_name)| {
                            retro_junk_archive::NewReleaseFile {
                                release_id: *release_id,
                                source_file: path,
                                category: if *asset_type == AssetType::Video {
                                    retro_junk_archive::ReleaseFileCategory::Video
                                } else {
                                    retro_junk_archive::ReleaseFileCategory::Artwork
                                },
                                asset_type: asset_name,
                                source: "screenscraper",
                                source_url,
                                caption: "",
                            }
                        })
                        .collect::<Vec<_>>();
                    let archive_result =
                        retro_junk_archive::ArchiveLock::acquire(&profile.archive_root)
                            .map_err(|error| error.to_string())
                            .and_then(|_archive_lock| {
                                retro_junk_archive::add_release_files(
                                    &profile.archive_root,
                                    &requests,
                                    &cancel,
                                )
                                .map_err(|error| error.to_string())
                            });
                    match archive_result {
                        Ok(results) => {
                            archive_changed = results.iter().any(|result| result.added);
                            for (file_num, _, asset_type, path, _) in &downloaded_for_archive {
                                let item = &work[*file_num];
                                if let Err(error) =
                                    project_asset(path, &media_dir, &item.rom_stem, *asset_type)
                                {
                                    log::warn!(
                                        "Could not project newly archived {asset_type} for {}: {error}",
                                        item.entry_name
                                    );
                                }
                            }
                        }
                        Err(error) => {
                            let _ = tx.send(AppMessage::ScrapeFatalError {
                                message: format!(
                                    "Downloaded media could not be added to the archive: {error}"
                                ),
                                op_id,
                            });
                        }
                    }
                }
                for item in work.iter().filter(|item| {
                    item.archive_release_id.is_some() && archive_profile.is_some()
                }) {
                    finish_playable_asset_update(
                        item,
                        &HashMap::new(),
                        &media_dir,
                        layout.as_ref(),
                        &folder_name,
                        &tx,
                        &ctx,
                    );
                }
                if archive_changed {
                    let _ = tx.send(AppMessage::ArchiveAssetsChanged);
                }
                if let Some(profile) = archive_profile.as_ref() {
                    let scratch = profile
                        .workspace_root
                        .join("archive-scrape")
                        .join(op_id.to_string());
                    if let Err(error) = std::fs::remove_dir_all(&scratch)
                        && error.kind() != std::io::ErrorKind::NotFound
                    {
                        log::warn!(
                            "Could not remove completed scrape workspace {}: {error}",
                            scratch.display()
                        );
                    }
                }

                let _ = tx.send(AppMessage::OperationComplete { op_id });
                ctx.request_repaint();
            });
        },
    );
}

fn asset_type_from_archive_name(value: &str) -> Option<AssetType> {
    match value.trim().to_ascii_lowercase().as_str() {
        "cover" | "box-front" => Some(AssetType::Cover),
        "3d box" | "cover3d" | "cover-3d" => Some(AssetType::Cover3D),
        "screenshot" => Some(AssetType::Screenshot),
        "title screen" | "titlescreen" => Some(AssetType::TitleScreen),
        "marquee" => Some(AssetType::Marquee),
        "video" => Some(AssetType::Video),
        "fanart" => Some(AssetType::Fanart),
        "physical media" | "physicalmedia" => Some(AssetType::PhysicalMedia),
        "miximage" => Some(AssetType::Miximage),
        _ => None,
    }
}

fn screenscraper_region(region: &str) -> &'static str {
    match region.trim().to_ascii_lowercase().as_str() {
        "japan" | "jpn" | "jp" => "jp",
        "europe" | "eur" => "eu",
        "world" | "wld" => "wor",
        "australia" | "aus" => "au",
        "korea" | "kor" => "kr",
        "china" | "chn" => "cn",
        "brazil" | "bra" => "br",
        _ => "us",
    }
}

fn report_scrape_failure(
    tx: &crate::state::AppMessageSender,
    folder_name: &str,
    item: &ScrapeWorkItem,
    error: String,
    op_id: u64,
) {
    if let Some(entry_id) = item.entry_id {
        let _ = tx.send(AppMessage::ScrapeEntryFailed {
            folder_name: folder_name.to_owned(),
            entry_id,
            entry_name: item.entry_name.clone(),
            error,
        });
    } else {
        let _ = tx.send(AppMessage::ScrapeFatalError {
            message: format!("{}: {error}", item.entry_name),
            op_id,
        });
    }
}

fn finish_playable_asset_update(
    item: &ScrapeWorkItem,
    downloaded: &HashMap<AssetType, PathBuf>,
    media_dir: &Path,
    layout: Option<&retro_junk_frontend::miximage_layout::MiximageLayout>,
    folder_name: &str,
    tx: &crate::state::AppMessageSender,
    ctx: &egui::Context,
) {
    let Some(entry_id) = item.entry_id else {
        return;
    };
    for path in downloaded.values() {
        ctx.forget_image(&state::asset_image_uri(path));
    }
    let final_media = if let Some(layout) = layout {
        generate_miximage_for_entry(media_dir, &item.rom_stem, layout, ctx)
    } else {
        state::collect_existing_assets(media_dir, &item.rom_stem)
    };
    let _ = tx.send(AppMessage::AssetsLoaded {
        folder_name: folder_name.to_owned(),
        entry_id,
        assets: final_media,
    });
    ctx.request_repaint();
}

fn project_asset(
    source: &Path,
    media_dir: &Path,
    rom_stem: &str,
    asset_type: AssetType,
) -> std::io::Result<PathBuf> {
    let extension = source
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_else(|| asset_type.default_extension());
    let directory = media_dir.join(retro_junk_scraper::assets::asset_subdir(asset_type));
    std::fs::create_dir_all(&directory)?;
    let destination = directory.join(format!("{rom_stem}.{extension}"));
    let temporary = directory.join(format!(
        ".{rom_stem}.{}.tmp",
        retro_junk_archive::SupportingFileId::new()
    ));
    std::fs::copy(source, &temporary)?;
    if destination.exists() {
        let backup = directory.join(format!(
            ".{rom_stem}.{}.backup",
            retro_junk_archive::SupportingFileId::new()
        ));
        std::fs::rename(&destination, &backup)?;
        if let Err(error) = std::fs::rename(&temporary, &destination) {
            let _ = std::fs::rename(&backup, &destination);
            let _ = std::fs::remove_file(&temporary);
            return Err(error);
        }
        std::fs::remove_file(backup)?;
    } else {
        std::fs::rename(&temporary, &destination)?;
    }
    Ok(destination)
}

/// Adopt already-downloaded frontend media for releases present in a freshly
/// scanned archive. The caller owns the archive write lock.
pub fn adopt_playable_artwork(
    connection: &retro_junk_db::Connection,
    snapshot: &retro_junk_archive::ArchiveIndexSnapshot,
    profile: &retro_junk_archive::CollectionProfile,
    media_dir_setting: &str,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<usize, String> {
    let archive_releases = snapshot
        .releases
        .iter()
        .filter_map(|release| {
            let catalog_id = release.manifest.catalog_binding.catalog_release_id.trim();
            (!catalog_id.is_empty())
                .then_some((catalog_id.to_owned(), release.manifest.archive_release_id))
        })
        .collect::<HashMap<_, _>>();
    if archive_releases.is_empty() {
        return Ok(0);
    }
    let selection = retro_junk_scraper::AssetSelection::all();
    let mut assets = Vec::new();
    for candidate in
        retro_junk_db::playable_artwork_candidates(connection).map_err(|error| error.to_string())?
    {
        let Some(&release_id) = archive_releases.get(&candidate.catalog_release_id) else {
            continue;
        };
        let Ok(game_entry) =
            serde_json::from_str::<retro_junk_lib::scanner::GameEntry>(&candidate.game_entry_json)
        else {
            continue;
        };
        let Some(media_dir) = state::asset_dir_for_console(
            &profile.playable_root,
            &candidate.folder_name,
            media_dir_setting,
        ) else {
            continue;
        };
        assets.extend(
            retro_junk_scraper::assets::collect_existing_assets(
                &selection,
                &media_dir,
                game_entry.rom_stem(),
            )
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

/// Re-generate miximages from existing on-disk media for selected entries.
///
/// Composites miximages using already-scraped component images (screenshot, box art, etc.)
/// without contacting `ScreenScraper`. Uses a sync background thread (no tokio needed).
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

    let selected_archive_ids = if app.ui_state.selected_archive_releases.is_empty() {
        app.ui_state
            .focused_archive_release
            .iter()
            .cloned()
            .collect::<Vec<_>>()
    } else {
        app.ui_state
            .selected_archive_releases
            .iter()
            .cloned()
            .collect::<Vec<_>>()
    };
    let represented_archive_ids = work
        .iter()
        .filter_map(|item| item.archive_release_id.map(|id| id.to_string()))
        .collect::<std::collections::HashSet<_>>();
    for release in archive_rows.iter().filter(|release| {
        selected_archive_ids.contains(&release.summary.archive_release_id)
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
    let ctx = ctx.clone();
    let description = format!("Re-generating miximages ({} entries)", work.len());

    spawn_background_op(
        app,
        description,
        OperationKind::Other,
        folder_name.clone(),
        ProgressDisplay::Count,
        move |op_id, cancel, tx| {
            let Some(media_dir) =
                state::asset_dir_for_console(&root_path, &folder_name, &media_dir_setting)
            else {
                log::error!("Cannot determine media directory for {folder_name}");
                let _ = tx.send(AppMessage::OperationComplete { op_id });
                return;
            };

            let layout =
                match retro_junk_frontend::miximage_layout::MiximageLayout::load_or_create() {
                    Ok(l) => l,
                    Err(e) => {
                        log::error!("Failed to load miximage layout: {e}");
                        let _ = tx.send(AppMessage::OperationComplete { op_id });
                        return;
                    }
                };

            let scratch_root = archive_profile.as_ref().map(|profile| {
                profile
                    .workspace_root
                    .join("miximages")
                    .join(op_id.to_string())
            });
            let mut generated = Vec::new();
            let mut failures = Vec::new();
            for (file_num, item) in work.iter().enumerate() {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }

                let _ = tx.send(AppMessage::OperationProgress {
                    op_id,
                    current: file_num as u64,
                    total: work.len() as u64,
                });
                ctx.request_repaint();

                let target = if item.entry_id.is_some() {
                    media_dir.clone()
                } else if let Some(scratch_root) = scratch_root.as_ref() {
                    scratch_root.join(file_num.to_string())
                } else {
                    failures.push(format!(
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
                        ctx.forget_image(&state::asset_image_uri(&output_path));
                        generated.push((item.archive_release_id, output_path));
                    }
                    Err(error) => failures.push(format!("{}: {error}", item.entry_name)),
                }
                let updated_media = state::collect_existing_assets(&target, &item.rom_stem);

                // Invalidate any currently displayed component images without
                // loading bulk-operation results into memory.
                for (mt, path) in &updated_media {
                    if *mt != AssetType::Miximage {
                        let uri = state::asset_image_uri(path);
                        ctx.forget_image(&uri);
                    }
                }

                if let Some(entry_id) = item.entry_id {
                    let _ = tx.send(AppMessage::AssetsLoaded {
                        folder_name: folder_name.clone(),
                        entry_id,
                        assets: updated_media,
                    });
                }
                ctx.request_repaint();
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
                match retro_junk_archive::ArchiveLock::acquire(&profile.archive_root)
                    .map_err(|error| error.to_string())
                    .and_then(|_lock| {
                        retro_junk_archive::add_release_files(
                            &profile.archive_root,
                            &requests,
                            &cancel,
                        )
                        .map_err(|error| error.to_string())
                    }) {
                    Ok(results) => {
                        if results.iter().any(|result| result.added) {
                            let _ = tx.send(AppMessage::ArchiveAssetsChanged);
                        }
                    }
                    Err(error) => failures.push(format!(
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
            let _ = tx.send(AppMessage::MiximageComplete {
                generated: generated.len(),
                failures,
            });
            let _ = tx.send(AppMessage::OperationComplete { op_id });
            ctx.request_repaint();
        },
    );
}

fn archived_asset_paths(
    release: &retro_junk_db::ArchivedLibraryListItem,
) -> HashMap<AssetType, PathBuf> {
    release
        .archived_assets
        .iter()
        .filter_map(|asset| {
            asset_type_from_archive_name(&asset.asset_type)
                .map(|asset_type| (asset_type, PathBuf::from(&asset.absolute_path)))
        })
        .collect()
}

fn generate_miximage_with_archived_assets(
    archived_assets: &HashMap<AssetType, PathBuf>,
    media_dir: &Path,
    rom_stem: &str,
    layout: &retro_junk_frontend::miximage_layout::MiximageLayout,
) -> Result<PathBuf, String> {
    for (asset_type, source) in archived_assets {
        project_asset(source, media_dir, rom_stem, *asset_type)
            .map_err(|error| format!("could not restore {asset_type}: {error}"))?;
    }
    retro_junk_lib::archive_assets::generate_frontend_miximage(media_dir, rom_stem, layout)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "a screenshot is required before a miximage can be generated".to_owned())
}

/// Generate a miximage for a single entry from its existing on-disk media.
///
/// Returns the updated media map (existing media + the new miximage) if generation
/// succeeded, or the existing media map if it was skipped/failed.
fn generate_miximage_for_entry(
    media_dir: &Path,
    rom_stem: &str,
    layout: &retro_junk_frontend::miximage_layout::MiximageLayout,
    ctx: &egui::Context,
) -> HashMap<AssetType, PathBuf> {
    let existing = state::collect_existing_assets(media_dir, rom_stem);
    let miximage_dir = media_dir.join("miximages");
    let output_path = miximage_dir.join(format!("{rom_stem}.png"));

    match retro_junk_frontend::miximage::generate_miximage(&existing, &output_path, layout) {
        Ok(generated) => {
            if generated {
                let uri = state::asset_image_uri(&output_path);
                ctx.forget_image(&uri);
            }
        }
        Err(e) => {
            log::warn!("Failed to generate miximage for {rom_stem}: {e}");
        }
    }

    // Re-collect to pick up the new/updated miximage
    state::collect_existing_assets(media_dir, rom_stem)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artwork_only_selection_excludes_video_but_keeps_images() {
        let selection = default_asset_selection(true);
        assert!(!selection.types.contains(&AssetType::Video));
        assert!(selection.types.contains(&AssetType::Cover));
        assert!(selection.types.contains(&AssetType::Screenshot));
    }
    use std::sync::atomic::AtomicBool;

    #[test]
    fn dump_ingest_adopts_existing_playable_artwork_once() {
        let temp = tempfile::tempdir().unwrap();
        let archive_root = temp.path().join("archive");
        let root_manifest = retro_junk_archive::ArchiveRootManifest::new("Artwork");
        retro_junk_archive::initialize_archive(&archive_root, &root_manifest).unwrap();
        let playable_root = temp.path().join("roms");
        let rom = playable_root.join("nes/game.nes");
        std::fs::create_dir_all(rom.parent().unwrap()).unwrap();
        std::fs::write(&rom, b"rom").unwrap();
        retro_junk_archive::ingest_new_carrier_dump(
            &archive_root,
            &rom,
            retro_junk_archive::NewCarrierDump {
                platform_id: "nes".to_owned(),
                title: "Game".to_owned(),
                region: "usa".to_owned(),
                revision: String::new(),
                variant: String::new(),
                owner_id: "default".to_owned(),
                physical_copy_label: String::new(),
                serial: String::new(),
                sequence_number: 0,
                carrier_label: String::new(),
                carrier_kind: retro_junk_archive::CarrierKind::Cartridge,
                format: retro_junk_archive::RepresentationFormat::Rom,
                catalog_binding: retro_junk_archive::CatalogBinding {
                    catalog_release_id: "release-game".to_owned(),
                    catalog_media_id: "media-game".to_owned(),
                    ..Default::default()
                },
                source_package: retro_junk_archive::SourcePackageRecord::default(),
                expected_files: Vec::new(),
                physical_copy_id: None,
            },
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();

        let media_root = temp.path().join("media");
        let cover = media_root.join("nes/covers/game.png");
        std::fs::create_dir_all(cover.parent().unwrap()).unwrap();
        std::fs::write(&cover, b"existing cover").unwrap();
        let connection = retro_junk_db::open_memory().unwrap();
        connection.execute("INSERT INTO platforms(id,display_name,short_name,manufacturer,generation,media_type,release_year,description,core_platform) VALUES('nes','NES','NES','Nintendo',3,'cartridge',1983,'','Nes')", []).unwrap();
        connection
            .execute(
                "INSERT INTO works(id,canonical_name) VALUES('work-game','Game')",
                [],
            )
            .unwrap();
        connection.execute("INSERT INTO releases(id,work_id,platform_id,region,title) VALUES('release-game','work-game','nes','usa','Game')", []).unwrap();
        connection.execute("INSERT INTO media(id,release_id,dat_source) VALUES('media-game','release-game','no-intro')", []).unwrap();
        connection
            .execute(
                "INSERT INTO library_roots(id,root_path) VALUES(1,?1)",
                [playable_root.to_string_lossy().as_ref()],
            )
            .unwrap();
        connection.execute(
            "INSERT INTO library_consoles(id,root_id,platform,folder_name,folder_path,fingerprint_hash,scan_state) VALUES(1,1,'Nes','nes',?1,'fp','ready')",
            [playable_root.join("nes").to_string_lossy().as_ref()],
        ).unwrap();
        let game_entry_json =
            serde_json::to_string(&retro_junk_lib::scanner::GameEntry::SingleFile(rom)).unwrap();
        connection.execute(
            "INSERT INTO library_entries(id,console_id,entry_key,display_name,game_entry_json) VALUES(1,1,'file:game.nes','game.nes',?1)",
            [&game_entry_json],
        ).unwrap();
        connection.execute(
            "INSERT INTO library_entry_media_bindings(library_entry_id,catalog_media_id,match_method) VALUES(1,'media-game','test')",
            [],
        ).unwrap();

        let profile = retro_junk_archive::CollectionProfile {
            profile_id: root_manifest.profile_id,
            display_name: "Artwork".to_owned(),
            archive_root: archive_root.clone(),
            playable_root,
            workspace_root: temp.path().join("workspace"),
            network_mode: true,
            platform_defaults: Vec::new(),
        };
        let snapshot = retro_junk_archive::scan_archive(&archive_root).unwrap();
        let cancel = AtomicBool::new(false);
        let media_setting = media_root.to_string_lossy();
        let _lock = retro_junk_archive::ArchiveLock::acquire(&archive_root).unwrap();
        assert_eq!(
            adopt_playable_artwork(&connection, &snapshot, &profile, &media_setting, &cancel,)
                .unwrap(),
            1
        );
        let rescanned = retro_junk_archive::scan_archive(&archive_root).unwrap();
        assert_eq!(rescanned.releases[0].supporting_files.len(), 1);
        assert_eq!(
            std::fs::read(
                rescanned.releases[0].supporting_files[0]
                    .directory
                    .join(&rescanned.releases[0].supporting_files[0].manifest.file.path)
            )
            .unwrap(),
            b"existing cover"
        );
        assert_eq!(
            adopt_playable_artwork(&connection, &rescanned, &profile, &media_setting, &cancel,)
                .unwrap(),
            0
        );
    }

    #[test]
    fn miximage_generation_restores_archived_components_to_playable_media() {
        let temp = tempfile::tempdir().unwrap();
        let screenshot = temp.path().join("archived-screenshot.png");
        image::RgbaImage::from_pixel(64, 64, image::Rgba([20, 40, 60, 255]))
            .save(&screenshot)
            .unwrap();
        let media_dir = temp.path().join("roms-media/nes");
        let archived_assets = HashMap::from([(AssetType::Screenshot, screenshot)]);

        let output = generate_miximage_with_archived_assets(
            &archived_assets,
            &media_dir,
            "Game",
            &retro_junk_frontend::miximage_layout::MiximageLayout::default(),
        )
        .unwrap();

        assert!(media_dir.join("screenshots/Game.png").is_file());
        assert_eq!(output, media_dir.join("miximages/Game.png"));
        assert!(output.is_file());
    }
}
