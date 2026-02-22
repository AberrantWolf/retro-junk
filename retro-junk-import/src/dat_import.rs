//! Import DAT file entries into the catalog database.
//!
//! Each `DatGame` is parsed via the name parser to extract title, region, revision,
//! and status. These are mapped to Work → Release → Media entities in the database.

use retro_junk_catalog::name_parser::{self, DumpStatus};
use retro_junk_catalog::types::*;
use retro_junk_core::Platform;
use retro_junk_dat::DatFile;
use retro_junk_db::operations::{self, OperationError};
use rusqlite::Connection;
use thiserror::Error;

use crate::progress::ImportProgress;

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("Database error: {0}")]
    Db(#[from] OperationError),
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("No platform mapping for DAT: {0}")]
    UnknownPlatform(String),
}

/// Statistics from a single DAT import.
#[derive(Debug, Default)]
pub struct ImportStats {
    pub works_created: u64,
    pub works_existing: u64,
    pub releases_created: u64,
    pub releases_existing: u64,
    pub media_created: u64,
    pub media_updated: u64,
    pub media_unchanged: u64,
    pub skipped_bad: u64,
    pub total_games: u64,
    pub disagreements_found: u64,
}

/// Import a parsed DAT file into the catalog database.
///
/// `platform` identifies the target platform (converted to string at the DB boundary).
/// `dat_source` is "no-intro" or "redump".
///
/// The optional `progress` callback is invoked after each game is processed.
pub fn import_dat(
    conn: &Connection,
    dat: &DatFile,
    platform: Platform,
    dat_source: &str,
    progress: Option<&dyn ImportProgress>,
) -> Result<ImportStats, ImportError> {
    let mut stats = ImportStats::default();
    stats.total_games = dat.games.len() as u64;

    let tx = conn.unchecked_transaction()?;

    for (i, game) in dat.games.iter().enumerate() {
        import_game(&tx, game, platform, dat_source, &mut stats)?;

        if let Some(p) = progress {
            p.on_game(i + 1, dat.games.len(), &game.name);
        }
    }

    tx.commit()?;

    Ok(stats)
}

/// Import a single DatGame entry.
fn import_game(
    conn: &Connection,
    game: &retro_junk_dat::DatGame,
    platform: Platform,
    dat_source: &str,
    stats: &mut ImportStats,
) -> Result<(), ImportError> {
    let platform_id = platform.short_name();
    let parsed = name_parser::parse_dat_name(&game.name);

    // Skip bad dumps by default
    if parsed.status == DumpStatus::BadDump {
        stats.skipped_bad += 1;
        return Ok(());
    }

    // Determine the status
    let status = match parsed.status {
        DumpStatus::Verified => {
            // Check flags for proto/beta/sample
            if parsed.flags.iter().any(|f| {
                let lower = f.to_lowercase();
                lower == "proto" || lower == "prototype"
            }) {
                MediaStatus::Prototype
            } else if parsed.flags.iter().any(|f| f.to_lowercase() == "beta") {
                MediaStatus::Beta
            } else if parsed.flags.iter().any(|f| f.to_lowercase() == "sample") {
                MediaStatus::Sample
            } else {
                MediaStatus::Verified
            }
        }
        DumpStatus::BadDump => MediaStatus::Bad,
        DumpStatus::Overdump => MediaStatus::Overdump,
    };

    // Determine canonical title for the Work
    let canonical_title = parsed.title.clone();
    if canonical_title.is_empty() {
        // Edge case: some DAT entries have no parseable title
        log::warn!("Skipping DAT entry with empty title: {}", game.name);
        return Ok(());
    }

    // Generate work ID from title + platform
    let work_id = make_work_id(&canonical_title, platform_id);

    // Find or create Work (check by generated ID, not by name, to avoid
    // false positives from cross-platform titles like "Tetris")
    let work_exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM works WHERE id = ?1)",
        [&work_id],
        |row| row.get(0),
    )?;
    if work_exists {
        stats.works_existing += 1;
    } else {
        operations::insert_work(conn, &work_id, &canonical_title)?;
        stats.works_created += 1;
    }

    // Determine regions — use parsed regions, fallback to DAT-level region or "unknown"
    let regions = if !parsed.regions.is_empty() {
        parsed
            .regions
            .iter()
            .map(|r| name_parser::region_to_slug(r).to_string())
            .collect::<Vec<_>>()
    } else if let Some(ref dat_region) = game.region {
        vec![name_parser::region_to_slug(dat_region).to_string()]
    } else {
        vec!["unknown".to_string()]
    };

    // For multi-region games, use the first region as the primary release region
    // (e.g., "USA, Europe" → release for "usa")
    let primary_region = &regions[0];
    let release_id = make_release_id(&work_id, platform_id, primary_region);

    // Find or create Release
    let existing_release =
        operations::find_release(conn, &work_id, platform_id, primary_region)?;
    if existing_release.is_some() {
        stats.releases_existing += 1;
    } else {
        let release = Release {
            id: release_id.clone(),
            work_id: work_id.clone(),
            platform_id: platform_id.to_string(),
            region: primary_region.clone(),
            title: parsed.title.clone(),
            alt_title: None,
            publisher_id: None,
            developer_id: None,
            release_date: None,
            game_serial: None,
            genre: None,
            players: None,
            rating: None,
            description: None,
            screenscraper_id: None,
            created_at: String::new(),
            updated_at: String::new(),
        };
        operations::upsert_release(conn, &release)?;
        stats.releases_created += 1;
    }

    // Create Media entries — one per ROM in the DatGame
    for rom in &game.roms {
        let media_id = make_media_id(&release_id, &rom.name);

        // Check if this media already exists
        let existing = operations::find_media_by_dat_name(conn, &game.name)?;
        if let Some(ref existing_media) = existing {
            // Check if anything changed
            let same_hashes = existing_media.crc32.as_deref() == Some(&rom.crc)
                && existing_media.sha1.as_deref() == rom.sha1.as_deref()
                && existing_media.file_size == Some(rom.size as i64);
            if same_hashes {
                stats.media_unchanged += 1;
                continue;
            }
            stats.media_updated += 1;
        } else {
            stats.media_created += 1;
        }

        let media = Media {
            id: media_id,
            release_id: release_id.clone(),
            media_serial: rom.serial.clone(),
            disc_number: parsed.disc_number.map(|n| n as i32),
            disc_label: parsed.disc_label.clone(),
            revision: parsed.revision.clone(),
            status,
            dat_name: Some(game.name.clone()),
            dat_source: Some(dat_source.to_string()),
            file_size: Some(rom.size as i64),
            crc32: Some(rom.crc.clone()),
            sha1: rom.sha1.clone(),
            md5: rom.md5.clone(),
            created_at: String::new(),
            updated_at: String::new(),
        };
        operations::upsert_media(conn, &media)?;
    }

    Ok(())
}

/// Log an import run in the import_log table.
pub fn log_import(
    conn: &Connection,
    source_type: &str,
    source_name: &str,
    source_version: Option<&str>,
    stats: &ImportStats,
) -> Result<i64, ImportError> {
    let now = chrono::Utc::now().to_rfc3339();
    let log_entry = ImportLog {
        id: 0,
        source_type: source_type.to_string(),
        source_name: source_name.to_string(),
        source_version: source_version.map(|s| s.to_string()),
        imported_at: now,
        records_created: stats.media_created as i64,
        records_updated: stats.media_updated as i64,
        records_unchanged: stats.media_unchanged as i64,
        disagreements_found: stats.disagreements_found as i64,
    };
    let id = operations::insert_import_log(conn, &log_entry)?;
    Ok(id)
}

// ── ID Generation ───────────────────────────────────────────────────────────

/// Generate a stable work ID from title and platform.
///
/// Uses a simple slug: lowercase, alphanumeric + hyphens.
fn make_work_id(title: &str, platform_id: &str) -> String {
    let slug = slugify(title);
    format!("{platform_id}:{slug}")
}

/// Generate a stable release ID from work + platform + region.
fn make_release_id(work_id: &str, platform_id: &str, region: &str) -> String {
    format!("{work_id}:{platform_id}:{region}")
}

/// Generate a stable media ID from release + ROM name.
fn make_media_id(release_id: &str, rom_name: &str) -> String {
    let slug = slugify(rom_name);
    format!("{release_id}:{slug}")
}

/// Convert a string to a URL-safe slug.
fn slugify(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut last_was_separator = false;

    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            result.push(c.to_ascii_lowercase());
            last_was_separator = false;
        } else if !last_was_separator && !result.is_empty() {
            result.push('-');
            last_was_separator = true;
        }
    }

    // Trim trailing separator
    if result.ends_with('-') {
        result.pop();
    }

    result
}

/// Map a `DatSource` to the string used in the catalog.
pub fn dat_source_str(source: &retro_junk_core::DatSource) -> &'static str {
    match source {
        retro_junk_core::DatSource::NoIntro => "no-intro",
        retro_junk_core::DatSource::Redump => "redump",
    }
}
