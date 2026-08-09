//! Public data types shared by rename planning and execution.

use std::collections::HashMap;
use std::path::PathBuf;

use retro_junk_dat::matcher::MatchMethod;

/// A broken file reference found in a CUE or M3U file.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BrokenReference {
    /// The reference file (CUE or M3U) containing broken entries.
    pub ref_file: PathBuf,
    /// Format label (e.g., "CUE" or "M3U").
    pub format: String,
    /// Filenames referenced by the ref file that do not exist on disk.
    pub missing_targets: Vec<String>,
}

/// A planned rename action.
#[derive(Debug, Clone)]
pub struct RenameAction {
    /// Original file path.
    pub source: PathBuf,
    /// Target file path (same directory, new name).
    pub target: PathBuf,
    /// The canonical game name from the DAT.
    pub game_name: String,
    /// How the match was determined.
    pub matched_by: MatchMethod,
}

/// Progress information for callbacks.
#[derive(Debug, Clone)]
pub enum RenameProgress {
    /// Starting to scan a console folder.
    ScanningConsole {
        short_name: String,
        file_count: usize,
    },
    /// Analyzing/matching a file.
    MatchingFile {
        file_name: String,
        file_index: usize,
        total: usize,
    },
    /// Hashing a file for authoritative catalog matching.
    Hashing {
        file_name: String,
        bytes_done: u64,
        bytes_total: u64,
    },
    /// Finished all consoles.
    Done,
}

/// Options controlling rename behavior.
#[derive(Debug, Clone, Default)]
pub struct RenameOptions {
    /// Maximum number of ROMs to process.
    pub limit: Option<usize>,
}

/// Summary of a rename operation.
#[derive(Debug, Clone, Default)]
pub struct RenameSummary {
    pub renamed: usize,
    pub already_correct: usize,
    pub unmatched: usize,
    pub errors: Vec<String>,
    pub conflicts: Vec<String>,
    pub m3u_folders_renamed: usize,
    pub m3u_playlists_written: usize,
    pub cue_files_updated: usize,
    pub m3u_references_updated: usize,
    pub m3u_playlists_renamed: usize,
    /// Media files moved alongside their games (inside the same transaction).
    pub media_renamed: usize,
    /// gamelist.xml files updated alongside renames.
    pub gamelists_updated: usize,
}

/// A disc set that was identified as a problem during planning and will not
/// be renamed.
#[derive(Debug, Clone)]
pub struct SetProblem {
    pub cue: PathBuf,
    pub kind: SetProblemKind,
}

/// Why a disc set could not be safely planned.
#[derive(Debug, Clone)]
pub enum SetProblemKind {
    /// The cue references files that do not exist.
    Broken { missing: Vec<String> },
    /// A game was identified but the set failed full-track verification.
    NotVerified {
        /// Identified game name; empty when unidentified.
        game_name: String,
        issues: Vec<String>,
    },
    /// No DAT game matched.
    Unmatched { issues: Vec<String> },
}

/// Callback that, given a game's old-stem → new-stem map, returns full-file
/// rewrites (path, new content) to include in the rename transaction.
pub type GamelistRewriter<'a> = &'a dyn Fn(&HashMap<String, String>) -> Vec<(PathBuf, String)>;

/// Context for executing renames: where companion data lives so it moves
/// inside the same transaction as the game files.
#[derive(Default)]
pub struct ExecutionContext<'a> {
    /// Console-specific media directory (e.g., `roms-media/psx`), containing
    /// per-asset-type subdirectories. When set, matching media files are
    /// renamed in the same transaction as the game.
    pub media_dir: Option<PathBuf>,
    /// Given a game's old-stem → new-stem map, returns full-file rewrites
    /// (path, new content) to include in the transaction — used by callers
    /// to keep gamelist.xml in sync without this crate knowing the format.
    pub gamelist_rewriter: Option<GamelistRewriter<'a>>,
}

/// CRC32 and size of hashed data, recorded when hashing was attempted.
#[derive(Debug, Clone)]
pub struct HashInfo {
    /// CRC32 hash of the file data.
    pub crc32: String,
    /// Data size that was hashed (after header stripping).
    pub data_size: u64,
}

/// A file that couldn't be matched by serial or hash.
#[derive(Debug, Clone)]
pub struct UnmatchedFile {
    pub file: PathBuf,
    /// Hash details, present if hashing was attempted.
    pub hash_info: Option<HashInfo>,
}

/// A discrepancy between diagnostic serial evidence and authoritative hashes.
#[derive(Debug, Clone)]
pub struct MatchDiscrepancy {
    pub file: PathBuf,
    pub serial_game: String,
    pub hash_game: String,
}

/// A serial-related diagnostic warning.
#[derive(Debug, Clone)]
pub struct SerialWarning {
    pub file: PathBuf,
    pub kind: SerialWarningKind,
    /// Hash details, present if hash matching was attempted.
    pub hash_info: Option<HashInfo>,
    /// Whether the file ultimately matched by hash despite serial failure.
    pub matched_by_hash: bool,
}

/// The kind of serial warning.
#[derive(Debug, Clone)]
pub enum SerialWarningKind {
    /// Serial found in ROM header but no DAT match.
    NoMatch {
        full_serial: String,
        /// Extracted game code used for lookup; empty when none.
        game_code: String,
    },
    /// Serial matches multiple DAT entries (ambiguous) — fell back to hash.
    Ambiguous {
        full_serial: String,
        /// Extracted game code used for lookup; empty when none.
        game_code: String,
        candidates: Vec<String>,
    },
    /// Platform expects serial but none was found in ROM.
    Missing,
}
