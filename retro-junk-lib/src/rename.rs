use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use retro_junk_core::{AnalysisOptions, RomAnalyzer};
use retro_junk_dat::cache;
use retro_junk_dat::error::DatError;
use retro_junk_dat::matcher::{DatIndex, MatchMethod, MatchResult, SerialLookupResult};

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
    pub cue_files_updated: usize,
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

/// Result of planning renames for a single console folder.
#[derive(Debug)]
pub struct RenamePlan {
    pub renames: Vec<RenameAction>,
    pub already_correct: Vec<PathBuf>,
    pub unmatched: Vec<UnmatchedFile>,
    pub conflicts: Vec<(PathBuf, String)>,
    /// Files where serial and hash matched different games (--hash mode only)
    pub discrepancies: Vec<MatchDiscrepancy>,
    /// Serial-related diagnostics (serial lookup failed, or missing serial)
    pub serial_warnings: Vec<SerialWarning>,
    /// M3U folder renames and playlist writes
    pub m3u_actions: Vec<M3uAction>,
    /// CUE files with broken FILE references (pre-existing from prior renames)
    pub broken_cue_files: Vec<PathBuf>,
}

use retro_junk_core::disc::{extract_disc_number, strip_disc_tag};

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

        // Track hash info for diagnostics if the file ends up unmatched
        let mut last_hash: Option<(String, u64)> = None;

        let match_result = if options.hash_mode {
            // Hash mode: hash is authoritative, but also check serial for discrepancies
            let hash_outcome = match_by_hash(file_path, &index, analyzer, progress)?;
            last_hash = Some((hash_outcome.crc32, hash_outcome.data_size));
            let serial_outcome = match_by_serial(file_path, analyzer, &analysis_options, &index);

            // Report discrepancy if both matched but to different games
            if let (Some(hr), Some(sr)) = (&hash_outcome.result, &serial_outcome.result) {
                if hr.game_index != sr.game_index {
                    discrepancies.push(MatchDiscrepancy {
                        file: file_path.clone(),
                        serial_game: index.games[sr.game_index].name.clone(),
                        hash_game: index.games[hr.game_index].name.clone(),
                    });
                }
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
                            full_serial: serial_outcome
                                .full_serial
                                .clone()
                                .unwrap_or_default(),
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

    // Detect pre-existing broken CUE file references
    let broken_cue_files = detect_broken_cue_files(&files);

    Ok(RenamePlan {
        renames: clean_renames,
        already_correct,
        unmatched,
        conflicts,
        discrepancies,
        serial_warnings,
        m3u_actions,
        broken_cue_files,
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

    // Step 1.5: Fix CUE file references
    // Build per-directory rename maps from successful renames
    let mut dir_rename_maps: HashMap<PathBuf, HashMap<String, String>> = HashMap::new();
    let mut cue_dirs: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    for rename in &plan.renames {
        if rename.source == rename.target {
            continue;
        }
        let dir = rename.source.parent().unwrap_or(Path::new(".")).to_path_buf();
        let old_name = rename
            .source
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let new_name = rename
            .target
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        dir_rename_maps
            .entry(dir.clone())
            .or_default()
            .insert(old_name, new_name);
        cue_dirs.insert(dir);
    }

    // Also include directories containing pre-existing broken CUE files
    for cue_path in &plan.broken_cue_files {
        if let Some(dir) = cue_path.parent() {
            cue_dirs.insert(dir.to_path_buf());
        }
    }

    // Fix CUE references in all affected directories
    for dir in &cue_dirs {
        let empty_map = HashMap::new();
        let rename_map = dir_rename_maps.get(dir).unwrap_or(&empty_map);
        summary.cue_files_updated +=
            fix_cue_references_in_dir(dir, rename_map, &mut summary.errors);
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

// ---------------------------------------------------------------------------
// CUE file reference fixing
// ---------------------------------------------------------------------------

/// Detect CUE files with broken FILE references in the directories containing
/// the given files. Returns paths to .cue files with at least one broken reference.
fn detect_broken_cue_files(files: &[PathBuf]) -> Vec<PathBuf> {
    let mut broken = Vec::new();
    let mut checked_dirs: std::collections::HashSet<PathBuf> =
        std::collections::HashSet::new();

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
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            if !ext.eq_ignore_ascii_case("cue") {
                continue;
            }

            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            if has_broken_file_references(&content, &dir) {
                broken.push(path);
            }
        }
    }

    broken
}

/// Check if a CUE sheet has any FILE references that don't resolve to existing files.
fn has_broken_file_references(content: &str, dir: &Path) -> bool {
    for line in content.lines() {
        let trimmed = line.trim();
        if !trimmed.to_uppercase().starts_with("FILE ") {
            continue;
        }

        if let Some((filename, _)) = parse_cue_file_directive(trimmed) {
            if !dir.join(&filename).exists() {
                return true;
            }
        }
    }
    false
}

/// Parse a CUE FILE directive, returning (filename, file_type).
///
/// Handles both quoted and unquoted filenames, case-insensitive keyword:
///   FILE "filename.bin" BINARY
///   File filename.bin BINARY
fn parse_cue_file_directive(line: &str) -> Option<(String, String)> {
    // Case-insensitive: find "FILE " prefix
    let upper = line.to_uppercase();
    if !upper.starts_with("FILE ") {
        return None;
    }
    let rest = &line[5..]; // skip "FILE "

    if rest.starts_with('"') {
        let end_quote = rest[1..].find('"')?;
        let filename = rest[1..1 + end_quote].to_string();
        let remainder = rest[2 + end_quote..].trim().to_string();
        Some((filename, remainder))
    } else {
        let mut parts = rest.splitn(2, ' ');
        let filename = parts.next()?.to_string();
        let remainder = parts.next().unwrap_or("").trim().to_string();
        Some((filename, remainder))
    }
}

/// Fix CUE file references in a directory.
///
/// For each .cue file, checks if FILE references resolve. Broken references
/// are fixed using the rename map or filesystem heuristics (stem matching,
/// track number matching).
///
/// Returns the number of .cue files that were updated.
fn fix_cue_references_in_dir(
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
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if !ext.eq_ignore_ascii_case("cue") {
            continue;
        }

        match fix_single_cue_file(&path, dir, rename_map) {
            Ok(true) => updated += 1,
            Ok(false) => {} // no changes needed
            Err(e) => errors.push(format!(
                "CUE fix error for {}: {}",
                path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?"),
                e
            )),
        }
    }

    updated
}

/// Fix FILE references in a single CUE file.
///
/// Returns Ok(true) if the file was modified, Ok(false) if no changes were needed.
fn fix_single_cue_file(
    cue_path: &Path,
    dir: &Path,
    rename_map: &HashMap<String, String>,
) -> Result<bool, String> {
    let content =
        fs::read_to_string(cue_path).map_err(|e| format!("read error: {}", e))?;

    let cue_stem = cue_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    let mut new_content = String::with_capacity(content.len());
    let mut changed = false;

    let mut unfixed = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.to_uppercase().starts_with("FILE ") {
            if let Some((old_filename, file_type)) = parse_cue_file_directive(trimmed) {
                if !dir.join(&old_filename).exists() {
                    // Reference is broken — try to find the correct filename
                    if let Some(new_name) =
                        find_correct_bin_filename(&old_filename, cue_stem, dir, rename_map)
                    {
                        let indent = &line[..line.len() - trimmed.len()];
                        new_content
                            .push_str(&format!("{}FILE \"{}\" {}", indent, new_name, file_type));
                        new_content.push('\n');
                        changed = true;
                        continue;
                    } else {
                        unfixed.push(old_filename);
                    }
                }
            }
        }

        new_content.push_str(line);
        new_content.push('\n');
    }

    if !unfixed.is_empty() {
        return Err(format!(
            "could not resolve FILE references: {}",
            unfixed
                .iter()
                .map(|f| format!("\"{}\"", f))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    if changed {
        // Preserve original: don't add trailing newline if original didn't have one
        if !content.ends_with('\n') && new_content.ends_with('\n') {
            new_content.pop();
        }
        fs::write(cue_path, &new_content).map_err(|e| format!("write error: {}", e))?;
    }

    Ok(changed)
}

/// Try to find the correct filename for a broken CUE FILE reference.
///
/// Strategies (in order):
/// 1. Check the rename map (covers newly-renamed files)
/// 2. CUE stem + referenced extension (covers single-bin games)
/// 3. Track number matching (covers multi-bin/track games)
/// 4. Disc ordinal matching (e.g., "dragoon1.bin" → "(Disc 1).bin",
///    "FF8(disc2).iso" → "(Disc 2).iso")
/// 5. Sole candidate by extension (last resort for single-bin)
fn find_correct_bin_filename(
    old_filename: &str,
    cue_stem: &str,
    dir: &Path,
    rename_map: &HashMap<String, String>,
) -> Option<String> {
    // Strategy 1: Check rename map
    if let Some(new_name) = rename_map.get(old_filename) {
        if dir.join(new_name).exists() {
            return Some(new_name.clone());
        }
    }

    let ref_ext = Path::new(old_filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin");

    // Strategy 2: CUE stem + referenced file extension
    // e.g., "Game.cue" references "OldName.bin" → try "Game.bin"
    let stem_candidate = format!("{}.{}", cue_stem, ref_ext);
    if dir.join(&stem_candidate).exists() {
        return Some(stem_candidate);
    }

    // Collect matching-extension files in the directory (used by strategies 3-5)
    let bin_files: Vec<String> = fs::read_dir(dir)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.flatten())
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file() {
                return None;
            }
            let ext = path.extension().and_then(|e| e.to_str())?;
            if !ext.eq_ignore_ascii_case(ref_ext) {
                return None;
            }
            path.file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
        })
        .collect();

    // Strategy 3: Match by "(Track N)" pattern
    // e.g., old "Game (Track 2).bin" → find file with "(Track 2)" in directory
    if let Some(track_num) = extract_track_number(old_filename) {
        for name in &bin_files {
            if extract_track_number(name) == Some(track_num) {
                return Some(name.clone());
            }
        }
    }

    // Strategy 4: Disc ordinal matching
    // Extract a disc/ordinal number from the old filename using various patterns:
    //   "(Disc 1)", "(disc1)", "_disc1", "Disc1", or trailing number like "dragoon1"
    // Then match against canonical "(Disc N)" in actual files.
    if let Some(old_num) = extract_ordinal_from_filename(old_filename) {
        for name in &bin_files {
            if extract_disc_number(name) == Some(old_num) {
                return Some(name.clone());
            }
        }
        // Fallback for ambiguous trailing numbers (e.g., "dq71" = DQ7 disc 1):
        // if the full number didn't match any disc, try just the last digit.
        if old_num >= 10 {
            let last_digit = old_num % 10;
            if last_digit > 0 {
                for name in &bin_files {
                    if extract_disc_number(name) == Some(last_digit) {
                        return Some(name.clone());
                    }
                }
            }
        }
    }

    // Strategy 5: If there's exactly one file with the matching extension, use it
    if bin_files.len() == 1 {
        return Some(bin_files.into_iter().next().unwrap());
    }

    None
}

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
///   - "(Disc N)" — canonical Redump format
///   - "disc" followed by digits — e.g., "(disc1)", "_disc2", "Disc3"
///   - Trailing digits on the stem — e.g., "dragoon1", "dq71"
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
        if let Ok(n) = digits.parse::<u32>() {
            if n > 0 {
                return Some(n);
            }
        }
    }

    // Try trailing number on the stem
    let stem = Path::new(filename).file_stem()?.to_str()?;
    extract_trailing_number(stem)
}

/// Extract trailing digits from a filename stem: "dragoon1" → 1, "disc02" → 2.
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
