//! Import DAT file entries into the catalog database.
//!
//! Each `DatGame` is parsed via the name parser to extract title, region, revision,
//! and status. These are mapped to Work → Release → Media entities in the database.

use retro_junk_catalog::name_parser::{self, DumpStatus};
use retro_junk_catalog::types::{ImportLog, Media, MediaStatus, Release};
use retro_junk_core::Platform;
use retro_junk_dat::DatFile;
use retro_junk_db::operations::{self, OperationError};
use rusqlite::Connection;
use thiserror::Error;

use crate::progress::ImportProgress;
use crate::slugify;

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("Database error: {0}")]
    Db(#[from] OperationError),
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("No platform mapping for DAT: {0}")]
    UnknownPlatform(String),
    #[error("DAT/GDB error: {0}")]
    Dat(String),
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
    progress: &dyn ImportProgress,
) -> Result<ImportStats, ImportError> {
    let mut stats = ImportStats {
        total_games: dat.games.len() as u64,
        ..Default::default()
    };

    let tx = conn.unchecked_transaction()?;

    for (i, game) in dat.games.iter().enumerate() {
        import_game(&tx, game, platform, dat_source, &mut stats)?;

        progress.on_game(i + 1, dat.games.len(), &game.name);
    }

    tx.commit()?;

    Ok(stats)
}

/// Import a single `DatGame` entry.
#[allow(clippy::too_many_lines)] // linear ETL mapping of one DAT entry; splitting would scatter tightly coupled locals
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
    let media_status = match parsed.status {
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
    let revision = compute_release_revision(&parsed);
    let variant = compute_release_variant(&parsed);
    let release_id = make_release_id(&work_id, platform_id, primary_region, &revision, &variant);

    // Find or create Release
    let existing_release = operations::find_release(
        conn,
        &work_id,
        platform_id,
        primary_region,
        &revision,
        &variant,
    )?;
    let effective_release_id = if let Some(ref existing) = existing_release {
        stats.releases_existing += 1;
        existing.id.clone()
    } else {
        let release = Release {
            id: release_id.clone(),
            work_id: work_id.clone(),
            platform_id: platform_id.to_string(),
            region: primary_region.clone(),
            revision: revision.clone(),
            variant: variant.clone(),
            title: parsed.title.clone(),
            alt_title: String::new(),
            publisher_id: None,
            developer_id: None,
            release_date: String::new(),
            game_serial: String::new(),
            genre: String::new(),
            players: String::new(),
            rating: None,
            description: String::new(),
            screen_title: String::new(),
            cover_title: String::new(),
            screenscraper_id: None,
            scraper_not_found: false,
            created_at: String::new(),
            updated_at: String::new(),
        };
        operations::upsert_release(conn, &release)?;
        stats.releases_created += 1;
        release_id.clone()
    };

    // Separate track ROMs from the primary data track.
    // Full Redump DATs include CUE files and per-track BIN entries.
    // We create one Media entry per game (not per track) and store
    // individual tracks in the media_tracks table.
    let (track_roms, non_track_roms): (Vec<_>, Vec<_>) = game
        .roms
        .iter()
        .partition(|rom| is_multi_track_game(&game.roms) && !is_cue(&rom.name));

    // For multi-track games, find the largest data track for the media entry's hashes
    let primary_rom = if !track_roms.is_empty() {
        track_roms.iter().max_by_key(|r| r.size).unwrap()
    } else if !non_track_roms.is_empty() {
        // Single-ROM game or CUE-only — use first non-CUE ROM
        non_track_roms
            .iter()
            .find(|r| !is_cue(&r.name))
            .unwrap_or(&non_track_roms[0])
    } else {
        // Edge case: game with no ROMs
        return Ok(());
    };

    let media_id = make_media_id(&effective_release_id, &game.name);

    // Check if this media already exists
    let existing = operations::find_media_by_dat_name(conn, &game.name)?;
    if let Some(ref existing_media) = existing {
        let same_hashes = existing_media.crc32 == primary_rom.crc
            && existing_media.sha1 == primary_rom.sha1.as_deref().unwrap_or("")
            && existing_media.file_size == i64::try_from(primary_rom.size).unwrap_or(0);
        if same_hashes {
            stats.media_unchanged += 1;
            return Ok(());
        }
        stats.media_updated += 1;
    } else {
        stats.media_created += 1;
    }

    let media_serial = game
        .serial
        .clone()
        .or_else(|| primary_rom.serial.clone())
        .unwrap_or_default();

    let media = Media {
        id: media_id.clone(),
        release_id: effective_release_id.clone(),
        media_serial,
        disc_number: parsed
            .disc_number
            .and_then(|n| i32::try_from(n).ok())
            .unwrap_or(0),
        disc_label: parsed.disc_label.clone().unwrap_or_default(),
        revision: parsed.revision.clone().unwrap_or_default(),
        status: media_status,
        tag: None,
        dat_name: game.name.clone(),
        dat_source: dat_source.to_string(),
        file_size: i64::try_from(primary_rom.size).unwrap_or(0),
        crc32: primary_rom.crc.clone(),
        sha1: primary_rom.sha1.clone().unwrap_or_default(),
        md5: primary_rom.md5.clone().unwrap_or_default(),
        created_at: String::new(),
        updated_at: String::new(),
    };
    operations::upsert_media(conn, &media)?;

    // Insert per-track entries for multi-track games
    for rom in &track_roms {
        let track_number = extract_track_number(&rom.name);
        let track = operations::MediaTrack {
            media_id: media_id.clone(),
            track_number,
            track_name: rom.name.clone(),
            file_size: i64::try_from(rom.size).unwrap_or(0),
            crc32: rom.crc.clone(),
            sha1: rom.sha1.clone().unwrap_or_default(),
            md5: rom.md5.clone().unwrap_or_default(),
        };
        operations::insert_media_track(conn, &track)?;
    }

    Ok(())
}

/// Log an import run in the `import_log` table.
pub fn log_import(
    conn: &Connection,
    source_type: &str,
    source_name: &str,
    source_version: &str,
    stats: &ImportStats,
) -> Result<i64, ImportError> {
    let now = chrono::Utc::now().to_rfc3339();
    let log_entry = ImportLog {
        id: 0,
        source_type: source_type.to_string(),
        source_name: source_name.to_string(),
        source_version: source_version.to_string(),
        imported_at: now,
        records_created: stats.media_created as i64,
        records_updated: stats.media_updated as i64,
        records_unchanged: stats.media_unchanged as i64,
        disagreements_found: stats.disagreements_found as i64,
    };
    let id = operations::insert_import_log(conn, &log_entry)?;
    Ok(id)
}

// ── Edition Computation ─────────────────────────────────────────────────

/// Compute the release revision from a parsed DAT name.
///
/// Prefers explicit revision (e.g., "Rev A"), falls back to version (e.g., "v1.0").
fn compute_release_revision(parsed: &name_parser::ParsedDatName) -> String {
    parsed
        .revision
        .clone()
        .or_else(|| parsed.version.clone())
        .unwrap_or_default()
}

/// Compute the release variant from a parsed DAT name.
///
/// Flags like "Greatest Hits", "Player's Choice", "Virtual Console", "Proto",
/// "Beta", etc. all become variant identifiers that distinguish releases.
fn compute_release_variant(parsed: &name_parser::ParsedDatName) -> String {
    if parsed.flags.is_empty() {
        String::new()
    } else {
        parsed.flags.join(", ")
    }
}

// ── ID Generation ───────────────────────────────────────────────────────────

/// Generate a stable work ID from title and platform.
///
/// Uses a simple slug: lowercase, alphanumeric + hyphens.
fn make_work_id(title: &str, platform_id: &str) -> String {
    let slug = slugify(title);
    format!("{platform_id}:{slug}")
}

/// Generate a stable release ID from work + platform + region + revision + variant.
fn make_release_id(
    work_id: &str,
    platform_id: &str,
    region: &str,
    revision: &str,
    variant: &str,
) -> String {
    let mut id = format!("{work_id}:{platform_id}:{region}");
    if !revision.is_empty() {
        id.push(':');
        id.push_str(&crate::slugify(revision));
    }
    if !variant.is_empty() {
        id.push(':');
        id.push_str(&crate::slugify(variant));
    }
    id
}

/// Generate a stable media ID from release + ROM name.
fn make_media_id(release_id: &str, rom_name: &str) -> String {
    let slug = slugify(rom_name);
    format!("{release_id}:{slug}")
}

/// Map a `DatSource` to the string used in the catalog.
#[must_use]
pub fn dat_source_str(source: &retro_junk_core::DatSource) -> &'static str {
    match source {
        retro_junk_core::DatSource::NoIntro => "no-intro",
        retro_junk_core::DatSource::Redump => "redump",
    }
}

/// Check if a ROM name has a `.cue` extension (case-insensitive).
fn is_cue(name: &str) -> bool {
    std::path::Path::new(name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("cue"))
}

/// Check if a game has multiple non-CUE ROMs (typical of full Redump DATs
/// which include CUE + per-track BIN entries).
fn is_multi_track_game(roms: &[retro_junk_dat::DatRom]) -> bool {
    let non_cue_count = roms.iter().filter(|r| !is_cue(&r.name)).count();
    non_cue_count > 1
}

/// Extract a track number from a Redump ROM name like "Game (Track 02).bin".
/// Falls back to 0 if no track number is found.
fn extract_track_number(name: &str) -> i32 {
    // Look for "(Track NN)" pattern
    if let Some(start) = name.find("(Track ") {
        let after = &name[start + 7..];
        if let Some(end) = after.find(')')
            && let Ok(n) = after[..end].trim().parse::<i32>()
        {
            return n;
        }
    }
    // Fallback: try to extract from ".bin" suffix pattern
    0
}
