//! Folder-shaped scraping: scan a console directory, work out each ROM's
//! identity, and hand the result to the shared core in [`crate::session`].
//!
//! Everything specific to *folders* lives here — the directory scan, ROM
//! header analysis, on-demand hashing, multi-disc grouping, and assembling
//! ES-DE `ScrapedGame` metadata. The scrape itself does not: that is one
//! implementation shared with the GUI and the convergence executor.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use retro_junk_core::disc;
use retro_junk_core::{AnalysisOptions, Region, RomAnalyzer};
use retro_junk_frontend::{AssetSelection, AssetType, ScrapedGame};
use retro_junk_lib::scanner::{self, GameEntry};
use tokio::sync::mpsc;

use crate::derivation::Derivation;
use crate::error::ScrapeError;
use crate::log::{LogEntry, ScrapeLog};
use crate::lookup::{RomHashes, RomInfo};
use crate::session::{
    self, ArchivePublication, MiximageMode, ScrapeDestination, ScrapeEvent, ScrapeRequest,
    ScrapeSessionOptions, ScrapeTarget, TargetState,
};
use crate::systems;
use crate::types::GameInfo;

/// Options for a scraping session.
// Independent CLI-style flags; grouping them into enums would break the public API used by CLI/GUI.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
pub struct ScrapeOptions {
    /// Root path containing console folders
    pub root: PathBuf,
    /// Where this collection's portable marks live — the user's own decisions
    /// about which files are homebrew and which are mods of what.
    ///
    /// Defaults to the directory holding the ROM tree, which is where a
    /// collection profile puts them. A collection with no marks costs nothing.
    pub collection_root: Option<PathBuf>,
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
    /// Leave games that already have any selected media completely alone.
    /// Without it, a scrape still fills in the types that are missing.
    pub skip_existing: bool,
    /// Disable scrape log file
    pub no_log: bool,
    /// Maximum number of ROMs to process per console
    pub limit: Option<usize>,
    /// Whether and how to generate miximages
    pub miximage: MiximageMode,
    /// Force redownload of all media, ignoring existing files
    pub force_redownload: bool,
    /// Stop when the daily request budget falls to this many remaining.
    pub request_reserve: u32,
}

impl ScrapeOptions {
    /// Create default options for a root path.
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        let metadata_dir = retro_junk_lib::util::default_metadata_dir(&root);
        let media_dir = retro_junk_lib::util::default_media_dir(&root);
        let collection_root = root.parent().map(Path::to_path_buf);

        Self {
            root,
            collection_root,
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
            request_reserve: 0,
        }
    }
}

/// Result of scraping a single console folder.
#[derive(Debug)]
pub struct ScrapeResult {
    pub games: Vec<ScrapedGame>,
    pub log: ScrapeLog,
    /// Supporting files newly added to the archive (0 without a binding).
    pub published: usize,
}

/// Binds scraped media to archived releases so a folder scrape is
/// archive-first like every other surface.
pub struct FolderArchiveBinding<'a> {
    pub archive_root: &'a Path,
    /// Scratch directory for in-flight downloads.
    pub scratch_root: PathBuf,
    /// Release per playable filename, from [`session::release_ids_by_output`].
    pub releases_by_filename: HashMap<String, retro_junk_archive::ArchiveReleaseId>,
}

/// One folder scrape.
pub struct FolderScrapeRequest<'a> {
    pub client: &'a crate::client::ScreenScraperClient,
    pub folder_path: &'a Path,
    pub analyzer: &'a dyn RomAnalyzer,
    pub options: &'a ScrapeOptions,
    /// Console folder name; also the media/metadata subdirectory.
    pub folder_name: &'a str,
    pub max_workers: usize,
    pub archive: Option<&'a FolderArchiveBinding<'a>>,
    pub events: mpsc::UnboundedSender<ScrapeEvent>,
    pub cancel: &'a AtomicBool,
}

/// Per-target context the folder adapter needs once the core reports back.
struct TargetContext {
    rom_stem: String,
    filename: String,
    locale: GameLocale,
    /// Disc-group index when this target is a primary disc.
    primary_group: Option<usize>,
    /// The file's own identity, kept for unidentified reporting.
    rom: RomInfo,
    /// What the user decided this file is, which decides both how it was
    /// looked up and what the gamelist entry ends up called.
    derivation: Derivation,
    /// The name the user gave a derivative. A mod wears its parent's artwork
    /// but keeps its own name, or the collection lists two "Super Mario World"
    /// entries and only one of them is.
    name_override: String,
}

/// Scrape every ROM in one console folder.
pub async fn scrape_folder(request: FolderScrapeRequest<'_>) -> Result<ScrapeResult, ScrapeError> {
    let FolderScrapeRequest {
        client,
        folder_path,
        analyzer,
        options,
        folder_name,
        max_workers,
        archive,
        events,
        cancel,
    } = request;

    let platform = analyzer.platform();
    let short_name = platform.short_name();
    let system_id = systems::screenscraper_system_id(platform).ok_or_else(|| {
        ScrapeError::Config(format!("No ScreenScraper system ID for '{short_name}'"))
    })?;

    let extensions = scanner::extension_set(analyzer.file_extensions());
    let _ = events.send(ScrapeEvent::Scanning);
    let mut game_entries = scanner::scan_game_entries(folder_path, &extensions)
        .map_err(|error| ScrapeError::Config(format!("Error reading folder: {error}")))?;
    if let Some(max) = options.limit {
        game_entries.truncate(max);
    }

    let system_media_dir = options.media_dir.join(folder_name);
    let plan = plan_work(&game_entries);
    // The denominator callers render progress against is the work they will
    // actually see; secondary discs resolve without a lookup.
    let _ = events.send(ScrapeEvent::ScanComplete {
        total: plan.work_items.len(),
    });

    let marks = collection_marks(options);
    let mut log = ScrapeLog::new();
    let (targets, contexts) = build_targets(
        &plan,
        analyzer,
        options,
        &system_media_dir,
        archive,
        &marks,
        &events,
        &mut log,
    );

    let session_options = ScrapeSessionOptions {
        force_redownload: options.force_redownload,
        dry_run: options.dry_run,
        skip_scraped: options.skip_existing,
        language_fallback: options.language_fallback.clone(),
        miximage: options.miximage.clone(),
        request_reserve: options.request_reserve,
    };
    let publication = archive.map(|binding| ArchivePublication {
        archive_root: binding.archive_root,
        scratch_root: binding.scratch_root.clone(),
        acquire_lock: true,
    });
    let run = session::run_scrape(&ScrapeRequest {
        client,
        system_id,
        targets: &targets,
        selection: &options.asset_selection,
        options: &session_options,
        archive: publication.as_ref(),
        max_workers,
        events: &events,
        cancel,
    })
    .await;

    let mut games = Vec::new();
    let mut primary_results: HashMap<usize, ScrapedGame> = HashMap::new();
    for outcome in run.outcomes {
        let Some(context) = contexts.get(&outcome.key) else {
            continue;
        };
        let Some(scraped) = fold_outcome(options, context, outcome.state, &mut log) else {
            continue;
        };
        if let Some(group) = context.primary_group {
            primary_results.insert(group, scraped.clone());
        }
        games.push(scraped);
    }

    resolve_secondary_discs(
        &plan,
        &game_entries,
        &primary_results,
        &events,
        &mut games,
        &mut log,
    );

    Ok(ScrapeResult {
        games,
        log,
        published: run.published,
    })
}

/// Work out each scanned entry's identity and where its media belongs.
///
/// The per-entry ROM analysis and hashing happen here, once, before any
/// network call: the core takes identities, not files.
// One entry's identity, destination, and derivation are decided together;
// splitting the loop would mean re-deriving the parts from each other.
#[allow(clippy::too_many_arguments)]
fn build_targets(
    plan: &WorkPlan<'_>,
    analyzer: &dyn RomAnalyzer,
    options: &ScrapeOptions,
    system_media_dir: &Path,
    archive: Option<&FolderArchiveBinding<'_>>,
    marks: &retro_junk_archive::MarkIndex,
    events: &mpsc::UnboundedSender<ScrapeEvent>,
    log: &mut ScrapeLog,
) -> (Vec<ScrapeTarget>, HashMap<u64, TargetContext>) {
    let platform = analyzer.platform();
    let mut targets = Vec::with_capacity(plan.work_items.len());
    let mut contexts: HashMap<u64, TargetContext> = HashMap::new();

    for (position, (_, entry, rom_stem, primary_group)) in plan.work_items.iter().enumerate() {
        let key = position as u64;
        let filename = entry.display_name().to_string();
        let rom_path = entry.analysis_path();

        let analysis_options = AnalysisOptions::new().quick(true).file_path(rom_path);
        let (serial, rom_regions) = match std::fs::File::open(rom_path) {
            Ok(mut file) => match analyzer.analyze(&mut file, &analysis_options) {
                Ok(info) => (info.serial_number, info.regions),
                // An unreadable header is not fatal: filename and hash tiers
                // can still identify the game.
                Err(_) => (String::new(), Vec::new()),
            },
            Err(error) => {
                let message = format!("Failed to open file: {error}");
                let _ = events.send(ScrapeEvent::GameFailed {
                    index: position,
                    file: filename.clone(),
                    reason: message.clone(),
                });
                log.add(LogEntry::Error {
                    file: filename,
                    message,
                });
                continue;
            }
        };

        let scraper_serial = if serial.is_empty() {
            String::new()
        } else {
            analyzer.extract_scraper_serial(&serial).unwrap_or_default()
        };
        let rom = RomInfo {
            serial,
            scraper_serial,
            filename: filename.clone(),
            file_size: rom_path.metadata().map_or(0, |metadata| metadata.len()),
            hashes: compute_rom_hashes(analyzer, options, rom_path, &filename),
            platform,
            expects_serial: analyzer.expects_serial(),
        };
        let mark = file_mark(marks, &rom);
        let derivation = mark.map_or(Derivation::Own, Derivation::from_mark);
        // Only a mod needs its name defended: what came back describes its
        // parent. Homebrew was looked up as itself, so the scraper's name for
        // it is better than a filename.
        let name_override = match derivation {
            Derivation::Parent(_) | Derivation::UnknownParent => {
                mark.map_or_else(String::new, |mark| mark.name.clone())
            }
            Derivation::Own | Derivation::Standalone => String::new(),
        };
        let locale = GameLocale::from_rom(options, &rom_regions);
        // A file the archive built is scraped archive-first; anything else
        // (loose ROMs, unmanaged consoles) goes straight to the media tree.
        let destination = archive
            .and_then(|binding| binding.releases_by_filename.get(&filename).copied())
            .map_or_else(
                || ScrapeDestination::Playable {
                    media_dir: system_media_dir.to_path_buf(),
                },
                |release_id| ScrapeDestination::Archive {
                    release_id,
                    media_dir: system_media_dir.to_path_buf(),
                },
            );

        targets.push(ScrapeTarget {
            key,
            label: filename.clone(),
            rom_stem: rom_stem.clone(),
            region: locale.region.clone(),
            language: locale.language.clone(),
            rom: rom.clone(),
            derivation: derivation.clone(),
            destination,
            archived_assets: HashMap::new(),
        });
        contexts.insert(
            key,
            TargetContext {
                rom_stem: rom_stem.clone(),
                filename,
                locale,
                primary_group: *primary_group,
                rom,
                derivation,
                name_override,
            },
        );
    }
    (targets, contexts)
}

/// The user's own decisions about the files in this collection, read from
/// beside them rather than from a database: a folder scrape has to be
/// derivation-aware on a machine that has never imported a DAT.
///
/// Unreadable marks are a warning, not a failure — losing a scrape because a
/// curation file is malformed would be the wrong trade.
fn collection_marks(options: &ScrapeOptions) -> retro_junk_archive::MarkIndex {
    options
        .collection_root
        .as_deref()
        .map(retro_junk_archive::MarkIndex::load)
        .transpose()
        .unwrap_or_else(|error| {
            log::warn!("Could not read collection marks: {error}");
            None
        })
        .unwrap_or_default()
}

/// The user's decision about one scanned file, by content where this run
/// hashed it and by name and size where it did not.
///
/// Serial-expecting platforms skip hashing for speed, so the fallback is not
/// an edge case: on those consoles it is the only key available.
fn file_mark<'a>(
    marks: &'a retro_junk_archive::MarkIndex,
    rom: &RomInfo,
) -> Option<&'a retro_junk_archive::CollectionMark> {
    if marks.is_empty() {
        return None;
    }
    rom.hashes
        .as_ref()
        .and_then(|hashes| marks.find(&hashes.sha1, &hashes.crc32))
        .or_else(|| marks.find_by_name(&rom.filename, rom.file_size))
}

/// Turn one core verdict into a gamelist entry and a log line.
fn fold_outcome(
    options: &ScrapeOptions,
    context: &TargetContext,
    state: TargetState,
    log: &mut ScrapeLog,
) -> Option<ScrapedGame> {
    match state {
        TargetState::Scraped {
            game,
            method,
            warnings,
            assets,
        } => {
            let game_name = if context.name_override.is_empty() {
                game.name_for_region(&context.locale.region)
                    .unwrap_or("Unknown")
                    .to_owned()
            } else {
                context.name_override.clone()
            };
            let media_names = assets
                .keys()
                .map(|asset_type| asset_type.subdirectory().to_owned())
                .collect();
            log.add(if warnings.is_empty() {
                LogEntry::Success {
                    file: context.filename.clone(),
                    game_name: game_name.clone(),
                    method,
                    media_downloaded: media_names,
                }
            } else {
                LogEntry::Partial {
                    file: context.filename.clone(),
                    game_name: game_name.clone(),
                    warnings,
                }
            });
            Some(build_scraped_game(
                options, &game, context, game_name, assets,
            ))
        }
        // A dry run reports what it would do; it must not leave gamelist
        // entries behind as though it had done it.
        TargetState::Skipped { assets, .. } if !options.dry_run => Some(ScrapedGame {
            rom_stem: context.rom_stem.clone(),
            rom_filename: context.filename.clone(),
            name: context.filename.clone(),
            description: String::new(),
            developer: String::new(),
            publisher: String::new(),
            genre: String::new(),
            players: String::new(),
            rating: None,
            release_date: String::new(),
            assets,
            cover_title: String::new(),
        }),
        TargetState::Skipped { .. } | TargetState::NotReached => None,
        TargetState::NotFound { warnings } => {
            // Report what was actually offered, not what the file holds: for a
            // mod those are different, and a log claiming its own hashes were
            // tried would send the reader looking for a DAT entry that cannot
            // exist.
            let attempted = context
                .derivation
                .identify(&context.rom)
                .unwrap_or_else(|| context.rom.clone());
            let (crc32, md5, sha1) =
                attempted
                    .hashes
                    .as_ref()
                    .map_or_else(Default::default, |hashes| {
                        (
                            hashes.crc32.clone(),
                            hashes.md5.clone(),
                            hashes.sha1.clone(),
                        )
                    });
            log.add(LogEntry::Unidentified {
                file: context.filename.clone(),
                scraper_serial_tried: attempted.scraper_serial.clone(),
                serial_tried: attempted.serial.clone(),
                filename_tried: true,
                hashes_tried: attempted.hashes.is_some(),
                crc32,
                md5,
                sha1,
                errors: if warnings.is_empty() {
                    vec!["Game not found in ScreenScraper".to_owned()]
                } else {
                    warnings
                },
            });
            None
        }
        TargetState::Failed { message } => {
            log.add(LogEntry::Error {
                file: context.filename.clone(),
                message,
            });
            None
        }
    }
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

/// Compute the full CRC32+MD5+SHA1 triple when the lookup path needs it
/// (non-serial consoles or `force_hash`).
///
/// The hash lookup tier needs the whole triple, so a partial result counts as
/// no hashes.
fn compute_rom_hashes(
    analyzer: &dyn RomAnalyzer,
    options: &ScrapeOptions,
    rom_path: &Path,
    filename: &str,
) -> Option<RomHashes> {
    if systems::expects_serial(analyzer.platform()) && !options.force_hash {
        return None;
    }
    let mut file = std::fs::File::open(rom_path).ok()?;
    match retro_junk_lib::hasher::compute_all_hashes(&mut file, analyzer, Some(rom_path)) {
        Ok(hashes) => match (hashes.md5, hashes.sha1) {
            (Some(md5), Some(sha1)) => Some(RomHashes {
                crc32: hashes.crc32,
                md5,
                sha1,
            }),
            _ => None,
        },
        Err(error) => {
            log::debug!("Failed to hash {filename}: {error}");
            None
        }
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
            |region| systems::region_to_ss_code(region).to_string(),
        );
        let language = if options.language == "match" {
            rom_regions.first().map_or_else(
                || options.language_fallback.clone(),
                |region| systems::region_to_language(region).to_string(),
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
    context: &TargetContext,
    game_name: String,
    assets: HashMap<AssetType, PathBuf>,
) -> ScrapedGame {
    let locale = &context.locale;
    let description = localized(&locale.language, &options.language_fallback, |language| {
        game.synopsis_for_language(language)
    })
    .map(std::string::ToString::to_string)
    .unwrap_or_default();

    let genre = localized(&locale.language, &options.language_fallback, |language| {
        game.genre_for_language(language)
    })
    .unwrap_or_default();

    ScrapedGame {
        rom_stem: context.rom_stem.clone(),
        rom_filename: context.filename.clone(),
        name: game_name,
        description,
        developer: game
            .developpeur
            .as_ref()
            .map(|developer| developer.text.clone())
            .unwrap_or_default(),
        publisher: game
            .editeur
            .as_ref()
            .map(|publisher| publisher.text.clone())
            .unwrap_or_default(),
        genre,
        players: game
            .joueurs
            .as_ref()
            .map(|players| players.text.clone())
            .unwrap_or_default(),
        rating: game.rating_normalized(),
        release_date: game
            .date_for_region(&locale.region)
            .map(std::string::ToString::to_string)
            .unwrap_or_default(),
        assets,
        cover_title: String::new(),
    }
}

#[cfg(test)]
#[path = "tests/scrape_tests.rs"]
mod tests;
