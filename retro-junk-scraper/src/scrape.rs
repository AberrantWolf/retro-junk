use std::path::{Path, PathBuf};

use retro_junk_core::{AnalysisOptions, Region, RomAnalyzer};
use retro_junk_frontend::ScrapedGame;

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
}

impl ScrapeOptions {
    /// Create default options for a root path.
    pub fn new(root: PathBuf) -> Self {
        let metadata_dir = root
            .parent()
            .unwrap_or(&root)
            .join(format!("{}-metadata", root.file_name().unwrap_or_default().to_string_lossy()));
        let media_dir = root
            .parent()
            .unwrap_or(&root)
            .join(format!("{}-media", root.file_name().unwrap_or_default().to_string_lossy()));

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
        }
    }
}

/// Progress callback for scraping.
#[derive(Debug, Clone)]
pub enum ScrapeProgress {
    Scanning,
    LookingUp {
        file: String,
        index: usize,
        total: usize,
    },
    Downloading {
        media_type: String,
        file: String,
    },
    Skipped {
        file: String,
        reason: String,
    },
    Done,
}

/// Result of scraping a single console folder.
#[derive(Debug)]
pub struct ScrapeResult {
    pub games: Vec<ScrapedGame>,
    pub log: ScrapeLog,
}

/// Scrape all ROMs in a folder for a given console.
pub async fn scrape_folder(
    client: &ScreenScraperClient,
    folder_path: &Path,
    analyzer: &dyn RomAnalyzer,
    options: &ScrapeOptions,
    progress: &dyn Fn(ScrapeProgress),
) -> Result<ScrapeResult, ScrapeError> {
    let short_name = analyzer.short_name();
    let system_id = systems::screenscraper_system_id(short_name)
        .ok_or_else(|| ScrapeError::Config(format!("No ScreenScraper system ID for '{}'", short_name)))?;

    let extensions: std::collections::HashSet<String> = analyzer
        .file_extensions()
        .iter()
        .map(|e| e.to_lowercase())
        .collect();

    // Collect ROM files
    progress(ScrapeProgress::Scanning);
    let mut rom_files: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(folder_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase())
                .unwrap_or_default();
            if extensions.contains(&ext) {
                rom_files.push(path);
            }
        }
    }
    rom_files.sort();
    if let Some(max) = options.limit {
        rom_files.truncate(max);
    }

    let total = rom_files.len();
    let mut games = Vec::new();
    let mut log = ScrapeLog::new();

    let system_media_dir = options.media_dir.join(short_name);

    for (index, rom_path) in rom_files.iter().enumerate() {
        let filename = rom_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        let rom_stem = rom_path
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();

        progress(ScrapeProgress::LookingUp {
            file: filename.clone(),
            index,
            total,
        });

        // Analyze the ROM to extract serial and regions
        let analysis_opts = AnalysisOptions::new().quick(true);
        let (serial, rom_regions): (Option<String>, Vec<Region>) =
            match std::fs::File::open(rom_path) {
                Ok(mut f) => match analyzer.analyze(&mut f, &analysis_opts) {
                    Ok(info) => (info.serial_number, info.regions),
                    Err(_) => (None, Vec::new()),
                },
                Err(e) => {
                    log.add(LogEntry::Error {
                        file: filename.clone(),
                        message: format!("Failed to open file: {}", e),
                    });
                    continue;
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
        let (crc32, md5, sha1) = if !systems::expects_serial(short_name) || options.force_hash {
            match std::fs::File::open(rom_path) {
                Ok(mut f) => match retro_junk_lib::hasher::compute_all_hashes(&mut f, analyzer) {
                    Ok(hashes) => (
                        Some(hashes.crc32),
                        hashes.md5,
                        hashes.sha1,
                    ),
                    Err(e) => {
                        log.add(LogEntry::Error {
                            file: filename.clone(),
                            message: format!("Failed to hash file: {}", e),
                        });
                        (None, None, None)
                    }
                },
                Err(_) => (None, None, None),
            }
        } else {
            (None, None, None)
        };

        let rom_info = RomInfo {
            serial: serial.clone(),
            filename: filename.clone(),
            file_size,
            crc32,
            md5,
            sha1,
            short_name: short_name.to_string(),
        };

        if options.dry_run {
            let method = if serial.is_some() {
                "serial"
            } else if !systems::expects_serial(short_name) {
                "hash"
            } else {
                "filename"
            };
            progress(ScrapeProgress::Skipped {
                file: filename.clone(),
                reason: format!("dry run (would try {})", method),
            });
            continue;
        }

        // Look up the game
        match lookup::lookup_game(client, system_id, &rom_info, options.force_hash).await {
            Ok(result) => {
                let game_name = result
                    .game
                    .name_for_region(&effective_region)
                    .unwrap_or("Unknown")
                    .to_string();

                // Download media
                progress(ScrapeProgress::Downloading {
                    media_type: "all".to_string(),
                    file: filename.clone(),
                });

                let media_map = media::download_game_media(
                    client,
                    &result.game,
                    &options.media_selection,
                    &system_media_dir,
                    &rom_stem,
                    &effective_region,
                )
                .await
                .unwrap_or_default();

                let media_names: Vec<String> = media_map
                    .keys()
                    .map(|mt| media_subdir(*mt).to_string())
                    .collect();

                if result.warnings.is_empty() {
                    log.add(LogEntry::Success {
                        file: filename.clone(),
                        game_name: game_name.clone(),
                        method: result.method,
                        media_downloaded: media_names,
                    });
                } else {
                    log.add(LogEntry::Partial {
                        file: filename.clone(),
                        game_name: game_name.clone(),
                        warnings: result.warnings,
                    });
                }

                let description = result
                    .game
                    .synopsis_for_language(&effective_language)
                    .or_else(|| result.game.synopsis_for_language(&options.language_fallback))
                    .or_else(|| result.game.synopsis_for_language("en"))
                    .map(|s| s.to_string());

                let genre = result
                    .game
                    .genre_for_language(&effective_language)
                    .or_else(|| result.game.genre_for_language(&options.language_fallback))
                    .or_else(|| result.game.genre_for_language("en"));

                let scraped = ScrapedGame {
                    rom_stem,
                    rom_filename: filename,
                    name: game_name,
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

                games.push(scraped);
            }
            Err(ScrapeError::NotFound) => {
                log.add(LogEntry::Unidentified {
                    file: filename,
                    serial_tried: serial,
                    filename_tried: true,
                    hashes_tried: rom_info.crc32.is_some(),
                    errors: vec!["Game not found in ScreenScraper".to_string()],
                });
            }
            Err(e) => {
                log.add(LogEntry::Error {
                    file: filename,
                    message: e.to_string(),
                });
            }
        }
    }

    progress(ScrapeProgress::Done);

    Ok(ScrapeResult { games, log })
}
