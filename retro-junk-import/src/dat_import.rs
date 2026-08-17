//! Import DAT file entries into the catalog database.
//!
//! Each `DatGame` is parsed via the name parser to extract title, region, revision,
//! and status. These are mapped to Work → Release → Media entities in the database.

use retro_junk_catalog::content_id::{self, ContentPart};
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
    /// Entries whose digests cannot name anything — no SHA-1, or a zero-byte
    /// ROM whose digest every other empty file shares. Importing them would
    /// hand out an id that means "some empty thing", so they are left out and
    /// counted where someone can see them.
    pub skipped_unidentifiable: u64,
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
    let inferred_disc_roles = infer_position_specific_disc_roles(dat);

    for (i, game) in dat.games.iter().enumerate() {
        import_game(
            &tx,
            game,
            platform,
            dat_source,
            &dat.name,
            &inferred_disc_roles,
            &mut stats,
        )?;

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
    dat_system: &str,
    inferred_disc_roles: &std::collections::HashSet<DiscRoleKey>,
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
    let variant = compute_release_variant(&parsed, dat_source, inferred_disc_roles);

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

    // The medium's identity is its complete ordered track set — never the
    // primary track alone: 1029 catalog rows share their primary hash with
    // another row on the same platform, and matching on that would merge
    // genuinely different games. A medium the DAT lists as one file is the
    // same statement with a one-item list.
    //
    // This is the only lookup. The three that came before it were all keyed on
    // a name — the game name, the ROM filename, and a release id that embedded
    // the title slug — so a corrected DAT name made the existing row
    // unfindable and minted a whole new work/release/media triple beside it
    // with identical hashes. That is how one XML-entity fix produced 871
    // duplicate entries, and how content-based re-binding then found two
    // candidates for one disc and refused to identify it at all.
    let media_id = match content_id::media_id(&content_parts(&track_roms, primary_rom)) {
        Ok(id) => id,
        Err(error) => {
            log::warn!(
                "Skipping DAT entry that cannot be identified by content ({}): {error}",
                game.name
            );
            stats.skipped_unidentifiable += 1;
            return Ok(());
        }
    };
    let existing = retro_junk_db::queries::get_media_by_id(conn, &media_id)?;

    let natural = ReleaseKey {
        platform_id,
        region: primary_region,
        revision: &revision,
        variant: &variant,
    };
    let effective_release_id = place_release(
        conn,
        existing.as_ref(),
        &canonical_title,
        &parsed.title,
        &natural,
        stats,
    )?;

    if let Some(ref existing_media) = existing {
        // The digests already agree — that is what found this row. What is
        // left to notice is a changed label or classification.
        let existing_disc_designator: String = conn.query_row(
            "SELECT disc_designator FROM media WHERE id=?1",
            [&media_id],
            |row| row.get(0),
        )?;
        let unchanged = existing_media.release_id == effective_release_id
            && existing_media.rom_name == primary_rom.name
            && existing_media.dat_name == game.name
            && existing_media.dat_system == dat_system
            && existing_media.revision == media_revision
            && existing_media.status == media_status
            && existing_disc_designator == parsed.disc_designator.as_deref().unwrap_or_default();
        if unchanged {
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
        revision: media_revision,
        status: media_status,
        tag: None,
        dat_name: game.name.clone(),
        rom_name: primary_rom.name.clone(),
        dat_source: dat_source.to_string(),
        dat_system: dat_system.to_owned(),
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
    // `disc_number = 0` historically meant both "unnumbered" and "Disc 0".
    // Preserve the explicit DAT designator separately so completeness can
    // distinguish those cases and alphabetic disc sets remain lossless.
    conn.execute(
        "UPDATE media SET disc_designator=?2 WHERE id=?1",
        rusqlite::params![
            media_id,
            parsed.disc_designator.as_deref().unwrap_or_default()
        ],
    )?;

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
fn compute_release_variant(
    parsed: &name_parser::ParsedDatName,
    dat_source: &str,
    inferred_disc_roles: &std::collections::HashSet<DiscRoleKey>,
) -> String {
    let scope = EditionScope::from_parsed(parsed);
    let flags = parsed
        .flags
        .iter()
        .filter(|flag| {
            !dat_source.eq_ignore_ascii_case("redump")
                || (!name_parser::is_carrier_only_flag(flag)
                    && !inferred_disc_roles.contains(&DiscRoleKey {
                        edition: scope.clone(),
                        flag: (*flag).clone(),
                    }))
        })
        .cloned()
        .collect::<Vec<_>>();
    if flags.is_empty() {
        String::new()
    } else {
        flags.join(", ")
    }
}

/// The parts of a DAT name that identify an edition before its free-form
/// parentheticals are interpreted. Disc-role inference must never cross this
/// boundary: the same word can be a role in one regional/revised set and a
/// real edition label in another.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct EditionScope {
    title: String,
    regions: Vec<String>,
    revision: String,
}

impl EditionScope {
    fn from_parsed(parsed: &name_parser::ParsedDatName) -> Self {
        Self {
            title: parsed.title.clone(),
            regions: parsed.regions.clone(),
            revision: compute_release_revision(parsed),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DiscRoleKey {
    edition: EditionScope,
    flag: String,
}

/// Redump sometimes labels scenario/character discs with a separate unknown
/// parenthetical instead of `Disc N - Label`. Infer those labels only when a
/// work has multiple explicit positions and position-unique labels cover the
/// complete set. This handles `(Disc 1) (Leon)` / `(Disc 2) (Claire)` without
/// teaching arbitrary unknown tags that they are safe to merge.
fn infer_position_specific_disc_roles(dat: &DatFile) -> std::collections::HashSet<DiscRoleKey> {
    use std::collections::{HashMap, HashSet};

    #[derive(Default)]
    struct Group {
        positions: HashSet<String>,
        flags: HashMap<String, HashSet<String>>,
    }
    let mut groups: HashMap<EditionScope, Group> = HashMap::new();
    for game in &dat.games {
        let parsed = name_parser::parse_dat_name(&game.name);
        let Some(position) = parsed.disc_designator.clone() else {
            continue;
        };
        let key = EditionScope::from_parsed(&parsed);
        let group = groups.entry(key).or_default();
        group.positions.insert(position.clone());
        for flag in parsed
            .flags
            .iter()
            .filter(|flag| !name_parser::is_carrier_only_flag(flag))
        {
            group
                .flags
                .entry(flag.clone())
                .or_default()
                .insert(position.clone());
        }
    }
    let mut inferred = HashSet::new();
    for (edition, group) in groups {
        if !positions_form_complete_set(&group.positions) {
            continue;
        }
        let mut candidates_by_position: HashMap<&str, Vec<&str>> = HashMap::new();
        for (flag, positions) in &group.flags {
            if positions.len() == 1 {
                let position = positions.iter().next().expect("one position");
                candidates_by_position
                    .entry(position)
                    .or_default()
                    .push(flag);
            }
        }
        // Only infer a role when every disc has exactly one position-specific
        // unknown label. If there are zero or several candidates anywhere,
        // preserving them as edition metadata is safer than merging releases.
        if group.positions.iter().any(|position| {
            candidates_by_position
                .get(position.as_str())
                .is_none_or(|flags| flags.len() != 1)
        }) {
            continue;
        }
        for flags in candidates_by_position.values() {
            inferred.insert(DiscRoleKey {
                edition: edition.clone(),
                flag: flags[0].to_owned(),
            });
        }
    }
    inferred
}

/// Require a credible complete disc sequence before interpreting unknown
/// labels structurally. Observed `Disc 1` and `Disc 3` alone are not evidence
/// of a complete two-disc set. Numeric sets may intentionally start at zero;
/// alphabetic sets must start at A.
fn positions_form_complete_set(positions: &std::collections::HashSet<String>) -> bool {
    if positions.len() < 2 {
        return false;
    }
    let mut numeric = positions
        .iter()
        .map(|position| position.parse::<u32>())
        .collect::<Result<Vec<_>, _>>();
    if let Ok(ref mut numeric) = numeric {
        numeric.sort_unstable();
        let first = numeric[0];
        return (first == 0 || first == 1)
            && numeric.iter().enumerate().all(|(offset, position)| {
                u32::try_from(offset)
                    .ok()
                    .and_then(|offset| first.checked_add(offset))
                    == Some(*position)
            });
    }
    let mut alphabetic = positions
        .iter()
        .filter_map(|position| {
            let bytes = position.as_bytes();
            (bytes.len() == 1 && bytes[0].is_ascii_alphabetic())
                .then_some(bytes[0].to_ascii_uppercase())
        })
        .collect::<Vec<_>>();
    if alphabetic.len() != positions.len() {
        return false;
    }
    alphabetic.sort_unstable();
    alphabetic
        .iter()
        .copied()
        .eq((b'A'..=u8::MAX).take(alphabetic.len()))
}

// ── Identity ────────────────────────────────────────────────────────────────

/// The digests that name one medium: every non-CUE track in the order the DAT
/// lists them, or the single ROM's own digests when the DAT lists it as one
/// file.
fn content_parts(
    track_roms: &[&retro_junk_dat::DatRom],
    primary_rom: &retro_junk_dat::DatRom,
) -> Vec<ContentPart> {
    if track_roms.is_empty() {
        vec![ContentPart::new(
            primary_rom.size,
            primary_rom.sha1.clone().unwrap_or_default(),
        )]
    } else {
        track_roms
            .iter()
            .map(|rom| ContentPart::new(rom.size, rom.sha1.clone().unwrap_or_default()))
            .collect()
    }
}

/// What distinguishes one release of a work from another.
struct ReleaseKey<'a> {
    platform_id: &'a str,
    region: &'a str,
    revision: &'a str,
    variant: &'a str,
}

/// Which release a DAT entry belongs to, creating the release and its work
/// when nothing yet describes it.
///
/// The interesting case is the first one: this medium's bytes are already in
/// the catalog, and the release holding them still answers to the same region,
/// revision and variant. Then the DAT has not moved the medium anywhere — it
/// has renamed it. So the existing work and release keep their ids and take
/// the new title as a *label*, which is the whole point of keying on content:
/// correcting `Tom &amp; Jerry` to `Tom & Jerry` must not orphan anything.
///
/// Anything else — a medium the catalog has never seen, or one the DAT has
/// genuinely reclassified into a different region or revision — falls through
/// to the ordinary find-or-mint: a work is found by its canonical name on this
/// platform, a release by its natural key beneath that work, and either is
/// minted with a fresh id when absent.
fn place_release(
    conn: &Connection,
    existing_media: Option<&Media>,
    canonical_title: &str,
    release_title: &str,
    natural: &ReleaseKey<'_>,
    stats: &mut ImportStats,
) -> Result<String, ImportError> {
    if let Some(media) = existing_media
        && let Some(release) = retro_junk_db::queries::get_release_by_id(conn, &media.release_id)?
        && release.platform_id == natural.platform_id
        && release.region == natural.region
        && release.revision == natural.revision
        && release.variant == natural.variant
    {
        stats.works_existing += 1;
        stats.releases_existing += 1;
        relabel(conn, &release, canonical_title, release_title)?;
        return Ok(release.id);
    }

    let work_id = if let Some(found) =
        operations::find_work_by_name_on_platform(conn, canonical_title, natural.platform_id)?
    {
        stats.works_existing += 1;
        found
    } else {
        let minted = content_id::new_work_id();
        operations::insert_work(conn, &minted, canonical_title)?;
        stats.works_created += 1;
        minted
    };

    let existing_release = operations::find_release(
        conn,
        &work_id,
        natural.platform_id,
        natural.region,
        natural.revision,
        natural.variant,
    )?;
    if let Some(existing) = existing_release {
        stats.releases_existing += 1;
        return Ok(existing.id);
    }

    let release = Release {
        id: content_id::new_release_id(),
        work_id,
        platform_id: natural.platform_id.to_owned(),
        region: natural.region.to_owned(),
        revision: natural.revision.to_owned(),
        variant: natural.variant.to_owned(),
        title: release_title.to_owned(),
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
    Ok(release.id)
}

/// Take the DAT's new wording for a work and release whose identity is settled.
fn relabel(
    conn: &Connection,
    release: &Release,
    canonical_title: &str,
    release_title: &str,
) -> Result<(), ImportError> {
    if release.title != release_title {
        conn.execute(
            "UPDATE releases SET title=?2,updated_at=datetime('now') WHERE id=?1",
            rusqlite::params![release.id, release_title],
        )?;
    }
    let current_name: String = conn.query_row(
        "SELECT canonical_name FROM works WHERE id=?1",
        [&release.work_id],
        |row| row.get(0),
    )?;
    if current_name != canonical_title {
        log::info!(
            "Catalog work {} is now called '{canonical_title}' (was '{current_name}')",
            release.work_id
        );
        operations::update_work_name(conn, &release.work_id, canonical_title)?;
    }
    Ok(())
}

/// Map a `DatSource` to the string used in the catalog.
#[must_use]
pub fn dat_source_str(source: &retro_junk_core::DatSource) -> &'static str {
    match source {
        retro_junk_core::DatSource::NoIntro => "no-intro",
        retro_junk_core::DatSource::Redump => "redump",
    }
}
