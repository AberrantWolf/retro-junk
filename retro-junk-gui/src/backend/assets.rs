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
    entry_id: retro_junk_db::LibraryEntryId,
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

/// Re-scrape media (force redownload of all media types).
pub fn rescrape_media_for_selection(
    app: &mut RetroJunkApp,
    console_idx: usize,
    ctx: &egui::Context,
) {
    scrape_media_for_selection(app, console_idx, ctx, true);
}

/// Scrape only missing media (skip types that already exist on disk).
pub fn scrape_missing_media_for_selection(
    app: &mut RetroJunkApp,
    console_idx: usize,
    ctx: &egui::Context,
) {
    scrape_media_for_selection(app, console_idx, ctx, false);
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
) {
    let console = &app.browser.consoles[console_idx];
    let platform = console.platform;
    let folder_name = console.folder_name.clone();

    let Some(root_path) = app.root_path.clone() else {
        return;
    };

    // Borrow the analyzer for extract_scraper_serial (UI thread only)
    let analyzer = app.context.get_by_platform(platform);

    // Collect work items from selected entries
    let work: Vec<ScrapeWorkItem> = app
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

            Some(ScrapeWorkItem {
                entry_id: i,
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
            })
        })
        .collect();

    if work.is_empty() {
        return;
    }

    // When force-redownloading, clear cached asset_paths so the UI shows them as loading.
    // When scraping missing only, keep paths visible during the operation.
    if force_redownload {
        for item in &work {
            if let Some(entry) = app.browser.consoles[console_idx].entry_by_id_mut(item.entry_id) {
                entry.asset_paths = None;
            }
        }
    }

    let media_dir_setting = app.settings.general.assets_dir.clone();
    let ctx = ctx.clone();
    let verb = if force_redownload {
        "Scraping media"
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
                item.file_size =
                    std::fs::metadata(&item.analysis_path).map_or(0, |metadata| metadata.len());
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

                let selection = retro_junk_scraper::AssetSelection::default();
                // Event channel for download_game_media (we don't consume events, just log)
                let (event_tx, _event_rx) = tokio::sync::mpsc::unbounded_channel();

                // Load miximage layout once for auto-generation after each entry
                let layout =
                    retro_junk_frontend::miximage_layout::MiximageLayout::load_or_create().ok();

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
                            let _ = tx.send(AppMessage::ScrapeEntryFailed {
                                folder_name: folder_name.clone(),
                                entry_id: item.entry_id,
                                entry_name: item.entry_name.clone(),
                                error: e.to_string(),
                            });
                            ctx.request_repaint();
                            continue;
                        }
                    };

                    // Download media
                    let downloaded = match cancellable(
                        retro_junk_scraper::assets::download_game_assets(
                            &client,
                            &retro_junk_scraper::assets::AssetDownloadRequest {
                                game: &lookup_result.game,
                                selection: &selection,
                                media_dir: &media_dir,
                                rom_stem: &item.rom_stem,
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
                            let _ = tx.send(AppMessage::ScrapeEntryFailed {
                                folder_name: folder_name.clone(),
                                entry_id: item.entry_id,
                                entry_name: item.entry_name.clone(),
                                error: e.to_string(),
                            });
                            ctx.request_repaint();
                            continue;
                        }
                    };

                    // Invalidate an image if it is currently displayed. Do not
                    // register bytes for bulk scrape results.
                    for path in downloaded.values() {
                        let uri = state::asset_image_uri(path);
                        ctx.forget_image(&uri);
                    }

                    // Auto-generate miximage from the freshly downloaded media
                    let final_media = if let Some(ref layout) = layout {
                        generate_miximage_for_entry(&media_dir, &item.rom_stem, layout, &ctx)
                    } else {
                        downloaded
                    };

                    let _ = tx.send(AppMessage::AssetsLoaded {
                        folder_name: folder_name.clone(),
                        entry_id: item.entry_id,
                        assets: final_media,
                    });
                    ctx.request_repaint();
                }

                let _ = tx.send(AppMessage::OperationComplete { op_id });
                ctx.request_repaint();
            });
        },
    );
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

    // Collect (entry_name, rom_stem) for selected entries
    let work: Vec<(retro_junk_db::LibraryEntryId, String, String)> = app
        .ui_state
        .selected_entries
        .iter()
        .copied()
        .filter_map(|i| {
            let entry = console.entry_by_id(i)?;
            Some((
                i,
                entry.game_entry.display_name().to_string(),
                entry.game_entry.rom_stem().to_string(),
            ))
        })
        .collect();

    if work.is_empty() {
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

            for (file_num, (entry_id, _entry_name, rom_stem)) in work.iter().enumerate() {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }

                let _ = tx.send(AppMessage::OperationProgress {
                    op_id,
                    current: file_num as u64,
                    total: work.len() as u64,
                });
                ctx.request_repaint();

                let updated_media =
                    generate_miximage_for_entry(&media_dir, rom_stem, &layout, &ctx);

                // Invalidate any currently displayed component images without
                // loading bulk-operation results into memory.
                for (mt, path) in &updated_media {
                    if *mt != AssetType::Miximage {
                        let uri = state::asset_image_uri(path);
                        ctx.forget_image(&uri);
                    }
                }

                let _ = tx.send(AppMessage::AssetsLoaded {
                    folder_name: folder_name.clone(),
                    entry_id: *entry_id,
                    assets: updated_media,
                });
                ctx.request_repaint();
            }

            let _ = tx.send(AppMessage::OperationComplete { op_id });
            ctx.request_repaint();
        },
    );
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
