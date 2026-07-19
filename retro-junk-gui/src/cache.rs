use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

use retro_junk_catalog::CatalogTag;
use retro_junk_dat::MatchMethod;
use retro_junk_db::{Connection, LibraryEntryRow};
use retro_junk_lib::{AnalysisContext, Platform, Region};

use crate::state::{
    ConsoleState, DatMatchInfo, DatStatus, EntryStatus, Library, LibraryEntry, ScanStatus,
};

// ── Error Type ──────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum CacheError {
    Db(retro_junk_db::LibraryError),
    Json(serde_json::Error),
}

impl From<retro_junk_db::LibraryError> for CacheError {
    fn from(e: retro_junk_db::LibraryError) -> Self {
        CacheError::Db(e)
    }
}

impl From<serde_json::Error> for CacheError {
    fn from(e: serde_json::Error) -> Self {
        CacheError::Json(e)
    }
}

impl std::fmt::Display for CacheError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CacheError::Db(e) => write!(f, "database error: {e}"),
            CacheError::Json(e) => write!(f, "serialization error: {e}"),
        }
    }
}

// ── Fingerprint ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct FolderFingerprint {
    /// Hash of sorted filenames in the folder (no metadata/stat calls needed).
    pub name_hash: String,
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Compute a quick fingerprint of a folder by hashing sorted filenames.
///
/// Only uses `read_dir` + `file_type()` (no `metadata()`/`stat()` calls),
/// so this is fast even for folders with thousands of entries.
pub fn compute_fingerprint(path: &Path) -> FolderFingerprint {
    let mut names = Vec::new();

    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            // Check subdirectories using file_type() (free on macOS/Linux via d_type)
            if let Ok(ft) = entry.file_type()
                && ft.is_dir()
                && let Ok(sub_entries) = std::fs::read_dir(entry.path())
            {
                for sub in sub_entries.flatten() {
                    names.push(format!("{}/{}", name, sub.file_name().to_string_lossy()));
                }
            }
            names.push(name);
        }
    }

    names.sort();
    let hash = Sha256::digest(names.join("\n").as_bytes());
    FolderFingerprint {
        name_hash: hex_encode(&hash[..8]),
    }
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Full library save — used on exit and root switch.
pub fn save_library(conn: &Connection, root: &Path, library: &Library) -> Result<(), CacheError> {
    let scanned_count = library
        .consoles
        .iter()
        .filter(|c| c.scan_status == ScanStatus::Scanned)
        .count();

    // Don't overwrite existing data with an empty save (e.g., on_exit before scans finish)
    if scanned_count == 0 {
        let root_str = root.to_string_lossy();
        if let Ok(Some(_)) = retro_junk_db::get_library_root_id(conn, &root_str) {
            log::info!(
                "Skipping cache save: no scanned consoles and DB data already exists for {}",
                root.display()
            );
            return Ok(());
        }
    }

    log::info!("Saving library cache: {scanned_count} scanned consoles");

    let root_str = root.to_string_lossy();
    let root_id = retro_junk_db::upsert_library_root(conn, &root_str)?;

    for console in &library.consoles {
        if console.scan_status != ScanStatus::Scanned {
            continue;
        }
        save_console_inner(conn, root_id, console)?;
    }
    Ok(())
}

/// Full library load — returns library + stale folder names.
pub fn load_library(
    conn: &Connection,
    root: &Path,
    context: &AnalysisContext,
) -> Option<(Library, Vec<String>)> {
    let root_str = root.to_string_lossy();
    let root_id = retro_junk_db::get_library_root_id(conn, &root_str).ok()??;

    let console_rows = retro_junk_db::load_consoles_for_root(conn, root_id).ok()?;
    if console_rows.is_empty() {
        return None;
    }

    let mut consoles = Vec::new();
    let mut stale_folders = Vec::new();

    for cr in console_rows {
        let platform: Platform = match serde_json::from_str(&format!("\"{}\"", cr.platform)) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("Unknown platform '{}' in cache: {}", cr.platform, e);
                continue;
            }
        };

        let registered = context.get_by_platform(platform);
        let (manufacturer, platform_name) = if let Some(r) = registered {
            (r.metadata.manufacturer, r.metadata.platform_name)
        } else {
            log::warn!(
                "Platform {:?} not registered, skipping cached console {}",
                platform,
                cr.folder_name
            );
            continue;
        };

        let entry_rows = retro_junk_db::load_entries_for_console(conn, cr.id).ok()?;
        let entries: Vec<LibraryEntry> = entry_rows.into_iter().filter_map(row_to_entry).collect();

        let folder_path = PathBuf::from(&cr.folder_path);
        let current_fp = compute_fingerprint(&folder_path);
        let is_stale = current_fp.name_hash != cr.fingerprint_hash;

        let dat_status = if cr.dat_game_count > 0 {
            DatStatus::Loaded {
                game_count: cr.dat_game_count as usize,
            }
        } else {
            DatStatus::NotLoaded
        };

        if is_stale {
            stale_folders.push(cr.folder_name.clone());
            consoles.push(ConsoleState {
                platform,
                folder_name: cr.folder_name,
                folder_path,
                manufacturer,
                platform_name,
                scan_status: ScanStatus::NotScanned,
                entries,
                dat_status,
                fingerprint: None,
                loose_disc_files: Vec::new(),
            });
        } else {
            consoles.push(ConsoleState {
                platform,
                folder_name: cr.folder_name,
                folder_path,
                manufacturer,
                platform_name,
                scan_status: ScanStatus::Scanned,
                entries,
                dat_status,
                fingerprint: Some(current_fp),
                loose_disc_files: Vec::new(),
            });
        }
    }

    // Sort same as handle_message does
    consoles.sort_by(|a, b| {
        a.manufacturer
            .cmp(b.manufacturer)
            .then(a.platform_name.cmp(b.platform_name))
            .then(a.folder_name.cmp(&b.folder_name))
    });

    if consoles.is_empty() {
        return None;
    }

    Some((Library { consoles }, stale_folders))
}

/// Save all entries for one console — used after scan/DAT/hash complete.
pub fn save_console(
    conn: &Connection,
    root: &Path,
    console: &ConsoleState,
) -> Result<(), CacheError> {
    let root_id = ensure_root_id(conn, root)?;
    save_console_inner(conn, root_id, console)
}

/// Save specific entries within a console — used for tag, region override, etc.
pub fn save_entries(
    conn: &Connection,
    root: &Path,
    console: &ConsoleState,
    entry_indices: &[usize],
) -> Result<(), CacheError> {
    let root_id = ensure_root_id(conn, root)?;
    let console_id = ensure_console_id(conn, root_id, console)?;
    let rows: Vec<LibraryEntryRow> = entry_indices
        .iter()
        .filter_map(|&i| console.entries.get(i))
        .map(entry_to_row)
        .collect::<Result<Vec<_>, _>>()?;
    retro_junk_db::upsert_entries(conn, console_id, &rows)?;
    Ok(())
}

// ── Cache Management ────────────────────────────────────────────────────────

/// Delete the cached library data for a given root path.
pub fn delete_cache(conn: &Connection, root: &Path) -> Result<(), CacheError> {
    let root_str = root.to_string_lossy();
    if let Some(root_id) = retro_junk_db::get_library_root_id(conn, &root_str)? {
        retro_junk_db::delete_library_root(conn, root_id)?;
    }
    Ok(())
}

/// Delete all library cache data from the database.
pub fn clear_all_caches(conn: &Connection) -> Result<(), CacheError> {
    conn.execute_batch(
        "DELETE FROM library_entries;
         DELETE FROM library_consoles;
         DELETE FROM library_roots;",
    )
    .map_err(|e| CacheError::Db(retro_junk_db::LibraryError::from(e)))?;
    Ok(())
}

// ── JSON Migration ──────────────────────────────────────────────────────────

/// One-time migration from JSON cache files to `SQLite`.
/// Reads old JSON format, writes to DB, deletes JSON file.
pub fn migrate_json_cache(conn: &Connection, root: &Path, context: &AnalysisContext) {
    let json_path = legacy_cache_path(root);
    if !json_path.exists() {
        return;
    }

    log::info!("Migrating JSON cache to SQLite: {}", json_path.display());

    let contents = match std::fs::read_to_string(&json_path) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Failed to read JSON cache for migration: {e}");
            return;
        }
    };

    let cached: LegacyLibraryCache = match serde_json::from_str(&contents) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Failed to parse JSON cache for migration: {e}");
            // Delete corrupt cache file
            let _ = std::fs::remove_file(&json_path);
            return;
        }
    };

    // Load via the old code path to get a Library, then save to DB
    if let Some((library, _stale)) = load_library_from_legacy(&cached, context)
        && let Err(e) = save_library(conn, root, &library)
    {
        log::warn!("Failed to save migrated cache to DB: {e}");
        return;
    }

    // Delete the old JSON file
    if let Err(e) = std::fs::remove_file(&json_path) {
        log::warn!("Failed to delete old JSON cache: {e}");
    } else {
        log::info!("JSON cache migrated and deleted: {}", json_path.display());
    }
}

// ── Legacy JSON Types (for migration only) ──────────────────────────────────

use serde::Deserialize;

#[derive(Deserialize)]
struct LegacyLibraryCache {
    version: u32,
    consoles: Vec<LegacyCachedConsole>,
}

#[derive(Deserialize)]
struct LegacyCachedConsole {
    platform: Platform,
    folder_name: String,
    folder_path: PathBuf,
    fingerprint: LegacyFingerprint,
    entries: Vec<LegacyCachedEntry>,
    dat_game_count: Option<usize>,
}

#[derive(Clone, Deserialize)]
struct LegacyFingerprint {
    name_hash: String,
}

#[derive(Deserialize)]
struct LegacyCachedEntry {
    game_entry: retro_junk_lib::scanner::GameEntry,
    identification: Option<retro_junk_lib::RomIdentification>,
    hashes: Option<retro_junk_dat::FileHashes>,
    dat_match: Option<DatMatchInfo>,
    status: EntryStatus,
    ambiguous_candidates: Vec<String>,
    #[serde(default)]
    region_override: Option<Region>,
    #[serde(default)]
    cover_title: Option<String>,
    #[serde(default)]
    screen_title: Option<String>,
    #[serde(default)]
    disc_identifications: Option<Vec<crate::state::DiscIdentification>>,
    #[serde(default)]
    broken_references: Option<Vec<retro_junk_lib::rename::BrokenReference>>,
    #[serde(default)]
    tag: Option<CatalogTag>,
}

const LEGACY_CACHE_VERSION: u32 = 6;

fn legacy_cache_dir() -> PathBuf {
    let cache = dirs::cache_dir().unwrap_or_else(|| PathBuf::from("."));
    cache.join("retro-junk").join("library")
}

fn legacy_cache_key(root: &Path) -> String {
    let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let hash = Sha256::digest(canonical.to_string_lossy().as_bytes());
    hex_encode(&hash[..8])
}

fn legacy_cache_path(root: &Path) -> PathBuf {
    legacy_cache_dir().join(format!("{}.json", legacy_cache_key(root)))
}

fn load_library_from_legacy(
    cached: &LegacyLibraryCache,
    context: &AnalysisContext,
) -> Option<(Library, Vec<String>)> {
    if cached.version != LEGACY_CACHE_VERSION {
        log::info!(
            "Legacy cache version mismatch ({}), skipping migration",
            cached.version
        );
        return None;
    }

    let mut consoles = Vec::new();
    let mut stale_folders = Vec::new();

    for cc in &cached.consoles {
        let registered = context.get_by_platform(cc.platform);
        let (manufacturer, platform_name) = match registered {
            Some(r) => (r.metadata.manufacturer, r.metadata.platform_name),
            None => continue,
        };

        let current_fp = compute_fingerprint(&cc.folder_path);
        let is_stale = current_fp.name_hash != cc.fingerprint.name_hash;

        let entries = cc
            .entries
            .iter()
            .map(|ce| LibraryEntry {
                game_entry: ce.game_entry.clone(),
                identification: ce.identification.clone(),
                hashes: ce.hashes.clone(),
                dat_match: ce.dat_match.clone(),
                status: ce.status,
                ambiguous_candidates: ce.ambiguous_candidates.clone(),
                asset_paths: None,
                region_override: ce.region_override,
                cover_title: ce.cover_title.clone().unwrap_or_default(),
                screen_title: ce.screen_title.clone().unwrap_or_default(),
                disc_identifications: ce.disc_identifications.clone(),
                broken_references: ce.broken_references.clone(),
                cue_compat_issues: None,
                tag: ce.tag,
            })
            .collect();

        let dat_status = match cc.dat_game_count {
            Some(gc) => DatStatus::Loaded { game_count: gc },
            None => DatStatus::NotLoaded,
        };

        if is_stale {
            stale_folders.push(cc.folder_name.clone());
        }

        consoles.push(ConsoleState {
            platform: cc.platform,
            folder_name: cc.folder_name.clone(),
            folder_path: cc.folder_path.clone(),
            manufacturer,
            platform_name,
            scan_status: if is_stale {
                ScanStatus::NotScanned
            } else {
                ScanStatus::Scanned
            },
            entries,
            dat_status,
            fingerprint: if is_stale { None } else { Some(current_fp) },
            loose_disc_files: Vec::new(),
        });
    }

    consoles.sort_by(|a, b| {
        a.manufacturer
            .cmp(b.manufacturer)
            .then(a.platform_name.cmp(b.platform_name))
            .then(a.folder_name.cmp(&b.folder_name))
    });

    if consoles.is_empty() {
        return None;
    }

    Some((Library { consoles }, stale_folders))
}

// ── Private Helpers ─────────────────────────────────────────────────────────

fn ensure_root_id(
    conn: &Connection,
    root: &Path,
) -> Result<retro_junk_db::LibraryRootId, CacheError> {
    let root_str = root.to_string_lossy();
    Ok(retro_junk_db::upsert_library_root(conn, &root_str)?)
}

fn ensure_console_id(
    conn: &Connection,
    root_id: retro_junk_db::LibraryRootId,
    console: &ConsoleState,
) -> Result<retro_junk_db::LibraryConsoleId, CacheError> {
    let platform_str = serde_json::to_string(&console.platform)?;
    // serde_json wraps enums in quotes: "\"NES\"" — strip them
    let platform_str = platform_str.trim_matches('"');
    let fingerprint = console.fingerprint.as_ref().map_or_else(
        || compute_fingerprint(&console.folder_path).name_hash,
        |fp| fp.name_hash.clone(),
    );
    let dat_game_count = match &console.dat_status {
        DatStatus::Loaded { game_count } => *game_count as i64,
        _ => 0,
    };
    let entries: Vec<LibraryEntryRow> = console
        .entries
        .iter()
        .map(entry_to_row)
        .collect::<Result<Vec<_>, _>>()?;

    let console_id = retro_junk_db::save_console_bulk(
        conn,
        &retro_junk_db::ConsoleRecord {
            root_id,
            platform: platform_str,
            folder_name: &console.folder_name,
            folder_path: &console.folder_path.to_string_lossy(),
            fingerprint_hash: &fingerprint,
            dat_game_count,
        },
        &entries,
    )?;
    Ok(console_id)
}

fn save_console_inner(
    conn: &Connection,
    root_id: retro_junk_db::LibraryRootId,
    console: &ConsoleState,
) -> Result<(), CacheError> {
    ensure_console_id(conn, root_id, console).map(|_| ())
}

// ── Row ↔ Domain Conversion ─────────────────────────────────────────────────

fn entry_to_row(entry: &LibraryEntry) -> Result<LibraryEntryRow, serde_json::Error> {
    let display_name = entry.game_entry.display_name().to_string();
    let game_entry_json = serde_json::to_string(&entry.game_entry)?;

    let (status_str, tag_str) = status_to_str(entry.effective_status());

    let (crc32, sha1, md5, data_size) = match &entry.hashes {
        Some(h) => (
            h.crc32.clone(),
            h.sha1.clone().unwrap_or_default(),
            h.md5.clone().unwrap_or_default(),
            h.data_size as i64,
        ),
        None => (String::new(), String::new(), String::new(), 0),
    };

    let (dat_game_name, dat_rom_name, dat_match_method) = match &entry.dat_match {
        Some(dm) => (
            dm.game_name.clone(),
            dm.rom_name.clone(),
            match_method_to_str(&dm.method).to_string(),
        ),
        None => (String::new(), String::new(), String::new()),
    };

    let region_override = entry
        .region_override
        .map(|r| r.name().to_string())
        .unwrap_or_default();

    let identification_json = entry
        .identification
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;

    let disc_identifications_json = entry
        .disc_identifications
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;

    let broken_references_json = entry
        .broken_references
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;

    let cue_compat_issues_json = entry
        .cue_compat_issues
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;

    let ambiguous_candidates_json = if entry.ambiguous_candidates.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&entry.ambiguous_candidates)?)
    };

    Ok(LibraryEntryRow {
        display_name,
        game_entry_json,
        status: status_str.to_string(),
        tag: tag_str.unwrap_or("").to_string(),
        crc32,
        sha1,
        md5,
        data_size,
        dat_game_name,
        dat_rom_name,
        dat_match_method,
        region_override,
        cover_title: entry.cover_title.clone(),
        screen_title: entry.screen_title.clone(),
        identification_json,
        disc_identifications_json,
        broken_references_json,
        ambiguous_candidates_json,
        cue_compat_issues_json,
    })
}

fn row_to_entry(row: LibraryEntryRow) -> Option<LibraryEntry> {
    let game_entry = serde_json::from_str(&row.game_entry_json).ok()?;

    let status = str_to_status(&row.status);
    let tag = str_to_tag(&row.tag);

    // Empty string in the row means "not set" — map back to the GUI's Option fields.
    let hashes = if row.crc32.is_empty() {
        None
    } else {
        Some(retro_junk_dat::FileHashes {
            crc32: row.crc32,
            sha1: (!row.sha1.is_empty()).then_some(row.sha1),
            md5: (!row.md5.is_empty()).then_some(row.md5),
            data_size: row.data_size as u64,
            warnings: vec![],
        })
    };

    let dat_match = if row.dat_game_name.is_empty() {
        None
    } else {
        Some(DatMatchInfo {
            game_name: row.dat_game_name,
            rom_name: row.dat_rom_name,
            method: str_to_match_method(&row.dat_match_method),
            region: String::new(),
            cross_region: false,
        })
    };

    let region_override = Region::ALL
        .iter()
        .find(|r| r.name() == row.region_override)
        .copied();

    let identification = row
        .identification_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

    let disc_identifications = row
        .disc_identifications_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

    let broken_references = row
        .broken_references_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

    let cue_compat_issues = row
        .cue_compat_issues_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

    let ambiguous_candidates: Vec<String> = row
        .ambiguous_candidates_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();

    Some(LibraryEntry {
        game_entry,
        identification,
        hashes,
        dat_match,
        status,
        ambiguous_candidates,
        asset_paths: None, // re-discovered lazily
        region_override,
        cover_title: row.cover_title,
        screen_title: row.screen_title,
        disc_identifications,
        broken_references,
        cue_compat_issues,
        tag,
    })
}

fn status_to_str(status: EntryStatus) -> (&'static str, Option<&'static str>) {
    match status {
        EntryStatus::Unknown => ("unknown", None),
        EntryStatus::Unrecognized => ("unrecognized", None),
        EntryStatus::Ambiguous => ("ambiguous", None),
        EntryStatus::Matched => ("matched", None),
        EntryStatus::Tagged(CatalogTag::Homebrew) => ("tagged", Some("homebrew")),
        EntryStatus::Tagged(CatalogTag::Modded) => ("tagged", Some("modded")),
    }
}

fn str_to_status(s: &str) -> EntryStatus {
    match s {
        "unrecognized" => EntryStatus::Unrecognized,
        "ambiguous" => EntryStatus::Ambiguous,
        "matched" => EntryStatus::Matched,
        // "unknown", "tagged" (tag column provides the real tag), and anything else
        _ => EntryStatus::Unknown,
    }
}

fn str_to_tag(s: &str) -> Option<CatalogTag> {
    match s {
        "homebrew" => Some(CatalogTag::Homebrew),
        "modded" => Some(CatalogTag::Modded),
        _ => None,
    }
}

fn match_method_to_str(m: &MatchMethod) -> &'static str {
    match m {
        MatchMethod::Serial => "serial",
        MatchMethod::Crc32 => "crc32",
        MatchMethod::Sha1 => "sha1",
    }
}

fn str_to_match_method(s: &str) -> MatchMethod {
    match s {
        "serial" => MatchMethod::Serial,
        "sha1" => MatchMethod::Sha1,
        // "crc32" and anything else default to CRC32
        _ => MatchMethod::Crc32,
    }
}
