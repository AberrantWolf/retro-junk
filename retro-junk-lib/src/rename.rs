use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use retro_junk_core::{AnalysisOptions, RomAnalyzer};
use retro_junk_dat::cache;
use retro_junk_dat::error::DatError;
use retro_junk_dat::matcher::{DatIndex, MatchMethod, MatchResult, SerialLookupResult};

use crate::hasher;
use crate::scanner::GameEntry;

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

/// Check a game entry for broken CUE/M3U references.
///
/// For `SingleFile` entries, checks the parent directory for CUE/M3U files.
/// For `MultiDisc` entries, checks each disc file's parent directory.
/// Returns an empty vec if no broken references are found.
pub fn check_broken_references(entry: &GameEntry) -> Vec<BrokenReference> {
    let dirs: Vec<PathBuf> = match entry {
        GameEntry::SingleFile(path) => path
            .parent()
            .map(|d| vec![d.to_path_buf()])
            .unwrap_or_default(),
        GameEntry::MultiDisc { files, .. } => {
            let mut seen = std::collections::HashSet::new();
            files
                .iter()
                .filter_map(|p| p.parent().map(|d| d.to_path_buf()))
                .filter(|d| seen.insert(d.clone()))
                .collect()
        }
    };

    let mut broken = Vec::new();
    let formats: &[&dyn RefFileFormat] = &[&CueFormat, &M3uFormat];

    for dir in &dirs {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for dir_entry in entries.flatten() {
            let path = dir_entry.path();
            if !path.is_file() {
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

            for &fmt in formats {
                if !ext.eq_ignore_ascii_case(fmt.extension()) {
                    continue;
                }

                let content = match fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let missing: Vec<String> = content
                    .lines()
                    .filter_map(|line| {
                        let ref_line = fmt.extract_reference(line)?;
                        if !dir.join(&ref_line.filename).exists() {
                            Some(ref_line.filename)
                        } else {
                            None
                        }
                    })
                    .collect();

                if !missing.is_empty() {
                    broken.push(BrokenReference {
                        ref_file: path.clone(),
                        format: fmt.label().to_string(),
                        missing_targets: missing,
                    });
                }
            }
        }
    }

    broken
}

/// File extensions that represent disc-image entry points for M3U playlists.
const M3U_ENTRY_POINT_EXTENSIONS: &[&str] = &["cue", "chd", "iso", "gdi", "cso", "pbp"];

/// A planned rename action.
#[derive(Debug, Clone)]
pub struct RenameAction {
    /// Original file path
    pub source: PathBuf,
    /// Target file path (same directory, new name)
    pub target: PathBuf,
    /// The canonical game name from the DAT
    pub game_name: String,
    /// How the match was determined
    pub matched_by: MatchMethod,
}

/// Progress information for callbacks.
#[derive(Debug, Clone)]
pub enum RenameProgress {
    /// Starting to scan a console folder
    ScanningConsole {
        short_name: String,
        file_count: usize,
    },
    /// Analyzing/matching a file
    MatchingFile {
        file_name: String,
        file_index: usize,
        total: usize,
    },
    /// Hashing a file (with --hash)
    Hashing {
        file_name: String,
        bytes_done: u64,
        bytes_total: u64,
    },
    /// Finished all consoles
    Done,
}

/// Options controlling rename behavior.
#[derive(Debug, Clone, Default)]
pub struct RenameOptions {
    /// Force CRC32-based matching instead of serial/name
    pub hash_mode: bool,
    /// Custom DAT directory (instead of cache)
    pub dat_dir: Option<PathBuf>,
    /// Maximum number of ROMs to process
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
}

/// A file that couldn't be matched by serial or hash.
#[derive(Debug, Clone)]
pub struct UnmatchedFile {
    pub file: PathBuf,
    /// CRC32 hash of the file data (if hashing was attempted)
    pub crc32: Option<String>,
    /// Data size that was hashed (after header stripping)
    pub data_size: Option<u64>,
}

/// A discrepancy between serial-based and hash-based matching (reported in --hash mode).
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
    /// CRC32 of the file data (if hash matching was attempted)
    pub crc32: Option<String>,
    /// Data size that was hashed (after header stripping)
    pub data_size: Option<u64>,
    /// Whether the file ultimately matched by hash despite serial failure
    pub matched_by_hash: bool,
}

/// The kind of serial warning.
#[derive(Debug, Clone)]
pub enum SerialWarningKind {
    /// Serial found in ROM header but no DAT match
    NoMatch {
        full_serial: String,
        game_code: Option<String>,
    },
    /// Serial matches multiple DAT entries (ambiguous) — fell back to hash
    Ambiguous {
        full_serial: String,
        game_code: Option<String>,
        candidates: Vec<String>,
    },
    /// Platform expects serial but none was found in ROM
    Missing,
}

/// Internal result from serial matching, carrying diagnostic info.
struct SerialMatchOutcome {
    result: Option<MatchResult>,
    /// Full serial from ROM header (e.g., "NUS-NSME-USA")
    full_serial: Option<String>,
    /// Extracted game code used for DAT lookup (e.g., "NSME")
    game_code: Option<String>,
    /// When serial matched multiple games, the candidate names
    ambiguous_candidates: Option<Vec<String>>,
}

/// A planned M3U folder rename + playlist write for a multi-disc set.
#[derive(Debug, Clone)]
pub struct M3uAction {
    /// Current .m3u folder path
    pub source_folder: PathBuf,
    /// Target .m3u folder path (may equal source if already correct)
    pub target_folder: PathBuf,
    /// Canonical game name without disc number
    pub game_name: String,
    /// Ordered disc filenames for the .m3u playlist contents
    pub playlist_entries: Vec<String>,
}

/// Input data for a single disc in a multi-disc set, used by `plan_m3u_action()`.
#[derive(Debug, Clone)]
pub struct DiscMatchData {
    pub file_path: PathBuf,
    /// DatGame.name (e.g., "Final Fantasy VII (USA) (Disc 1)")
    pub game_name: String,
    /// DatRom.name — what the file should be renamed to (e.g., "Final Fantasy VII (USA) (Disc 1).chd")
    pub target_filename: String,
}

/// Plan M3U actions for a multi-disc set given pre-resolved disc data.
///
/// When `game_name_override` is `Some`, uses that name directly instead of
/// deriving it from per-disc DAT names. This is used by the GUI when the
/// catalog DB has already resolved the canonical game name.
///
/// Returns `None` if the folder and playlist are already correct.
pub fn plan_m3u_action(
    source_folder: &Path,
    discs: &[DiscMatchData],
    existing_m3u_contents: Option<&str>,
    game_name_override: Option<&str>,
) -> Option<M3uAction> {
    if discs.is_empty() {
        return None;
    }

    // Use override if provided, otherwise derive from per-disc DAT names
    let base_game_name = match game_name_override {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => {
            let game_names: Vec<&str> = discs.iter().map(|d| d.game_name.as_str()).collect();
            derive_base_game_name(&game_names)
        }
    };
    if base_game_name.is_empty() {
        return None;
    }

    // Target folder: parent / "{base_game_name}.m3u"
    let target_folder = match source_folder.parent() {
        Some(p) => p.join(format!("{}.m3u", base_game_name)),
        None => return None,
    };

    // Build playlist entries: target filenames for entry-point files, sorted by disc number
    // or alphabetically if no disc numbers are present.
    let mut playlist_entries: Vec<(Option<u32>, String)> = discs
        .iter()
        .filter(|d| is_m3u_entry_point(&d.target_filename))
        .map(|d| (extract_disc_number(&d.game_name), d.target_filename.clone()))
        .collect();
    if playlist_entries.iter().any(|(d, _)| d.is_some()) {
        playlist_entries.sort_by_key(|(disc, _)| disc.unwrap_or(u32::MAX));
    } else {
        playlist_entries.sort_by(|(_, a), (_, b)| a.cmp(b));
    }
    let playlist_entries: Vec<String> =
        playlist_entries.into_iter().map(|(_, name)| name).collect();

    // Check if everything is already correct
    let folder_correct = source_folder == target_folder;
    let existing_m3u_correct = if folder_correct {
        if let Some(contents) = existing_m3u_contents {
            let existing_lines: Vec<&str> = contents.lines().filter(|l| !l.is_empty()).collect();
            existing_lines
                == playlist_entries
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
        } else {
            // Check for the expected .m3u file on disk
            let expected_m3u_path = source_folder.join(format!("{}.m3u", base_game_name));
            if expected_m3u_path.exists() {
                let contents = fs::read_to_string(&expected_m3u_path).unwrap_or_default();
                let existing_lines: Vec<&str> =
                    contents.lines().filter(|l| !l.is_empty()).collect();
                existing_lines
                    == playlist_entries
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
            } else {
                false
            }
        }
    } else {
        false
    };

    if folder_correct && existing_m3u_correct {
        return None;
    }

    Some(M3uAction {
        source_folder: source_folder.to_path_buf(),
        target_folder,
        game_name: base_game_name,
        playlist_entries,
    })
}

/// Result of executing a single M3U action (internal to rename module).
#[derive(Debug, Default)]
struct M3uExecutionResult {
    playlist_written: bool,
    folder_renamed: bool,
}

/// Execute a single M3U action: write playlist file, rename folder.
///
/// This does NOT rename individual disc files — the caller is responsible for that.
/// Execution order: write playlist first (using source_folder path), then rename folder.
fn execute_m3u_action(action: &M3uAction, errors: &mut Vec<String>) -> M3uExecutionResult {
    let mut result = M3uExecutionResult::default();

    // Write .m3u playlist file (using source folder path, before folder rename)
    if !action.playlist_entries.is_empty() {
        // Delete any existing .m3u files inside the folder
        if let Ok(entries) = fs::read_dir(&action.source_folder) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file()
                    && let Some(ext) = path.extension().and_then(|e| e.to_str())
                    && ext.eq_ignore_ascii_case("m3u")
                {
                    let _ = fs::remove_file(&path);
                }
            }
        }

        let playlist_name = format!("{}.m3u", action.game_name);
        let playlist_path = action.source_folder.join(&playlist_name);
        let contents = action.playlist_entries.join("\n") + "\n";
        match fs::write(&playlist_path, contents) {
            Ok(()) => result.playlist_written = true,
            Err(e) => {
                errors.push(format!(
                    "Failed to write playlist {}: {}",
                    playlist_path.display(),
                    e,
                ));
            }
        }
    }

    // Rename .m3u folder (last, so playlist write used valid path)
    if action.source_folder != action.target_folder {
        if action.target_folder.exists() {
            errors.push(format!(
                "Target folder already exists: {}",
                action.target_folder.display()
            ));
        } else {
            match fs::rename(&action.source_folder, &action.target_folder) {
                Ok(()) => result.folder_renamed = true,
                Err(e) => {
                    errors.push(format!(
                        "Failed to rename folder {:?} -> {:?}: {}",
                        action.source_folder.file_name().unwrap_or_default(),
                        action.target_folder.file_name().unwrap_or_default(),
                        e,
                    ));
                }
            }
        }
    }

    result
}

/// Input for a single M3U folder rename operation.
///
/// Encapsulates all data needed to rename disc files, fix CUE/M3U references,
/// write playlists, and rename the M3U folder. Used by both CLI and GUI.
#[derive(Debug, Clone)]
pub struct M3uRenameJob {
    /// The .m3u folder containing disc files
    pub source_folder: PathBuf,
    /// Per-disc rename data (file_path → game_name + target_filename)
    pub discs: Vec<DiscMatchData>,
    /// Pre-resolved game name (from catalog DB); skips derive_base_game_name
    pub game_name_override: Option<String>,
}

/// Result of executing a single M3U folder rename via `execute_m3u_rename()`.
#[derive(Debug, Default)]
pub struct M3uRenameResult {
    pub discs_renamed: usize,
    pub cue_files_updated: usize,
    pub m3u_references_updated: usize,
    pub playlist_written: bool,
    pub playlist_renamed: bool,
    pub folder_renamed: bool,
    pub final_folder: PathBuf,
    pub errors: Vec<String>,
}

/// Execute the full rename flow for a single M3U folder.
///
/// Steps:
/// 1. Rename disc files
/// 2. Fix CUE FILE references broken by the renames
/// 3. Fix M3U playlist entries broken by the renames
/// 4. Plan M3U action (folder rename + playlist write)
/// 5. Rename misnamed inner `.m3u` file (if playlist won't be rewritten)
/// 6. Execute M3U action (write playlist, rename folder)
pub fn execute_m3u_rename(job: &M3uRenameJob) -> M3uRenameResult {
    let mut result = M3uRenameResult {
        final_folder: job.source_folder.clone(),
        ..Default::default()
    };

    // Step 1: Rename disc files
    let mut rename_map: HashMap<String, String> = HashMap::new();
    for disc in &job.discs {
        let target = job.source_folder.join(&disc.target_filename);
        if disc.file_path == target {
            continue;
        }
        let old_name = disc
            .file_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        match fs::rename(&disc.file_path, &target) {
            Ok(()) => {
                result.discs_renamed += 1;
                rename_map.insert(old_name, disc.target_filename.clone());
            }
            Err(e) => {
                result.errors.push(format!(
                    "Failed to rename '{}': {}",
                    disc.file_path.display(),
                    e,
                ));
            }
        }
    }

    // Step 2: Fix CUE FILE references
    result.cue_files_updated =
        fix_cue_references_in_dir(&job.source_folder, &rename_map, &mut result.errors);

    // Step 3: Fix M3U playlist entries
    result.m3u_references_updated =
        fix_m3u_references_in_dir(&job.source_folder, &rename_map, &mut result.errors);

    // Step 4: Plan M3U action
    if let Some(action) = plan_m3u_action(
        &job.source_folder,
        &job.discs,
        None,
        job.game_name_override.as_deref(),
    ) {
        // Step 5: Rename misnamed inner .m3u (only when playlist won't be rewritten)
        if action.playlist_entries.is_empty() {
            let expected = format!("{}.m3u", action.game_name);
            if let Some((src, dst)) = detect_misnamed_m3u(&job.source_folder, &expected) {
                match fs::rename(&src, &dst) {
                    Ok(()) => result.playlist_renamed = true,
                    Err(e) => {
                        result
                            .errors
                            .push(format!("Failed to rename inner playlist: {}", e));
                    }
                }
            }
        }

        // Step 6: Execute M3U action (write playlist, rename folder)
        let m3u_exec = execute_m3u_action(&action, &mut result.errors);
        result.playlist_written = m3u_exec.playlist_written;
        result.folder_renamed = m3u_exec.folder_renamed;
        if m3u_exec.folder_renamed {
            result.final_folder = action.target_folder;
        }
    }

    result
}

/// Result of planning renames for a single console folder.
#[derive(Debug)]
pub struct RenamePlan {
    /// Single-file renames (non-M3U). Disc renames live inside `m3u_jobs`.
    pub renames: Vec<RenameAction>,
    pub already_correct: Vec<PathBuf>,
    pub unmatched: Vec<UnmatchedFile>,
    pub conflicts: Vec<(PathBuf, String)>,
    /// Files where serial and hash matched different games (--hash mode only)
    pub discrepancies: Vec<MatchDiscrepancy>,
    /// Serial-related diagnostics (serial lookup failed, or missing serial)
    pub serial_warnings: Vec<SerialWarning>,
    /// M3U folder jobs: disc renames + CUE/M3U fix + playlist + folder rename
    pub m3u_jobs: Vec<M3uRenameJob>,
    /// CUE files with broken FILE references in non-M3U dirs (pre-existing)
    pub broken_cue_files: Vec<PathBuf>,
    /// M3U playlist files with broken entries in non-M3U dirs (pre-existing)
    pub broken_m3u_files: Vec<PathBuf>,
}

impl RenamePlan {
    /// Total number of file rename operations (single files + M3U disc renames).
    pub fn total_renames(&self) -> usize {
        self.renames.len() + self.m3u_jobs.iter().map(|j| j.discs.len()).sum::<usize>()
    }

    /// Whether this plan has any work to do.
    pub fn has_actions(&self) -> bool {
        !self.renames.is_empty() || !self.m3u_jobs.is_empty()
    }

    /// Whether this plan has any problems (conflicts, unmatched, broken refs).
    pub fn has_problems(&self) -> bool {
        !self.conflicts.is_empty()
            || !self.unmatched.is_empty()
            || !self.broken_cue_files.is_empty()
            || !self.broken_m3u_files.is_empty()
    }
}

use retro_junk_core::disc::{derive_base_game_name, extract_disc_number};

/// Returns true for file extensions that are M3U entry points (playable disc images).
/// Returns false for companion data files (.bin, .img) that shouldn't appear in playlists.
pub fn is_m3u_entry_point(filename: &str) -> bool {
    let ext = Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    M3U_ENTRY_POINT_EXTENSIONS.iter().any(|&e| e == ext)
}

/// Plan renames for a single console folder.
///
/// Uses the analyzer to extract serial/name from each file, then matches
/// against the DAT index. Falls back to hashing when serial/name matching
/// fails (unless `hash_mode` is set, in which case all files are hashed).
pub fn plan_renames(
    folder: &Path,
    analyzer: &dyn RomAnalyzer,
    options: &RenameOptions,
    progress: &dyn Fn(RenameProgress),
) -> Result<RenamePlan, DatError> {
    let dat_names = analyzer.dat_names();
    if dat_names.is_empty() {
        return Err(DatError::cache(format!(
            "No DAT support for platform '{}'",
            analyzer.platform_name()
        )));
    }

    // Load DATs and merge into a single index
    let dat_source = analyzer.dat_source();
    let download_ids = analyzer.dat_download_ids();
    let dats = cache::load_dats(
        analyzer.short_name(),
        dat_names,
        download_ids,
        options.dat_dir.as_deref(),
        dat_source,
    )?;
    let index = DatIndex::from_dats(dats);

    // Collect ROM files (including inside .m3u subdirectories)
    let extensions = crate::scanner::extension_set(analyzer.file_extensions());
    let game_entries = crate::scanner::scan_game_entries(folder, &extensions)
        .map_err(|e| DatError::cache(format!("Error scanning {}: {}", folder.display(), e)))?;

    let mut files: Vec<PathBuf> = game_entries
        .iter()
        .flat_map(|entry| entry.all_files())
        .cloned()
        .collect();
    if let Some(max) = options.limit {
        files.truncate(max);
    }

    progress(RenameProgress::ScanningConsole {
        short_name: analyzer.short_name().to_string(),
        file_count: files.len(),
    });

    let mut renames = Vec::new();
    let mut already_correct = Vec::new();
    let mut unmatched = Vec::new();
    let mut discrepancies = Vec::new();
    let mut serial_warnings = Vec::new();
    // Track file → (game_name, target_filename) for M3U post-processing
    let mut file_game_names: HashMap<PathBuf, (String, String)> = HashMap::new();
    let analysis_options = AnalysisOptions::new().quick(true);

    for (i, file_path) in files.iter().enumerate() {
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();

        progress(RenameProgress::MatchingFile {
            file_name: file_name.clone(),
            file_index: i,
            total: files.len(),
        });

        // Track hash info for diagnostics if the file ends up unmatched
        let mut last_hash: Option<(String, u64)> = None;

        let match_result = if options.hash_mode {
            // Hash mode: hash is authoritative, but also check serial for discrepancies
            let hash_outcome = match_by_hash(file_path, &index, analyzer, progress)?;
            last_hash = Some((hash_outcome.crc32, hash_outcome.data_size));
            let serial_outcome = match_by_serial(file_path, analyzer, &analysis_options, &index);

            // Report discrepancy if both matched but to different games
            if let (Some(hr), Some(sr)) = (&hash_outcome.result, &serial_outcome.result)
                && hr.game_index != sr.game_index
            {
                discrepancies.push(MatchDiscrepancy {
                    file: file_path.clone(),
                    serial_game: index.games[sr.game_index].name.clone(),
                    hash_game: index.games[hr.game_index].name.clone(),
                });
            }

            hash_outcome.result
        } else {
            // Default mode: try serial first, then always fall back to hash
            let serial_outcome = match_by_serial(file_path, analyzer, &analysis_options, &index);

            if serial_outcome.result.is_some() {
                serial_outcome.result
            } else {
                // Serial failed — try hash, then create serial warning with hash info
                let hash_outcome = match_by_hash(file_path, &index, analyzer, progress)?;
                last_hash = Some((hash_outcome.crc32.clone(), hash_outcome.data_size));

                if let Some(ref candidates) = serial_outcome.ambiguous_candidates {
                    // Serial matched multiple games — report ambiguity
                    serial_warnings.push(SerialWarning {
                        file: file_path.clone(),
                        kind: SerialWarningKind::Ambiguous {
                            full_serial: serial_outcome.full_serial.clone().unwrap_or_default(),
                            game_code: serial_outcome.game_code.clone(),
                            candidates: candidates.clone(),
                        },
                        crc32: Some(hash_outcome.crc32.clone()),
                        data_size: Some(hash_outcome.data_size),
                        matched_by_hash: hash_outcome.result.is_some(),
                    });
                } else if let Some(ref full_serial) = serial_outcome.full_serial {
                    serial_warnings.push(SerialWarning {
                        file: file_path.clone(),
                        kind: SerialWarningKind::NoMatch {
                            full_serial: full_serial.clone(),
                            game_code: serial_outcome.game_code.clone(),
                        },
                        crc32: Some(hash_outcome.crc32.clone()),
                        data_size: Some(hash_outcome.data_size),
                        matched_by_hash: hash_outcome.result.is_some(),
                    });
                } else if analyzer.expects_serial() {
                    serial_warnings.push(SerialWarning {
                        file: file_path.clone(),
                        kind: SerialWarningKind::Missing,
                        crc32: Some(hash_outcome.crc32.clone()),
                        data_size: Some(hash_outcome.data_size),
                        matched_by_hash: hash_outcome.result.is_some(),
                    });
                }

                hash_outcome.result
            }
        };

        if let Some(result) = match_result {
            let game = &index.games[result.game_index];
            let rom = &game.roms[result.rom_index];

            // Target path: same directory as the source file, DAT-canonical stem + original extension
            let parent = file_path.parent().unwrap_or(folder);
            let dat_stem = Path::new(&rom.name)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(&rom.name);
            let target = match file_path.extension().and_then(|e| e.to_str()) {
                Some(ext) => parent.join(format!("{}.{}", dat_stem, ext)),
                None => parent.join(&rom.name),
            };

            let target_filename = target
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            file_game_names.insert(file_path.clone(), (game.name.clone(), target_filename));

            if *file_path == target {
                already_correct.push(file_path.clone());
            } else {
                renames.push(RenameAction {
                    source: file_path.clone(),
                    target,
                    game_name: game.name.clone(),
                    matched_by: result.method,
                });
            }
        } else {
            let (crc32, data_size) = match last_hash {
                Some((c, s)) => (Some(c), Some(s)),
                None => (None, None),
            };
            unmatched.push(UnmatchedFile {
                file: file_path.clone(),
                crc32,
                data_size,
            });
        }
    }

    // Detect conflicts: multiple files mapping to the same target
    let mut target_map: HashMap<PathBuf, Vec<usize>> = HashMap::new();
    for (i, rename) in renames.iter().enumerate() {
        target_map.entry(rename.target.clone()).or_default().push(i);
    }

    let mut conflicts = Vec::new();
    let conflict_indices: std::collections::HashSet<usize> = target_map
        .iter()
        .filter(|(_, indices)| indices.len() > 1)
        .flat_map(|(target, indices)| {
            conflicts.push((
                target.clone(),
                format!(
                    "Multiple files map to {:?}: {}",
                    target.file_name().unwrap_or_default(),
                    indices
                        .iter()
                        .map(|&i| renames[i]
                            .source
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
            indices.clone()
        })
        .collect();

    // Remove conflicting renames
    let clean_renames: Vec<RenameAction> = renames
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !conflict_indices.contains(i))
        .map(|(_, r)| r)
        .collect();

    // M3U post-processing: build M3uRenameJobs for multi-disc sets.
    // Each job owns its disc renames, CUE/M3U fixing, playlist, and folder rename.
    let mut m3u_jobs = Vec::new();
    for entry in &game_entries {
        if let crate::scanner::GameEntry::MultiDisc { name: _, files } = entry {
            let discs: Vec<DiscMatchData> = files
                .iter()
                .filter_map(|f| {
                    file_game_names
                        .get(f)
                        .map(|(game_name, target_name)| DiscMatchData {
                            file_path: f.clone(),
                            game_name: game_name.clone(),
                            target_filename: target_name.clone(),
                        })
                })
                .collect();

            if discs.is_empty() {
                continue;
            }

            let source_folder = match files[0].parent() {
                Some(p) => p.to_path_buf(),
                None => continue,
            };

            // Only create a job if there's actual work: disc renames or M3U action needed
            let any_disc_rename = discs
                .iter()
                .any(|d| d.file_path != source_folder.join(&d.target_filename));
            let needs_m3u_action = plan_m3u_action(&source_folder, &discs, None, None).is_some();

            if any_disc_rename || needs_m3u_action {
                m3u_jobs.push(M3uRenameJob {
                    source_folder,
                    discs,
                    game_name_override: None,
                });
            }
        }
    }

    // Separate disc renames from single-file renames: disc files in M3U jobs
    // are handled by execute_m3u_rename(), not by the top-level rename step.
    let m3u_job_files: std::collections::HashSet<PathBuf> = m3u_jobs
        .iter()
        .flat_map(|j| j.discs.iter().map(|d| d.file_path.clone()))
        .collect();

    let single_renames: Vec<RenameAction> = clean_renames
        .into_iter()
        .filter(|r| !m3u_job_files.contains(&r.source))
        .collect();
    let single_already_correct: Vec<PathBuf> = already_correct
        .into_iter()
        .filter(|p| !m3u_job_files.contains(p))
        .collect();

    // Detect pre-existing broken CUE and M3U references in non-M3U dirs only.
    // M3U dirs get their references fixed inside execute_m3u_rename().
    let m3u_dir_set: std::collections::HashSet<PathBuf> =
        m3u_jobs.iter().map(|j| j.source_folder.clone()).collect();
    let non_m3u_files: Vec<PathBuf> = files
        .iter()
        .filter(|f| {
            !f.parent()
                .is_some_and(|p| m3u_dir_set.contains(&p.to_path_buf()))
        })
        .cloned()
        .collect();
    let broken_cue_files = detect_broken_cue_files(&non_m3u_files);
    let broken_m3u_files = detect_broken_m3u_playlists(&non_m3u_files);

    Ok(RenamePlan {
        renames: single_renames,
        already_correct: single_already_correct,
        unmatched,
        conflicts,
        discrepancies,
        serial_warnings,
        m3u_jobs,
        broken_cue_files,
        broken_m3u_files,
    })
}

/// Try to match a file by serial number only (no hashing).
///
/// Returns a `SerialMatchOutcome` with diagnostic info regardless of success,
/// so the caller can generate appropriate warnings.
fn match_by_serial(
    file_path: &Path,
    analyzer: &dyn RomAnalyzer,
    analysis_options: &AnalysisOptions,
    index: &DatIndex,
) -> SerialMatchOutcome {
    let no_match = SerialMatchOutcome {
        result: None,
        full_serial: None,
        game_code: None,
        ambiguous_candidates: None,
    };

    let mut file = match fs::File::open(file_path) {
        Ok(f) => f,
        Err(_) => return no_match,
    };
    let info = match analyzer.analyze(&mut file, analysis_options) {
        Ok(i) => i,
        Err(_) => return no_match,
    };

    let serial = match info.serial_number {
        Some(s) => s,
        None => return no_match,
    };

    let game_code = analyzer.extract_dat_game_code(&serial);
    let lookup = index.match_by_serial(&serial, game_code.as_deref());

    match lookup {
        SerialLookupResult::Match(result) => SerialMatchOutcome {
            result: Some(result),
            full_serial: Some(serial),
            game_code,
            ambiguous_candidates: None,
        },
        SerialLookupResult::Ambiguous { candidates } => SerialMatchOutcome {
            result: None,
            full_serial: Some(serial),
            game_code,
            ambiguous_candidates: Some(candidates),
        },
        SerialLookupResult::NotFound => SerialMatchOutcome {
            result: None,
            full_serial: Some(serial),
            game_code,
            ambiguous_candidates: None,
        },
    }
}

/// Result of a hash matching attempt, carrying hash info regardless of match success.
struct HashMatchOutcome {
    result: Option<MatchResult>,
    /// CRC32 of the hashed data
    crc32: String,
    /// Size of data that was hashed (after header stripping)
    data_size: u64,
}

/// Match a file by computing its CRC32 hash (with SHA1 fallback).
fn match_by_hash(
    file_path: &Path,
    index: &DatIndex,
    analyzer: &dyn RomAnalyzer,
    progress: &dyn Fn(RenameProgress),
) -> Result<HashMatchOutcome, DatError> {
    let mut file = fs::File::open(file_path)?;
    let file_name = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("?")
        .to_string();

    let hashes = hasher::compute_crc32_with_progress(&mut file, analyzer, &|done, total| {
        progress(RenameProgress::Hashing {
            file_name: file_name.clone(),
            bytes_done: done,
            bytes_total: total,
        });
    })?;

    let crc32 = hashes.crc32.clone();
    let data_size = hashes.data_size;

    if let Some(result) = index.match_by_hash(hashes.data_size, &hashes) {
        return Ok(HashMatchOutcome {
            result: Some(result),
            crc32,
            data_size,
        });
    }

    // If CRC32 didn't match, try SHA1 (recompute with SHA1)
    // Only do this if we have size candidates (to avoid pointless rehashing)
    if index.candidates_by_size(hashes.data_size).is_some() {
        let mut file = fs::File::open(file_path)?;
        let full_hashes = hasher::compute_crc32_sha1(&mut file, analyzer)?;
        if let Some(result) = index.match_by_hash(full_hashes.data_size, &full_hashes) {
            return Ok(HashMatchOutcome {
                result: Some(result),
                crc32,
                data_size,
            });
        }
    }

    Ok(HashMatchOutcome {
        result: None,
        crc32,
        data_size,
    })
}

/// Execute a rename plan, performing the actual file renames and M3U operations.
///
/// Execution order:
/// 1. Rename single files (non-M3U)
/// 2. Fix CUE/M3U references in non-M3U directories
/// 3. Execute each M3U job (disc renames + CUE/M3U fix + playlist + folder rename)
pub fn execute_renames(plan: &RenamePlan) -> RenameSummary {
    let mut summary = RenameSummary {
        already_correct: plan.already_correct.len(),
        ..Default::default()
    };

    for (_, msg) in &plan.conflicts {
        summary.conflicts.push(msg.clone());
    }

    // Step 1: Rename single files (disc renames are handled by M3U jobs)
    for rename in &plan.renames {
        if rename.target.exists() && rename.source != rename.target {
            summary.errors.push(format!(
                "Target already exists: {}",
                rename.target.display()
            ));
            continue;
        }

        match fs::rename(&rename.source, &rename.target) {
            Ok(()) => summary.renamed += 1,
            Err(e) => {
                summary.errors.push(format!(
                    "Failed to rename {:?} -> {:?}: {}",
                    rename.source.file_name().unwrap_or_default(),
                    rename.target.file_name().unwrap_or_default(),
                    e,
                ));
            }
        }
    }

    // Step 2: Fix CUE/M3U references in non-M3U directories
    let mut dir_rename_maps: HashMap<PathBuf, HashMap<String, String>> = HashMap::new();
    let mut fix_dirs: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    for rename in &plan.renames {
        if rename.source == rename.target {
            continue;
        }
        let dir = rename
            .source
            .parent()
            .unwrap_or(Path::new("."))
            .to_path_buf();
        let old_name = rename
            .source
            .file_name()
            .unwrap_or(std::ffi::OsStr::new("?"))
            .to_string_lossy()
            .to_string();
        let new_name = rename
            .target
            .file_name()
            .unwrap_or(std::ffi::OsStr::new("?"))
            .to_string_lossy()
            .to_string();
        dir_rename_maps
            .entry(dir.clone())
            .or_default()
            .insert(old_name, new_name);
        fix_dirs.insert(dir);
    }

    for cue_path in &plan.broken_cue_files {
        if let Some(dir) = cue_path.parent() {
            fix_dirs.insert(dir.to_path_buf());
        }
    }
    for m3u_path in &plan.broken_m3u_files {
        if let Some(dir) = m3u_path.parent() {
            fix_dirs.insert(dir.to_path_buf());
        }
    }

    for dir in &fix_dirs {
        let empty_map = HashMap::new();
        let rename_map = dir_rename_maps.get(dir).unwrap_or(&empty_map);
        summary.cue_files_updated +=
            fix_cue_references_in_dir(dir, rename_map, &mut summary.errors);
        summary.m3u_references_updated +=
            fix_m3u_references_in_dir(dir, rename_map, &mut summary.errors);
    }

    // Step 3: Execute M3U jobs (each handles disc renames + CUE/M3U fix + playlist + folder)
    for job in &plan.m3u_jobs {
        let result = execute_m3u_rename(job);
        summary.renamed += result.discs_renamed;
        summary.cue_files_updated += result.cue_files_updated;
        summary.m3u_references_updated += result.m3u_references_updated;
        if result.playlist_written {
            summary.m3u_playlists_written += 1;
        }
        if result.playlist_renamed {
            summary.m3u_playlists_renamed += 1;
        }
        if result.folder_renamed {
            summary.m3u_folders_renamed += 1;
        }
        summary.errors.extend(result.errors);
    }

    summary
}

/// Format a MatchMethod for display.
pub fn format_match_method(method: &MatchMethod) -> &'static str {
    match method {
        MatchMethod::Serial => "serial",
        MatchMethod::Crc32 => "CRC32",
        MatchMethod::Sha1 => "SHA1",
    }
}

// ---------------------------------------------------------------------------
// Unified reference-file fixing (CUE sheets and M3U playlists)
// ---------------------------------------------------------------------------
//
// CUE and M3U files both contain references to other files that can break
// when those files are renamed. The detection, directory-level fixing, single-
// file fixing, and filename-matching logic is structurally identical for both
// formats. The `RefFileFormat` trait captures the differences so all four
// layers share a single implementation.

/// Describes how a reference-file format (CUE or M3U) works, so the generic
/// fix/detect code can handle both without duplication.
trait RefFileFormat {
    /// File extension for the reference file itself (e.g., "cue" or "m3u").
    fn extension(&self) -> &'static str;

    /// Human-readable label for error messages (e.g., "CUE" or "M3U").
    fn label(&self) -> &'static str;

    /// Extract referenced filenames from the file content, paired with the
    /// original line. Returns `None` for lines that are not references (e.g.,
    /// CUE TRACK/INDEX lines, M3U comments/blanks).
    fn extract_reference<'a>(&self, line: &'a str) -> Option<RefLine<'a>>;

    /// Reconstruct a line with the corrected filename.
    fn rebuild_line(&self, original_line: &str, new_filename: &str, ref_line: &RefLine) -> String;

    /// Serialize lines back to file content after fixing.
    fn serialize(&self, original_content: &str, lines: &[String]) -> String;

    /// Find the correct replacement for a broken reference.
    fn find_correction(
        &self,
        old_ref: &str,
        ref_file_path: &Path,
        dir: &Path,
        rename_map: &HashMap<String, String>,
    ) -> Option<String>;
}

/// A parsed reference extracted from a single line.
struct RefLine<'a> {
    /// The referenced filename (e.g., "game.bin" from a CUE FILE directive).
    filename: String,
    /// Opaque format-specific data needed by `rebuild_line` (e.g., CUE file type).
    extra: &'a str,
}

// --- CUE format implementation ---

struct CueFormat;

impl RefFileFormat for CueFormat {
    fn extension(&self) -> &'static str {
        "cue"
    }

    fn label(&self) -> &'static str {
        "CUE"
    }

    fn extract_reference<'a>(&self, line: &'a str) -> Option<RefLine<'a>> {
        let trimmed = line.trim();
        if !trimmed.to_uppercase().starts_with("FILE ") {
            return None;
        }
        // We need to parse the FILE directive. Since RefLine borrows from the
        // original line for `extra`, we store nothing there and reconstruct.
        // Actually, we need the file_type portion. We'll use a small trick:
        // store the trimmed suffix offset so rebuild_line can grab it.
        let (filename, _file_type) = parse_cue_file_directive(trimmed)?;
        Some(RefLine {
            filename,
            extra: trimmed, // pass trimmed line so rebuild_line can re-parse
        })
    }

    fn rebuild_line(&self, original_line: &str, new_filename: &str, ref_line: &RefLine) -> String {
        // Re-parse the file type from the stored trimmed line
        let file_type = parse_cue_file_directive(ref_line.extra)
            .map(|(_, ft)| ft)
            .unwrap_or_default();
        let trimmed = original_line.trim();
        let indent = &original_line[..original_line.len() - trimmed.len()];
        format!("{}FILE \"{}\" {}", indent, new_filename, file_type)
    }

    fn serialize(&self, original_content: &str, lines: &[String]) -> String {
        let mut out = lines.join("\n");
        out.push('\n');
        // Preserve original: don't add trailing newline if original didn't have one
        if !original_content.ends_with('\n') && out.ends_with('\n') {
            out.pop();
        }
        out
    }

    fn find_correction(
        &self,
        old_ref: &str,
        ref_file_path: &Path,
        dir: &Path,
        rename_map: &HashMap<String, String>,
    ) -> Option<String> {
        let cue_stem = ref_file_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        find_correct_bin_filename(old_ref, cue_stem, dir, rename_map)
    }
}

// --- M3U format implementation ---

struct M3uFormat;

impl RefFileFormat for M3uFormat {
    fn extension(&self) -> &'static str {
        "m3u"
    }

    fn label(&self) -> &'static str {
        "M3U"
    }

    fn extract_reference<'a>(&self, line: &'a str) -> Option<RefLine<'a>> {
        let entry = line.trim();
        if entry.is_empty() || entry.starts_with('#') {
            return None;
        }
        Some(RefLine {
            filename: entry.to_string(),
            extra: "",
        })
    }

    fn rebuild_line(
        &self,
        _original_line: &str,
        new_filename: &str,
        _ref_line: &RefLine,
    ) -> String {
        new_filename.to_string()
    }

    fn serialize(&self, _original_content: &str, lines: &[String]) -> String {
        lines.join("\n") + "\n"
    }

    fn find_correction(
        &self,
        old_ref: &str,
        _ref_file_path: &Path,
        dir: &Path,
        rename_map: &HashMap<String, String>,
    ) -> Option<String> {
        find_correct_m3u_entry(old_ref, dir, rename_map)
    }
}

// --- Generic operations on any RefFileFormat ---

/// Detect reference files with broken entries in directories containing
/// the given files. Returns paths to reference files with at least one broken entry.
fn detect_broken_ref_files(fmt: &dyn RefFileFormat, files: &[PathBuf]) -> Vec<PathBuf> {
    let mut broken = Vec::new();
    let mut checked_dirs: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    for file in files {
        let dir = match file.parent() {
            Some(d) => d.to_path_buf(),
            None => continue,
        };
        if !checked_dirs.insert(dir.clone()) {
            continue;
        }

        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !ext.eq_ignore_ascii_case(fmt.extension()) {
                continue;
            }

            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let has_broken = content.lines().any(|line| {
                if let Some(ref_line) = fmt.extract_reference(line) {
                    !dir.join(&ref_line.filename).exists()
                } else {
                    false
                }
            });

            if has_broken {
                broken.push(path);
            }
        }
    }

    broken
}

/// Fix broken references in all matching files in a directory.
///
/// Returns the number of reference files that were updated.
fn fix_references_in_dir(
    fmt: &dyn RefFileFormat,
    dir: &Path,
    rename_map: &HashMap<String, String>,
    errors: &mut Vec<String>,
) -> usize {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };

    let mut updated = 0;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !ext.eq_ignore_ascii_case(fmt.extension()) {
            continue;
        }

        match fix_single_ref_file(fmt, &path, dir, rename_map) {
            Ok(true) => updated += 1,
            Ok(false) => {}
            Err(e) => errors.push(format!(
                "{} fix error for {}: {}",
                fmt.label(),
                path.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
                e
            )),
        }
    }

    updated
}

/// Fix broken references in a single reference file.
///
/// Returns Ok(true) if the file was modified, Ok(false) if no changes were needed.
fn fix_single_ref_file(
    fmt: &dyn RefFileFormat,
    ref_path: &Path,
    dir: &Path,
    rename_map: &HashMap<String, String>,
) -> Result<bool, String> {
    let content = fs::read_to_string(ref_path).map_err(|e| format!("read error: {}", e))?;

    let mut output_lines = Vec::new();
    let mut changed = false;
    let mut unfixed = Vec::new();

    for line in content.lines() {
        if let Some(ref_line) = fmt.extract_reference(line) {
            if !dir.join(&ref_line.filename).exists() {
                // Reference is broken — try to find the correct filename
                if let Some(new_name) =
                    fmt.find_correction(&ref_line.filename, ref_path, dir, rename_map)
                {
                    output_lines.push(fmt.rebuild_line(line, &new_name, &ref_line));
                    changed = true;
                    continue;
                } else {
                    unfixed.push(ref_line.filename.clone());
                }
            }
        }

        output_lines.push(line.to_string());
    }

    if !unfixed.is_empty() {
        let ref_kind = if fmt.extension() == "cue" {
            "FILE references"
        } else {
            "entries"
        };
        return Err(format!(
            "could not resolve {}: {}",
            ref_kind,
            unfixed
                .iter()
                .map(|f| format!("\"{}\"", f))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    if changed {
        let new_content = fmt.serialize(&content, &output_lines);
        fs::write(ref_path, &new_content).map_err(|e| format!("write error: {}", e))?;
    }

    Ok(changed)
}

// --- Public API wrappers (unchanged signatures) ---

/// Detect CUE files with broken FILE references.
fn detect_broken_cue_files(files: &[PathBuf]) -> Vec<PathBuf> {
    detect_broken_ref_files(&CueFormat, files)
}

/// Detect M3U playlist files with broken entries.
fn detect_broken_m3u_playlists(files: &[PathBuf]) -> Vec<PathBuf> {
    detect_broken_ref_files(&M3uFormat, files)
}

/// Fix CUE file references in a directory. Returns the number of .cue files updated.
fn fix_cue_references_in_dir(
    dir: &Path,
    rename_map: &HashMap<String, String>,
    errors: &mut Vec<String>,
) -> usize {
    fix_references_in_dir(&CueFormat, dir, rename_map, errors)
}

/// Fix M3U playlist entries in a directory. Returns the number of .m3u files updated.
fn fix_m3u_references_in_dir(
    dir: &Path,
    rename_map: &HashMap<String, String>,
    errors: &mut Vec<String>,
) -> usize {
    fix_references_in_dir(&M3uFormat, dir, rename_map, errors)
}

// --- CUE-specific helpers (not duplicated, used only by CueFormat) ---

/// Parse a CUE FILE directive, returning (filename, file_type).
///
/// Handles both quoted and unquoted filenames, case-insensitive keyword:
///   FILE "filename.bin" BINARY
///   File filename.bin BINARY
fn parse_cue_file_directive(line: &str) -> Option<(String, String)> {
    let upper = line.to_uppercase();
    if !upper.starts_with("FILE ") {
        return None;
    }
    let rest = &line[5..]; // skip "FILE "

    if let Some(after_quote) = rest.strip_prefix('"') {
        let end_quote = after_quote.find('"')?;
        let filename = after_quote[..end_quote].to_string();
        let remainder = after_quote[end_quote + 1..].trim().to_string();
        Some((filename, remainder))
    } else {
        let mut parts = rest.splitn(2, ' ');
        let filename = parts.next()?.to_string();
        let remainder = parts.next().unwrap_or("").trim().to_string();
        Some((filename, remainder))
    }
}

/// Try to find the correct filename for a broken CUE FILE reference.
///
/// Strategies (in order):
/// 1. Check the rename map (covers newly-renamed files)
/// 2. CUE stem + referenced extension (covers single-bin games)
/// 3. Track number matching (covers multi-bin/track games)
/// 4. Disc ordinal matching (e.g., "dragoon1.bin" -> "(Disc 1).bin")
/// 5. Sole candidate by extension (last resort for single-bin)
fn find_correct_bin_filename(
    old_filename: &str,
    cue_stem: &str,
    dir: &Path,
    rename_map: &HashMap<String, String>,
) -> Option<String> {
    // Strategy 1: Check rename map
    if let Some(new_name) = rename_map.get(old_filename)
        && dir.join(new_name).exists()
    {
        return Some(new_name.clone());
    }

    let ref_ext = Path::new(old_filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin");

    // Strategy 2: CUE stem + referenced file extension
    let stem_candidate = format!("{}.{}", cue_stem, ref_ext);
    if dir.join(&stem_candidate).exists() {
        return Some(stem_candidate);
    }

    // Collect matching-extension files in the directory (used by strategies 3-5)
    let bin_files = collect_files_by_ext(dir, ref_ext);

    // Strategy 3: Match by "(Track N)" pattern
    if let Some(track_num) = extract_track_number(old_filename) {
        for name in &bin_files {
            if extract_track_number(name) == Some(track_num) {
                return Some(name.clone());
            }
        }
    }

    // Strategy 4: Disc ordinal matching
    if let Some(new_name) = match_by_disc_ordinal(old_filename, &bin_files) {
        return Some(new_name);
    }

    // Strategy 5: Sole candidate by extension
    if bin_files.len() == 1 {
        return Some(bin_files.into_iter().next().unwrap());
    }

    None
}

// --- M3U-specific helpers (not duplicated, used only by M3uFormat) ---

/// Try to find the correct file for a broken M3U playlist entry.
///
/// Strategies:
/// 1. Check the rename map
/// 2. Same stem, preferred entry-point extension (.cue > .chd > .iso)
/// 3. Disc ordinal matching against entry-point files
/// 4. Sole candidate with an entry-point extension
fn find_correct_m3u_entry(
    old_entry: &str,
    dir: &Path,
    rename_map: &HashMap<String, String>,
) -> Option<String> {
    // Strategy 1: Check rename map
    if let Some(new_name) = rename_map.get(old_entry)
        && dir.join(new_name).exists()
    {
        return Some(new_name.clone());
    }

    let old_stem = Path::new(old_entry)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    // Strategy 2: Same stem — try the original extension first, then the
    // preferred entry-point list.  This avoids e.g. replacing a .cue entry
    // with a .iso file just because .iso appears earlier in the fallback list.
    let original_ext = Path::new(old_entry)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    if !original_ext.is_empty() {
        let candidate = format!("{}.{}", old_stem, original_ext);
        if dir.join(&candidate).exists() {
            return Some(candidate);
        }
    }
    for ext in M3U_ENTRY_POINT_EXTENSIONS {
        if *ext == original_ext {
            continue; // already tried above
        }
        let candidate = format!("{}.{}", old_stem, ext);
        if dir.join(&candidate).exists() {
            return Some(candidate);
        }
    }

    // Collect entry-point files for strategies 3-4
    let entry_point_files = collect_entry_point_files(dir);

    // Strategy 3: Disc ordinal matching against entry-point files
    if let Some(new_name) = match_by_disc_ordinal(old_entry, &entry_point_files) {
        return Some(new_name);
    }

    // Strategy 4: Sole candidate with an entry-point extension
    if entry_point_files.len() == 1 {
        return Some(entry_point_files.into_iter().next().unwrap());
    }

    None
}

// --- Shared helpers used by both CUE and M3U filename matching ---

/// Collect filenames in `dir` that match a specific extension (case-insensitive).
fn collect_files_by_ext(dir: &Path, target_ext: &str) -> Vec<String> {
    fs::read_dir(dir)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.flatten())
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file() {
                return None;
            }
            let ext = path.extension().and_then(|e| e.to_str())?;
            if !ext.eq_ignore_ascii_case(target_ext) {
                return None;
            }
            path.file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
        })
        .collect()
}

/// Collect entry-point filenames (cue/chd/iso/gdi/cso/pbp) in a directory.
fn collect_entry_point_files(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.flatten())
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file() {
                return None;
            }
            let name = path.file_name()?.to_str()?;
            if is_m3u_entry_point(name) {
                Some(name.to_string())
            } else {
                None
            }
        })
        .collect()
}

/// Match a broken reference by disc ordinal number against a list of candidate files.
///
/// Extracts a disc/ordinal number from the old reference, then looks for a file
/// with a matching canonical "(Disc N)" in the candidates. Falls back to last-digit
/// matching for ambiguous trailing numbers (e.g., "dq71" = DQ7 disc 1).
fn match_by_disc_ordinal(old_ref: &str, candidates: &[String]) -> Option<String> {
    let old_num = extract_ordinal_from_filename(old_ref)?;

    for name in candidates {
        if extract_disc_number(name) == Some(old_num) {
            return Some(name.clone());
        }
    }

    // Fallback for ambiguous trailing numbers (e.g., "dq71" = DQ7 disc 1):
    // if the full number didn't match any disc, try just the last digit.
    if old_num >= 10 {
        let last_digit = old_num % 10;
        if last_digit > 0 {
            for name in candidates {
                if extract_disc_number(name) == Some(last_digit) {
                    return Some(name.clone());
                }
            }
        }
    }

    None
}

// --- Shared utility functions ---

/// Extract a track number from a filename like "Game (Track 2).bin".
fn extract_track_number(filename: &str) -> Option<u32> {
    let lower = filename.to_lowercase();
    let start = lower.find("(track ")?;
    let rest = &lower[start + 7..];
    let end = rest.find(')')?;
    rest[..end].trim().parse().ok()
}

/// Extract a disc/ordinal number from a filename using various common patterns.
///
/// Handles (in priority order):
///   - "(Disc N)" -- canonical Redump format
///   - "disc" followed by digits -- e.g., "(disc1)", "_disc2", "Disc3"
///   - Trailing digits on the stem -- e.g., "dragoon1", "dq71"
fn extract_ordinal_from_filename(filename: &str) -> Option<u32> {
    // Try canonical "(Disc N)" first
    if let Some(n) = extract_disc_number(filename) {
        return Some(n);
    }

    // Try case-insensitive "disc" followed by digits
    let lower = filename.to_lowercase();
    if let Some(pos) = lower.rfind("disc") {
        let after = &lower[pos + 4..];
        let digits: String = after
            .chars()
            .skip_while(|c| !c.is_ascii_digit())
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(n) = digits.parse::<u32>()
            && n > 0
        {
            return Some(n);
        }
    }

    // Try trailing number on the stem
    let stem = Path::new(filename).file_stem()?.to_str()?;
    extract_trailing_number(stem)
}

/// Extract trailing digits from a filename stem: "dragoon1" -> 1, "disc02" -> 2.
fn extract_trailing_number(stem: &str) -> Option<u32> {
    let digits: String = stem
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

// --- M3U misnamed detection (unique to M3U, no CUE equivalent) ---

/// Detect if a .m3u folder contains a playlist file that doesn't match the expected name.
///
/// Returns `Some((current_path, target_path))` if a rename is needed, `None` otherwise.
/// Only triggers when there's exactly one existing .m3u file with the wrong name.
fn detect_misnamed_m3u(dir: &Path, expected_name: &str) -> Option<(PathBuf, PathBuf)> {
    let expected_path = dir.join(expected_name);
    if expected_path.exists() {
        return None;
    }

    let existing: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.is_file() {
                let ext = p.extension()?.to_str()?;
                if ext.eq_ignore_ascii_case("m3u") {
                    return Some(p);
                }
            }
            None
        })
        .collect();

    if existing.len() == 1 {
        Some((existing[0].clone(), expected_path))
    } else {
        None
    }
}
