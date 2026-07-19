//! Shared constructors for GUI tests.
//!
//! `LibraryEntry`/`ConsoleState` have many fields that are irrelevant to most
//! tests; these builders fill them with neutral defaults so each test module
//! doesn't have to repeat the full struct literal.

use std::path::PathBuf;

use retro_junk_lib::Platform;
use retro_junk_lib::scanner::GameEntry;

use crate::state::{ConsoleState, DatStatus, EntryStatus, LibraryEntry, ScanStatus};

/// Build a minimal `LibraryEntry` around a `GameEntry` — the other fields are
/// irrelevant to most tests and start empty.
pub fn test_entry(game_entry: GameEntry) -> LibraryEntry {
    LibraryEntry {
        id: None,
        revision: 0,
        source_revision: 0,
        game_entry,
        identification: None,
        hashes: None,
        dat_match: None,
        status: EntryStatus::Unknown,
        ambiguous_candidates: Vec::new(),
        asset_paths: None,
        region_override: None,
        cover_title: String::new(),
        screen_title: String::new(),
        disc_identifications: None,
        broken_references: None,
        cue_compat_issues: None,
        tag: None,
    }
}

/// Build a minimal `PlayStation` `ConsoleState` holding the given entries.
pub fn test_console(folder_name: &str, entries: Vec<LibraryEntry>) -> ConsoleState {
    ConsoleState {
        id: None,
        revision: 0,
        platform: Platform::Ps1,
        folder_name: folder_name.to_string(),
        folder_path: PathBuf::from("/roms").join(folder_name),
        manufacturer: "Sony",
        platform_name: "PlayStation",
        scan_status: ScanStatus::Scanned,
        entries,
        dat_status: DatStatus::NotLoaded,
        fingerprint: None,
        loose_disc_files: Vec::new(),
    }
}
