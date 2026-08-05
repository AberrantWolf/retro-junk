//! Import DAT file entries into the catalog database.
//!
//! Each `DatGame` is parsed via the name parser to extract title, region, revision,
//! and status. These are mapped to Work → Release → Media entities in the database.

use retro_junk_catalog::name_parser::{self, DumpStatus};
use retro_junk_catalog::types::{ImportLog, ImportLogId, Media, MediaStatus, Release};
use retro_junk_core::Platform;
use retro_junk_dat::DatFile;
// Track structure lives in `retro_junk_dat::tracks` — the naming rule that
// depends on it (a whole-disc container must not inherit a member track's
// name) has other callers, and both must read the DAT the same way.
use retro_junk_dat::tracks::{
    is_cue_name as is_cue, is_multi_track as is_multi_track_game,
    track_number as extract_track_number,
};
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
    let media_revision = if revision.is_empty() {
        game.version
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_default()
            .to_owned()
    } else {
        revision.clone()
    };
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

    // A DAT may carry multiple byte-order/container representations under the
    // same game name. N64 DATs, for example, have separate `.z64` and `.v64`
    // records with different hashes. Treat the ROM filename as part of the
    // media identity so one representation cannot overwrite another.
    let existing = operations::find_media_by_release_and_rom_name(
        conn,
        &effective_release_id,
        &primary_rom.name,
    )?;
    let existing = if existing.is_some() {
        existing
    } else {
        // Very old catalogs did not persist the ROM filename. Claim that row
        // only when it belongs to this release and is genuinely blank; a
        // non-empty different filename is a distinct DAT representation.
        operations::find_media_by_dat_name(conn, &game.name)?
            .filter(|media| media.release_id == effective_release_id && media.rom_name.is_empty())
    };
    // Last resort: find it by what it *is* rather than what it is called.
    //
    // Both lookups above are keyed on the name — the release id embeds the
    // title slug — so a corrected DAT name makes the existing row unfindable
    // and mints a whole new work/release/media triple beside it, with
    // identical hashes. That is how one entity-escaping fix produced 871
    // duplicate entries, and how content-based re-binding then found two
    // candidates for one disc and refused to identify it at all.
    //
    // The key is the complete ordered track set, never the primary track: 1029
    // catalog rows share their primary hash with another row on the same
    // platform, and matching on it would merge genuinely different games.
    let existing = match existing {
        Some(found) => Some(found),
        None => find_media_by_content(conn, platform_id, &track_roms, primary_rom)?,
    };
    if let Some(ref existing_media) = existing {
        let same_hashes = existing_media.crc32 == primary_rom.crc
            && existing_media.sha1 == primary_rom.sha1.as_deref().unwrap_or("")
            && existing_media.file_size == i64::try_from(primary_rom.size).unwrap_or(0)
            && existing_media.revision == media_revision;
        if same_hashes && existing_media.rom_name == primary_rom.name {
            stats.media_unchanged += 1;
            return Ok(());
        }
        stats.media_updated += 1;
    } else {
        stats.media_created += 1;
    }

    // Preserve an existing ID when upgrading a catalog created by the old
    // game-name key. This repairs it in place on re-import without orphaning
    // collection rows or media assets that reference that ID.
    let media_id = existing.as_ref().map_or_else(
        || make_media_id(&effective_release_id, &primary_rom.name),
        |media| media.id.clone(),
    );

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
        revision: media_revision,
        status: media_status,
        tag: None,
        dat_name: game.name.clone(),
        rom_name: primary_rom.name.clone(),
        dat_source: dat_source.to_string(),
        file_size: i64::try_from(primary_rom.size).unwrap_or(0),
        crc32: primary_rom.crc.to_ascii_lowercase(),
        sha1: primary_rom
            .sha1
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase(),
        md5: primary_rom
            .md5
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase(),
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
) -> Result<ImportLogId, ImportError> {
    let now = chrono::Utc::now().to_rfc3339();
    let log_entry = ImportLog {
        id: ImportLogId(0),
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
    conn.execute(
        "INSERT OR IGNORE INTO catalog_source_snapshots(source,system,version,imported_at,content_sha256)
         VALUES(?1,?2,?3,?4,'')",
        rusqlite::params![source_type, source_name, source_version, log_entry.imported_at],
    )?;
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

/// Find an existing catalog medium by its content, so a renamed game keeps
/// its identity instead of gaining a twin.
///
/// The content key is the complete ordered track set — every non-CUE ROM's
/// sha1 and size — falling back to the single ROM's digests for media the
/// catalog stores without tracks. `match_complete_catalog_media` owns that
/// rule, including the refusal to let one data track vouch for a multi-track
/// disc, so this asks it rather than growing a second definition.
fn find_media_by_content(
    conn: &Connection,
    platform_id: &str,
    track_roms: &[&retro_junk_dat::DatRom],
    primary_rom: &retro_junk_dat::DatRom,
) -> Result<Option<Media>, OperationError> {
    let tracks = if track_roms.is_empty() {
        vec![(primary_rom.size, primary_rom.sha1.clone())]
    } else {
        track_roms
            .iter()
            .map(|rom| (rom.size, rom.sha1.clone()))
            .collect()
    };
    if tracks.iter().any(|(_, sha1)| sha1.is_none()) {
        return Ok(None);
    }
    let digests = tracks
        .into_iter()
        .enumerate()
        .map(|(index, (size, sha1))| retro_junk_db::TrackDigest {
            number: u32::try_from(index + 1).unwrap_or(1),
            size,
            crc32: String::new(),
            md5: String::new(),
            sha1: sha1.unwrap_or_default().to_lowercase(),
        })
        .collect::<Vec<_>>();
    let matches =
        retro_junk_db::archive::match_complete_catalog_media(conn, platform_id, &digests)?;
    // Only an unambiguous answer may reclaim an id. Two catalog rows sharing a
    // complete track set would be a catalog problem, and guessing between them
    // is how the wrong game gets renamed.
    let [found] = matches.as_slice() else {
        return Ok(None);
    };
    retro_junk_db::queries::get_media_by_id(conn, &found.media_id)
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
