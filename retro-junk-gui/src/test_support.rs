//! Shared constructors and harness helpers for GUI tests.
//!
//! `LibraryEntry`/`ConsoleState` have many fields that are irrelevant to most
//! tests; these builders fill them with neutral defaults so each test module
//! doesn't have to repeat the full struct literal.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use egui_kittest::Harness;
use retro_junk_lib::Platform;
use retro_junk_lib::scanner::GameEntry;

use crate::state::{ConsoleState, EntryStatus, LibraryEntry, ScanStatus};

static NEXT_ENTRY_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_CONSOLE_ID: AtomicU64 = AtomicU64::new(1);

/// How many frames [`settle`] advances — `Harness::run`'s own limit, so tests
/// converted from `run` see the same number of frames.
const SETTLE_FRAMES: usize = 4;

/// Advance the UI enough to observe a change, without requiring it to go
/// quiescent.
///
/// `Harness::run` loops until nothing requests a repaint, and panics if that
/// never happens. Plenty here keeps requesting: the startup modal's spinner
/// while the database prepares, and the inbox and backlog spinners while their
/// background loads are in flight. Whether a test sees one depends on whether a
/// background thread finished before the first frame, so `run` fails as a
/// load-dependent flake rather than a reliable error — the least useful kind of
/// failure. Stepping a fixed number of frames is deterministic.
pub fn settle<T>(harness: &mut Harness<'_, T>) {
    for _ in 0..SETTLE_FRAMES {
        harness.step();
    }
}

/// Build a minimal `LibraryEntry` around a `GameEntry` — the other fields are
/// irrelevant to most tests and start empty.
pub fn test_entry(game_entry: GameEntry) -> LibraryEntry {
    LibraryEntry {
        id: Some(retro_junk_db::LibraryEntryId(
            NEXT_ENTRY_ID.fetch_add(1, Ordering::Relaxed),
        )),
        revision: 0,
        source_revision: 0,
        game_entry,
        identification: None,
        hashes: None,
        disc_verification: Default::default(),
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
        id: Some(retro_junk_db::LibraryConsoleId(
            NEXT_CONSOLE_ID.fetch_add(1, Ordering::Relaxed),
        )),
        revision: 0,
        platform: Platform::Ps1,
        folder_name: folder_name.to_string(),
        folder_path: PathBuf::from("/roms").join(folder_name),
        manufacturer: "Sony",
        platform_name: "PlayStation",
        scan_status: ScanStatus::Scanned,
        entries,
        fingerprint: None,
        loose_disc_files: Vec::new(),
    }
}
