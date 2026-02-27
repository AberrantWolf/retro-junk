use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc;

use retro_junk_core::Platform;
use retro_junk_lib::async_util::cancellable;
use retro_junk_scraper::ScrapeError;

use crate::app::RetroJunkApp;
use crate::backend::worker::spawn_background_op;
use crate::state::{self, AppMessage};

/// Load media files for an entry on a background thread.
///
/// Discovers media files on disk and registers their bytes with egui,
/// then sends a `MediaLoaded` message to update the entry's `media_paths`.
pub fn load_media_for_entry(
    tx: mpsc::Sender<AppMessage>,
    ctx: egui::Context,
    root_path: PathBuf,
    folder_name: String,
    entry_index: usize,
    rom_stem: String,
    media_dir_setting: String,
) {
    std::thread::spawn(move || {
        let media_dir =
            match state::media_dir_for_console(&root_path, &folder_name, &media_dir_setting) {
                Some(d) => d,
                None => {
                    let _ = tx.send(AppMessage::MediaLoaded {
                        folder_name,
                        entry_index,
                        media: HashMap::new(),
                    });
                    ctx.request_repaint();
                    return;
                }
            };

        let found = state::collect_existing_media(&media_dir, &rom_stem);

        // Register image bytes with egui before sending the message,
        // so they're available by the time the UI renders.
        for path in found.values() {
            let uri = format!("bytes://media/{}", path.display());
            if let Ok(bytes) = std::fs::read(path) {
                ctx.include_bytes(uri, bytes);
            }
        }

        let _ = tx.send(AppMessage::MediaLoaded {
            folder_name,
            entry_index,
            media: found,
        });
        ctx.request_repaint();
    });
}

/// Data collected on the UI thread for each entry to scrape.
struct ScrapeWorkItem {
    entry_index: usize,
    rom_stem: String,
    filename: String,
    file_size: u64,
    serial: Option<String>,
    scraper_serial: Option<String>,
    crc32: Option<String>,
    md5: Option<String>,
    sha1: Option<String>,
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

/// Scrape media from ScreenScraper for selected entries.
///
/// Collects work items on the UI thread, then spawns a background thread
/// that creates a tokio runtime and calls the real ScreenScraper API.
pub fn rescrape_media_for_selection(
    app: &mut RetroJunkApp,
    console_idx: usize,
    ctx: &egui::Context,
) {
    let console = &app.library.consoles[console_idx];
    let platform = console.platform;
    let folder_name = console.folder_name.clone();

    let root_path = match app.root_path.clone() {
        Some(p) => p,
        None => return,
    };

    // Borrow the analyzer for extract_scraper_serial (UI thread only)
    let analyzer = app.context.get_by_platform(platform);

    // Collect work items from selected entries
    let work: Vec<ScrapeWorkItem> = app
        .selected_entries
        .iter()
        .copied()
        .filter_map(|i| {
            let entry = console.entries.get(i)?;
            let analysis_path = entry.game_entry.analysis_path();
            let filename = analysis_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            let file_size = std::fs::metadata(analysis_path)
                .map(|m| m.len())
                .unwrap_or(0);

            let serial = entry
                .identification
                .as_ref()
                .and_then(|id| id.serial_number.clone());
            let scraper_serial = serial
                .as_deref()
                .and_then(|s| analyzer.and_then(|a| a.analyzer.extract_scraper_serial(s)));

            let regions = entry.effective_regions();
            let preferred_region = regions
                .first()
                .map(|r| retro_junk_scraper::region_to_ss_code(r).to_string())
                .unwrap_or_else(|| "us".to_string());

            Some(ScrapeWorkItem {
                entry_index: i,
                rom_stem: entry.game_entry.rom_stem().to_string(),
                filename,
                file_size,
                serial,
                scraper_serial,
                crc32: entry.hashes.as_ref().map(|h| h.crc32.clone()),
                md5: entry.hashes.as_ref().and_then(|h| h.md5.clone()),
                sha1: entry.hashes.as_ref().and_then(|h| h.sha1.clone()),
                preferred_region,
                platform,
            })
        })
        .collect();

    if work.is_empty() {
        return;
    }

    // Clear cached media_paths for selected entries so the UI shows them as loading
    for item in &work {
        if let Some(entry) = app.library.consoles[console_idx]
            .entries
            .get_mut(item.entry_index)
        {
            entry.media_paths = None;
        }
    }

    let media_dir_setting = app.settings.general.media_dir.clone();
    let ctx = ctx.clone();
    let description = format!("Scraping media ({} entries)", work.len());

    spawn_background_op(app, description, move |op_id, cancel, tx| {
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => {
                log::error!("Failed to create tokio runtime: {}", e);
                let _ = tx.send(AppMessage::ScrapeFatalError {
                    message: format!("Failed to create async runtime: {}", e),
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
                        log::error!("Failed to connect to ScreenScraper: {}", e);
                        let _ = tx.send(AppMessage::ScrapeFatalError {
                            message: format!("ScreenScraper connection failed: {}", e),
                            op_id,
                        });
                        let _ = tx.send(AppMessage::OperationComplete { op_id });
                        return;
                    }
                };

            let system_id = match retro_junk_scraper::screenscraper_system_id(platform) {
                Some(id) => id,
                None => {
                    log::error!("No ScreenScraper system ID for {:?}", platform);
                    let _ = tx.send(AppMessage::ScrapeFatalError {
                        message: format!(
                            "Platform {:?} is not supported by ScreenScraper",
                            platform
                        ),
                        op_id,
                    });
                    let _ = tx.send(AppMessage::OperationComplete { op_id });
                    return;
                }
            };

            let media_dir =
                match state::media_dir_for_console(&root_path, &folder_name, &media_dir_setting) {
                    Some(d) => d,
                    None => {
                        log::error!("Cannot determine media directory for {}", folder_name);
                        let _ = tx.send(AppMessage::ScrapeFatalError {
                            message: "Cannot determine media directory".to_string(),
                            op_id,
                        });
                        let _ = tx.send(AppMessage::OperationComplete { op_id });
                        return;
                    }
                };

            let selection = retro_junk_scraper::MediaSelection::default();
            // Event channel for download_game_media (we don't consume events, just log)
            let (event_tx, _event_rx) = tokio::sync::mpsc::unbounded_channel();

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
                    crc32: item.crc32.clone(),
                    md5: item.md5.clone(),
                    sha1: item.sha1.clone(),
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
                            log::error!("Fatal scrape error: {}", e);
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
                            entry_index: item.entry_index,
                            error: e.to_string(),
                        });
                        ctx.request_repaint();
                        continue;
                    }
                };

                // Download media
                let downloaded = match cancellable(
                    retro_junk_scraper::media::download_game_media(
                        &client,
                        &lookup_result.game,
                        &selection,
                        &media_dir,
                        &item.rom_stem,
                        &item.preferred_region,
                        true, // force redownload
                        file_num,
                        &item.filename,
                        &event_tx,
                    ),
                    &cancel,
                )
                .await
                {
                    None => break,
                    Some(Ok(media)) => media,
                    Some(Err(e)) => {
                        if is_fatal_scrape_error(&e) {
                            log::error!("Fatal scrape error during download: {}", e);
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
                            entry_index: item.entry_index,
                            error: e.to_string(),
                        });
                        ctx.request_repaint();
                        continue;
                    }
                };

                // Register downloaded images with egui and invalidate old ones
                for path in downloaded.values() {
                    let uri = format!("bytes://media/{}", path.display());
                    ctx.forget_image(&uri);
                    if let Ok(bytes) = std::fs::read(path) {
                        ctx.include_bytes(uri, bytes);
                    }
                }

                let _ = tx.send(AppMessage::MediaLoaded {
                    folder_name: folder_name.clone(),
                    entry_index: item.entry_index,
                    media: downloaded,
                });
                ctx.request_repaint();
            }

            let _ = tx.send(AppMessage::OperationComplete { op_id });
            ctx.request_repaint();
        });
    });
}

/// Re-generate miximages from existing on-disk media for selected entries.
///
/// Composites miximages using already-scraped component images (screenshot, box art, etc.)
/// without contacting ScreenScraper. Uses a sync background thread (no tokio needed).
pub fn regenerate_miximages_for_selection(
    app: &mut RetroJunkApp,
    console_idx: usize,
    ctx: &egui::Context,
) {
    let console = &app.library.consoles[console_idx];
    let folder_name = console.folder_name.clone();

    let root_path = match app.root_path.clone() {
        Some(p) => p,
        None => return,
    };

    // Collect (entry_index, rom_stem) for selected entries
    let work: Vec<(usize, String)> = app
        .selected_entries
        .iter()
        .copied()
        .filter_map(|i| {
            let entry = console.entries.get(i)?;
            Some((i, entry.game_entry.rom_stem().to_string()))
        })
        .collect();

    if work.is_empty() {
        return;
    }

    let media_dir_setting = app.settings.general.media_dir.clone();
    let ctx = ctx.clone();
    let description = format!("Re-generating miximages ({} entries)", work.len());

    spawn_background_op(app, description, move |op_id, cancel, tx| {
        let media_dir =
            match state::media_dir_for_console(&root_path, &folder_name, &media_dir_setting) {
                Some(d) => d,
                None => {
                    log::error!("Cannot determine media directory for {}", folder_name);
                    let _ = tx.send(AppMessage::OperationComplete { op_id });
                    return;
                }
            };

        let layout = match retro_junk_frontend::miximage_layout::MiximageLayout::load_or_create() {
            Ok(l) => l,
            Err(e) => {
                log::error!("Failed to load miximage layout: {}", e);
                let _ = tx.send(AppMessage::OperationComplete { op_id });
                return;
            }
        };

        let miximage_dir = media_dir.join("miximages");

        for (file_num, (entry_index, rom_stem)) in work.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                break;
            }

            let _ = tx.send(AppMessage::OperationProgress {
                op_id,
                current: file_num as u64,
                total: work.len() as u64,
            });
            ctx.request_repaint();

            let existing = state::collect_existing_media(&media_dir, rom_stem);
            let output_path = miximage_dir.join(format!("{}.png", rom_stem));

            match retro_junk_frontend::miximage::generate_miximage(&existing, &output_path, &layout)
            {
                Ok(generated) => {
                    if generated {
                        // Invalidate old egui texture and register the new one
                        let uri = format!("bytes://media/{}", output_path.display());
                        ctx.forget_image(&uri);
                        if let Ok(bytes) = std::fs::read(&output_path) {
                            ctx.include_bytes(uri, bytes);
                        }
                    }
                }
                Err(e) => {
                    log::warn!("Failed to generate miximage for {}: {}", rom_stem, e);
                }
            }

            // Re-collect media to pick up the new/updated miximage
            let updated_media = state::collect_existing_media(&media_dir, rom_stem);
            for path in updated_media.values() {
                let uri = format!("bytes://media/{}", path.display());
                if !matches!(
                    path.parent()
                        .and_then(|p| p.file_name())
                        .and_then(|n| n.to_str()),
                    Some("miximages")
                ) {
                    // Only register non-miximage paths (miximage already handled above)
                    if let Ok(bytes) = std::fs::read(path) {
                        ctx.include_bytes(uri, bytes);
                    }
                }
            }

            let _ = tx.send(AppMessage::MediaLoaded {
                folder_name: folder_name.clone(),
                entry_index: *entry_index,
                media: updated_media,
            });
            ctx.request_repaint();
        }

        let _ = tx.send(AppMessage::OperationComplete { op_id });
        ctx.request_repaint();
    });
}
