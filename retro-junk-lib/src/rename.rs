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
}

/// A discrepancy between serial-based and hash-based matching (reported in --hash mode).
#[derive(Debug, Clone)]
pub struct MatchDiscrepancy {
    pub file: PathBuf,
    pub serial_game: String,
    pub hash_game: String,
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
    let dats = cache::load_dats(analyzer.short_name(), dat_names, options.dat_dir.as_deref())?;
    let index = DatIndex::from_dats(dats);

    // Collect ROM files
    let extensions: std::collections::HashSet<String> = analyzer
        .file_extensions()
        .iter()
        .map(|e| e.to_lowercase())
        .collect();

    let mut files: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = fs::read_dir(folder) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase())
                .unwrap_or_default();
            if extensions.contains(&ext) {
                files.push(path);
            }
        }
    }
    files.sort();
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
            let serial_result = match_by_serial(file_path, analyzer, &analysis_options, &index);

            // Report discrepancy if both matched but to different games
            if let (Some(hr), Some(sr)) = (&hash_result, &serial_result) {
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
            let serial_result = match_by_serial(file_path, analyzer, &analysis_options, &index);
            if serial_result.is_some() {
                serial_result
            } else {
                match_by_hash(file_path, &index, analyzer, progress)?
            }
        };

        if let Some(result) = match_result {
            let game = &index.games[result.game_index];
            let rom = &game.roms[result.rom_index];

            // Target path: same directory, DAT-canonical filename
            let target = folder.join(&rom.name);

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

    Ok(RenamePlan {
        renames: clean_renames,
        already_correct,
        unmatched,
        conflicts,
        discrepancies,
    })
}

/// Try to match a file by serial number only (no hashing).
fn match_by_serial(
    file_path: &Path,
    analyzer: &dyn RomAnalyzer,
    analysis_options: &AnalysisOptions,
    index: &DatIndex,
) -> Option<MatchResult> {
    let mut file = fs::File::open(file_path).ok()?;
    let info = analyzer.analyze(&mut file, analysis_options).ok()?;
    let serial = info.serial_number.as_ref()?;
    let game_code = analyzer.extract_dat_game_code(serial);

    match index.match_by_serial(serial, game_code.as_deref()) {
        Some(result) => {
            // let game_code = game_code.unwrap_or("NONE".into());
            // println!(
            //     "Serial {} matched successfully for file {}",
            //     game_code,
            //     file_path.display()
            // );
            Some(result)
        }
        None => {
            let game_code = game_code.unwrap_or("NONE".into());
            println!(
                "Serial {} failed to match for file {}",
                game_code,
                file_path.display()
            );
            None
        }
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

/// Execute a rename plan, performing the actual file renames.
pub fn execute_renames(plan: &RenamePlan) -> RenameSummary {
    let mut summary = RenameSummary {
        already_correct: plan.already_correct.len(),
        ..Default::default()
    };

    for (_, msg) in &plan.conflicts {
        summary.conflicts.push(msg.clone());
    }

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
