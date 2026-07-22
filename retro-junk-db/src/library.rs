//! SQLite-authoritative storage for the GUI library.
//!
//! Catalog tables deliberately remain outside this module.  Library writes are
//! ID-addressed, revisioned, and transactional; expensive filesystem work is
//! represented by descriptors produced before a transaction begins.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path};

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use sha2::{Digest, Sha256};
use thiserror::Error;

macro_rules! library_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(transparent)]
        pub struct $name(pub u64);
    };
}

library_id!(LibraryRootId);
library_id!(LibraryConsoleId);
library_id!(LibraryEntryId);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LibrarySourceKey(String);

impl LibrarySourceKey {
    pub fn new(value: impl Into<String>) -> Result<Self, LibraryError> {
        let value = value.into();
        let (kind, path) = value
            .split_once(':')
            .ok_or(LibraryError::UnsafeSourcePath)?;
        if !matches!(kind, "file" | "set") || path.is_empty() {
            return Err(LibraryError::UnsafeSourcePath);
        }
        normalize_relative_path(Path::new(path))?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn invalid(row_id: u64) -> Self {
        Self(format!("invalid:{row_id}"))
    }
}

impl std::fmt::Display for LibrarySourceKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Error)]
pub enum LibraryError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("source path must be relative, Unicode, and contain no parent traversal")]
    UnsafeSourcePath,
    #[error("source file error: {0}")]
    SourceIo(#[from] std::io::Error),
    #[error("stale library command")]
    StaleCommand,
    #[error("library row not found")]
    NotFound,
    #[error("invalid persisted scan state: {0}")]
    InvalidScanState(String),
    #[error("catalog mutation failed: {0}")]
    CatalogMutation(String),
}

impl From<LibraryError> for crate::schema::SchemaError {
    fn from(value: LibraryError) -> Self {
        match value {
            LibraryError::Sqlite(e) => Self::Sqlite(e),
            other => Self::LibraryMigration(other.to_string()),
        }
    }
}

pub fn normalize_relative_path(path: &Path) -> Result<String, LibraryError> {
    let portable = path
        .to_str()
        .ok_or(LibraryError::UnsafeSourcePath)?
        .replace('\\', "/");
    if portable.starts_with('/')
        || portable
            .split('/')
            .next()
            .is_some_and(|part| part.ends_with(':'))
    {
        return Err(LibraryError::UnsafeSourcePath);
    }
    let mut parts = Vec::new();
    for component in Path::new(&portable).components() {
        match component {
            Component::Normal(part) => {
                parts.push(part.to_str().ok_or(LibraryError::UnsafeSourcePath)?);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(LibraryError::UnsafeSourcePath);
            }
        }
    }
    if parts.is_empty() {
        return Err(LibraryError::UnsafeSourcePath);
    }
    Ok(parts.join("/"))
}

pub fn file_source_key(relative_path: &Path) -> Result<LibrarySourceKey, LibraryError> {
    LibrarySourceKey::new(format!("file:{}", normalize_relative_path(relative_path)?))
}

pub fn set_source_key(relative_directory: &Path) -> Result<LibrarySourceKey, LibraryError> {
    LibrarySourceKey::new(format!(
        "set:{}",
        normalize_relative_path(relative_directory)?
    ))
}

/// Derive the stable source identity from the serialized legacy `GameEntry`.
/// This intentionally understands only the migration representation, keeping
/// the database crate independent from the scanner crate.
pub fn source_key_from_game_entry_json(
    json: &str,
    console_folder: &Path,
) -> Result<LibrarySourceKey, LibraryError> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|_| LibraryError::UnsafeSourcePath)?;
    if let Some(path) = value.get("SingleFile").and_then(serde_json::Value::as_str) {
        let path = Path::new(path);
        let relative = if path.is_absolute() {
            path.strip_prefix(console_folder)
                .map_err(|_| LibraryError::UnsafeSourcePath)?
        } else {
            path
        };
        return file_source_key(relative);
    }
    if let Some(set) = value.get("MultiDisc") {
        let name = set
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or(LibraryError::UnsafeSourcePath)?;
        return set_source_key(Path::new(name));
    }
    Err(LibraryError::UnsafeSourcePath)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceFileKind {
    File,
    Cue,
    M3u,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFileDescriptor {
    pub relative_path: String,
    pub kind: SourceFileKind,
    pub size: u64,
    pub modified_seconds: i64,
    pub modified_nanos: u32,
    /// Contents are supplied for small source-defining descriptors (CUE/M3U).
    pub descriptor_contents: Option<Vec<u8>>,
}

pub fn source_fingerprint(files: &[SourceFileDescriptor]) -> Result<String, LibraryError> {
    let mut files = files.to_vec();
    for file in &mut files {
        file.relative_path = normalize_relative_path(Path::new(&file.relative_path))?;
    }
    files.sort_by(|a, b| {
        a.relative_path
            .cmp(&b.relative_path)
            .then(kind_name(&a.kind).cmp(kind_name(&b.kind)))
    });
    let mut hash = Sha256::new();
    for file in files {
        hash.update(file.relative_path.as_bytes());
        hash.update([0]);
        hash.update(kind_name(&file.kind).as_bytes());
        hash.update([0]);
        hash.update(file.size.to_le_bytes());
        hash.update(file.modified_seconds.to_le_bytes());
        hash.update(file.modified_nanos.to_le_bytes());
        if let Some(contents) = file.descriptor_contents {
            hash.update((contents.len() as u64).to_le_bytes());
            hash.update(contents);
        } else {
            hash.update(0_u64.to_le_bytes());
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}

/// Collect and fingerprint one logical entry. All filesystem I/O happens
/// before a reconciliation transaction begins.
pub fn source_fingerprint_from_game_entry_json(
    json: &str,
    console_folder: &Path,
) -> Result<String, LibraryError> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|_| LibraryError::UnsafeSourcePath)?;
    let mut paths = Vec::new();
    if let Some(path) = value.get("SingleFile").and_then(serde_json::Value::as_str) {
        paths.push(resolve_source_path(Path::new(path), console_folder)?);
    } else if let Some(set) = value.get("MultiDisc") {
        let name = set
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or(LibraryError::UnsafeSourcePath)?;
        let set_dir = console_folder.join(normalize_relative_path(Path::new(name))?);
        let files = set
            .get("files")
            .and_then(serde_json::Value::as_array)
            .ok_or(LibraryError::UnsafeSourcePath)?;
        for file in files {
            paths.push(resolve_source_path(
                Path::new(file.as_str().ok_or(LibraryError::UnsafeSourcePath)?),
                console_folder,
            )?);
        }
        if let Ok(children) = std::fs::read_dir(set_dir) {
            paths.extend(children.flatten().map(|e| e.path()).filter(|p| {
                p.extension()
                    .and_then(|v| v.to_str())
                    .is_some_and(|v| v.eq_ignore_ascii_case("m3u"))
            }));
        }
    } else {
        return Err(LibraryError::UnsafeSourcePath);
    }
    let descriptor_paths = paths.clone();
    for descriptor in descriptor_paths {
        if descriptor
            .extension()
            .and_then(|v| v.to_str())
            .is_some_and(|v| v.eq_ignore_ascii_case("cue"))
        {
            let contents = std::fs::read_to_string(&descriptor)?;
            let parent = descriptor.parent().ok_or(LibraryError::UnsafeSourcePath)?;
            for referenced in cue_file_references(&contents) {
                paths.push(parent.join(referenced));
            }
        }
    }
    paths.sort();
    paths.dedup();
    let descriptors = paths
        .into_iter()
        .map(|path| source_descriptor(&path, console_folder))
        .collect::<Result<Vec<_>, _>>()?;
    source_fingerprint(&descriptors)
}

fn cue_file_references(contents: &str) -> Vec<&str> {
    contents
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line
                .strip_prefix("FILE ")
                .or_else(|| line.strip_prefix("file "))?
                .trim_start();
            if let Some(rest) = rest.strip_prefix('"') {
                return rest.split_once('"').map(|(name, _)| name);
            }
            rest.split_whitespace().next()
        })
        .collect()
}

fn resolve_source_path(
    path: &Path,
    console_folder: &Path,
) -> Result<std::path::PathBuf, LibraryError> {
    if path.is_absolute() {
        path.strip_prefix(console_folder)
            .map_err(|_| LibraryError::UnsafeSourcePath)?;
        Ok(path.to_path_buf())
    } else {
        Ok(console_folder.join(normalize_relative_path(path)?))
    }
}

fn source_descriptor(
    path: &Path,
    console_folder: &Path,
) -> Result<SourceFileDescriptor, LibraryError> {
    let relative = path
        .strip_prefix(console_folder)
        .map_err(|_| LibraryError::UnsafeSourcePath)?;
    let relative_path = normalize_relative_path(relative)?;
    let metadata = path.metadata()?;
    let extension = path.extension().and_then(|v| v.to_str()).unwrap_or("");
    let kind = if extension.eq_ignore_ascii_case("cue") {
        SourceFileKind::Cue
    } else if extension.eq_ignore_ascii_case("m3u") {
        SourceFileKind::M3u
    } else {
        SourceFileKind::File
    };
    let descriptor_contents = if matches!(kind, SourceFileKind::Cue | SourceFileKind::M3u)
        && metadata.len() <= 1024 * 1024
    {
        Some(std::fs::read(path)?)
    } else {
        None
    };
    let duration = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok());
    Ok(SourceFileDescriptor {
        relative_path,
        kind,
        size: metadata.len(),
        modified_seconds: duration.map_or(0, |v| v.as_secs() as i64),
        modified_nanos: duration.map_or(0, |v| v.subsec_nanos()),
        descriptor_contents,
    })
}

fn kind_name(kind: &SourceFileKind) -> &str {
    match kind {
        SourceFileKind::File => "file",
        SourceFileKind::Cue => "cue",
        SourceFileKind::M3u => "m3u",
        SourceFileKind::Other(v) => v,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryScanState {
    Unscanned,
    Ready,
    Stale,
}
impl LibraryScanState {
    fn parse(s: String) -> Result<Self, LibraryError> {
        match s.as_str() {
            "unscanned" => Ok(Self::Unscanned),
            "ready" => Ok(Self::Ready),
            "stale" => Ok(Self::Stale),
            _ => Err(LibraryError::InvalidScanState(s)),
        }
    }
}

pub struct LibraryConsoleRow {
    pub id: LibraryConsoleId,
    pub platform: String,
    pub folder_name: String,
    pub folder_path: String,
    pub fingerprint_hash: String,
    pub dat_game_count: i64,
    pub revision: u64,
    pub scan_generation: u64,
    pub scan_state: LibraryScanState,
}

/// Compatibility row used by legacy conversion and full-detail serialization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryEntryRow {
    pub display_name: String,
    pub game_entry_json: String,
    pub status: String,
    pub tag: String,
    pub crc32: String,
    pub sha1: String,
    pub md5: String,
    pub data_size: i64,
    pub hash_warnings_json: Option<String>,
    pub disc_verification: String,
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

#[derive(Debug, Clone)]
pub struct LibraryConsoleDescriptor {
    pub root_id: LibraryRootId,
    pub platform: String,
    pub folder_name: String,
    pub folder_path: String,
}

/// Ensure a lightweight console shell exists without starting or completing a
/// scan. This is used when folder discovery precedes entry reconciliation.
pub fn ensure_library_console(
    conn: &Connection,
    descriptor: &LibraryConsoleDescriptor,
) -> Result<LibraryConsoleId, LibraryError> {
    conn.execute(
        "INSERT INTO library_consoles(root_id,platform,folder_name,folder_path,fingerprint_hash)
         VALUES(?1,?2,?3,?4,'')
         ON CONFLICT(root_id,folder_name) DO UPDATE SET
             platform=excluded.platform,folder_path=excluded.folder_path",
        params![
            descriptor.root_id.0,
            descriptor.platform,
            descriptor.folder_name,
            descriptor.folder_path
        ],
    )?;
    Ok(LibraryConsoleId(conn.query_row(
        "SELECT id FROM library_consoles WHERE root_id=?1 AND folder_name=?2",
        params![descriptor.root_id.0, descriptor.folder_name],
        |row| row.get(0),
    )?))
}

#[derive(Debug, Clone)]
pub struct LibraryConsoleSummary {
    pub id: LibraryConsoleId,
    pub root_id: LibraryRootId,
    pub platform: String,
    pub folder_name: String,
    pub folder_path: String,
    pub scan_state: LibraryScanState,
    pub dat_game_count: u64,
    pub entry_count: u64,
    pub matched_count: u64,
    pub unknown_count: u64,
    pub unrecognized_count: u64,
    pub ambiguous_count: u64,
    pub likely_count: u64,
    pub tagged_count: u64,
    pub revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LibraryEntryFilter {
    All,
    Matched,
    Unmatched,
    Ambiguous,
    Error,
    Tagged,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LibraryEntrySortField {
    DisplayName,
    Status,
    Region,
    Size,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SortDirection {
    Ascending,
    Descending,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LibraryEntryListQuery {
    pub console_id: LibraryConsoleId,
    pub search: String,
    pub filter: LibraryEntryFilter,
    pub sort: LibraryEntrySortField,
    pub direction: SortDirection,
    pub offset: u64,
    pub limit: u64,
}
impl LibraryEntryListQuery {
    pub const DEFAULT_PAGE_SIZE: u64 = 300;
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
pub struct LibraryEntryListItem {
    pub id: LibraryEntryId,
    pub display_name: String,
    pub status: String,
    pub tag: String,
    pub region_override: String,
    pub data_size: u64,
    /// Compact columns rendered by the table. Keeping these in the list
    /// projection avoids loading and deserializing every rich entry payload.
    pub crc32: String,
    pub dat_game_name: String,
    pub serial: String,
    pub internal_name: String,
    /// Serialized `Region` variant names from the persisted identification.
    pub detected_regions: Vec<String>,
    pub has_hash_warnings: bool,
    pub has_broken_references: bool,
    pub has_cue_compat_issues: bool,
    pub revision: u64,
    pub source_revision: u64,
    /// True when this playable entry is bound to a catalog medium represented
    /// by a physical carrier in the active archive projection.
    pub archived: bool,
    /// Actual playable representation format, from archive evidence when
    /// available and otherwise inferred from the library filename.
    pub playable_format: String,
    /// Effective carrier/platform policy projected from the archive.
    pub preferred_format: Option<String>,
    pub archive_release_id: Option<String>,
}
#[derive(Debug, Clone, Default)]
pub struct LibraryEntryCounts {
    pub total: u64,
    pub matched: u64,
    pub unknown: u64,
    pub ambiguous: u64,
    pub likely: u64,
    pub unrecognized: u64,
    pub tagged: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibraryAvailabilityCounts {
    pub playable_only: u64,
    pub archived_and_playable: u64,
    pub preferred_format_mismatch: u64,
    pub archived_not_playable: u64,
}

/// An archival carrier whose preferred playable representation is not present.
/// This deliberately is not a `LibraryEntryListItem`: no playable filesystem
/// entry exists yet, so inventing a library entry ID would blur the authority
/// boundary and break selection/detail loading.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivedPlayableGap {
    pub archive_release_id: String,
    pub carrier_id: String,
    pub dump_id: Option<String>,
    pub title: String,
    pub region: String,
    pub sequence_number: u32,
    pub source_format: Option<String>,
    pub preferred_format: Option<String>,
    pub allow_unverified: bool,
    pub retain_intermediate: bool,
    pub catalog_verified: bool,
    pub buildable: bool,
}

#[derive(Debug, Clone)]
pub struct LibraryEntryListPage {
    pub console_id: LibraryConsoleId,
    pub console_revision: u64,
    pub total_count: u64,
    pub counts: LibraryEntryCounts,
    pub availability_counts: LibraryAvailabilityCounts,
    pub archived_playable_gaps: Vec<ArchivedPlayableGap>,
    pub offset: u64,
    pub rows: Vec<LibraryEntryListItem>,
}
#[derive(Debug, Clone)]
pub struct LibraryEntryDetail {
    pub id: LibraryEntryId,
    pub console_id: LibraryConsoleId,
    pub entry_key: LibrarySourceKey,
    pub revision: u64,
    pub source_revision: u64,
    pub source_fingerprint: String,
    pub row: LibraryEntryRow,
}

/// Minimal authoritative projection used by whole-console frontend exports.
#[derive(Debug, Clone)]
pub struct LibraryExportEntry {
    pub game_entry_json: String,
    pub dat_game_name: String,
    pub cover_title: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibraryChangeSet {
    pub affected_entries: Vec<LibraryEntryId>,
    pub removed_entries: Vec<LibraryEntryId>,
    pub root_revision: Option<(LibraryRootId, u64)>,
    pub console_revision: Option<(LibraryConsoleId, u64)>,
    pub entry_revisions: Vec<(LibraryEntryId, u64, u64)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsoleScanToken {
    pub console_id: LibraryConsoleId,
    pub generation: u64,
}

#[derive(Debug, Clone)]
pub struct ScannedLibraryEntry {
    pub entry_key: LibrarySourceKey,
    pub source_fingerprint: String,
    pub row: LibraryEntryRow,
}

pub fn upsert_library_root(
    conn: &Connection,
    root_path: &str,
) -> Result<LibraryRootId, LibraryError> {
    conn.execute(
        "INSERT OR IGNORE INTO library_roots (root_path) VALUES (?1)",
        [root_path],
    )?;
    Ok(LibraryRootId(conn.query_row(
        "SELECT id FROM library_roots WHERE root_path=?1",
        [root_path],
        |r| r.get(0),
    )?))
}
pub fn get_library_root_id(
    conn: &Connection,
    root_path: &str,
) -> Result<Option<LibraryRootId>, LibraryError> {
    Ok(conn
        .query_row(
            "SELECT id FROM library_roots WHERE root_path=?1",
            [root_path],
            |r| r.get::<_, u64>(0).map(LibraryRootId),
        )
        .optional()?)
}

pub fn delete_library_root(
    conn: &Connection,
    root_id: LibraryRootId,
) -> Result<LibraryChangeSet, LibraryError> {
    let removed = entry_ids_for_root(conn, root_id)?;
    conn.execute("DELETE FROM library_roots WHERE id=?1", [root_id.0])?;
    Ok(LibraryChangeSet {
        removed_entries: removed,
        ..Default::default()
    })
}

pub fn clear_library_cache(conn: &Connection) -> Result<LibraryChangeSet, LibraryError> {
    let removed = conn
        .prepare("SELECT id FROM library_entries")?
        .query_map([], |r| r.get::<_, u64>(0).map(LibraryEntryId))?
        .collect::<Result<Vec<_>, _>>()?;
    conn.execute("DELETE FROM library_roots", [])?;
    Ok(LibraryChangeSet {
        removed_entries: removed,
        ..Default::default()
    })
}

pub fn load_consoles_for_root(
    conn: &Connection,
    root_id: LibraryRootId,
) -> Result<Vec<LibraryConsoleRow>, LibraryError> {
    let mut stmt = conn.prepare("SELECT id,platform,folder_name,folder_path,fingerprint_hash,dat_game_count,revision,scan_generation,scan_state FROM library_consoles WHERE root_id=?1 ORDER BY folder_name COLLATE NOCASE,id")?;
    let rows = stmt.query_map([root_id.0], |r| {
        Ok((
            r.get::<_, u64>(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
            r.get::<_, u64>(6)?,
            r.get::<_, u64>(7)?,
            r.get::<_, String>(8)?,
        ))
    })?;
    rows.map(|r| {
        let (
            id,
            platform,
            folder_name,
            folder_path,
            fingerprint_hash,
            dat_game_count,
            revision,
            scan_generation,
            state,
        ) = r?;
        Ok(LibraryConsoleRow {
            id: LibraryConsoleId(id),
            platform,
            folder_name,
            folder_path,
            fingerprint_hash,
            dat_game_count,
            revision,
            scan_generation,
            scan_state: LibraryScanState::parse(state)?,
        })
    })
    .collect()
}

pub fn begin_console_scan(
    conn: &Connection,
    console_id: LibraryConsoleId,
) -> Result<ConsoleScanToken, LibraryError> {
    if conn.execute(
        "UPDATE library_consoles SET scan_generation=scan_generation+1 WHERE id=?1",
        [console_id.0],
    )? == 0
    {
        return Err(LibraryError::NotFound);
    }
    let generation = conn.query_row(
        "SELECT scan_generation FROM library_consoles WHERE id=?1",
        [console_id.0],
        |r| r.get(0),
    )?;
    Ok(ConsoleScanToken {
        console_id,
        generation,
    })
}

pub fn reconcile_console_scan(
    conn: &mut Connection,
    token: ConsoleScanToken,
    fingerprint_hash: &str,
    entries: &[ScannedLibraryEntry],
) -> Result<LibraryChangeSet, LibraryError> {
    let tx = conn.transaction()?;
    let (root_id, generation, old_fingerprint, old_scan_state): (u64, u64, String, String) = tx
        .query_row(
            "SELECT root_id,scan_generation,fingerprint_hash,scan_state
             FROM library_consoles WHERE id=?1",
            [token.console_id.0],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?
        .ok_or(LibraryError::NotFound)?;
    if generation != token.generation {
        return Err(LibraryError::StaleCommand);
    }
    let mut keys = HashSet::new();
    if entries.iter().any(|e| !keys.insert(e.entry_key.as_str())) {
        return Err(LibraryError::UnsafeSourcePath);
    }
    let mut affected = Vec::new();
    let mut revisions = Vec::new();
    for entry in entries {
        let existing: Option<(u64, String, u64, u64)> = tx
            .query_row(
                "SELECT id,source_fingerprint,revision,source_revision FROM library_entries WHERE console_id=?1 AND entry_key=?2",
                params![token.console_id.0, entry.entry_key.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        match existing {
            Some((id, old_fp, revision, source_revision)) if old_fp == entry.source_fingerprint => {
                let current =
                    load_entry_detail(&tx, LibraryEntryId(id))?.ok_or(LibraryError::NotFound)?;
                let mut merged = entry.clone();
                // These fields belong to the user, never to a scan.
                merged.row.tag.clone_from(&current.row.tag);
                merged
                    .row
                    .region_override
                    .clone_from(&current.row.region_override);
                // A hash update may have committed after this scan read its
                // starting snapshot. Preserve that newer, more authoritative
                // match group while still accepting fresh identification and
                // diagnostics from the scan.
                if hash_evidence_differs(&current.row, &entry.row) {
                    preserve_hash_match_fields(&mut merged.row, &current.row);
                }
                if current.row != merged.row {
                    update_changed_source(&tx, id, &merged, revision + 1, source_revision)?;
                    affected.push(LibraryEntryId(id));
                    revisions.push((LibraryEntryId(id), revision + 1, source_revision));
                }
            }
            Some((id, _, revision, source_revision)) => {
                update_changed_source(&tx, id, entry, revision + 1, source_revision + 1)?;
                affected.push(LibraryEntryId(id));
                revisions.push((LibraryEntryId(id), revision + 1, source_revision + 1));
            }
            None => {
                insert_scanned_entry(&tx, token.console_id, entry)?;
                let id = tx.last_insert_rowid() as u64;
                affected.push(LibraryEntryId(id));
                revisions.push((LibraryEntryId(id), 0, 0));
            }
        }
    }
    let existing = tx
        .prepare("SELECT id,entry_key FROM library_entries WHERE console_id=?1")?
        .query_map([token.console_id.0], |r| {
            Ok((LibraryEntryId(r.get(0)?), r.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let removed: Vec<_> = existing
        .into_iter()
        .filter(|(_, k)| !keys.contains(k.as_str()))
        .map(|(id, _)| id)
        .collect();
    for id in &removed {
        tx.execute("DELETE FROM library_entries WHERE id=?1", [id.0])?;
    }
    if affected.is_empty()
        && removed.is_empty()
        && old_fingerprint == fingerprint_hash
        && old_scan_state == "ready"
    {
        tx.commit()?;
        return Ok(LibraryChangeSet::default());
    }
    tx.execute("UPDATE library_consoles SET fingerprint_hash=?1,scan_state='ready',revision=revision+1 WHERE id=?2", params![fingerprint_hash,token.console_id.0])?;
    tx.execute(
        "UPDATE library_roots SET revision=revision+1 WHERE id=?1",
        [root_id],
    )?;
    let cr = revision_of(&tx, "library_consoles", token.console_id.0)?;
    let rr = revision_of(&tx, "library_roots", root_id)?;
    tx.commit()?;
    Ok(LibraryChangeSet {
        affected_entries: affected,
        removed_entries: removed,
        root_revision: Some((LibraryRootId(root_id), rr)),
        console_revision: Some((token.console_id, cr)),
        entry_revisions: revisions,
    })
}

fn hash_evidence_differs(current: &LibraryEntryRow, scanned: &LibraryEntryRow) -> bool {
    current.crc32 != scanned.crc32
        || current.sha1 != scanned.sha1
        || current.md5 != scanned.md5
        || current.data_size != scanned.data_size
        || current.hash_warnings_json != scanned.hash_warnings_json
        || current.disc_verification != scanned.disc_verification
}

fn preserve_hash_match_fields(target: &mut LibraryEntryRow, current: &LibraryEntryRow) {
    target.status.clone_from(&current.status);
    target.crc32.clone_from(&current.crc32);
    target.sha1.clone_from(&current.sha1);
    target.md5.clone_from(&current.md5);
    target.data_size = current.data_size;
    target
        .hash_warnings_json
        .clone_from(&current.hash_warnings_json);
    target
        .disc_verification
        .clone_from(&current.disc_verification);
    target.dat_game_name.clone_from(&current.dat_game_name);
    target.dat_rom_name.clone_from(&current.dat_rom_name);
    target
        .dat_match_method
        .clone_from(&current.dat_match_method);
    target.cover_title.clone_from(&current.cover_title);
    target.screen_title.clone_from(&current.screen_title);
    target
        .disc_identifications_json
        .clone_from(&current.disc_identifications_json);
    target
        .ambiguous_candidates_json
        .clone_from(&current.ambiguous_candidates_json);
}

pub fn mark_console_stale(
    conn: &mut Connection,
    console_id: LibraryConsoleId,
) -> Result<LibraryChangeSet, LibraryError> {
    mutate_console(conn, console_id, |tx| {
        tx.execute(
            "UPDATE library_consoles SET scan_state='stale',revision=revision+1 WHERE id=?1",
            [console_id.0],
        )?;
        Ok(Vec::new())
    })
}

pub fn set_entry_region_override(
    conn: &mut Connection,
    id: LibraryEntryId,
    value: Option<&str>,
) -> Result<LibraryChangeSet, LibraryError> {
    set_user_field(conn, id, "region_override", value.unwrap_or(""))
}
pub fn set_entry_tag(
    conn: &mut Connection,
    id: LibraryEntryId,
    value: Option<&str>,
) -> Result<LibraryChangeSet, LibraryError> {
    set_user_field(conn, id, "tag", value.unwrap_or(""))
}

pub fn create_homebrew_and_tag_entry(
    conn: &mut Connection,
    id: LibraryEntryId,
    name: &str,
    platform_id: &str,
    region: &str,
) -> Result<LibraryChangeSet, LibraryError> {
    mutate_entry_with_catalog(conn, id, "homebrew", |tx| {
        crate::operations::create_homebrew_work(tx, name, platform_id, region)
            .map(|_| ())
            .map_err(|error| LibraryError::CatalogMutation(error.to_string()))
    })
}

pub fn create_modded_and_tag_entry(
    conn: &mut Connection,
    id: LibraryEntryId,
    work_id: &str,
    platform_id: &str,
    region: &str,
    hashes: Option<&crate::operations::MediaHashes>,
) -> Result<LibraryChangeSet, LibraryError> {
    mutate_entry_with_catalog(conn, id, "modded", |tx| {
        crate::operations::create_modded_media(tx, work_id, platform_id, region, hashes)
            .map(|_| ())
            .map_err(|error| LibraryError::CatalogMutation(error.to_string()))
    })
}

#[derive(Debug, Clone)]
pub struct EntryAnalysisUpdate {
    pub status: String,
    pub crc32: String,
    pub sha1: String,
    pub md5: String,
    pub data_size: i64,
    pub hash_warnings_json: Option<String>,
    pub disc_verification: String,
    pub dat_game_name: String,
    pub dat_rom_name: String,
    pub dat_match_method: String,
    pub cover_title: String,
    pub screen_title: String,
    pub identification_json: Option<String>,
    pub disc_identifications_json: Option<String>,
    pub broken_references_json: Option<String>,
    pub ambiguous_candidates_json: Option<String>,
    pub cue_compat_issues_json: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EntryAnalysisCommand {
    pub entry_id: LibraryEntryId,
    pub expected_source_revision: u64,
    pub update: EntryAnalysisUpdate,
}

/// Hash/match fields produced by an explicit checksum job. Keeping this
/// separate from [`EntryAnalysisUpdate`] prevents a long-running hash from
/// overwriting newer diagnostics or identification data that completed while
/// it was running.
#[derive(Debug, Clone)]
pub struct EntryHashUpdate {
    pub status: String,
    pub crc32: String,
    pub sha1: String,
    pub md5: String,
    pub data_size: i64,
    pub hash_warnings_json: Option<String>,
    pub disc_verification: String,
    pub dat_game_name: String,
    pub dat_rom_name: String,
    pub dat_match_method: String,
    pub cover_title: String,
    pub screen_title: String,
    pub disc_identifications_json: Option<String>,
    pub ambiguous_candidates_json: Option<String>,
}

pub fn apply_entry_analysis(
    conn: &mut Connection,
    id: LibraryEntryId,
    expected_source_revision: u64,
    a: &EntryAnalysisUpdate,
) -> Result<LibraryChangeSet, LibraryError> {
    mutate_entry(conn, id, expected_source_revision, |tx| {
        tx.execute("UPDATE library_entries SET status=?1,crc32=?2,sha1=?3,md5=?4,data_size=?5,hash_warnings_json=?6,disc_verification=?7,dat_game_name=?8,dat_rom_name=?9,dat_match_method=?10,cover_title=?11,screen_title=?12,identification_json=?13,disc_identifications_json=?14,broken_references_json=?15,ambiguous_candidates_json=?16,cue_compat_issues_json=?17,revision=revision+1 WHERE id=?18",params![a.status,a.crc32,a.sha1,a.md5,a.data_size,a.hash_warnings_json,a.disc_verification,a.dat_game_name,a.dat_rom_name,a.dat_match_method,a.cover_title,a.screen_title,a.identification_json,a.disc_identifications_json,a.broken_references_json,a.ambiguous_candidates_json,a.cue_compat_issues_json,id.0])?;
        Ok(())
    })
}

/// Apply a same-console analysis batch in one transaction. Entries whose
/// source changed or disappeared while analysis was running are skipped;
/// unaffected entries still commit.
pub fn apply_entry_analysis_batch(
    conn: &mut Connection,
    commands: &[EntryAnalysisCommand],
) -> Result<LibraryChangeSet, LibraryError> {
    if commands.is_empty() {
        return Ok(LibraryChangeSet::default());
    }
    let tx = conn.transaction()?;
    let mut affected = Vec::new();
    let mut scope: Option<(u64, u64)> = None;
    for command in commands {
        let current: Option<(u64, u64, u64)> = tx
            .query_row(
                "SELECT console_id,(SELECT root_id FROM library_consoles WHERE id=console_id),source_revision FROM library_entries WHERE id=?1",
                [command.entry_id.0],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((console_id, root_id, source_revision)) = current else {
            continue;
        };
        if source_revision != command.expected_source_revision {
            continue;
        }
        if let Some(existing) = scope {
            if existing != (console_id, root_id) {
                return Err(LibraryError::StaleCommand);
            }
        } else {
            scope = Some((console_id, root_id));
        }
        let a = &command.update;
        tx.execute("UPDATE library_entries SET status=?1,crc32=?2,sha1=?3,md5=?4,data_size=?5,hash_warnings_json=?6,disc_verification=?7,dat_game_name=?8,dat_rom_name=?9,dat_match_method=?10,cover_title=?11,screen_title=?12,identification_json=?13,disc_identifications_json=?14,broken_references_json=?15,ambiguous_candidates_json=?16,cue_compat_issues_json=?17,revision=revision+1 WHERE id=?18",params![a.status,a.crc32,a.sha1,a.md5,a.data_size,a.hash_warnings_json,a.disc_verification,a.dat_game_name,a.dat_rom_name,a.dat_match_method,a.cover_title,a.screen_title,a.identification_json,a.disc_identifications_json,a.broken_references_json,a.ambiguous_candidates_json,a.cue_compat_issues_json,command.entry_id.0])?;
        affected.push(command.entry_id);
    }
    let Some((console_id, root_id)) = scope else {
        tx.commit()?;
        return Ok(LibraryChangeSet::default());
    };
    tx.execute(
        "UPDATE library_consoles SET revision=revision+1 WHERE id=?1",
        [console_id],
    )?;
    tx.execute(
        "UPDATE library_roots SET revision=revision+1 WHERE id=?1",
        [root_id],
    )?;
    let entry_revisions = affected
        .iter()
        .map(|id| {
            tx.query_row(
                "SELECT revision,source_revision FROM library_entries WHERE id=?1",
                [id.0],
                |row| Ok((*id, row.get(0)?, row.get(1)?)),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let console_revision = revision_of(&tx, "library_consoles", console_id)?;
    let root_revision = revision_of(&tx, "library_roots", root_id)?;
    tx.commit()?;
    Ok(LibraryChangeSet {
        affected_entries: affected,
        root_revision: Some((LibraryRootId(root_id), root_revision)),
        console_revision: Some((LibraryConsoleId(console_id), console_revision)),
        entry_revisions,
        ..Default::default()
    })
}

pub fn apply_entry_hash_update(
    conn: &mut Connection,
    id: LibraryEntryId,
    expected_source_revision: u64,
    update: &EntryHashUpdate,
) -> Result<LibraryChangeSet, LibraryError> {
    mutate_entry(conn, id, expected_source_revision, |tx| {
        tx.execute(
            "UPDATE library_entries SET status=?1,crc32=?2,sha1=?3,md5=?4,data_size=?5,hash_warnings_json=?6,disc_verification=?7,dat_game_name=?8,dat_rom_name=?9,dat_match_method=?10,cover_title=?11,screen_title=?12,disc_identifications_json=?13,ambiguous_candidates_json=?14,revision=revision+1 WHERE id=?15",
            params![
                update.status,
                update.crc32,
                update.sha1,
                update.md5,
                update.data_size,
                update.hash_warnings_json,
                update.disc_verification,
                update.dat_game_name,
                update.dat_rom_name,
                update.dat_match_method,
                update.cover_title,
                update.screen_title,
                update.disc_identifications_json,
                update.ambiguous_candidates_json,
                id.0,
            ],
        )?;
        Ok(())
    })
}

pub fn apply_filesystem_transition(
    conn: &mut Connection,
    id: LibraryEntryId,
    expected_source_revision: u64,
    new_entry: &ScannedLibraryEntry,
) -> Result<LibraryChangeSet, LibraryError> {
    mutate_entry(conn, id, expected_source_revision, |tx| {
        let revision = revision_of(tx, "library_entries", id.0)?;
        update_changed_source(
            tx,
            id.0,
            new_entry,
            revision + 1,
            expected_source_revision + 1,
        )
    })
}

pub fn list_console_summaries(
    conn: &Connection,
    root_id: LibraryRootId,
) -> Result<Vec<LibraryConsoleSummary>, LibraryError> {
    let mut stmt=conn.prepare("SELECT c.id,c.platform,c.folder_name,c.folder_path,c.scan_state,c.dat_game_count,c.revision,COUNT(e.id),COALESCE(SUM(CASE WHEN e.status='matched' AND e.tag='' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN e.status='unknown' AND e.tag='' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN e.status='unrecognized' AND e.tag='' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN e.status='ambiguous' AND e.tag='' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN e.status='likely' AND e.tag='' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN e.tag<>'' THEN 1 ELSE 0 END),0) FROM library_consoles c LEFT JOIN library_entries e ON e.console_id=c.id WHERE c.root_id=?1 GROUP BY c.id ORDER BY c.folder_name COLLATE NOCASE,c.id")?;
    let rows = stmt.query_map([root_id.0], |r| {
        Ok((
            r.get::<_, u64>(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, u64>(5)?,
            r.get::<_, u64>(6)?,
            r.get::<_, u64>(7)?,
            r.get::<_, u64>(8)?,
            r.get::<_, u64>(9)?,
            r.get::<_, u64>(10)?,
            r.get::<_, u64>(11)?,
            r.get::<_, u64>(12)?,
            r.get::<_, u64>(13)?,
        ))
    })?;
    rows.map(|r| {
        let (
            id,
            platform,
            folder_name,
            folder_path,
            state,
            dat,
            rev,
            count,
            matched,
            unknown,
            unrecognized,
            ambiguous,
            likely,
            tagged,
        ) = r?;
        Ok(LibraryConsoleSummary {
            id: LibraryConsoleId(id),
            root_id,
            platform,
            folder_name,
            folder_path,
            scan_state: LibraryScanState::parse(state)?,
            dat_game_count: dat,
            entry_count: count,
            matched_count: matched,
            unknown_count: unknown,
            unrecognized_count: unrecognized,
            ambiguous_count: ambiguous,
            likely_count: likely,
            tagged_count: tagged,
            revision: rev,
        })
    })
    .collect()
}

pub fn query_entry_list(
    conn: &Connection,
    q: &LibraryEntryListQuery,
) -> Result<LibraryEntryListPage, LibraryError> {
    let revision = revision_of(conn, "library_consoles", q.console_id.0)?;
    let filter = filter_sql(q.filter);
    let pattern = format!(
        "%{}%",
        q.search
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    let where_sql =
        format!("console_id=?1 AND display_name LIKE ?2 ESCAPE '\\' COLLATE NOCASE {filter}");
    let total = conn.query_row(
        &format!("SELECT COUNT(*) FROM library_entries WHERE {where_sql}"),
        params![q.console_id.0, pattern],
        |r| r.get(0),
    )?;
    let counts = entry_counts(conn, q.console_id)?;
    let (availability_counts, archived_playable_gaps) = query_availability(conn, q.console_id)?;
    let order = order_sql(q.sort, q.direction);
    let sql = format!(
        "SELECT id,display_name,status,tag,region_override,data_size,
                crc32,dat_game_name,
                broken_references_json IS NOT NULL AND broken_references_json <> '[]',
                cue_compat_issues_json IS NOT NULL AND cue_compat_issues_json <> '[]',
                revision,source_revision,identification_json,hash_warnings_json,
                disc_identifications_json,entry_key,
                EXISTS(SELECT 1 FROM library_entry_media_bindings b
                       JOIN carriers c ON c.catalog_media_id=b.catalog_media_id
                       JOIN physical_copies pc ON pc.id=c.physical_copy_id
                       JOIN archive_releases ar ON ar.id=pc.archive_release_id
                       JOIN archive_profiles ap ON ap.id=ar.profile_id
                       WHERE b.library_entry_id=library_entries.id
                         AND ap.playable_root=(SELECT lr.root_path FROM library_consoles lc
                              JOIN library_roots lr ON lr.id=lc.root_id
                              WHERE lc.id=library_entries.console_id)),
                (SELECT rep.format FROM library_entry_media_bindings b
                 JOIN representations rep ON rep.id=b.representation_id
                 JOIN carriers rc ON rc.id=rep.carrier_id
                 JOIN physical_copies rpc ON rpc.id=rc.physical_copy_id
                 JOIN archive_releases rar ON rar.id=rpc.archive_release_id
                 JOIN archive_profiles rap ON rap.id=rar.profile_id
                 WHERE b.library_entry_id=library_entries.id AND rep.role='playable'
                   AND rap.playable_root=(SELECT lr.root_path FROM library_consoles lc
                        JOIN library_roots lr ON lr.id=lc.root_id
                        WHERE lc.id=library_entries.console_id)
                 ORDER BY rep.id LIMIT 1),
                (SELECT pp.format FROM library_entry_media_bindings b
                 JOIN carriers c ON c.catalog_media_id=b.catalog_media_id
                 JOIN physical_copies pc ON pc.id=c.physical_copy_id
                 JOIN archive_releases ar ON ar.id=pc.archive_release_id
                 JOIN archive_profiles ap ON ap.id=ar.profile_id
                 JOIN playable_policies pp ON pp.scope_type='carrier' AND pp.scope_id=c.id
                 WHERE b.library_entry_id=library_entries.id
                   AND ap.playable_root=(SELECT lr.root_path FROM library_consoles lc
                        JOIN library_roots lr ON lr.id=lc.root_id
                        WHERE lc.id=library_entries.console_id)
                 ORDER BY c.id LIMIT 1),
                (SELECT ar.id FROM library_entry_media_bindings b
                 JOIN carriers c ON c.catalog_media_id=b.catalog_media_id
                 JOIN physical_copies pc ON pc.id=c.physical_copy_id
                 JOIN archive_releases ar ON ar.id=pc.archive_release_id
                 JOIN archive_profiles ap ON ap.id=ar.profile_id
                 WHERE b.library_entry_id=library_entries.id
                   AND ap.playable_root=(SELECT lr.root_path FROM library_consoles lc
                        JOIN library_roots lr ON lr.id=lc.root_id
                        WHERE lc.id=library_entries.console_id)
                 ORDER BY ar.id LIMIT 1)
         FROM library_entries WHERE {where_sql}
         ORDER BY {order} LIMIT ?3 OFFSET ?4"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(
            params![q.console_id.0, pattern, q.limit.clamp(1, 2000), q.offset],
            |r| {
                let identification_json: Option<String> = r.get(12)?;
                let hash_warnings_json: Option<String> = r.get(13)?;
                let disc_identifications_json: Option<String> = r.get(14)?;
                let entry_key: String = r.get(15)?;
                let projected_format: Option<String> = r.get(17)?;
                let (serial, internal_name, detected_regions) = project_identification(
                    identification_json.as_deref(),
                    disc_identifications_json.as_deref(),
                );
                Ok(LibraryEntryListItem {
                    id: LibraryEntryId(r.get(0)?),
                    display_name: r.get(1)?,
                    status: r.get(2)?,
                    tag: r.get(3)?,
                    region_override: r.get(4)?,
                    data_size: r.get::<_, u64>(5)?,
                    crc32: r.get(6)?,
                    dat_game_name: r.get(7)?,
                    serial,
                    internal_name,
                    detected_regions,
                    has_hash_warnings: json_array_is_nonempty(hash_warnings_json.as_deref())
                        || disc_hash_warnings_are_nonempty(disc_identifications_json.as_deref()),
                    has_broken_references: r.get(8)?,
                    has_cue_compat_issues: r.get(9)?,
                    revision: r.get(10)?,
                    source_revision: r.get(11)?,
                    archived: r.get(16)?,
                    playable_format: projected_format
                        .unwrap_or_else(|| playable_format_from_entry_key(&entry_key)),
                    preferred_format: r.get(18)?,
                    archive_release_id: r.get(19)?,
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(LibraryEntryListPage {
        console_id: q.console_id,
        console_revision: revision,
        total_count: total,
        counts,
        availability_counts,
        archived_playable_gaps,
        offset: q.offset,
        rows,
    })
}

#[allow(clippy::too_many_lines)]
fn query_availability(
    conn: &Connection,
    console_id: LibraryConsoleId,
) -> Result<(LibraryAvailabilityCounts, Vec<ArchivedPlayableGap>), LibraryError> {
    let (folder_name, platform): (String, String) = conn.query_row(
        "SELECT folder_name,platform FROM library_consoles WHERE id=?1",
        [console_id.0],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let mut playable_only = 0;
    let mut archived_and_playable = 0;
    let mut preferred_format_mismatch = 0;
    let mut entry_statement = conn.prepare(
        "SELECT e.entry_key,
                EXISTS(SELECT 1 FROM library_entry_media_bindings b
                       JOIN carriers c ON c.catalog_media_id=b.catalog_media_id
                       JOIN physical_copies pc ON pc.id=c.physical_copy_id
                       JOIN archive_releases ar ON ar.id=pc.archive_release_id
                       JOIN archive_profiles ap ON ap.id=ar.profile_id
                       WHERE b.library_entry_id=e.id
                         AND ap.playable_root=(SELECT lr.root_path FROM library_roots lr
                              JOIN library_consoles lc ON lc.root_id=lr.id WHERE lc.id=e.console_id)),
                (SELECT rep.format FROM library_entry_media_bindings b
                 JOIN representations rep ON rep.id=b.representation_id
                 JOIN carriers rc ON rc.id=rep.carrier_id
                 JOIN physical_copies rpc ON rpc.id=rc.physical_copy_id
                 JOIN archive_releases rar ON rar.id=rpc.archive_release_id
                 JOIN archive_profiles rap ON rap.id=rar.profile_id
                 WHERE b.library_entry_id=e.id AND rep.role='playable'
                   AND rap.playable_root=(SELECT lr.root_path FROM library_roots lr
                        JOIN library_consoles lc ON lc.root_id=lr.id WHERE lc.id=e.console_id)
                 LIMIT 1),
                (SELECT pp.format FROM library_entry_media_bindings b
                 JOIN carriers c ON c.catalog_media_id=b.catalog_media_id
                 JOIN physical_copies pc ON pc.id=c.physical_copy_id
                 JOIN archive_releases ar ON ar.id=pc.archive_release_id
                 JOIN archive_profiles ap ON ap.id=ar.profile_id
                 JOIN playable_policies pp ON pp.scope_type='carrier' AND pp.scope_id=c.id
                 WHERE b.library_entry_id=e.id
                   AND ap.playable_root=(SELECT lr.root_path FROM library_roots lr
                        JOIN library_consoles lc ON lc.root_id=lr.id WHERE lc.id=e.console_id)
                 LIMIT 1)
         FROM library_entries e WHERE e.console_id=?1",
    )?;
    let entries = entry_statement.query_map([console_id.0], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, bool>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    })?;
    for entry in entries {
        let (entry_key, archived, projected, preferred) = entry?;
        if !archived {
            playable_only += 1;
            continue;
        }
        let actual = projected.unwrap_or_else(|| playable_format_from_entry_key(&entry_key));
        if preferred
            .as_deref()
            .is_some_and(|preferred| !format_satisfies_policy(&actual, preferred))
        {
            preferred_format_mismatch += 1;
        } else {
            archived_and_playable += 1;
        }
    }

    let mut statement = conn.prepare(
        "SELECT ar.id,c.id,
                (SELECT de.id FROM dump_events de WHERE de.carrier_id=c.id
                 ORDER BY de.captured_at DESC,de.id DESC LIMIT 1),
                ar.title,ar.region,c.sequence_number,
                (SELECT de.format FROM dump_events de WHERE de.carrier_id=c.id
                 ORDER BY de.captured_at DESC,de.id DESC LIMIT 1),
                pp.format,COALESCE(pp.allow_unverified,0),COALESCE(pp.retain_intermediate,0),
                EXISTS(SELECT 1 FROM dump_events de JOIN verification_events ve
                       ON ve.representation_id=de.representation_id
                       WHERE de.carrier_id=c.id AND ve.kind='catalog' AND ve.outcome='verified'),
                (SELECT COUNT(*) FROM representation_files rf
                 JOIN dump_events de ON de.representation_id=rf.representation_id
                 WHERE de.id=(SELECT newest.id FROM dump_events newest WHERE newest.carrier_id=c.id
                              ORDER BY newest.captured_at DESC,newest.id DESC LIMIT 1))
         FROM archive_releases ar
         JOIN archive_profiles ap ON ap.id=ar.profile_id
         JOIN physical_copies pc ON pc.archive_release_id=ar.id
         JOIN carriers c ON c.physical_copy_id=pc.id
         LEFT JOIN playable_policies pp ON pp.scope_type='carrier' AND pp.scope_id=c.id
         WHERE (lower(ar.platform_id)=lower(?1) OR lower(ar.platform_id)=lower(?2))
           AND ap.playable_root=(SELECT lr.root_path FROM library_roots lr
                                JOIN library_consoles lc ON lc.root_id=lr.id WHERE lc.id=?3)
           AND NOT EXISTS(SELECT 1 FROM representations rep
                          WHERE rep.carrier_id=c.id AND rep.role='playable'
                            AND rep.presence_state='present'
                            AND (pp.format IS NULL OR rep.format=pp.format))
           AND NOT EXISTS(SELECT 1 FROM library_entry_media_bindings b
                          JOIN library_entries e ON e.id=b.library_entry_id
                          JOIN library_consoles lc ON lc.id=e.console_id
                          WHERE b.catalog_media_id=c.catalog_media_id
                            AND (lower(lc.folder_name)=lower(?1) OR lower(lc.platform)=lower(?2))
                            AND (pp.format IS NULL
                                 OR (pp.format='rom' AND lower(e.entry_key) NOT LIKE '%.chd'
                                     AND lower(e.entry_key) NOT LIKE '%.rvz'
                                     AND lower(e.entry_key) NOT LIKE '%.iso'
                                     AND lower(e.entry_key) NOT LIKE '%.cue'
                                     AND lower(e.entry_key) NOT LIKE '%.bin')
                                 OR (pp.format='cue_bin' AND lower(e.entry_key) LIKE '%.cue')
                                 OR lower(e.entry_key) LIKE '%.' || pp.format))
         ORDER BY ar.title COLLATE NOCASE,c.sequence_number,c.id",
    )?;
    let archived_playable_gaps = statement
        .query_map(params![folder_name, platform, console_id.0], |row| {
            let source_format: Option<String> = row.get(6)?;
            let preferred_format: Option<String> = row.get(7)?;
            let file_count: u64 = row.get(11)?;
            let buildable = match (source_format.as_deref(), preferred_format.as_deref()) {
                (Some("redumper_raw" | "cue_bin" | "iso"), Some("chd")) => true,
                (Some(source), Some(preferred)) if source == preferred && file_count == 1 => true,
                _ => false,
            };
            Ok(ArchivedPlayableGap {
                archive_release_id: row.get(0)?,
                carrier_id: row.get(1)?,
                dump_id: row.get(2)?,
                title: row.get(3)?,
                region: row.get(4)?,
                sequence_number: u32::try_from(row.get::<_, i64>(5)?).unwrap_or(0),
                source_format,
                preferred_format,
                allow_unverified: row.get(8)?,
                retain_intermediate: row.get(9)?,
                catalog_verified: row.get(10)?,
                buildable,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let availability_counts = LibraryAvailabilityCounts {
        playable_only,
        archived_and_playable,
        preferred_format_mismatch,
        archived_not_playable: archived_playable_gaps.len() as u64,
    };
    Ok((availability_counts, archived_playable_gaps))
}

fn format_satisfies_policy(actual: &str, preferred: &str) -> bool {
    let actual = actual.to_ascii_lowercase().replace('-', "_");
    let preferred = preferred.to_ascii_lowercase().replace('-', "_");
    actual == preferred
        || (preferred == "cue_bin" && matches!(actual.as_str(), "cue" | "bin"))
        || (preferred == "rom"
            && !matches!(
                actual.as_str(),
                "chd" | "rvz" | "iso" | "cue" | "bin" | "gdi" | "cso" | "dax"
            ))
}

fn playable_format_from_entry_key(entry_key: &str) -> String {
    let path = entry_key.strip_prefix("file:").unwrap_or(entry_key);
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn project_identification(
    identification_json: Option<&str>,
    disc_identifications_json: Option<&str>,
) -> (String, String, Vec<String>) {
    let identification = identification_json.and_then(|json| serde_json::from_str(json).ok());
    let serial = identification
        .as_ref()
        .and_then(|value: &serde_json::Value| value.get("serial_number"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let internal_name = identification
        .as_ref()
        .and_then(|value| value.get("internal_name"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let detected_regions = identification
        .as_ref()
        .and_then(|value| value.get("regions"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::to_owned)
        .collect();
    let serial = if serial.is_empty() {
        disc_identifications_json
            .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
            .and_then(|value| {
                value.as_array()?.iter().find_map(|disc| {
                    let serial = disc.get("identification")?.get("serial_number")?.as_str()?;
                    (!serial.is_empty()).then(|| serial.to_owned())
                })
            })
            .unwrap_or_default()
    } else {
        serial.to_owned()
    };
    (serial, internal_name, detected_regions)
}

fn json_array_is_nonempty(json: Option<&str>) -> bool {
    json.and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        .and_then(|value| value.as_array().map(|items| !items.is_empty()))
        .unwrap_or(false)
}

fn disc_hash_warnings_are_nonempty(json: Option<&str>) -> bool {
    json.and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        .and_then(|value| {
            value.as_array().map(|discs| {
                discs.iter().any(|disc| {
                    disc.get("hashes")
                        .and_then(|hashes| hashes.get("warnings"))
                        .and_then(serde_json::Value::as_array)
                        .is_some_and(|warnings| !warnings.is_empty())
                })
            })
        })
        .unwrap_or(false)
}

pub fn load_entry_detail(
    conn: &Connection,
    id: LibraryEntryId,
) -> Result<Option<LibraryEntryDetail>, LibraryError> {
    let sql = "SELECT console_id,entry_key,revision,source_revision,source_fingerprint,display_name,game_entry_json,status,tag,crc32,sha1,md5,data_size,hash_warnings_json,disc_verification,dat_game_name,dat_rom_name,dat_match_method,region_override,cover_title,screen_title,identification_json,disc_identifications_json,broken_references_json,ambiguous_candidates_json,cue_compat_issues_json FROM library_entries WHERE id=?1";
    Ok(conn
        .query_row(sql, [id.0], |r| {
            Ok(LibraryEntryDetail {
                id,
                console_id: LibraryConsoleId(r.get(0)?),
                entry_key: LibrarySourceKey(r.get(1)?),
                revision: r.get(2)?,
                source_revision: r.get(3)?,
                source_fingerprint: r.get(4)?,
                row: LibraryEntryRow {
                    display_name: r.get(5)?,
                    game_entry_json: r.get(6)?,
                    status: r.get(7)?,
                    tag: r.get(8)?,
                    crc32: r.get(9)?,
                    sha1: r.get(10)?,
                    md5: r.get(11)?,
                    data_size: r.get(12)?,
                    hash_warnings_json: r.get(13)?,
                    disc_verification: r.get(14)?,
                    dat_game_name: r.get(15)?,
                    dat_rom_name: r.get(16)?,
                    dat_match_method: r.get(17)?,
                    region_override: r.get(18)?,
                    cover_title: r.get(19)?,
                    screen_title: r.get(20)?,
                    identification_json: r.get(21)?,
                    disc_identifications_json: r.get(22)?,
                    broken_references_json: r.get(23)?,
                    ambiguous_candidates_json: r.get(24)?,
                    cue_compat_issues_json: r.get(25)?,
                },
            })
        })
        .optional()?)
}

pub fn load_entry_details(
    conn: &Connection,
    ids: &[LibraryEntryId],
) -> Result<Vec<LibraryEntryDetail>, LibraryError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT id,console_id,entry_key,revision,source_revision,source_fingerprint,
                display_name,game_entry_json,status,tag,crc32,sha1,md5,data_size,
                hash_warnings_json,disc_verification,dat_game_name,dat_rom_name,dat_match_method,region_override,cover_title,
                screen_title,identification_json,disc_identifications_json,
                broken_references_json,ambiguous_candidates_json,cue_compat_issues_json
         FROM library_entries WHERE id IN ({placeholders})"
    );
    Ok(conn
        .prepare(&sql)?
        .query_map(rusqlite::params_from_iter(ids.iter().map(|id| id.0)), |r| {
            Ok(LibraryEntryDetail {
                id: LibraryEntryId(r.get(0)?),
                console_id: LibraryConsoleId(r.get(1)?),
                entry_key: LibrarySourceKey(r.get(2)?),
                revision: r.get(3)?,
                source_revision: r.get(4)?,
                source_fingerprint: r.get(5)?,
                row: LibraryEntryRow {
                    display_name: r.get(6)?,
                    game_entry_json: r.get(7)?,
                    status: r.get(8)?,
                    tag: r.get(9)?,
                    crc32: r.get(10)?,
                    sha1: r.get(11)?,
                    md5: r.get(12)?,
                    data_size: r.get(13)?,
                    hash_warnings_json: r.get(14)?,
                    disc_verification: r.get(15)?,
                    dat_game_name: r.get(16)?,
                    dat_rom_name: r.get(17)?,
                    dat_match_method: r.get(18)?,
                    region_override: r.get(19)?,
                    cover_title: r.get(20)?,
                    screen_title: r.get(21)?,
                    identification_json: r.get(22)?,
                    disc_identifications_json: r.get(23)?,
                    broken_references_json: r.get(24)?,
                    ambiguous_candidates_json: r.get(25)?,
                    cue_compat_issues_json: r.get(26)?,
                },
            })
        })?
        .collect::<Result<Vec<_>, _>>()?)
}

pub fn load_entries_for_console(
    conn: &Connection,
    console_id: LibraryConsoleId,
) -> Result<Vec<LibraryEntryRow>, LibraryError> {
    load_entry_details_for_console(conn, console_id)?
        .into_iter()
        .map(|detail| Ok(detail.row))
        .collect()
}

pub fn load_export_entries_for_console(
    conn: &Connection,
    console_id: LibraryConsoleId,
) -> Result<Vec<LibraryExportEntry>, LibraryError> {
    Ok(conn
        .prepare(
            "SELECT game_entry_json,dat_game_name,cover_title
             FROM library_entries WHERE console_id=?1
             ORDER BY display_name COLLATE NOCASE,id",
        )?
        .query_map([console_id.0], |r| {
            Ok(LibraryExportEntry {
                game_entry_json: r.get(0)?,
                dat_game_name: r.get(1)?,
                cover_title: r.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?)
}

pub fn load_entry_details_for_console(
    conn: &Connection,
    console_id: LibraryConsoleId,
) -> Result<Vec<LibraryEntryDetail>, LibraryError> {
    let sql = "SELECT id,entry_key,revision,source_revision,source_fingerprint,
                      display_name,game_entry_json,status,tag,crc32,sha1,md5,data_size,
                      hash_warnings_json,disc_verification,dat_game_name,dat_rom_name,dat_match_method,region_override,
                      cover_title,screen_title,identification_json,disc_identifications_json,
                      broken_references_json,ambiguous_candidates_json,cue_compat_issues_json
               FROM library_entries WHERE console_id=?1
               ORDER BY display_name COLLATE NOCASE,id";
    Ok(conn
        .prepare(sql)?
        .query_map([console_id.0], |r| {
            Ok(LibraryEntryDetail {
                id: LibraryEntryId(r.get(0)?),
                console_id,
                entry_key: LibrarySourceKey(r.get(1)?),
                revision: r.get(2)?,
                source_revision: r.get(3)?,
                source_fingerprint: r.get(4)?,
                row: LibraryEntryRow {
                    display_name: r.get(5)?,
                    game_entry_json: r.get(6)?,
                    status: r.get(7)?,
                    tag: r.get(8)?,
                    crc32: r.get(9)?,
                    sha1: r.get(10)?,
                    md5: r.get(11)?,
                    data_size: r.get(12)?,
                    hash_warnings_json: r.get(13)?,
                    disc_verification: r.get(14)?,
                    dat_game_name: r.get(15)?,
                    dat_rom_name: r.get(16)?,
                    dat_match_method: r.get(17)?,
                    region_override: r.get(18)?,
                    cover_title: r.get(19)?,
                    screen_title: r.get(20)?,
                    identification_json: r.get(21)?,
                    disc_identifications_json: r.get(22)?,
                    broken_references_json: r.get(23)?,
                    ambiguous_candidates_json: r.get(24)?,
                    cue_compat_issues_json: r.get(25)?,
                },
            })
        })?
        .collect::<Result<Vec<_>, _>>()?)
}

fn insert_scanned_entry(
    conn: &Connection,
    cid: LibraryConsoleId,
    e: &ScannedLibraryEntry,
) -> Result<(), LibraryError> {
    conn.execute(
        "INSERT INTO library_entries(
            console_id,entry_key,display_name,game_entry_json,source_fingerprint,
            status,tag,crc32,sha1,md5,data_size,hash_warnings_json,disc_verification,dat_game_name,dat_rom_name,
            dat_match_method,region_override,cover_title,screen_title,
            identification_json,disc_identifications_json,broken_references_json,
            ambiguous_candidates_json,cue_compat_issues_json
         ) VALUES(
            ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,
            ?18,?19,?20,?21,?22,?23,?24
         )",
        params![
            cid.0,
            e.entry_key.as_str(),
            e.row.display_name,
            e.row.game_entry_json,
            e.source_fingerprint,
            e.row.status,
            e.row.tag,
            e.row.crc32,
            e.row.sha1,
            e.row.md5,
            e.row.data_size,
            e.row.hash_warnings_json,
            e.row.disc_verification,
            e.row.dat_game_name,
            e.row.dat_rom_name,
            e.row.dat_match_method,
            e.row.region_override,
            e.row.cover_title,
            e.row.screen_title,
            e.row.identification_json,
            e.row.disc_identifications_json,
            e.row.broken_references_json,
            e.row.ambiguous_candidates_json,
            e.row.cue_compat_issues_json,
        ],
    )?;
    Ok(())
}
fn update_changed_source(
    conn: &Connection,
    id: u64,
    e: &ScannedLibraryEntry,
    revision: u64,
    source_revision: u64,
) -> Result<(), LibraryError> {
    conn.execute(
        "UPDATE library_entries SET
            entry_key=?1,display_name=?2,game_entry_json=?3,revision=?4,
            source_revision=?5,source_fingerprint=?6,status=?7,crc32=?8,sha1=?9,
            md5=?10,data_size=?11,hash_warnings_json=?12,disc_verification=?13,dat_game_name=?14,dat_rom_name=?15,
            dat_match_method=?16,cover_title=?17,screen_title=?18,
            identification_json=?19,disc_identifications_json=?20,
            broken_references_json=?21,ambiguous_candidates_json=?22,
            cue_compat_issues_json=?23
         WHERE id=?24",
        params![
            e.entry_key.as_str(),
            e.row.display_name,
            e.row.game_entry_json,
            revision,
            source_revision,
            e.source_fingerprint,
            e.row.status,
            e.row.crc32,
            e.row.sha1,
            e.row.md5,
            e.row.data_size,
            e.row.hash_warnings_json,
            e.row.disc_verification,
            e.row.dat_game_name,
            e.row.dat_rom_name,
            e.row.dat_match_method,
            e.row.cover_title,
            e.row.screen_title,
            e.row.identification_json,
            e.row.disc_identifications_json,
            e.row.broken_references_json,
            e.row.ambiguous_candidates_json,
            e.row.cue_compat_issues_json,
            id,
        ],
    )?;
    Ok(())
}

fn set_user_field(
    conn: &mut Connection,
    id: LibraryEntryId,
    column: &str,
    value: &str,
) -> Result<LibraryChangeSet, LibraryError> {
    let sql = format!("UPDATE library_entries SET {column}=?1,revision=revision+1 WHERE id=?2");
    mutate_entry_any(conn, id, |tx| {
        tx.execute(&sql, params![value, id.0])?;
        Ok(())
    })
}

fn mutate_entry_with_catalog<F>(
    conn: &mut Connection,
    id: LibraryEntryId,
    tag: &str,
    catalog_mutation: F,
) -> Result<LibraryChangeSet, LibraryError>
where
    F: FnOnce(&Transaction<'_>) -> Result<(), LibraryError>,
{
    let tx = conn.transaction()?;
    let (console_id, root_id): (u64, u64) = tx
        .query_row(
            "SELECT console_id,(SELECT root_id FROM library_consoles WHERE id=console_id)
             FROM library_entries WHERE id=?1",
            [id.0],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or(LibraryError::NotFound)?;
    catalog_mutation(&tx)?;
    tx.execute(
        "UPDATE library_entries SET tag=?1,revision=revision+1 WHERE id=?2",
        params![tag, id.0],
    )?;
    tx.execute(
        "UPDATE library_consoles SET revision=revision+1 WHERE id=?1",
        [console_id],
    )?;
    tx.execute(
        "UPDATE library_roots SET revision=revision+1 WHERE id=?1",
        [root_id],
    )?;
    let entry_revision = revision_of(&tx, "library_entries", id.0)?;
    let source_revision = tx.query_row(
        "SELECT source_revision FROM library_entries WHERE id=?1",
        [id.0],
        |row| row.get(0),
    )?;
    let console_revision = revision_of(&tx, "library_consoles", console_id)?;
    let root_revision = revision_of(&tx, "library_roots", root_id)?;
    tx.commit()?;
    Ok(LibraryChangeSet {
        affected_entries: vec![id],
        root_revision: Some((LibraryRootId(root_id), root_revision)),
        console_revision: Some((LibraryConsoleId(console_id), console_revision)),
        entry_revisions: vec![(id, entry_revision, source_revision)],
        ..Default::default()
    })
}
fn mutate_entry<F>(
    conn: &mut Connection,
    id: LibraryEntryId,
    expected: u64,
    f: F,
) -> Result<LibraryChangeSet, LibraryError>
where
    F: FnOnce(&Transaction<'_>) -> Result<(), LibraryError>,
{
    let current: u64 = conn
        .query_row(
            "SELECT source_revision FROM library_entries WHERE id=?1",
            [id.0],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(LibraryError::NotFound)?;
    if current != expected {
        return Err(LibraryError::StaleCommand);
    }
    mutate_entry_any(conn, id, f)
}
fn mutate_entry_any<F>(
    conn: &mut Connection,
    id: LibraryEntryId,
    f: F,
) -> Result<LibraryChangeSet, LibraryError>
where
    F: FnOnce(&Transaction<'_>) -> Result<(), LibraryError>,
{
    let tx = conn.transaction()?;
    let(cid,rid):(u64,u64)=tx.query_row("SELECT console_id,(SELECT root_id FROM library_consoles WHERE id=console_id) FROM library_entries WHERE id=?1",[id.0],|r|Ok((r.get(0)?,r.get(1)?))).optional()?.ok_or(LibraryError::NotFound)?;
    f(&tx)?;
    tx.execute(
        "UPDATE library_consoles SET revision=revision+1 WHERE id=?1",
        [cid],
    )?;
    tx.execute(
        "UPDATE library_roots SET revision=revision+1 WHERE id=?1",
        [rid],
    )?;
    let er = revision_of(&tx, "library_entries", id.0)?;
    let sr = tx.query_row(
        "SELECT source_revision FROM library_entries WHERE id=?1",
        [id.0],
        |r| r.get(0),
    )?;
    let cr = revision_of(&tx, "library_consoles", cid)?;
    let rr = revision_of(&tx, "library_roots", rid)?;
    tx.commit()?;
    Ok(LibraryChangeSet {
        affected_entries: vec![id],
        root_revision: Some((LibraryRootId(rid), rr)),
        console_revision: Some((LibraryConsoleId(cid), cr)),
        entry_revisions: vec![(id, er, sr)],
        ..Default::default()
    })
}
fn mutate_console<F>(
    conn: &mut Connection,
    cid: LibraryConsoleId,
    f: F,
) -> Result<LibraryChangeSet, LibraryError>
where
    F: FnOnce(&Transaction<'_>) -> Result<Vec<LibraryEntryId>, LibraryError>,
{
    let tx = conn.transaction()?;
    let rid: u64 = tx
        .query_row(
            "SELECT root_id FROM library_consoles WHERE id=?1",
            [cid.0],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(LibraryError::NotFound)?;
    let affected = f(&tx)?;
    tx.execute(
        "UPDATE library_roots SET revision=revision+1 WHERE id=?1",
        [rid],
    )?;
    let cr = revision_of(&tx, "library_consoles", cid.0)?;
    let rr = revision_of(&tx, "library_roots", rid)?;
    tx.commit()?;
    Ok(LibraryChangeSet {
        affected_entries: affected,
        root_revision: Some((LibraryRootId(rid), rr)),
        console_revision: Some((cid, cr)),
        ..Default::default()
    })
}
fn revision_of(conn: &Connection, table: &str, id: u64) -> Result<u64, LibraryError> {
    debug_assert!(matches!(
        table,
        "library_roots" | "library_consoles" | "library_entries"
    ));
    Ok(conn.query_row(
        &format!("SELECT revision FROM {table} WHERE id=?1"),
        [id],
        |r| r.get(0),
    )?)
}
fn entry_ids_for_root(
    conn: &Connection,
    rid: LibraryRootId,
) -> Result<Vec<LibraryEntryId>, LibraryError> {
    Ok(conn.prepare("SELECT e.id FROM library_entries e JOIN library_consoles c ON c.id=e.console_id WHERE c.root_id=?1")?.query_map([rid.0],|r|r.get::<_,u64>(0).map(LibraryEntryId))?.collect::<Result<Vec<_>,_>>()?)
}
fn filter_sql(f: LibraryEntryFilter) -> &'static str {
    match f {
        LibraryEntryFilter::All => "",
        LibraryEntryFilter::Matched => "AND status='matched'",
        LibraryEntryFilter::Unmatched => "AND status='unknown'",
        LibraryEntryFilter::Ambiguous => "AND status='ambiguous'",
        LibraryEntryFilter::Error => "AND status='unrecognized' AND tag=''",
        LibraryEntryFilter::Tagged => "AND tag<>''",
    }
}
fn order_sql(field: LibraryEntrySortField, dir: SortDirection) -> String {
    let col = match field {
        LibraryEntrySortField::DisplayName => "display_name COLLATE NOCASE",
        LibraryEntrySortField::Status => "status",
        LibraryEntrySortField::Region => "region_override",
        LibraryEntrySortField::Size => "data_size",
    };
    let d = if dir == SortDirection::Ascending {
        "ASC"
    } else {
        "DESC"
    };
    format!("{col} {d}, id {d}")
}
fn entry_counts(
    conn: &Connection,
    cid: LibraryConsoleId,
) -> Result<LibraryEntryCounts, LibraryError> {
    Ok(conn.query_row("SELECT COUNT(*),COALESCE(SUM(CASE WHEN status='matched' AND tag='' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN status='unknown' AND tag='' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN status='ambiguous' AND tag='' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN status='likely' AND tag='' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN status='unrecognized' AND tag='' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN tag<>'' THEN 1 ELSE 0 END),0) FROM library_entries WHERE console_id=?1",[cid.0],|r|Ok(LibraryEntryCounts{total:r.get(0)?,matched:r.get(1)?,unknown:r.get(2)?,ambiguous:r.get(3)?,likely:r.get(4)?,unrecognized:r.get(5)?,tagged:r.get(6)?}))?)
}

/// v9 -> v10 library migration. It never touches the ROM filesystem.
pub(crate) fn migrate_library_v10(conn: &Connection) -> Result<(), LibraryError> {
    if conn
        .prepare("SELECT entry_key FROM library_entries LIMIT 0")
        .is_ok()
    {
        return Ok(());
    }
    conn.execute_batch("PRAGMA foreign_keys=OFF; BEGIN IMMEDIATE; ALTER TABLE library_roots ADD COLUMN revision INTEGER NOT NULL DEFAULT 0; ALTER TABLE library_consoles ADD COLUMN revision INTEGER NOT NULL DEFAULT 0; ALTER TABLE library_consoles ADD COLUMN scan_generation INTEGER NOT NULL DEFAULT 0; ALTER TABLE library_consoles ADD COLUMN scan_state TEXT NOT NULL DEFAULT 'stale'; CREATE TABLE library_entries_v10(id INTEGER PRIMARY KEY AUTOINCREMENT,console_id INTEGER NOT NULL REFERENCES library_consoles(id) ON DELETE CASCADE,entry_key TEXT NOT NULL,display_name TEXT NOT NULL,game_entry_json TEXT NOT NULL,revision INTEGER NOT NULL DEFAULT 0,source_revision INTEGER NOT NULL DEFAULT 0,source_fingerprint TEXT NOT NULL DEFAULT '',status TEXT NOT NULL DEFAULT 'unknown',tag TEXT NOT NULL DEFAULT '',crc32 TEXT NOT NULL DEFAULT '',sha1 TEXT NOT NULL DEFAULT '',md5 TEXT NOT NULL DEFAULT '',data_size INTEGER NOT NULL DEFAULT 0,dat_game_name TEXT NOT NULL DEFAULT '',dat_rom_name TEXT NOT NULL DEFAULT '',dat_match_method TEXT NOT NULL DEFAULT '',region_override TEXT NOT NULL DEFAULT '',cover_title TEXT NOT NULL DEFAULT '',screen_title TEXT NOT NULL DEFAULT '',identification_json TEXT,disc_identifications_json TEXT,broken_references_json TEXT,ambiguous_candidates_json TEXT,cue_compat_issues_json TEXT,UNIQUE(console_id,entry_key));")?;
    let rows=conn.prepare("SELECT e.id,e.console_id,e.display_name,e.game_entry_json,COALESCE(e.tag,''),COALESCE(e.region_override,''),c.folder_path FROM library_entries e JOIN library_consoles c ON c.id=e.console_id ORDER BY e.id")?.query_map([],|r|Ok((r.get::<_,u64>(0)?,r.get::<_,u64>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?,r.get::<_,String>(6)?)))?.collect::<Result<Vec<_>,_>>()?;
    let mut derived = Vec::new();
    let mut groups: HashMap<(u64, String), Vec<usize>> = HashMap::new();
    for (row_id, cid, _, json, _, _, folder) in &rows {
        let key = source_key_from_game_entry_json(json, Path::new(folder))
            .unwrap_or_else(|_| LibrarySourceKey::invalid(*row_id));
        let idx = derived.len();
        groups.entry((*cid, key.to_string())).or_default().push(idx);
        derived.push(key);
    }
    for indices in groups.values().filter(|v| v.len() > 1) {
        for &i in indices {
            derived[i] = LibrarySourceKey::invalid(rows[i].0);
        }
    }
    {
        let mut stmt=conn.prepare("INSERT INTO library_entries_v10(id,console_id,entry_key,display_name,game_entry_json,tag,region_override)VALUES(?1,?2,?3,?4,?5,?6,?7)")?;
        for ((id, cid, name, json, tag, region, _), key) in rows.iter().zip(&derived) {
            stmt.execute(params![id, cid, key.as_str(), name, json, tag, region])?;
        }
    }
    conn.execute_batch("DROP TABLE library_entries; ALTER TABLE library_entries_v10 RENAME TO library_entries; CREATE INDEX idx_library_entries_console ON library_entries(console_id); CREATE INDEX idx_library_entries_display ON library_entries(console_id,display_name COLLATE NOCASE,id); COMMIT; PRAGMA foreign_keys=ON;")?;
    Ok(())
}

/// v10 -> v11 repair for consoles whose derived analysis was lost during the
/// SQLite-authoritative cutover. Such consoles must not remain `ready`: doing
/// so presents every entry as gray/unknown indefinitely and prevents the
/// normal auto-scan queue from rebuilding their derived state.
pub(crate) fn migrate_library_v11(conn: &Connection) -> Result<(), LibraryError> {
    conn.execute(
        "UPDATE library_consoles AS c
         SET scan_state='stale', revision=revision+1
         WHERE scan_state='ready'
           AND EXISTS (
               SELECT 1 FROM library_entries e WHERE e.console_id=c.id
           )
           AND NOT EXISTS (
               SELECT 1 FROM library_entries e
               WHERE e.console_id=c.id
                 AND (e.status<>'unknown'
                      OR e.crc32<>''
                      OR e.identification_json IS NOT NULL
                      OR e.disc_identifications_json IS NOT NULL)
           )",
        [],
    )?;
    Ok(())
}
