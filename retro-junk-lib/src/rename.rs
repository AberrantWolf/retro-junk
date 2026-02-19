use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use retro_junk_core::{AnalysisOptions, RomAnalyzer};
use retro_junk_dat::cache;
use retro_junk_dat::error::DatError;
use retro_junk_dat::matcher::{DatIndex, MatchMethod, MatchResult};

use crate::hasher;

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
#[derive(Debug, Clone)]
pub struct RenameOptions {
    /// Force CRC32-based matching instead of serial/name
    pub hash_mode: bool,
    /// Custom DAT directory (instead of cache)
    pub dat_dir: Option<PathBuf>,
    /// Maximum number of ROMs to process
    pub limit: Option<usize>,
}

impl Default for RenameOptions {
    fn default() -> Self {
        Self {
            hash_mode: false,
            dat_dir: None,
            limit: None,
        }
    }
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
}

/// The kind of serial warning.
#[derive(Debug, Clone)]
pub enum SerialWarningKind {
    /// Serial found in ROM header but no DAT match
    NoMatch {
        full_serial: String,
        game_code: Option<String>,
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

/// Result of planning renames for a single console folder.
#[derive(Debug)]
pub struct RenamePlan {
    pub renames: Vec<RenameAction>,
    pub already_correct: Vec<PathBuf>,
    pub unmatched: Vec<PathBuf>,
    pub conflicts: Vec<(PathBuf, String)>,
    /// Files where serial and hash matched different games (--hash mode only)
    pub discrepancies: Vec<MatchDiscrepancy>,
    /// Serial-related diagnostics (serial lookup failed, or missing serial)
    pub serial_warnings: Vec<SerialWarning>,
    /// M3U folder renames and playlist writes
    pub m3u_actions: Vec<M3uAction>,
}

/// Remove " (Disc N)" from a game name, preserving other parenthesized tags.
///
/// Examples:
/// - `"Final Fantasy VII (Disc 1) (USA)"` → `"Final Fantasy VII (USA)"`
/// - `"Crash Bandicoot (USA)"` → `"Crash Bandicoot (USA)"` (unchanged)
fn strip_disc_tag(name: &str) -> String {
    const PREFIX: &str = " (Disc ";
    if let Some(start) = name.find(PREFIX) {
        let after = &name[start + PREFIX.len()..];
        // Find closing ')' after the digits
        if let Some(close) = after.find(')') {
            let digits = &after[..close];
            if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
                let mut result = String::with_capacity(name.len());
                result.push_str(&name[..start]);
                result.push_str(&after[close + 1..]);
                return result;
            }
        }
    }
    name.to_string()
}

/// Extract disc number from a filename or game name for sorting.
///
/// Examples:
/// - `"Final Fantasy VII (Disc 2) (USA).chd"` → `Some(2)`
/// - `"Crash Bandicoot (USA).chd"` → `None`
fn extract_disc_number(name: &str) -> Option<u32> {
    const PREFIX: &str = "(Disc ";
    let start = name.find(PREFIX)?;
    let after = &name[start + PREFIX.len()..];
    let close = after.find(')')?;
    after[..close].parse().ok()
}

/// Returns true for file extensions that are M3U entry points (playable disc images).
/// Returns false for companion data files (.bin, .img) that shouldn't appear in playlists.
fn is_m3u_entry_point(filename: &str) -> bool {
    let ext = Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    matches!(
        ext.as_str(),
        "cue" | "chd" | "iso" | "gdi" | "cso" | "pbp"
    )
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

        let match_result = if options.hash_mode {
            // Hash mode: hash is authoritative, but also check serial for discrepancies
            let hash_result = match_by_hash(file_path, &index, analyzer, progress)?;
            let serial_outcome = match_by_serial(file_path, analyzer, &analysis_options, &index);

            // Report discrepancy if both matched but to different games
            if let (Some(hr), Some(sr)) = (&hash_result, &serial_outcome.result) {
                if hr.game_index != sr.game_index {
                    discrepancies.push(MatchDiscrepancy {
                        file: file_path.clone(),
                        serial_game: index.games[sr.game_index].name.clone(),
                        hash_game: index.games[hr.game_index].name.clone(),
                    });
                }
            }

            hash_result
        } else {
            // Default mode: try serial first, then always fall back to hash
            let serial_outcome = match_by_serial(file_path, analyzer, &analysis_options, &index);

            // Collect serial warnings
            if serial_outcome.result.is_none() {
                if let Some(ref full_serial) = serial_outcome.full_serial {
                    // Serial found but DAT lookup failed
                    serial_warnings.push(SerialWarning {
                        file: file_path.clone(),
                        kind: SerialWarningKind::NoMatch {
                            full_serial: full_serial.clone(),
                            game_code: serial_outcome.game_code.clone(),
                        },
                    });
                } else if analyzer.expects_serial() {
                    // No serial found but platform expects one
                    serial_warnings.push(SerialWarning {
                        file: file_path.clone(),
                        kind: SerialWarningKind::Missing,
                    });
                }
            }

            if serial_outcome.result.is_some() {
                serial_outcome.result
            } else {
                match_by_hash(file_path, &index, analyzer, progress)?
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
            file_game_names.insert(
                file_path.clone(),
                (game.name.clone(), target_filename),
            );

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
            unmatched.push(file_path.clone());
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

    // M3U post-processing: plan folder renames and playlist writes for multi-disc sets
    let mut m3u_actions = Vec::new();
    for entry in &game_entries {
        if let crate::scanner::GameEntry::MultiDisc { name: _, files } = entry {
            // Collect game names for all matched files in this folder
            let matched: Vec<(&PathBuf, &str, &str)> = files
                .iter()
                .filter_map(|f| {
                    file_game_names
                        .get(f)
                        .map(|(game_name, target_name)| (f, game_name.as_str(), target_name.as_str()))
                })
                .collect();

            if matched.is_empty() {
                continue;
            }

            // Derive base game name: strip disc tag from the first matched game
            let base_game_name = strip_disc_tag(matched[0].1);

            // The source folder is the parent of any file in this multi-disc set
            let source_folder = match files[0].parent() {
                Some(p) => p.to_path_buf(),
                None => continue,
            };

            // Target folder: parent of source_folder + base_game_name.m3u
            let target_folder = match source_folder.parent() {
                Some(p) => p.join(format!("{}.m3u", base_game_name)),
                None => continue,
            };

            // Build playlist entries: target filenames for entry-point files, sorted by disc number
            let mut playlist_entries: Vec<(u32, String)> = matched
                .iter()
                .filter(|(_, _, target_name)| is_m3u_entry_point(target_name))
                .map(|(_, game_name, target_name)| {
                    let disc = extract_disc_number(game_name).unwrap_or(0);
                    (disc, target_name.to_string())
                })
                .collect();
            playlist_entries.sort_by_key(|(disc, _)| *disc);
            let playlist_entries: Vec<String> =
                playlist_entries.into_iter().map(|(_, name)| name).collect();

            // Check if everything is already correct
            let folder_correct = source_folder == target_folder;
            let existing_m3u_correct = if folder_correct {
                // Check if there's already a correct .m3u file inside
                let expected_m3u_name = format!("{}.m3u", base_game_name);
                let expected_m3u_path = source_folder.join(&expected_m3u_name);
                if expected_m3u_path.exists() {
                    let contents = fs::read_to_string(&expected_m3u_path).unwrap_or_default();
                    let existing_lines: Vec<&str> =
                        contents.lines().filter(|l| !l.is_empty()).collect();
                    existing_lines == playlist_entries.iter().map(|s| s.as_str()).collect::<Vec<_>>()
                } else {
                    false
                }
            } else {
                false
            };

            if !folder_correct || !existing_m3u_correct {
                m3u_actions.push(M3uAction {
                    source_folder,
                    target_folder,
                    game_name: base_game_name,
                    playlist_entries,
                });
            }
        }
    }

    Ok(RenamePlan {
        renames: clean_renames,
        already_correct,
        unmatched,
        conflicts,
        discrepancies,
        serial_warnings,
        m3u_actions,
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
    let result = index.match_by_serial(&serial, game_code.as_deref());

    SerialMatchOutcome {
        result,
        full_serial: Some(serial),
        game_code,
    }
}

/// Match a file by computing its CRC32 hash (with SHA1 fallback).
fn match_by_hash(
    file_path: &Path,
    index: &DatIndex,
    analyzer: &dyn RomAnalyzer,
    progress: &dyn Fn(RenameProgress),
) -> Result<Option<MatchResult>, DatError> {
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

    if let Some(result) = index.match_by_hash(hashes.data_size, &hashes) {
        return Ok(Some(result));
    }

    // If CRC32 didn't match, try SHA1 (recompute with SHA1)
    // Only do this if we have size candidates (to avoid pointless rehashing)
    if index.candidates_by_size(hashes.data_size).is_some() {
        let mut file = fs::File::open(file_path)?;
        let full_hashes = hasher::compute_crc32_sha1(&mut file, analyzer)?;
        if let Some(result) = index.match_by_hash(full_hashes.data_size, &full_hashes) {
            return Ok(Some(result));
        }
    }

    Ok(None)
}

/// Execute a rename plan, performing the actual file renames and M3U operations.
///
/// Execution order for M3U actions:
/// 1. Rename individual disc files (existing logic)
/// 2. Write/update .m3u playlist files inside folders (using source folder paths)
/// 3. Rename .m3u folders (last, so steps 1-2 use valid paths)
pub fn execute_renames(plan: &RenamePlan) -> RenameSummary {
    let mut summary = RenameSummary {
        already_correct: plan.already_correct.len(),
        ..Default::default()
    };

    for (_, msg) in &plan.conflicts {
        summary.conflicts.push(msg.clone());
    }

    // Step 1: Rename individual disc files
    for rename in &plan.renames {
        // Check if target already exists (and isn't the source)
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

    // Step 2: Write .m3u playlist files (using source folder paths, before folder rename)
    for action in &plan.m3u_actions {
        if action.playlist_entries.is_empty() {
            continue;
        }

        // Delete any existing .m3u files inside the folder
        if let Ok(entries) = fs::read_dir(&action.source_folder) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                        if ext.eq_ignore_ascii_case("m3u") {
                            let _ = fs::remove_file(&path);
                        }
                    }
                }
            }
        }

        // Write new .m3u playlist file
        let playlist_name = format!("{}.m3u", action.game_name);
        let playlist_path = action.source_folder.join(&playlist_name);
        let contents = action.playlist_entries.join("\n") + "\n";
        match fs::write(&playlist_path, contents) {
            Ok(()) => summary.m3u_playlists_written += 1,
            Err(e) => {
                summary.errors.push(format!(
                    "Failed to write playlist {}: {}",
                    playlist_path.display(),
                    e,
                ));
            }
        }
    }

    // Step 3: Rename .m3u folders (last, so steps 1-2 used valid paths)
    for action in &plan.m3u_actions {
        if action.source_folder == action.target_folder {
            continue;
        }

        if action.target_folder.exists() {
            summary.errors.push(format!(
                "Target folder already exists: {}",
                action.target_folder.display()
            ));
            continue;
        }

        match fs::rename(&action.source_folder, &action.target_folder) {
            Ok(()) => summary.m3u_folders_renamed += 1,
            Err(e) => {
                summary.errors.push(format!(
                    "Failed to rename folder {:?} -> {:?}: {}",
                    action.source_folder.file_name().unwrap_or_default(),
                    action.target_folder.file_name().unwrap_or_default(),
                    e,
                ));
            }
        }
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
