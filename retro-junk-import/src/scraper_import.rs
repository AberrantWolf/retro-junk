//! Import metadata from ScreenScraper into the catalog database.
//!
//! For each release in the database, looks up the game on ScreenScraper
//! using media hashes/serials/filenames, then enriches the release with
//! metadata (title, dates, genre, description, publisher, developer, rating)
//! and optionally downloads media assets.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures::stream::{self, StreamExt};
use retro_junk_catalog::types::*;
use retro_junk_core::Platform;
use retro_junk_db::{operations, queries};
use retro_junk_scraper::client::ScreenScraperClient;
use retro_junk_scraper::error::ScrapeError;
use retro_junk_scraper::lookup::{self, LookupMethod, LookupResult, RomInfo};
use retro_junk_scraper::systems;
use retro_junk_scraper::types::GameInfo;
use tokio::time::Duration;

/// Per-item timeout wrapping each lookup future.
/// Covers the entire lookup (serial + filename + hash tiers).
/// Should be above LOOKUP_TIMEOUT in the scraper crate (60s).
const ITEM_TIMEOUT: Duration = Duration::from_secs(90);

/// Timeout for downloading and cataloging all assets for a single release.
/// Each individual download has a 120s timeout, but the full batch (~7 assets)
/// should not exceed this.
const ASSET_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// Watchdog timeout for the Phase 3 event loop.
/// If no worker result arrives within this duration, all workers are likely stuck
/// (e.g., after a laptop sleep killed connections). The loop breaks and returns
/// partial progress so the user can re-run.
const WATCHDOG_TIMEOUT: Duration = Duration::from_secs(180);

use crate::slugify;
use rusqlite::Connection;
use thiserror::Error;
use tokio::sync::mpsc::UnboundedSender;

use crate::merge;

#[derive(Debug, Error)]
pub enum EnrichError {
    #[error("Database error: {0}")]
    Db(#[from] operations::OperationError),
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Scraper error: {0}")]
    Scraper(#[from] ScrapeError),
    #[error("No platform mapping for '{0}' (missing core_platform)")]
    NoPlatformMapping(String),
    #[error("Unknown platform enum value: {0}")]
    UnknownPlatform(String),
    #[error("Import error: {0}")]
    Import(#[from] crate::ImportError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Options for the enrichment process.
pub struct EnrichOptions {
    /// Which platforms to enrich (empty = all platforms in DB).
    pub platform_ids: Vec<String>,
    /// Maximum releases to process per platform.
    pub limit: Option<u32>,
    /// Skip releases that already have a screenscraper_id.
    pub skip_existing: bool,
    /// Whether to download media assets.
    pub download_assets: bool,
    /// Directory for downloaded assets.
    pub asset_dir: Option<PathBuf>,
    /// Preferred region for name/media selection (e.g., "us", "eu", "jp").
    pub preferred_region: String,
    /// Preferred language for descriptions (e.g., "en", "ja").
    pub preferred_language: String,
}

impl Default for EnrichOptions {
    fn default() -> Self {
        Self {
            platform_ids: vec![],
            limit: None,
            skip_existing: true,
            download_assets: false,
            asset_dir: None,
            preferred_region: "us".to_string(),
            preferred_language: "en".to_string(),
        }
    }
}

/// Statistics from an enrichment run.
#[derive(Debug, Default, Clone)]
pub struct EnrichStats {
    pub releases_processed: u64,
    pub releases_enriched: u64,
    pub releases_not_found: u64,
    pub releases_skipped: u64,
    pub assets_downloaded: u64,
    pub disagreements_found: u64,
    pub companies_created: u64,
    pub errors: u64,
}

/// Events emitted during enrichment for real-time progress reporting.
#[derive(Debug)]
pub enum EnrichEvent {
    PlatformStarted {
        platform_id: String,
        platform_name: String,
        total: usize,
    },
    ReleaseFound {
        index: usize,
        title: String,
        ss_name: String,
        method: LookupMethod,
    },
    ReleaseNotFound {
        index: usize,
        title: String,
    },
    ReleaseSkipped {
        index: usize,
    },
    ReleaseError {
        index: usize,
        title: String,
        error: String,
    },
    FatalError {
        message: String,
    },
    PlatformDone {
        platform_id: String,
    },
    Done {
        stats: EnrichStats,
    },
}

// ── Three-Phase Enrichment Types ─────────────────────────────────────────

/// Pre-fetched work item for parallel API lookups.
struct EnrichWorkItem {
    index: usize,
    release: Release,
    media_entries: Vec<Media>,
    core_platform: Platform,
    system_id: u32,
}

/// Outcome of a single API lookup (Phase 2 result).
enum LookupOutcome {
    /// Game found — ready for DB enrichment.
    Found {
        index: usize,
        release: Release,
        result: LookupResult,
        mapped: MappedGameInfo,
    },
    /// ScreenScraper confirmed the game doesn't exist.
    NotFound { index: usize, release: Release },
    /// Release had no media entries or was otherwise skippable.
    Skipped { index: usize },
    /// Non-fatal error during lookup.
    Error {
        index: usize,
        release: Release,
        error: String,
    },
    /// Fatal error — must stop all processing (quota exceeded, server closed).
    Fatal { error: EnrichError },
}

/// Enrich releases in the database with ScreenScraper metadata.
///
/// Uses a three-phase batch model per platform to enable parallel API lookups
/// while keeping all DB access on the main thread (since `Connection` is `!Send`):
///
/// 1. **Phase 1 (DB Read):** Pre-fetch all releases + media entries (sequential)
/// 2. **Phase 2 (API Lookups):** Dispatch lookups via a worker pool (N persistent tasks)
/// 3. **Phase 3 (DB Write):** Apply results sequentially as they arrive
pub async fn enrich_releases(
    client: Arc<ScreenScraperClient>,
    conn: &Connection,
    options: &EnrichOptions,
    max_workers: usize,
    events: UnboundedSender<EnrichEvent>,
) -> Result<EnrichStats, EnrichError> {
    let mut stats = EnrichStats::default();

    // Determine which platforms to process
    let platform_ids = if options.platform_ids.is_empty() {
        queries::list_platforms(conn)?
            .into_iter()
            .map(|p| p.id)
            .collect()
    } else {
        options.platform_ids.clone()
    };

    for platform_id in &platform_ids {
        // Resolve platform to core Platform enum and ScreenScraper system ID
        let platform_row = queries::list_platforms(conn)?
            .into_iter()
            .find(|p| p.id == *platform_id);
        let platform_row = match platform_row {
            Some(p) => p,
            None => {
                log::warn!("Platform '{}' not found in database, skipping", platform_id);
                continue;
            }
        };

        let core_platform = match &platform_row.core_platform {
            Some(cp) => match cp.parse::<Platform>() {
                Ok(p) => p,
                Err(_) => {
                    log::warn!(
                        "Unknown core_platform '{}' for {}, skipping",
                        cp,
                        platform_id
                    );
                    continue;
                }
            },
            None => {
                log::debug!("No core_platform for {}, skipping enrichment", platform_id);
                continue;
            }
        };

        let system_id = match systems::screenscraper_system_id(core_platform) {
            Some(id) => id,
            None => {
                log::warn!(
                    "No ScreenScraper system ID for {:?}, skipping",
                    core_platform
                );
                continue;
            }
        };

        // If --force, clear not-found flags so they get re-queried
        if !options.skip_existing {
            let cleared = operations::clear_not_found_flags(conn, platform_id)?;
            if cleared > 0 {
                log::info!("Cleared {} not-found flags for {}", cleared, platform_id);
            }
        }

        // ── Phase 1: DB Read — pre-fetch releases + media ──────────────
        let releases =
            queries::releases_to_enrich(conn, platform_id, options.skip_existing, options.limit)?;

        if releases.is_empty() {
            log::debug!("No releases to enrich for {}", platform_id);
            continue;
        }

        log::debug!(
            "Enriching {} releases for {} (system {}, {} workers)",
            releases.len(),
            platform_row.display_name,
            system_id,
            max_workers,
        );

        let total = releases.len();
        let mut work_items = Vec::with_capacity(total);

        let _ = events.send(EnrichEvent::PlatformStarted {
            platform_id: platform_id.clone(),
            platform_name: platform_row.display_name.clone(),
            total,
        });

        for (i, release) in releases.into_iter().enumerate() {
            let media_entries = queries::media_for_release(conn, &release.id)?;
            work_items.push(EnrichWorkItem {
                index: i,
                release,
                media_entries,
                core_platform,
                system_id,
            });
        }

        // ── Phase 2+3: Parallel lookups with inline DB writes ────────
        let cancel = Arc::new(AtomicBool::new(false));

        let pool_client = client.clone();
        let pool_cancel = cancel.clone();
        let pool_region = options.preferred_region.clone();
        let pool_language = options.preferred_language.clone();

        let mut stream = stream::iter(work_items)
            .map(move |item| {
                let client = pool_client.clone();
                let cancel = pool_cancel.clone();
                let preferred_region = pool_region.clone();
                let preferred_language = pool_language.clone();
                // Spawn each lookup as an independent tokio task so it makes
                // progress regardless of whether the stream is being polled.
                // buffer_unordered still controls concurrency: it only pulls
                // (and spawns) a new item when a JoinHandle resolves.
                tokio::spawn(async move {
                    if cancel.load(Ordering::Relaxed) {
                        return LookupOutcome::Skipped { index: item.index };
                    }
                    if item.media_entries.is_empty() {
                        return LookupOutcome::Skipped { index: item.index };
                    }

                    let best_media = pick_best_media_for_lookup(&item.media_entries);
                    let rom_info = build_rom_info(best_media, item.core_platform);

                    match tokio::time::timeout(
                        ITEM_TIMEOUT,
                        lookup::lookup_game(&client, item.system_id, &rom_info),
                    )
                    .await
                    {
                        Ok(Ok(result)) => {
                            let mapped = map_game_info(
                                &result.game,
                                &preferred_region,
                                &preferred_language,
                            );
                            LookupOutcome::Found {
                                index: item.index,
                                release: item.release,
                                result,
                                mapped,
                            }
                        }
                        Ok(Err(ScrapeError::NotFound { .. })) => LookupOutcome::NotFound {
                            index: item.index,
                            release: item.release,
                        },
                        Ok(Err(e @ ScrapeError::QuotaExceeded { .. }))
                        | Ok(Err(e @ ScrapeError::ServerClosed(_))) => {
                            cancel.store(true, Ordering::Relaxed);
                            LookupOutcome::Fatal {
                                error: EnrichError::Scraper(e),
                            }
                        }
                        Ok(Err(e)) => LookupOutcome::Error {
                            index: item.index,
                            release: item.release,
                            error: e.to_string(),
                        },
                        Err(_) => LookupOutcome::Error {
                            index: item.index,
                            release: item.release,
                            error: format!(
                                "Lookup timed out after {}s",
                                ITEM_TIMEOUT.as_secs()
                            ),
                        },
                    }
                })
            })
            .buffer_unordered(max_workers);

        // Process results as they arrive
        let mut fatal_error = None;
        let mut consecutive_errors: u32 = 0;
        const CIRCUIT_BREAKER_THRESHOLD: u32 = 15;

        loop {
            let outcome = match tokio::time::timeout(WATCHDOG_TIMEOUT, stream.next()).await {
                Ok(Some(Ok(outcome))) => outcome,
                Ok(Some(Err(join_err))) => {
                    // Spawned task panicked — log and continue
                    log::debug!("Lookup task panicked: {}", join_err);
                    stats.releases_processed += 1;
                    stats.errors += 1;
                    continue;
                }
                Ok(None) => break,
                Err(_watchdog) => {
                    let msg = format!(
                        "Watchdog: no result in {}s, stopping with partial progress.",
                        WATCHDOG_TIMEOUT.as_secs(),
                    );
                    let _ = events.send(EnrichEvent::FatalError {
                        message: msg.clone(),
                    });
                    log::debug!("{}", msg);
                    break;
                }
            };
            stats.releases_processed += 1;

            match outcome {
                LookupOutcome::Found {
                    index,
                    release,
                    result,
                    mapped,
                } => {
                    consecutive_errors = 0;
                    let game = &result.game;

                    // Handle companies (find or create)
                    let publisher_id = if let Some(ref pub_name) = mapped.publisher {
                        Some(find_or_create_company(conn, pub_name, &mut stats)?)
                    } else {
                        None
                    };
                    let developer_id = if let Some(ref dev_name) = mapped.developer {
                        Some(find_or_create_company(conn, dev_name, &mut stats)?)
                    } else {
                        None
                    };

                    // Check for disagreements with existing data
                    let disagreement_count = merge::merge_release_fields(
                        conn,
                        &release.id,
                        &release,
                        "screenscraper",
                        mapped.title.as_deref(),
                        mapped.release_date.as_deref(),
                        mapped.genre.as_deref(),
                        mapped.players.as_deref(),
                        mapped.description.as_deref(),
                    )?;
                    stats.disagreements_found += disagreement_count as u64;

                    // Update release with enrichment data
                    operations::update_release_enrichment(
                        conn,
                        &release.id,
                        &game.id,
                        mapped.title.as_deref(),
                        mapped.release_date.as_deref(),
                        mapped.genre.as_deref(),
                        mapped.players.as_deref(),
                        mapped.rating,
                        mapped.description.as_deref(),
                        publisher_id.as_deref(),
                        developer_id.as_deref(),
                    )?;

                    // Download assets if requested
                    if options.download_assets {
                        if let Some(ref asset_dir) = options.asset_dir {
                            match tokio::time::timeout(
                                ASSET_DOWNLOAD_TIMEOUT,
                                download_and_catalog_assets(
                                    &client,
                                    conn,
                                    game,
                                    &release.id,
                                    asset_dir,
                                    &options.preferred_region,
                                ),
                            ).await {
                                Ok(Ok(downloaded)) => {
                                    stats.assets_downloaded += downloaded as u64;
                                }
                                Ok(Err(e)) => {
                                    log::debug!(
                                        "Asset download failed for '{}': {}",
                                        release.title, e
                                    );
                                }
                                Err(_timeout) => {
                                    log::debug!(
                                        "Asset download timed out after {}s for '{}'",
                                        ASSET_DOWNLOAD_TIMEOUT.as_secs(),
                                        release.title,
                                    );
                                }
                            }
                        }
                    }

                    stats.releases_enriched += 1;

                    let ss_name = result
                        .game
                        .name_for_region("us")
                        .unwrap_or(&release.title)
                        .to_string();

                    log::debug!(
                        "Enriched '{}' via {} (SS id: {}, {} disagreements)",
                        release.title,
                        result.method,
                        result.game.id,
                        disagreement_count,
                    );

                    let _ = events.send(EnrichEvent::ReleaseFound {
                        index,
                        title: release.title.clone(),
                        ss_name,
                        method: result.method,
                    });
                }
                LookupOutcome::NotFound { index, release } => {
                    consecutive_errors = 0;
                    let _ = events.send(EnrichEvent::ReleaseNotFound {
                        index,
                        title: release.title.clone(),
                    });
                    operations::mark_release_not_found(conn, &release.id)?;
                    stats.releases_not_found += 1;
                }
                LookupOutcome::Skipped { index } => {
                    // Skipped items intentionally do no DB write — releases remain
                    // retryable on the next enrichment run.
                    let _ = events.send(EnrichEvent::ReleaseSkipped { index });
                    stats.releases_skipped += 1;
                }
                LookupOutcome::Error {
                    index,
                    release,
                    error,
                } => {
                    consecutive_errors += 1;
                    let _ = events.send(EnrichEvent::ReleaseError {
                        index,
                        title: release.title.clone(),
                        error: error.clone(),
                    });
                    log::debug!("Error enriching '{}': {}", release.title, error);
                    stats.errors += 1;

                    // Circuit breaker: too many consecutive errors means the API
                    // is likely down. Set cancel flag so remaining items are skipped
                    // (no DB write → retryable on next run).
                    if consecutive_errors >= CIRCUIT_BREAKER_THRESHOLD {
                        cancel.store(true, Ordering::Relaxed);
                        let msg = format!(
                            "Circuit breaker: {} consecutive errors, stopping platform",
                            consecutive_errors,
                        );
                        let _ = events.send(EnrichEvent::FatalError {
                            message: msg.clone(),
                        });
                        log::debug!("{}", msg);
                    }
                }
                LookupOutcome::Fatal { error } => {
                    let _ = events.send(EnrichEvent::FatalError {
                        message: error.to_string(),
                    });
                    log::debug!("Fatal error during enrichment: {}", error);
                    fatal_error = Some(error);
                    // Don't break — remaining outcomes are Skipped due to cancel flag
                }
            }
        }

        let _ = events.send(EnrichEvent::PlatformDone {
            platform_id: platform_id.clone(),
        });

        if let Some(err) = fatal_error {
            // Log the fatal error details
            match &err {
                EnrichError::Scraper(ScrapeError::QuotaExceeded { used, max }) => {
                    log::debug!(
                        "ScreenScraper daily quota exceeded ({}/{}), stopping",
                        used,
                        max
                    );
                }
                EnrichError::Scraper(ScrapeError::ServerClosed(_)) => {
                    log::debug!("ScreenScraper API is closed, stopping");
                }
                _ => {}
            }
            let _ = events.send(EnrichEvent::Done {
                stats: stats.clone(),
            });
            return Ok(stats);
        }
    }

    let _ = events.send(EnrichEvent::Done {
        stats: stats.clone(),
    });

    Ok(stats)
}

// ── Mapping Functions ───────────────────────────────────────────────────────

/// Fields extracted from a GameInfo response.
pub struct MappedGameInfo {
    pub title: Option<String>,
    pub release_date: Option<String>,
    pub genre: Option<String>,
    pub players: Option<String>,
    pub rating: Option<f64>,
    pub description: Option<String>,
    pub publisher: Option<String>,
    pub developer: Option<String>,
}

/// Extract release-relevant fields from a ScreenScraper GameInfo response.
pub fn map_game_info(game: &GameInfo, region: &str, language: &str) -> MappedGameInfo {
    let ss_region = catalog_region_to_ss(region);

    MappedGameInfo {
        title: game.name_for_region(ss_region).map(|s| s.to_string()),
        release_date: game
            .date_for_region(ss_region)
            .map(|s| normalize_date(s)),
        genre: game.genre_for_language(language),
        players: game.joueurs.as_ref().map(|j| j.text.clone()),
        rating: game.rating_normalized().map(|r| r as f64),
        description: game.synopsis_for_language(language).map(|s| s.to_string()),
        publisher: game.editeur.as_ref().map(|e| e.text.clone()),
        developer: game.developpeur.as_ref().map(|d| d.text.clone()),
    }
}

/// Pick the best media entry for looking up a game on ScreenScraper.
///
/// Prefers entries with SHA1 hashes, then CRC32, then serial, then any.
fn pick_best_media_for_lookup(media: &[Media]) -> &Media {
    // Prefer media with SHA1 hash (strongest match)
    if let Some(m) = media.iter().find(|m| m.sha1.is_some()) {
        return m;
    }
    // Then CRC32
    if let Some(m) = media.iter().find(|m| m.crc32.is_some()) {
        return m;
    }
    // Then serial
    if let Some(m) = media.iter().find(|m| m.media_serial.is_some()) {
        return m;
    }
    // Fallback to first
    &media[0]
}

/// Build a RomInfo struct from catalog Media data.
fn build_rom_info(media: &Media, platform: Platform) -> RomInfo {
    let filename = media
        .dat_name
        .as_deref()
        .unwrap_or("")
        .to_string();

    RomInfo {
        serial: media.media_serial.clone(),
        scraper_serial: media.media_serial.clone(),
        filename,
        file_size: media.file_size.unwrap_or(0) as u64,
        crc32: media.crc32.clone().map(|s| s.to_uppercase()),
        md5: media.md5.clone(),
        sha1: media.sha1.clone(),
        platform,
        expects_serial: systems::expects_serial(platform),
    }
}

/// Find or create a company in the database by name.
///
/// First checks aliases, then creates a new company if not found.
fn find_or_create_company(
    conn: &Connection,
    name: &str,
    stats: &mut EnrichStats,
) -> Result<String, EnrichError> {
    // Check if a company with this alias already exists
    if let Some(company_id) = operations::find_company_by_alias(conn, name)? {
        return Ok(company_id);
    }

    // Also check by exact company name
    let slug = slugify(name);
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM companies WHERE id = ?1)",
        [&slug],
        |row| row.get(0),
    )?;
    if exists {
        return Ok(slug);
    }

    // Create new company
    let company = Company {
        id: slug.clone(),
        name: name.to_string(),
        country: None,
        aliases: vec![name.to_string()],
    };
    operations::upsert_company(conn, &company)?;
    stats.companies_created += 1;
    log::debug!("Created new company: {} ({})", name, slug);

    Ok(slug)
}

/// Download media assets and create catalog records.
async fn download_and_catalog_assets(
    client: &ScreenScraperClient,
    conn: &Connection,
    game: &GameInfo,
    release_id: &str,
    asset_dir: &Path,
    preferred_region: &str,
) -> Result<usize, EnrichError> {
    let ss_region = catalog_region_to_ss(preferred_region);
    let mut downloaded = 0;

    // Asset types to download and their ScreenScraper media type names
    let asset_mappings: &[(&str, &str)] = &[
        ("box-front", "box-2D"),
        ("box-back", "box-2D-back"),
        ("screenshot", "ss"),
        ("title-screen", "sstitle"),
        ("wheel", "wheel-hd"),
        ("fanart", "fanart"),
        ("cart-front", "support-2D"),
    ];

    for &(asset_type, ss_type) in asset_mappings {
        let media = match game.media_for_region(ss_type, ss_region) {
            Some(m) => m,
            None => {
                // Try wheel as fallback for wheel-hd
                if ss_type == "wheel-hd" {
                    match game.media_for_region("wheel", ss_region) {
                        Some(m) => m,
                        None => continue,
                    }
                } else {
                    continue;
                }
            }
        };

        let url = &media.url;
        let extension = if media.format.is_empty() {
            "png"
        } else {
            &media.format
        };

        // Create directory structure: asset_dir/release_id/
        let release_dir = asset_dir.join(release_id);
        std::fs::create_dir_all(&release_dir)?;

        let file_name = format!("{}.{}", asset_type, extension);
        let file_path = release_dir.join(&file_name);

        // Skip if already downloaded
        if file_path.exists() {
            continue;
        }

        // Download
        match client.download_media(url).await {
            Ok(data) => {
                std::fs::write(&file_path, &data)?;

                // Record in media_assets table
                let asset = MediaAsset {
                    id: 0,
                    release_id: Some(release_id.to_string()),
                    media_id: None,
                    asset_type: asset_type.to_string(),
                    region: Some(media.region.clone()),
                    source: "screenscraper".to_string(),
                    file_path: Some(file_path.to_string_lossy().to_string()),
                    source_url: Some(url.clone()),
                    scraped: true,
                    file_hash: None,
                    width: None,
                    height: None,
                    created_at: String::new(),
                };
                operations::insert_media_asset(conn, &asset)?;
                downloaded += 1;
            }
            Err(e) => {
                log::debug!(
                    "Failed to download {} for {}: {}",
                    asset_type,
                    release_id,
                    e
                );
            }
        }
    }

    Ok(downloaded)
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Map catalog region slug to ScreenScraper region code.
pub fn catalog_region_to_ss(region: &str) -> &str {
    match region {
        "usa" | "us" => "us",
        "europe" | "eu" => "eu",
        "japan" | "jp" => "jp",
        "australia" | "au" => "au",
        "korea" | "kr" => "kr",
        "china" | "cn" => "cn",
        "taiwan" | "tw" => "tw",
        "brazil" | "br" => "br",
        "world" | "wor" => "wor",
        _ => "us",
    }
}

/// Map ScreenScraper region code back to catalog region slug.
pub fn ss_region_to_catalog(ss_region: &str) -> &str {
    match ss_region {
        "us" => "usa",
        "eu" => "europe",
        "jp" => "japan",
        "au" => "australia",
        "kr" => "korea",
        "cn" => "china",
        "tw" => "taiwan",
        "br" => "brazil",
        "wor" => "world",
        _ => "unknown",
    }
}

/// Normalize a date from ScreenScraper format to YYYY-MM-DD.
///
/// ScreenScraper dates can be: "YYYY-MM-DD", "YYYY-MM", "YYYY", or other formats.
fn normalize_date(date: &str) -> String {
    // Already in good format
    let trimmed = date.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    trimmed.to_string()
}

/// Map ScreenScraper media type to our asset_type string.
pub fn ss_media_type_to_asset_type(ss_type: &str) -> Option<&'static str> {
    match ss_type {
        "box-2D" => Some("box-front"),
        "box-2D-back" => Some("box-back"),
        "box-3D" => Some("box-3d"),
        "ss" => Some("screenshot"),
        "sstitle" => Some("title-screen"),
        "wheel-hd" | "wheel" => Some("wheel"),
        "fanart" => Some("fanart"),
        "support-2D" => Some("cart-front"),
        "video-normalized" | "video" => Some("video"),
        "miximage" => Some("miximage"),
        _ => None,
    }
}
