use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use retro_junk_catalog::CatalogTag;
use retro_junk_db::{Connection, LibraryEntryRow};
use retro_junk_lib::{AnalysisContext, Platform, Region};

use crate::state::{
    ConsoleState, DatMatchInfo, EntryStatus, LibraryBrowserState, LibraryEntry, ScanStatus,
};

// The row ↔ entry conversions (and their error type) live in the backend's
// `library` module beside the entry model itself; re-exported so existing
// `crate::cache::` callers keep working. This file keeps only what is bound
// to GUI state: the legacy JSON cache migration.
pub use retro_junk_backend::library::CacheError;
#[cfg(test)]
pub(crate) use retro_junk_backend::library::row_to_entry;
pub(crate) use retro_junk_backend::library::{
    detail_to_entry, entry_analysis_update, entry_hash_update, entry_to_row,
    scanned_entry_for_folder,
};

// ── Public API ──────────────────────────────────────────────────────────────

/// One-time full import used only while converting the legacy JSON cache.
fn import_legacy_library(
    conn: &mut Connection,
    root: &Path,
    library: &LibraryBrowserState,
) -> Result<(), CacheError> {
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
        import_legacy_console(conn, root_id, console)?;
    }
    Ok(())
}

pub fn scanned_entry(
    console: &ConsoleState,
    entry: &LibraryEntry,
) -> Result<retro_junk_db::ScannedLibraryEntry, CacheError> {
    scanned_entry_for_folder(&console.folder_path, entry)
}

// ── JSON Migration ──────────────────────────────────────────────────────────

/// One-time migration from JSON cache files to `SQLite`.
/// Reads old JSON format, writes to DB, deletes JSON file.
pub fn migrate_json_cache(conn: &mut Connection, root: &Path, context: &AnalysisContext) {
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

    // Load via the old code path to get in-memory library state, then save to DB
    if let Some((library, _stale)) = load_library_from_legacy(&cached, context)
        && let Err(e) = import_legacy_library(conn, root, &library)
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
    hash[..8].iter().fold(String::new(), |mut output, byte| {
        use std::fmt::Write;
        let _ = write!(output, "{byte:02x}");
        output
    })
}

fn legacy_cache_path(root: &Path) -> PathBuf {
    legacy_cache_dir().join(format!("{}.json", legacy_cache_key(root)))
}

/// Whether a legacy JSON cache exists for this root. Lets startup skip the
/// migration path — including its database connection — with one file probe.
pub fn has_legacy_cache(root: &Path) -> bool {
    legacy_cache_path(root).exists()
}

fn load_library_from_legacy(
    cached: &LegacyLibraryCache,
    context: &AnalysisContext,
) -> Option<(LibraryBrowserState, Vec<String>)> {
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

        let current_fp = crate::fingerprint::compute_fingerprint(&cc.folder_path);
        let is_stale = current_fp.name_hash != cc.fingerprint.name_hash;

        let entries = cc
            .entries
            .iter()
            .map(|ce| LibraryEntry {
                id: None,
                revision: 0,
                source_revision: 0,
                game_entry: ce.game_entry.clone(),
                identification: ce.identification.clone(),
                hashes: ce.hashes.clone(),
                disc_verification: Default::default(),
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

        if is_stale {
            stale_folders.push(cc.folder_name.clone());
        }

        consoles.push(ConsoleState {
            id: None,
            revision: 0,
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

    Some((
        LibraryBrowserState {
            consoles,
            root_id: None,
            active_page: None,
            entry_counts: HashMap::new(),
            console_statuses: HashMap::new(),
            stale_consoles: std::collections::HashSet::new(),
            asset_discovery_in_flight: std::collections::HashSet::new(),
            asset_statuses: HashMap::new(),
            entries_with_miximages: std::collections::HashSet::new(),
            detail_asset_entry: None,
        },
        stale_folders,
    ))
}

// ── Private Helpers ─────────────────────────────────────────────────────────

fn ensure_console_id(
    conn: &mut Connection,
    root_id: retro_junk_db::LibraryRootId,
    console: &ConsoleState,
) -> Result<retro_junk_db::LibraryConsoleId, CacheError> {
    let platform_str = serde_json::to_string(&console.platform)?;
    // serde_json wraps enums in quotes: "\"NES\"" — strip them
    let platform_str = platform_str.trim_matches('"');
    let fingerprint = console.fingerprint.as_ref().map_or_else(
        || crate::fingerprint::compute_fingerprint(&console.folder_path).name_hash,
        |fp| fp.name_hash.clone(),
    );
    let rows: Vec<LibraryEntryRow> = console
        .entries
        .iter()
        .map(entry_to_row)
        .collect::<Result<Vec<_>, _>>()?;

    // Source identity and fingerprinting happen before the reconciliation
    // transaction. Unsupported/non-Unicode/out-of-root paths remain visible
    // errors instead of being lossy-normalized.
    let entries = rows
        .into_iter()
        .map(|row| {
            Ok(retro_junk_db::ScannedLibraryEntry {
                entry_key: retro_junk_db::source_key_from_game_entry_json(
                    &row.game_entry_json,
                    &console.folder_path,
                )?,
                source_fingerprint: retro_junk_db::source_fingerprint_from_game_entry_json(
                    &row.game_entry_json,
                    &console.folder_path,
                )?,
                row,
            })
        })
        .collect::<Result<Vec<_>, retro_junk_db::LibraryError>>()?;

    let console_id = retro_junk_db::ensure_library_console(
        conn,
        &retro_junk_db::LibraryConsoleDescriptor {
            root_id,
            platform: platform_str.to_owned(),
            folder_name: console.folder_name.clone(),
            folder_path: console.folder_path.to_string_lossy().into_owned(),
        },
    )?;
    let token = retro_junk_db::begin_console_scan(conn, console_id)?;
    retro_junk_db::reconcile_console_scan(conn, token, &fingerprint, &entries)?;
    Ok(console_id)
}

fn import_legacy_console(
    conn: &mut Connection,
    root_id: retro_junk_db::LibraryRootId,
    console: &ConsoleState,
) -> Result<(), CacheError> {
    let console_id = ensure_console_id(conn, root_id, console)?;
    let details = retro_junk_db::load_entry_details_for_console(conn, console_id)?;
    for entry in &console.entries {
        let scanned = scanned_entry(console, entry)?;
        let Some(detail) = details
            .iter()
            .find(|detail| detail.entry_key == scanned.entry_key)
        else {
            continue;
        };
        let update = entry_analysis_update(entry)?;
        retro_junk_db::apply_entry_analysis(conn, detail.id, detail.source_revision, &update)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::DiscVerification;

    #[test]
    fn standalone_disc_integrity_and_warnings_survive_row_round_trip() {
        let mut entry = crate::test_support::test_entry(
            retro_junk_lib::scanner::GameEntry::SingleFile("game.cue".into()),
        );
        entry.hashes = Some(retro_junk_dat::FileHashes {
            crc32: "12345678".into(),
            sha1: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()),
            md5: None,
            data_size: 2352,
            warnings: vec!["Incomplete disc: DAT Track 2 is missing".into()],
        });
        entry.disc_verification = DiscVerification::Incomplete;

        let restored = row_to_entry(entry_to_row(&entry).unwrap()).unwrap();

        assert_eq!(restored.disc_verification, DiscVerification::Incomplete);
        assert_eq!(
            restored.hashes.unwrap().warnings,
            vec!["Incomplete disc: DAT Track 2 is missing"]
        );
    }
}
