//! CRUD operations for the GUI library cache tables.
//!
//! Stores per-root, per-console, per-entry library state in `SQLite`
//! instead of JSON files. Row types use plain Strings at the DB boundary;
//! the GUI layer handles domain ↔ row conversion.

use rusqlite::{Connection, params};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LibraryError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

// ── Row Types ───────────────────────────────────────────────────────────────

pub struct LibraryConsoleRow {
    pub id: i64,
    pub platform: String,
    pub folder_name: String,
    pub folder_path: String,
    pub fingerprint_hash: String,
    /// Number of games in the matched DAT. 0 = no DAT loaded.
    pub dat_game_count: i64,
}

/// One library entry row. Text fields use `""` for "not set"; the `*_json`
/// blob columns stay `Option` because NULL ("never computed") is distinct
/// from a present-but-empty serialized list.
pub struct LibraryEntryRow {
    pub display_name: String,
    pub game_entry_json: String,
    pub status: String,
    pub tag: String,
    pub crc32: String,
    pub sha1: String,
    pub md5: String,
    /// Size in bytes. 0 = unknown.
    pub data_size: i64,
    pub dat_game_name: String,
    pub dat_rom_name: String,
    pub dat_match_method: String,
    pub region_override: String,
    pub cover_title: String,
    pub screen_title: String,
    pub identification_json: Option<String>,
    pub disc_identifications_json: Option<String>,
    pub broken_references_json: Option<String>,
    pub ambiguous_candidates_json: Option<String>,
    pub cue_compat_issues_json: Option<String>,
}

// ── Root Operations ─────────────────────────────────────────────────────────

/// Insert or find a library root, returning its id.
pub fn upsert_library_root(conn: &Connection, root_path: &str) -> Result<i64, LibraryError> {
    conn.execute(
        "INSERT OR IGNORE INTO library_roots (root_path) VALUES (?1)",
        params![root_path],
    )?;
    let id: i64 = conn.query_row(
        "SELECT id FROM library_roots WHERE root_path = ?1",
        params![root_path],
        |row| row.get(0),
    )?;
    Ok(id)
}

/// Get the id of a library root, if it exists.
pub fn get_library_root_id(
    conn: &Connection,
    root_path: &str,
) -> Result<Option<i64>, LibraryError> {
    let mut stmt = conn.prepare("SELECT id FROM library_roots WHERE root_path = ?1")?;
    let mut rows = stmt.query(params![root_path])?;
    match rows.next()? {
        Some(row) => Ok(Some(row.get(0)?)),
        None => Ok(None),
    }
}

/// Delete a library root and all its consoles/entries (CASCADE).
pub fn delete_library_root(conn: &Connection, root_id: i64) -> Result<(), LibraryError> {
    conn.execute("DELETE FROM library_roots WHERE id = ?1", params![root_id])?;
    Ok(())
}

// ── Console Operations ──────────────────────────────────────────────────────

/// Load all consoles for a root.
pub fn load_consoles_for_root(
    conn: &Connection,
    root_id: i64,
) -> Result<Vec<LibraryConsoleRow>, LibraryError> {
    let mut stmt = conn.prepare(
        "SELECT id, platform, folder_name, folder_path, fingerprint_hash, dat_game_count
         FROM library_consoles WHERE root_id = ?1",
    )?;
    let rows = stmt.query_map(params![root_id], |row| {
        Ok(LibraryConsoleRow {
            id: row.get(0)?,
            platform: row.get(1)?,
            folder_name: row.get(2)?,
            folder_path: row.get(3)?,
            fingerprint_hash: row.get(4)?,
            dat_game_count: row.get(5)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// The console-level columns written by [`save_console_bulk`].
pub struct ConsoleRecord<'a> {
    pub root_id: i64,
    pub platform: &'a str,
    pub folder_name: &'a str,
    pub folder_path: &'a str,
    pub fingerprint_hash: &'a str,
    /// Number of games in the matched DAT. 0 = no DAT loaded.
    pub dat_game_count: i64,
}

/// Save a console and all its entries in a single transaction.
///
/// Upserts the console row, deletes all existing entries for it, then
/// inserts all new entries. Returns the console id.
pub fn save_console_bulk(
    conn: &Connection,
    console: &ConsoleRecord<'_>,
    entries: &[LibraryEntryRow],
) -> Result<i64, LibraryError> {
    let tx = conn.unchecked_transaction()?;

    // Upsert console row
    tx.execute(
        "INSERT INTO library_consoles (root_id, platform, folder_name, folder_path, fingerprint_hash, dat_game_count)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(root_id, folder_name) DO UPDATE SET
             platform = excluded.platform,
             folder_path = excluded.folder_path,
             fingerprint_hash = excluded.fingerprint_hash,
             dat_game_count = excluded.dat_game_count",
        params![
            console.root_id,
            console.platform,
            console.folder_name,
            console.folder_path,
            console.fingerprint_hash,
            console.dat_game_count
        ],
    )?;

    let console_id: i64 = tx.query_row(
        "SELECT id FROM library_consoles WHERE root_id = ?1 AND folder_name = ?2",
        params![console.root_id, console.folder_name],
        |row| row.get(0),
    )?;

    // Delete existing entries and re-insert
    tx.execute(
        "DELETE FROM library_entries WHERE console_id = ?1",
        params![console_id],
    )?;

    insert_entries_batch(&tx, console_id, entries)?;

    tx.commit()?;
    Ok(console_id)
}

// ── Entry Operations ────────────────────────────────────────────────────────

/// Upsert a single entry by (`console_id`, `display_name`).
pub fn upsert_entry(
    conn: &Connection,
    console_id: i64,
    entry: &LibraryEntryRow,
) -> Result<(), LibraryError> {
    execute_upsert_entry(conn, console_id, entry)
}

/// Batch upsert entries in a transaction.
pub fn upsert_entries(
    conn: &Connection,
    console_id: i64,
    entries: &[LibraryEntryRow],
) -> Result<(), LibraryError> {
    let tx = conn.unchecked_transaction()?;
    for entry in entries {
        execute_upsert_entry(&tx, console_id, entry)?;
    }
    tx.commit()?;
    Ok(())
}

/// Load all entries for a console.
pub fn load_entries_for_console(
    conn: &Connection,
    console_id: i64,
) -> Result<Vec<LibraryEntryRow>, LibraryError> {
    let mut stmt = conn.prepare(
        "SELECT display_name, game_entry_json, status, tag,
                crc32, sha1, md5, data_size,
                dat_game_name, dat_rom_name, dat_match_method,
                region_override, cover_title, screen_title,
                identification_json, disc_identifications_json,
                broken_references_json, ambiguous_candidates_json,
                cue_compat_issues_json
         FROM library_entries WHERE console_id = ?1",
    )?;
    let rows = stmt.query_map(params![console_id], |row| {
        Ok(LibraryEntryRow {
            display_name: row.get(0)?,
            game_entry_json: row.get(1)?,
            status: row.get(2)?,
            tag: row.get(3)?,
            crc32: row.get(4)?,
            sha1: row.get(5)?,
            md5: row.get(6)?,
            data_size: row.get(7)?,
            dat_game_name: row.get(8)?,
            dat_rom_name: row.get(9)?,
            dat_match_method: row.get(10)?,
            region_override: row.get(11)?,
            cover_title: row.get(12)?,
            screen_title: row.get(13)?,
            identification_json: row.get(14)?,
            disc_identifications_json: row.get(15)?,
            broken_references_json: row.get(16)?,
            ambiguous_candidates_json: row.get(17)?,
            cue_compat_issues_json: row.get(18)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

// ── Shared Helpers ──────────────────────────────────────────────────────────

fn execute_upsert_entry(
    conn: &Connection,
    console_id: i64,
    e: &LibraryEntryRow,
) -> Result<(), LibraryError> {
    conn.execute(
        "INSERT INTO library_entries (
             console_id, display_name, game_entry_json, status, tag,
             crc32, sha1, md5, data_size,
             dat_game_name, dat_rom_name, dat_match_method,
             region_override, cover_title, screen_title,
             identification_json, disc_identifications_json,
             broken_references_json, ambiguous_candidates_json,
             cue_compat_issues_json
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)
         ON CONFLICT(console_id, display_name) DO UPDATE SET
             game_entry_json = excluded.game_entry_json,
             status = excluded.status,
             tag = excluded.tag,
             crc32 = excluded.crc32,
             sha1 = excluded.sha1,
             md5 = excluded.md5,
             data_size = excluded.data_size,
             dat_game_name = excluded.dat_game_name,
             dat_rom_name = excluded.dat_rom_name,
             dat_match_method = excluded.dat_match_method,
             region_override = excluded.region_override,
             cover_title = excluded.cover_title,
             screen_title = excluded.screen_title,
             identification_json = excluded.identification_json,
             disc_identifications_json = excluded.disc_identifications_json,
             broken_references_json = excluded.broken_references_json,
             ambiguous_candidates_json = excluded.ambiguous_candidates_json,
             cue_compat_issues_json = excluded.cue_compat_issues_json",
        params![
            console_id, e.display_name, e.game_entry_json, e.status, e.tag,
            e.crc32, e.sha1, e.md5, e.data_size,
            e.dat_game_name, e.dat_rom_name, e.dat_match_method,
            e.region_override, e.cover_title, e.screen_title,
            e.identification_json, e.disc_identifications_json,
            e.broken_references_json, e.ambiguous_candidates_json,
            e.cue_compat_issues_json,
        ],
    )?;
    Ok(())
}

fn insert_entries_batch(
    conn: &Connection,
    console_id: i64,
    entries: &[LibraryEntryRow],
) -> Result<(), LibraryError> {
    let mut stmt = conn.prepare(
        "INSERT INTO library_entries (
             console_id, display_name, game_entry_json, status, tag,
             crc32, sha1, md5, data_size,
             dat_game_name, dat_rom_name, dat_match_method,
             region_override, cover_title, screen_title,
             identification_json, disc_identifications_json,
             broken_references_json, ambiguous_candidates_json,
             cue_compat_issues_json
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
    )?;
    for e in entries {
        stmt.execute(params![
            console_id,
            e.display_name,
            e.game_entry_json,
            e.status,
            e.tag,
            e.crc32,
            e.sha1,
            e.md5,
            e.data_size,
            e.dat_game_name,
            e.dat_rom_name,
            e.dat_match_method,
            e.region_override,
            e.cover_title,
            e.screen_title,
            e.identification_json,
            e.disc_identifications_json,
            e.broken_references_json,
            e.ambiguous_candidates_json,
            e.cue_compat_issues_json,
        ])?;
    }
    Ok(())
}
