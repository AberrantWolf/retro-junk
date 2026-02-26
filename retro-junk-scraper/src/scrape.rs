use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::stream::{self, StreamExt};
use retro_junk_core::disc;
use retro_junk_core::{AnalysisOptions, Region, RomAnalyzer};
use retro_junk_frontend::ScrapedGame;
use retro_junk_frontend::miximage_layout::MiximageLayout;
use retro_junk_lib::scanner::{self, GameEntry};
use tokio::sync::{Mutex, mpsc};
use tokio::time::Duration;

/// Timeout for acquiring internal mutex locks (should be near-instant).
const LOCK_TIMEOUT: Duration = Duration::from_secs(5);

use crate::client::ScreenScraperClient;
use crate::error::ScrapeError;
use crate::log::{LogEntry, ScrapeLog};
use crate::lookup::{self, RomInfo};
use crate::media::{self, MediaSelection, media_subdir};
use crate::systems;

/// Options for a scraping session.
#[derive(Debug, Clone)]
pub struct ScrapeOptions {
    /// Root path containing console folders
    pub root: PathBuf,
    /// Preferred region for names/media (e.g., "us", "eu", "jp")
    pub region: String,
    /// Preferred language for descriptions (e.g., "en", "fr", "match")
    pub language: String,
    /// Fallback language when "match" mode has no data for the matched language
    pub language_fallback: String,
    /// Which media types to download
    pub media_selection: MediaSelection,
    /// Directory for metadata files (gamelist.xml etc.)
    pub metadata_dir: PathBuf,
    /// Directory for media files
    pub media_dir: PathBuf,
    /// Show what would be scraped without downloading
    pub dry_run: bool,
    /// Force hash-based lookup even for serial-expected consoles
    pub force_hash: bool,
    /// Skip games that already have metadata
    pub skip_existing: bool,
    /// Disable scrape log file
    pub no_log: bool,
    /// Maximum number of ROMs to process per console
    pub limit: Option<usize>,
    /// Disable miximage generation
    pub no_miximage: bool,
    /// Force redownload of all media, ignoring existing files
    pub force_redownload: bool,
    /// Layout config for miximage generation (None when no_miximage is true)
    pub miximage_layout: Option<MiximageLayout>,
}

impl ScrapeOptions {
    /// Create default options for a root path.
    pub fn new(root: PathBuf) -> Self {
        let metadata_dir = root.parent().unwrap_or(&root).join(format!(
            "{}-metadata",
            root.file_name().unwrap_or_default().to_string_lossy()
        ));
        let media_dir = root.parent().unwrap_or(&root).join(format!(
            "{}-media",
            root.file_name().unwrap_or_default().to_string_lossy()
        ));

        Self {
            root,
            region: "us".to_string(),
            language: "en".to_string(),
            language_fallback: "en".to_string(),
            media_selection: MediaSelection::default(),
            metadata_dir,
            media_dir,
            dry_run: false,
            force_hash: false,
            skip_existing: false,
            no_log: false,
            limit: None,
            no_miximage: false,
            force_redownload: false,
            miximage_layout: None,
        }
    }
}

/// Progress events emitted during scraping, consumed by the CLI or GUI.
#[derive(Debug, Clone)]
pub enum ScrapeEvent {
    /// Scanning the folder for ROM files.
    Scanning,
    /// Scan complete, total games found.
    ScanComplete { total: usize },
    /// A game has started processing (assigned to a worker).
    GameStarted { index: usize, file: String },
    /// Looking up a game on ScreenScraper.
    GameLookingUp { index: usize, file: String },
    /// Downloading media for a game.
    GameDownloading { index: usize, file: String },
    /// Downloading a specific media type for a game.
    GameDownloadingMedia {
        index: usize,
        file: String,
        media_type: String,
    },
    /// Game was skipped (existing media, dry run, etc.).
    GameSkipped {
        index: usize,
        file: String,
        reason: String,
    },
    /// Game was successfully scraped.
    GameCompleted {
        index: usize,
        file: String,
        game_name: String,
    },
    /// Game lookup failed (non-fatal).
    GameFailed {
        index: usize,
        file: String,
        reason: String,
    },
    /// A secondary disc was grouped with its primary.
    GameGrouped {
        index: usize,
        file: String,
        primary_file: String,
    },
    /// A fatal error occurred (quota, auth, server closed). Scraping will stop.
    FatalError { message: String },
    /// All games processed.
    Done,
}

/// Result of scraping a single console folder.
#[derive(Debug)]
pub struct ScrapeResult {
    pub games: Vec<ScrapedGame>,
    pub log: ScrapeLog,
}

/// Internal result from processing a single game.
enum GameResult {
    Scraped {
        scraped: ScrapedGame,
        log_entry: LogEntry,
        /// If this game is a primary disc, the group index for cloning.
        primary_group: Option<usize>,
    },
    Skipped {
        scraped: Option<ScrapedGame>,
        log_entry: Option<LogEntry>,
    },
    Failed {
        log_entry: LogEntry,
    },
    FatalError {
        message: String,
        log_entry: LogEntry,
    },
}

/// Try to generate a miximage into `media_map`, or register an existing one.
///
/// When `force` is true, always regenerates even if the file exists.
/// Inserts `MediaType::Miximage` into `media_map` on success.
fn try_generate_miximage(
    media_map: &mut HashMap<retro_junk_frontend::MediaType, PathBuf>,
    system_media_dir: &Path,
    rom_stem: &str,
    layout: &MiximageLayout,
    force: bool,
) {
    let miximage_path = miximage_path(system_media_dir, rom_stem);
    if force || !miximage_path.exists() {
        match retro_junk_frontend::miximage::generate_miximage(media_map, &miximage_path, layout) {
            Ok(true) => {
                media_map.insert(retro_junk_frontend::MediaType::Miximage, miximage_path);
            }
            Ok(false) => {} // no screenshot, skip
            Err(e) => {
                log::debug!("Failed to generate miximage: {}", e);
            }
        }
    } else {
        media_map.insert(retro_junk_frontend::MediaType::Miximage, miximage_path);
    }
}

/// Miximage output path for a ROM.
fn miximage_path(system_media_dir: &Path, rom_stem: &str) -> PathBuf {
    system_media_dir
        .join("miximages")
        .join(format!("{}.png", rom_stem))
}

/// Scrape all ROMs in a folder for a given console.
pub async fn scrape_folder(
    client: &ScreenScraperClient,
    folder_path: &Path,
    analyzer: &dyn RomAnalyzer,
    options: &ScrapeOptions,
    folder_name: &str,
    max_workers: usize,
    events: mpsc::UnboundedSender<ScrapeEvent>,
) -> Result<ScrapeResult, ScrapeError> {
    let platform = analyzer.platform();
    let short_name = platform.short_name();
    let system_id = systems::screenscraper_system_id(platform).ok_or_else(|| {
        ScrapeError::Config(format!("No ScreenScraper system ID for '{}'", short_name))
    })?;

    let extensions = scanner::extension_set(analyzer.file_extensions());

    // Collect game entries: top-level ROM files and .m3u directories
    let _ = events.send(ScrapeEvent::Scanning);
    let mut game_entries = scanner::scan_game_entries(folder_path, &extensions)
        .map_err(|e| ScrapeError::Config(format!("Error reading folder: {}", e)))?;
    if let Some(max) = options.limit {
        game_entries.truncate(max);
    }

    let total = game_entries.len();
    let _ = events.send(ScrapeEvent::ScanComplete { total });

    let system_media_dir = options.media_dir.join(folder_name);

    // Detect disc groups among loose single-file entries
    let disc_entries: Vec<(usize, &str)> = game_entries
        .iter()
        .enumerate()
        .filter_map(|(i, entry)| match entry {
            scanner::GameEntry::SingleFile(_) => Some((i, entry.rom_stem())),
            _ => None,
        })
        .collect();
    let disc_groups = disc::detect_disc_groups(&disc_entries);

    // Map from entry index → (group_index, is_primary)
    let mut disc_membership: HashMap<usize, (usize, bool)> = HashMap::new();
    for (gi, group) in disc_groups.iter().enumerate() {
        for &mi in &group.member_indices {
            disc_membership.insert(mi, (gi, mi == group.primary_index));
        }
    }

    // Classify entries into work items (primary + independent) and secondary items
    let mut work_items: Vec<(usize, &GameEntry, String, Option<usize>)> = Vec::new();
    let mut secondary_items: Vec<(usize, &GameEntry, usize)> = Vec::new();

    for (index, entry) in game_entries.iter().enumerate() {
        match disc_membership.get(&index) {
            Some(&(group_idx, false)) => {
                // Secondary disc — deferred
                secondary_items.push((index, entry, group_idx));
            }
            Some(&(group_idx, true)) => {
                // Primary disc — use base name for media
                let rom_stem = disc_groups[group_idx].base_name.clone();
                work_items.push((index, entry, rom_stem, Some(group_idx)));
            }
            None => {
                // Independent (non-disc) game
                let rom_stem = entry.rom_stem().to_string();
                work_items.push((index, entry, rom_stem, None));
            }
        }
    }

    // Shared state for concurrent processing
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let primary_results: Arc<Mutex<HashMap<usize, ScrapedGame>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // Process work items concurrently
    let results: Vec<GameResult> = stream::iter(work_items)
        .map(|(index, entry, rom_stem, primary_group)| {
            let events = events.clone();
            let cancel_flag = cancel_flag.clone();
            let primary_results = primary_results.clone();
            let system_media_dir = system_media_dir.clone();
            async move {
                if cancel_flag.load(Ordering::Relaxed) {
                    return GameResult::Skipped {
                        scraped: None,
                        log_entry: None,
                    };
                }

                let result = process_single_game(
                    client,
                    analyzer,
                    options,
                    folder_name,
                    system_id,
                    &system_media_dir,
                    index,
                    entry,
                    &rom_stem,
                    &events,
                    primary_group,
                )
                .await;

                // On fatal error, set the cancel flag
                if let GameResult::FatalError { ref message, .. } = result {
                    cancel_flag.store(true, Ordering::Relaxed);
                    let _ = events.send(ScrapeEvent::FatalError {
                        message: message.clone(),
                    });
                }

                // Stash primary disc results for secondary discs
                if let GameResult::Scraped {
                    primary_group: Some(group_idx),
                    ref scraped,
                    ..
                } = result
                {
                    match tokio::time::timeout(LOCK_TIMEOUT, primary_results.lock()).await {
                        Ok(mut guard) => {
                            guard.insert(group_idx, scraped.clone());
                        }
                        Err(_) => log::warn!("primary_results lock timed out"),
                    }
                }

                result
            }
        })
        .buffer_unordered(max_workers)
        .collect()
        .await;

    // Collect results
    let mut games = Vec::new();
    let mut log = ScrapeLog::new();

    for result in results {
        match result {
            GameResult::Scraped {
                scraped, log_entry, ..
            } => {
                games.push(scraped);
                log.add(log_entry);
            }
            GameResult::Skipped {
                scraped, log_entry, ..
            } => {
                if let Some(s) = scraped {
                    games.push(s);
                }
                if let Some(e) = log_entry {
                    log.add(e);
                }
            }
            GameResult::Failed { log_entry, .. } => {
                log.add(log_entry);
            }
            GameResult::FatalError { log_entry, .. } => {
                log.add(log_entry);
            }
        }
    }

    // Resolve secondary discs from primary results (no API calls needed)
    let primary_map = tokio::time::timeout(LOCK_TIMEOUT, primary_results.lock())
        .await
        .map_err(|_| ScrapeError::Api("primary_results lock timed out".to_string()))?;
    for (index, entry, group_idx) in &secondary_items {
        let filename = entry.display_name().to_string();

        if let Some(primary_scraped) = primary_map.get(group_idx) {
            let group = &disc_groups[*group_idx];
            let disc_num = disc::extract_disc_number(&filename).unwrap_or(0);
            let scraped = ScrapedGame {
                rom_filename: filename.clone(),
                rom_stem: group.base_name.clone(),
                name: format!("{} (Disc {})", primary_scraped.name, disc_num),
                ..primary_scraped.clone()
            };

            let primary_filename = game_entries[group.primary_index].display_name();
            let _ = events.send(ScrapeEvent::GameGrouped {
                index: *index,
                file: filename.clone(),
                primary_file: primary_filename.to_string(),
            });

            log.add(LogEntry::GroupedDisc {
                file: filename,
                primary_file: primary_filename.to_string(),
                game_name: scraped.name.clone(),
            });
            games.push(scraped);
        } else {
            // Primary not processed or failed — fall through to treat as
            // an unresolved secondary. Log it but don't fail.
            log.add(LogEntry::Error {
                file: filename,
                message: "Primary disc was not scraped; could not group".to_string(),
            });
        }
    }

    let _ = events.send(ScrapeEvent::Done);

    Ok(ScrapeResult { games, log })
}

/// Process a single game entry: analyze, look up, download media.
#[allow(clippy::too_many_arguments)]
async fn process_single_game(
    client: &ScreenScraperClient,
    analyzer: &dyn RomAnalyzer,
    options: &ScrapeOptions,
    _folder_name: &str,
    system_id: u32,
    system_media_dir: &Path,
    index: usize,
    entry: &GameEntry,
    rom_stem: &str,
    events: &mpsc::UnboundedSender<ScrapeEvent>,
    primary_group: Option<usize>,
) -> GameResult {
    let filename = entry.display_name().to_string();
    let rom_path = entry.analysis_path();

    let _ = events.send(ScrapeEvent::GameStarted {
        index,
        file: filename.clone(),
    });

    // Check if we can skip ScreenScraper entirely using existing media
    if !options.force_redownload {
        let existing =
            media::collect_existing_media(&options.media_selection, system_media_dir, rom_stem);

        let has_screenshot = existing.contains_key(&retro_junk_frontend::MediaType::Screenshot);
        let has_miximage = miximage_path(system_media_dir, rom_stem).exists();
        let needs_miximage = !options.no_miximage && !has_miximage;

        if has_screenshot && (!needs_miximage || options.miximage_layout.is_some()) {
            let mut media_map = existing;

            if needs_miximage {
                let layout = options.miximage_layout.as_ref().unwrap();
                try_generate_miximage(&mut media_map, system_media_dir, rom_stem, layout, false);
            } else if has_miximage {
                media_map.insert(
                    retro_junk_frontend::MediaType::Miximage,
                    miximage_path(system_media_dir, rom_stem),
                );
            }

            let reason = if needs_miximage {
                "media exists, generated miximage"
            } else {
                "media already exists"
            };
            let _ = events.send(ScrapeEvent::GameSkipped {
                index,
                file: filename.clone(),
                reason: reason.to_string(),
            });

            let scraped = ScrapedGame {
                rom_stem: rom_stem.to_string(),
                rom_filename: filename,
                name: entry.display_name().to_string(),
                description: None,
                developer: None,
                publisher: None,
                genre: None,
                players: None,
                rating: None,
                release_date: None,
                media: media_map,
            };
            return GameResult::Skipped {
                scraped: Some(scraped),
                log_entry: None,
            };
        }
    }

    let platform = analyzer.platform();

    // Analyze the ROM to extract serial and regions
    let analysis_opts = AnalysisOptions::new().quick(true).file_path(rom_path);
    let (serial, rom_regions): (Option<String>, Vec<Region>) = match std::fs::File::open(rom_path) {
        Ok(mut f) => match analyzer.analyze(&mut f, &analysis_opts) {
            Ok(info) => (info.serial_number, info.regions),
            Err(_) => (None, Vec::new()),
        },
        Err(e) => {
            let message = format!("Failed to open file: {}", e);
            let _ = events.send(ScrapeEvent::GameFailed {
                index,
                file: filename.clone(),
                reason: message.clone(),
            });
            return GameResult::Failed {
                log_entry: LogEntry::Error {
                    file: filename,
                    message,
                },
            };
        }
    };

    let file_size = rom_path.metadata().map(|m| m.len()).unwrap_or(0);

    // Compute effective region and language from ROM analysis
    let effective_region = rom_regions
        .first()
        .map(|r| systems::region_to_ss_code(r).to_string())
        .unwrap_or_else(|| options.region.clone());

    let effective_language = if options.language == "match" {
        rom_regions
            .first()
            .map(|r| systems::region_to_language(r).to_string())
            .unwrap_or_else(|| options.language_fallback.clone())
    } else {
        options.language.clone()
    };

    // Compute hashes if needed (for non-serial consoles or force_hash)
    let (crc32, md5, sha1) = if !systems::expects_serial(platform) || options.force_hash {
        match std::fs::File::open(rom_path) {
            Ok(mut f) => match retro_junk_lib::hasher::compute_all_hashes(&mut f, analyzer) {
                Ok(hashes) => (Some(hashes.crc32), hashes.md5, hashes.sha1),
                Err(e) => {
                    log::debug!("Failed to hash {}: {}", filename, e);
                    (None, None, None)
                }
            },
            Err(_) => (None, None, None),
        }
    } else {
        (None, None, None)
    };

    let scraper_serial = serial
        .as_ref()
        .and_then(|s| analyzer.extract_scraper_serial(s));

    let rom_info = RomInfo {
        serial: serial.clone(),
        scraper_serial,
        filename: filename.clone(),
        file_size,
        crc32,
        md5,
        sha1,
        platform,
        expects_serial: analyzer.expects_serial(),
    };

    if options.dry_run {
        let method = if serial.is_some() {
            "serial"
        } else if !systems::expects_serial(platform) {
            "hash"
        } else {
            "filename"
        };
        let _ = events.send(ScrapeEvent::GameSkipped {
            index,
            file: filename,
            reason: format!("dry run (would try {})", method),
        });
        return GameResult::Skipped {
            scraped: None,
            log_entry: None,
        };
    }

    // Look up the game
    let _ = events.send(ScrapeEvent::GameLookingUp {
        index,
        file: filename.clone(),
    });

    match lookup::lookup_game(client, system_id, &rom_info).await {
        Ok(result) => {
            let game_name = result
                .game
                .name_for_region(&effective_region)
                .unwrap_or("Unknown")
                .to_string();

            // Download media
            let _ = events.send(ScrapeEvent::GameDownloading {
                index,
                file: filename.clone(),
            });

            let mut media_map = media::download_game_media(
                client,
                &result.game,
                &options.media_selection,
                system_media_dir,
                rom_stem,
                &effective_region,
                options.force_redownload,
                index,
                &filename,
                events,
            )
            .await
            .unwrap_or_default();

            // Generate miximage if enabled
            if let Some(ref layout) = options.miximage_layout {
                try_generate_miximage(
                    &mut media_map,
                    system_media_dir,
                    rom_stem,
                    layout,
                    options.force_redownload,
                );
            }

            let media_names: Vec<String> = media_map
                .keys()
                .map(|mt| media_subdir(*mt).to_string())
                .collect();

            let log_entry = if result.warnings.is_empty() {
                LogEntry::Success {
                    file: filename.clone(),
                    game_name: game_name.clone(),
                    method: result.method,
                    media_downloaded: media_names,
                }
            } else {
                LogEntry::Partial {
                    file: filename.clone(),
                    game_name: game_name.clone(),
                    warnings: result.warnings,
                }
            };

            let description = result
                .game
                .synopsis_for_language(&effective_language)
                .or_else(|| {
                    result
                        .game
                        .synopsis_for_language(&options.language_fallback)
                })
                .or_else(|| result.game.synopsis_for_language("en"))
                .map(|s| s.to_string());

            let genre = result
                .game
                .genre_for_language(&effective_language)
                .or_else(|| result.game.genre_for_language(&options.language_fallback))
                .or_else(|| result.game.genre_for_language("en"));

            let scraped = ScrapedGame {
                rom_stem: rom_stem.to_string(),
                rom_filename: filename.clone(),
                name: game_name.clone(),
                description,
                developer: result.game.developpeur.as_ref().map(|d| d.text.clone()),
                publisher: result.game.editeur.as_ref().map(|p| p.text.clone()),
                genre,
                players: result.game.joueurs.as_ref().map(|j| j.text.clone()),
                rating: result.game.rating_normalized(),
                release_date: result
                    .game
                    .date_for_region(&effective_region)
                    .map(|d| d.to_string()),
                media: media_map,
            };

            let _ = events.send(ScrapeEvent::GameCompleted {
                index,
                file: filename,
                game_name,
            });

            GameResult::Scraped {
                scraped,
                log_entry,
                primary_group,
            }
        }
        Err(ScrapeError::NotFound { warnings }) => {
            let _ = events.send(ScrapeEvent::GameFailed {
                index,
                file: filename.clone(),
                reason: "Game not found".to_string(),
            });
            let errors = if warnings.is_empty() {
                vec!["Game not found in ScreenScraper".to_string()]
            } else {
                warnings
            };
            GameResult::Failed {
                log_entry: LogEntry::Unidentified {
                    file: filename,
                    scraper_serial_tried: rom_info.scraper_serial.clone(),
                    serial_tried: serial,
                    filename_tried: true,
                    hashes_tried: rom_info.crc32.is_some(),
                    crc32: rom_info.crc32.clone(),
                    md5: rom_info.md5.clone(),
                    sha1: rom_info.sha1.clone(),
                    errors,
                },
            }
        }
        Err(
            e @ ScrapeError::QuotaExceeded { .. }
            | e @ ScrapeError::InvalidCredentials(_)
            | e @ ScrapeError::ServerClosed(_),
        ) => {
            let message = e.to_string();
            GameResult::FatalError {
                message,
                log_entry: LogEntry::Error {
                    file: filename,
                    message: e.to_string(),
                },
            }
        }
        Err(e) => {
            let _ = events.send(ScrapeEvent::GameFailed {
                index,
                file: filename.clone(),
                reason: e.to_string(),
            });
            GameResult::Failed {
                log_entry: LogEntry::Error {
                    file: filename,
                    message: e.to_string(),
                },
            }
        }
    }
}
