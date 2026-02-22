//! Import metadata from ScreenScraper into the catalog database.
//!
//! For each release in the database, looks up the game on ScreenScraper
//! using media hashes/serials/filenames, then enriches the release with
//! metadata (title, dates, genre, description, publisher, developer, rating)
//! and optionally downloads media assets.

use std::path::{Path, PathBuf};

use retro_junk_catalog::types::*;
use retro_junk_core::Platform;
use retro_junk_db::{operations, queries};
use retro_junk_scraper::client::ScreenScraperClient;
use retro_junk_scraper::error::ScrapeError;
use retro_junk_scraper::lookup::{self, LookupMethod, RomInfo};
use retro_junk_scraper::systems;
use retro_junk_scraper::types::GameInfo;
use rusqlite::Connection;
use thiserror::Error;

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
    /// Force hash-based lookup even for serial consoles.
    pub force_hash: bool,
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
            force_hash: false,
        }
    }
}

/// Statistics from an enrichment run.
#[derive(Debug, Default)]
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

/// Progress callbacks for the enrichment process.
pub trait EnrichProgress {
    fn on_release(&self, current: usize, total: usize, title: &str);
    fn on_found(&self, current: usize, title: &str, ss_name: &str, method: &LookupMethod);
    fn on_not_found(&self, current: usize, title: &str);
    fn on_asset_downloaded(&self, current: usize, title: &str, asset_type: &str);
    fn on_error(&self, current: usize, title: &str, error: &str);
    fn on_complete(&self, stats: &EnrichStats);
}

/// Silent progress — no output.
pub struct SilentEnrichProgress;

impl EnrichProgress for SilentEnrichProgress {
    fn on_release(&self, _: usize, _: usize, _: &str) {}
    fn on_found(&self, _: usize, _: &str, _: &str, _: &LookupMethod) {}
    fn on_not_found(&self, _: usize, _: &str) {}
    fn on_asset_downloaded(&self, _: usize, _: &str, _: &str) {}
    fn on_error(&self, _: usize, _: &str, _: &str) {}
    fn on_complete(&self, _: &EnrichStats) {}
}

/// Enrich releases in the database with ScreenScraper metadata.
///
/// This is the main entry point for Phase 3. It queries the database for
/// releases to enrich, looks each up on ScreenScraper, and writes back
/// the metadata. Since ScreenScraper has aggressive rate limiting (~1 req/1.2s),
/// this processes releases sequentially.
///
/// Note: `Connection` is `!Send`, so this future is `!Send`. Call it from
/// the main tokio thread or use `spawn_local`.
pub async fn enrich_releases(
    client: &ScreenScraperClient,
    conn: &Connection,
    options: &EnrichOptions,
    progress: Option<&dyn EnrichProgress>,
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

        // Get releases to enrich for this platform
        let releases =
            queries::releases_to_enrich(conn, platform_id, options.skip_existing, options.limit)?;

        if releases.is_empty() {
            log::debug!("No releases to enrich for {}", platform_id);
            continue;
        }

        log::info!(
            "Enriching {} releases for {} (system {})",
            releases.len(),
            platform_row.display_name,
            system_id,
        );

        for (i, release) in releases.iter().enumerate() {
            if let Some(p) = progress {
                p.on_release(i + 1, releases.len(), &release.title);
            }

            stats.releases_processed += 1;

            match enrich_single_release(
                client,
                conn,
                &release,
                core_platform,
                system_id,
                options,
                &mut stats,
            )
            .await
            {
                Ok(true) => {
                    // Successfully enriched
                }
                Ok(false) => {
                    // Not found
                    if let Some(p) = progress {
                        p.on_not_found(i + 1, &release.title);
                    }
                }
                Err(EnrichError::Scraper(ScrapeError::QuotaExceeded { used, max })) => {
                    log::warn!(
                        "ScreenScraper daily quota exceeded ({}/{}), stopping",
                        used,
                        max
                    );
                    if let Some(p) = progress {
                        p.on_complete(&stats);
                    }
                    return Ok(stats);
                }
                Err(EnrichError::Scraper(ScrapeError::ServerClosed(_))) => {
                    log::warn!("ScreenScraper API is closed, stopping");
                    if let Some(p) = progress {
                        p.on_complete(&stats);
                    }
                    return Ok(stats);
                }
                Err(e) => {
                    log::warn!("Error enriching '{}': {}", release.title, e);
                    if let Some(p) = progress {
                        p.on_error(i + 1, &release.title, &e.to_string());
                    }
                    stats.errors += 1;
                }
            }
        }
    }

    if let Some(p) = progress {
        p.on_complete(&stats);
    }

    Ok(stats)
}

/// Enrich a single release. Returns `true` if the game was found and enriched.
async fn enrich_single_release(
    client: &ScreenScraperClient,
    conn: &Connection,
    release: &Release,
    core_platform: Platform,
    system_id: u32,
    options: &EnrichOptions,
    stats: &mut EnrichStats,
) -> Result<bool, EnrichError> {
    // Get media entries for this release to find lookup data
    let media_entries = queries::media_for_release(conn, &release.id)?;
    if media_entries.is_empty() {
        stats.releases_skipped += 1;
        return Ok(false);
    }

    // Pick the best media entry for lookup (prefer one with hashes)
    let best_media = pick_best_media_for_lookup(&media_entries);

    // Build RomInfo for the scraper lookup
    let rom_info = build_rom_info(best_media, core_platform);

    // Look up the game on ScreenScraper
    let result = match lookup::lookup_game(client, system_id, &rom_info, options.force_hash).await {
        Ok(r) => r,
        Err(ScrapeError::NotFound { .. }) => {
            stats.releases_not_found += 1;
            return Ok(false);
        }
        Err(e) => return Err(e.into()),
    };

    let game = &result.game;

    // Map GameInfo to release fields
    let mapped = map_game_info(game, &options.preferred_region, &options.preferred_language);

    // Handle companies (find or create)
    let publisher_id = if let Some(ref pub_name) = mapped.publisher {
        Some(find_or_create_company(conn, pub_name, stats)?)
    } else {
        None
    };
    let developer_id = if let Some(ref dev_name) = mapped.developer {
        Some(find_or_create_company(conn, dev_name, stats)?)
    } else {
        None
    };

    // Check for disagreements with existing data
    let disagreement_count = merge::merge_release_fields(
        conn,
        &release.id,
        release,
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
            let downloaded = download_and_catalog_assets(
                client,
                conn,
                game,
                &release.id,
                asset_dir,
                &options.preferred_region,
            )
            .await?;
            stats.assets_downloaded += downloaded as u64;
        }
    }

    stats.releases_enriched += 1;

    log::debug!(
        "Enriched '{}' via {} (SS id: {}, {} disagreements)",
        release.title,
        result.method,
        game.id,
        disagreement_count,
    );

    Ok(true)
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
    let slug = slugify_company(name);
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

/// Generate a slug for a company name.
fn slugify_company(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    let mut last_was_separator = false;

    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            result.push(c.to_ascii_lowercase());
            last_was_separator = false;
        } else if !last_was_separator && !result.is_empty() {
            result.push('-');
            last_was_separator = true;
        }
    }

    if result.ends_with('-') {
        result.pop();
    }

    result
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
