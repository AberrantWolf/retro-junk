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

use crate::assets::{self, AssetSelection, asset_subdir};
use crate::client::ScreenScraperClient;
use crate::error::ScrapeError;
use crate::log::{LogEntry, ScrapeLog};
use crate::lookup::{self, RomHashes, RomInfo};
use crate::systems;
use crate::types::GameInfo;

/// Whether and how to generate miximages.
#[derive(Debug, Clone)]
pub enum MiximageMode {
    /// Do not generate miximages.
    Disabled,
    /// Generate miximages using the given layout.
    Enabled(MiximageLayout),
}

impl MiximageMode {
    /// The layout to generate with, if generation is enabled.
    fn layout(&self) -> Option<&MiximageLayout> {
        match self {
            MiximageMode::Disabled => None,
            MiximageMode::Enabled(layout) => Some(layout),
        }
    }
}

/// Options for a scraping session.
// Independent CLI-style flags; grouping them into enums would break the public API used by CLI/GUI.
#[allow(clippy::struct_excessive_bools)]
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
    /// Which asset types to download
    pub asset_selection: AssetSelection,
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
    /// Whether and how to generate miximages
    pub miximage: MiximageMode,
    /// Force redownload of all media, ignoring existing files
    pub force_redownload: bool,
}

impl ScrapeOptions {
    /// Create default options for a root path.
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        let metadata_dir = retro_junk_lib::util::default_metadata_dir(&root);
        let media_dir = retro_junk_lib::util::default_media_dir(&root);

        Self {
            root,
            region: "us".to_string(),
            language: "en".to_string(),
            language_fallback: "en".to_string(),
            asset_selection: AssetSelection::default(),
            metadata_dir,
            media_dir,
            dry_run: false,
            force_hash: false,
            skip_existing: false,
            no_log: false,
            limit: None,
            miximage: MiximageMode::Disabled,
            force_redownload: false,
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
    /// Looking up a game on `ScreenScraper`.
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

/// Folder-scoped state shared by every game processed in a scrape run.
#[derive(Clone, Copy)]
struct FolderContext<'a> {
    client: &'a ScreenScraperClient,
    analyzer: &'a dyn RomAnalyzer,
    options: &'a ScrapeOptions,
    system_id: u32,
    system_media_dir: &'a Path,
    events: &'a mpsc::UnboundedSender<ScrapeEvent>,
}

/// Disc-group classification of the scanned game entries.
struct WorkPlan<'a> {
    /// Entries to scrape: (entry index, entry, media rom stem, primary disc-group index).
    work_items: Vec<(usize, &'a GameEntry, String, Option<usize>)>,
    /// Secondary discs resolved from their primary: (entry index, entry, disc-group index).
    secondary_items: Vec<(usize, &'a GameEntry, usize)>,
    /// Detected disc groups (indices refer into the scanned entry list).
    disc_groups: Vec<disc::DiscGroup>,
}

/// Detect disc groups among the scanned entries and classify each entry as a
/// work item (primary disc or independent game) or a deferred secondary disc.
fn plan_work(game_entries: &[GameEntry]) -> WorkPlan<'_> {
    // Detect disc groups among loose single-file entries
    let disc_entries: Vec<(usize, &str)> = game_entries
        .iter()
        .enumerate()
        .filter_map(|(i, entry)| match entry {
            GameEntry::SingleFile(_) => Some((i, entry.rom_stem())),
            GameEntry::MultiDisc { .. } => None,
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

    WorkPlan {
        work_items,
        secondary_items,
        disc_groups,
    }
}

/// Fold per-game worker results into the collected games and scrape log.
fn collect_results(results: Vec<GameResult>, games: &mut Vec<ScrapedGame>, log: &mut ScrapeLog) {
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
            GameResult::Failed { log_entry, .. } | GameResult::FatalError { log_entry, .. } => {
                log.add(log_entry);
            }
        }
    }
}

/// Resolve secondary discs by cloning their primary's scraped result
/// (no API calls needed).
fn resolve_secondary_discs(
    plan: &WorkPlan<'_>,
    game_entries: &[GameEntry],
    primary_map: &HashMap<usize, ScrapedGame>,
    events: &mpsc::UnboundedSender<ScrapeEvent>,
    games: &mut Vec<ScrapedGame>,
    log: &mut ScrapeLog,
) {
    for (index, entry, group_idx) in &plan.secondary_items {
        let filename = entry.display_name().to_string();

        if let Some(primary_scraped) = primary_map.get(group_idx) {
            let group = &plan.disc_groups[*group_idx];
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
}

/// Try to generate a miximage into `media_map`, or register an existing one.
///
/// When `force` is true, always regenerates even if the file exists.
/// Inserts `AssetType::Miximage` into `media_map` on success.
fn try_generate_miximage(
    media_map: &mut HashMap<retro_junk_frontend::AssetType, PathBuf>,
    system_media_dir: &Path,
    rom_stem: &str,
    layout: &MiximageLayout,
    force: bool,
) {
    let miximage_path = miximage_path(system_media_dir, rom_stem);
    if force || !miximage_path.exists() {
        match retro_junk_frontend::miximage::generate_miximage(media_map, &miximage_path, layout) {
            Ok(true) => {
                media_map.insert(retro_junk_frontend::AssetType::Miximage, miximage_path);
            }
            Ok(false) => {} // no screenshot, skip
            Err(e) => {
                log::debug!("Failed to generate miximage: {e}");
            }
        }
    } else {
        media_map.insert(retro_junk_frontend::AssetType::Miximage, miximage_path);
    }
}

/// Miximage output path for a ROM.
fn miximage_path(system_media_dir: &Path, rom_stem: &str) -> PathBuf {
    system_media_dir
        .join("miximages")
        .join(format!("{rom_stem}.png"))
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
        ScrapeError::Config(format!("No ScreenScraper system ID for '{short_name}'"))
    })?;

    let extensions = scanner::extension_set(analyzer.file_extensions());

    // Collect game entries: top-level ROM files and .m3u directories
    let _ = events.send(ScrapeEvent::Scanning);
    let mut game_entries = scanner::scan_game_entries(folder_path, &extensions)
        .map_err(|e| ScrapeError::Config(format!("Error reading folder: {e}")))?;
    if let Some(max) = options.limit {
        game_entries.truncate(max);
    }

    let total = game_entries.len();
    let _ = events.send(ScrapeEvent::ScanComplete { total });

    let system_media_dir = options.media_dir.join(folder_name);

    // Classify entries into work items (primary + independent) and secondary items
    let plan = plan_work(&game_entries);

    let ctx = FolderContext {
        client,
        analyzer,
        options,
        system_id,
        system_media_dir: &system_media_dir,
        events: &events,
    };

    // Shared state for concurrent processing
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let primary_results: Arc<Mutex<HashMap<usize, ScrapedGame>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // Process work items concurrently
    let results: Vec<GameResult> = stream::iter(plan.work_items.iter())
        .map(|&(index, entry, ref rom_stem, primary_group)| {
            let cancel_flag = cancel_flag.clone();
            let primary_results = primary_results.clone();
            async move {
                if cancel_flag.load(Ordering::Relaxed) {
                    return GameResult::Skipped {
                        scraped: None,
                        log_entry: None,
                    };
                }

                let result = process_single_game(&ctx, index, entry, rom_stem, primary_group).await;

                // On fatal error, set the cancel flag
                if let GameResult::FatalError { ref message, .. } = result {
                    cancel_flag.store(true, Ordering::Relaxed);
                    let _ = ctx.events.send(ScrapeEvent::FatalError {
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
    collect_results(results, &mut games, &mut log);

    // Resolve secondary discs from primary results (no API calls needed)
    let primary_map = tokio::time::timeout(LOCK_TIMEOUT, primary_results.lock())
        .await
        .map_err(|_| ScrapeError::Api("primary_results lock timed out".to_string()))?;
    resolve_secondary_discs(
        &plan,
        &game_entries,
        &primary_map,
        &events,
        &mut games,
        &mut log,
    );

    let _ = events.send(ScrapeEvent::Done);

    Ok(ScrapeResult { games, log })
}

/// If existing on-disk media lets us skip `ScreenScraper` entirely, build the
/// skip result (generating or registering a miximage as needed).
fn skip_with_existing_media(
    ctx: &FolderContext<'_>,
    index: usize,
    entry: &GameEntry,
    rom_stem: &str,
    filename: &str,
) -> Option<GameResult> {
    let existing = assets::collect_existing_assets(
        &ctx.options.asset_selection,
        ctx.system_media_dir,
        rom_stem,
    );

    if !existing.contains_key(&retro_junk_frontend::AssetType::Screenshot) {
        return None;
    }

    let has_miximage = miximage_path(ctx.system_media_dir, rom_stem).exists();
    let needs_miximage = ctx.options.miximage.layout().is_some() && !has_miximage;

    let mut media_map = existing;

    if needs_miximage {
        if let Some(layout) = ctx.options.miximage.layout() {
            try_generate_miximage(
                &mut media_map,
                ctx.system_media_dir,
                rom_stem,
                layout,
                false,
            );
        }
    } else if has_miximage {
        media_map.insert(
            retro_junk_frontend::AssetType::Miximage,
            miximage_path(ctx.system_media_dir, rom_stem),
        );
    }

    let reason = if needs_miximage {
        "media exists, generated miximage"
    } else {
        "media already exists"
    };
    let _ = ctx.events.send(ScrapeEvent::GameSkipped {
        index,
        file: filename.to_string(),
        reason: reason.to_string(),
    });

    let scraped = ScrapedGame {
        rom_stem: rom_stem.to_string(),
        rom_filename: filename.to_string(),
        name: entry.display_name().to_string(),
        description: String::new(),
        developer: String::new(),
        publisher: String::new(),
        genre: String::new(),
        players: String::new(),
        rating: None,
        release_date: String::new(),
        assets: media_map,
        cover_title: String::new(),
    };
    Some(GameResult::Skipped {
        scraped: Some(scraped),
        log_entry: None,
    })
}

/// Compute the full CRC32+MD5+SHA1 triple when the lookup path needs it
/// (non-serial consoles or `force_hash`).
///
/// The hash lookup tier needs the whole triple, so a partial result counts as
/// no hashes.
fn compute_rom_hashes(
    ctx: &FolderContext<'_>,
    rom_path: &Path,
    filename: &str,
) -> Option<RomHashes> {
    if systems::expects_serial(ctx.analyzer.platform()) && !ctx.options.force_hash {
        return None;
    }
    let mut f = std::fs::File::open(rom_path).ok()?;
    match retro_junk_lib::hasher::compute_all_hashes(&mut f, ctx.analyzer, Some(rom_path)) {
        Ok(hashes) => match (hashes.md5, hashes.sha1) {
            (Some(md5), Some(sha1)) => Some(RomHashes {
                crc32: hashes.crc32,
                md5,
                sha1,
            }),
            _ => None,
        },
        Err(e) => {
            log::debug!("Failed to hash {filename}: {e}");
            None
        }
    }
}

/// Report which lookup method a dry run would use and skip the game.
fn dry_run_skip(
    ctx: &FolderContext<'_>,
    index: usize,
    filename: String,
    serial: &str,
) -> GameResult {
    let method = if !serial.is_empty() {
        "serial"
    } else if !systems::expects_serial(ctx.analyzer.platform()) {
        "hash"
    } else {
        "filename"
    };
    let _ = ctx.events.send(ScrapeEvent::GameSkipped {
        index,
        file: filename,
        reason: format!("dry run (would try {method})"),
    });
    GameResult::Skipped {
        scraped: None,
        log_entry: None,
    }
}

/// Effective region and language for one game, derived from its ROM regions
/// and the session options.
struct GameLocale {
    region: String,
    language: String,
}

impl GameLocale {
    fn from_rom(options: &ScrapeOptions, rom_regions: &[Region]) -> Self {
        let region = rom_regions.first().map_or_else(
            || options.region.clone(),
            |r| systems::region_to_ss_code(r).to_string(),
        );
        let language = if options.language == "match" {
            rom_regions.first().map_or_else(
                || options.language_fallback.clone(),
                |r| systems::region_to_language(r).to_string(),
            )
        } else {
            options.language.clone()
        };
        Self { region, language }
    }
}

/// Look up localized text: effective language first, then the configured
/// fallback, then English.
fn localized<T>(language: &str, fallback: &str, lookup: impl Fn(&str) -> Option<T>) -> Option<T> {
    lookup(language)
        .or_else(|| lookup(fallback))
        .or_else(|| lookup("en"))
}

/// Assemble the `ScrapedGame` metadata for a successfully looked-up game.
fn build_scraped_game(
    options: &ScrapeOptions,
    game: &GameInfo,
    locale: &GameLocale,
    rom_stem: &str,
    filename: String,
    game_name: String,
    media_map: HashMap<retro_junk_frontend::AssetType, PathBuf>,
) -> ScrapedGame {
    let description = localized(&locale.language, &options.language_fallback, |l| {
        game.synopsis_for_language(l)
    })
    .map(std::string::ToString::to_string)
    .unwrap_or_default();

    let genre = localized(&locale.language, &options.language_fallback, |l| {
        game.genre_for_language(l)
    })
    .unwrap_or_default();

    ScrapedGame {
        rom_stem: rom_stem.to_string(),
        rom_filename: filename,
        name: game_name,
        description,
        developer: game
            .developpeur
            .as_ref()
            .map(|d| d.text.clone())
            .unwrap_or_default(),
        publisher: game
            .editeur
            .as_ref()
            .map(|p| p.text.clone())
            .unwrap_or_default(),
        genre,
        players: game
            .joueurs
            .as_ref()
            .map(|j| j.text.clone())
            .unwrap_or_default(),
        rating: game.rating_normalized(),
        release_date: game
            .date_for_region(&locale.region)
            .map(std::string::ToString::to_string)
            .unwrap_or_default(),
        assets: media_map,
        cover_title: String::new(),
    }
}

/// Download media, generate the miximage if enabled, and assemble the scraped
/// result for a successful lookup.
async fn finish_scraped_game(
    ctx: &FolderContext<'_>,
    index: usize,
    rom_stem: &str,
    filename: String,
    rom_regions: &[Region],
    result: lookup::LookupResult,
    primary_group: Option<usize>,
) -> GameResult {
    let options = ctx.options;
    let locale = GameLocale::from_rom(options, rom_regions);

    let game_name = result
        .game
        .name_for_region(&locale.region)
        .unwrap_or("Unknown")
        .to_string();

    // Download media
    let _ = ctx.events.send(ScrapeEvent::GameDownloading {
        index,
        file: filename.clone(),
    });

    let mut media_map = assets::download_game_assets(
        ctx.client,
        &assets::AssetDownloadRequest {
            game: &result.game,
            selection: &options.asset_selection,
            media_dir: ctx.system_media_dir,
            rom_stem,
            preferred_region: &locale.region,
            force_redownload: options.force_redownload,
            index,
            filename: &filename,
            events: ctx.events,
        },
    )
    .await
    .unwrap_or_default();

    // Generate miximage if enabled
    if let Some(layout) = options.miximage.layout() {
        try_generate_miximage(
            &mut media_map,
            ctx.system_media_dir,
            rom_stem,
            layout,
            options.force_redownload,
        );
    }

    let media_names: Vec<String> = media_map
        .keys()
        .map(|mt| asset_subdir(*mt).to_string())
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

    let scraped = build_scraped_game(
        options,
        &result.game,
        &locale,
        rom_stem,
        filename.clone(),
        game_name.clone(),
        media_map,
    );

    let _ = ctx.events.send(ScrapeEvent::GameCompleted {
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

/// Build the `Unidentified` failure result for a game that wasn't found.
fn unidentified_result(
    ctx: &FolderContext<'_>,
    index: usize,
    filename: String,
    serial: String,
    rom_info: &RomInfo,
    warnings: Vec<String>,
) -> GameResult {
    let _ = ctx.events.send(ScrapeEvent::GameFailed {
        index,
        file: filename.clone(),
        reason: "Game not found".to_string(),
    });
    let errors = if warnings.is_empty() {
        vec!["Game not found in ScreenScraper".to_string()]
    } else {
        warnings
    };
    let (crc32, md5, sha1) = match &rom_info.hashes {
        Some(h) => (h.crc32.clone(), h.md5.clone(), h.sha1.clone()),
        None => Default::default(),
    };
    GameResult::Failed {
        log_entry: LogEntry::Unidentified {
            file: filename,
            scraper_serial_tried: rom_info.scraper_serial.clone(),
            serial_tried: serial,
            filename_tried: true,
            hashes_tried: rom_info.hashes.is_some(),
            crc32,
            md5,
            sha1,
            errors,
        },
    }
}

/// Process a single game entry: analyze, look up, download media.
async fn process_single_game(
    ctx: &FolderContext<'_>,
    index: usize,
    entry: &GameEntry,
    rom_stem: &str,
    primary_group: Option<usize>,
) -> GameResult {
    let FolderContext {
        client,
        analyzer,
        options,
        system_id,
        events,
        ..
    } = *ctx;
    let filename = entry.display_name().to_string();
    let rom_path = entry.analysis_path();

    let _ = events.send(ScrapeEvent::GameStarted {
        index,
        file: filename.clone(),
    });

    // Check if we can skip ScreenScraper entirely using existing media
    if !options.force_redownload
        && let Some(skip) = skip_with_existing_media(ctx, index, entry, rom_stem, &filename)
    {
        return skip;
    }

    let platform = analyzer.platform();

    // Analyze the ROM to extract serial and regions
    let analysis_opts = AnalysisOptions::new().quick(true).file_path(rom_path);
    let (serial, rom_regions): (String, Vec<Region>) = match std::fs::File::open(rom_path) {
        Ok(mut f) => match analyzer.analyze(&mut f, &analysis_opts) {
            Ok(info) => (info.serial_number, info.regions),
            Err(_) => (String::new(), Vec::new()),
        },
        Err(e) => {
            let message = format!("Failed to open file: {e}");
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

    let file_size = rom_path.metadata().map_or(0, |m| m.len());
    let hashes = compute_rom_hashes(ctx, rom_path, &filename);

    let scraper_serial = if serial.is_empty() {
        String::new()
    } else {
        analyzer.extract_scraper_serial(&serial).unwrap_or_default()
    };

    let rom_info = RomInfo {
        serial: serial.clone(),
        scraper_serial,
        filename: filename.clone(),
        file_size,
        hashes,
        platform,
        expects_serial: analyzer.expects_serial(),
    };

    if options.dry_run {
        return dry_run_skip(ctx, index, filename, &serial);
    }

    // Look up the game
    let _ = events.send(ScrapeEvent::GameLookingUp {
        index,
        file: filename.clone(),
    });

    match lookup::lookup_game(client, system_id, &rom_info).await {
        Ok(result) => {
            finish_scraped_game(
                ctx,
                index,
                rom_stem,
                filename,
                &rom_regions,
                result,
                primary_group,
            )
            .await
        }
        Err(ScrapeError::NotFound { warnings }) => {
            unidentified_result(ctx, index, filename, serial, &rom_info, warnings)
        }
        Err(e) => lookup_error_result(ctx, index, filename, &e),
    }
}

/// Convert a lookup error into a fatal result (quota, auth, server closed) or
/// a per-game failure.
fn lookup_error_result(
    ctx: &FolderContext<'_>,
    index: usize,
    filename: String,
    e: &ScrapeError,
) -> GameResult {
    match e {
        ScrapeError::QuotaExceeded { .. }
        | ScrapeError::InvalidCredentials(_)
        | ScrapeError::ServerClosed(_) => GameResult::FatalError {
            message: e.to_string(),
            log_entry: LogEntry::Error {
                file: filename,
                message: e.to_string(),
            },
        },
        _ => {
            let _ = ctx.events.send(ScrapeEvent::GameFailed {
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
