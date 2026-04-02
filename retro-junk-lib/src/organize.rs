use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use retro_junk_core::RomAnalyzer;
use retro_junk_core::disc::extract_disc_number;
use retro_junk_dat::cache;
use retro_junk_dat::error::DatError;
use retro_junk_dat::matcher::DatIndex;

use crate::rename::{
    is_m3u_entry_point, parse_cue_file_directive, serial_lookup, write_m3u_playlist,
};

/// Options for the organize operation.
pub struct OrganizeOptions {
    pub dat_dir: Option<PathBuf>,
    pub limit: Option<usize>,
    /// Also organize single-disc games into .m3u folders (default: multi-disc only).
    pub include_single_disc: bool,
    /// Fall back to hashing when serial lookup fails (slower but catches more files).
    pub hash_fallback: bool,
}

/// Progress updates during an organize operation.
pub enum OrganizeProgress {
    Scanning {
        file_count: usize,
    },
    ReadingDisc {
        file_name: String,
        index: usize,
        total: usize,
    },
    Hashing {
        file_name: String,
        bytes_done: u64,
        bytes_total: u64,
    },
    Done,
}

/// A single file group to be organized into an .m3u folder.
#[derive(Debug, Clone)]
pub struct OrganizeJob {
    /// Target .m3u folder path (parent_dir / "{game_name}.m3u").
    pub target_folder: PathBuf,
    /// Canonical base game name (disc tag stripped).
    pub game_name: String,
    /// Entry-point files in disc-number order (these become playlist entries).
    pub entry_points: Vec<OrganizeEntry>,
    /// All companion files (BIN tracks, etc.) that must move with entry points.
    pub companion_files: Vec<PathBuf>,
}

/// A single entry-point file in an organize job.
#[derive(Debug, Clone)]
pub struct OrganizeEntry {
    pub source_path: PathBuf,
    /// DAT game name (e.g., "Final Fantasy VII (USA) (Disc 1)").
    pub dat_game_name: String,
    /// Target filename inside the .m3u folder (original filename, not DAT canonical).
    pub target_filename: String,
    pub disc_number: Option<u32>,
}

/// A file that couldn't be matched.
#[derive(Debug, Clone)]
pub struct UnorganizedFile {
    pub path: PathBuf,
    pub serial: Option<String>,
    pub reason: String,
}

/// Result of planning.
#[derive(Debug, Clone)]
pub struct OrganizePlan {
    pub jobs: Vec<OrganizeJob>,
    pub unmatched: Vec<UnorganizedFile>,
    pub already_organized: Vec<PathBuf>,
    pub skipped_single_disc: usize,
}

impl OrganizePlan {
    /// Whether there are any jobs to execute.
    pub fn has_actions(&self) -> bool {
        !self.jobs.is_empty()
    }
}

/// Result of executing a single organize job.
#[derive(Debug, Clone)]
pub struct OrganizeResult {
    pub folder_created: bool,
    pub files_moved: usize,
    pub playlist_written: bool,
    pub errors: Vec<String>,
    pub final_folder: PathBuf,
}

/// Collect companion files (BIN tracks) referenced by a CUE file.
///
/// Parses the CUE file's FILE directives and resolves them relative to the CUE's
/// parent directory. Returns only files that exist on disk.
pub(crate) fn collect_cue_companions(cue_path: &Path) -> Vec<PathBuf> {
    let parent = match cue_path.parent() {
        Some(p) => p,
        None => return Vec::new(),
    };

    let contents = match fs::read_to_string(cue_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut companions = Vec::new();
    for line in contents.lines() {
        if let Some((filename, _file_type)) = parse_cue_file_directive(line) {
            let companion_path = parent.join(&filename);
            if companion_path.exists() && companion_path != cue_path {
                companions.push(companion_path);
            }
        }
    }
    companions
}

/// Scan a console folder for loose entry-point files that could be organized.
///
/// Returns paths of files at the top level (not inside .m3u subdirectories)
/// that have disc entry-point extensions (.cue, .chd, .iso, .gdi, etc.).
pub fn find_loose_disc_files(folder: &Path) -> Vec<PathBuf> {
    let entries = match fs::read_dir(folder) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut loose = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            let filename = path.file_name().unwrap_or_default().to_string_lossy();
            if is_m3u_entry_point(&filename) {
                loose.push(path);
            }
        }
    }
    loose.sort();
    loose
}

/// Plan the organization of loose disc image files into .m3u folders.
///
/// Scans for loose entry-point files, extracts serials from disc data via
/// quick analysis, looks up game names in the Redump DAT, groups multi-disc
/// games, and builds an organize plan.
pub fn plan_organize(
    folder: &Path,
    analyzer: &dyn RomAnalyzer,
    options: &OrganizeOptions,
    progress: &dyn Fn(OrganizeProgress),
) -> Result<OrganizePlan, DatError> {
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
        false,
    )?;
    let index = DatIndex::from_dats(dats);

    // Find loose entry-point files at the top level
    let loose_files = find_loose_disc_files(folder);

    progress(OrganizeProgress::Scanning {
        file_count: loose_files.len(),
    });

    let mut files_to_process = loose_files;
    if let Some(max) = options.limit {
        files_to_process.truncate(max);
    }

    if files_to_process.is_empty() {
        return Ok(OrganizePlan {
            jobs: Vec::new(),
            unmatched: Vec::new(),
            already_organized: Vec::new(),
            skipped_single_disc: 0,
        });
    }

    // Phase 1: Serial/hash lookup for each file
    let total = files_to_process.len();
    let mut matched: Vec<MatchedDiscFile> = Vec::new();
    let mut unmatched: Vec<UnorganizedFile> = Vec::new();

    for (i, file_path) in files_to_process.iter().enumerate() {
        let file_name = file_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        progress(OrganizeProgress::ReadingDisc {
            file_name: file_name.clone(),
            index: i,
            total,
        });

        // Try serial lookup first
        let serial_outcome = serial_lookup(file_path, analyzer, &index);

        if let Some(result) = serial_outcome.result {
            let game = &index.games[result.game_index];
            let _rom = &game.roms[result.rom_index];
            matched.push(MatchedDiscFile {
                source_path: file_path.clone(),
                dat_game_name: game.name.clone(),
                original_filename: file_name,
            });
            continue;
        }

        // Hash fallback if enabled
        if options.hash_fallback {
            if let Some(match_result) = try_hash_fallback(file_path, analyzer, &index, &progress) {
                let game = &index.games[match_result.game_index];
                matched.push(MatchedDiscFile {
                    source_path: file_path.clone(),
                    dat_game_name: game.name.clone(),
                    original_filename: file_name,
                });
                continue;
            }
        }

        // Unmatched
        let reason = if serial_outcome.full_serial.is_none() {
            "No serial found in disc data".to_string()
        } else if serial_outcome.ambiguous_candidates.is_some() {
            format!(
                "Ambiguous serial '{}'",
                serial_outcome.full_serial.as_deref().unwrap_or("?")
            )
        } else {
            format!(
                "Serial '{}' not found in DAT",
                serial_outcome.full_serial.as_deref().unwrap_or("?")
            )
        };

        unmatched.push(UnorganizedFile {
            path: file_path.clone(),
            serial: serial_outcome.full_serial,
            reason,
        });
    }

    // Phase 2: Group matched files by base game name
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, m) in matched.iter().enumerate() {
        let base_name = if extract_disc_number(&m.dat_game_name).is_some() {
            retro_junk_core::disc::strip_disc_tag(&m.dat_game_name)
        } else {
            m.dat_game_name.clone()
        };
        groups.entry(base_name).or_default().push(i);
    }

    // Phase 3: Build organize jobs
    let mut jobs: Vec<OrganizeJob> = Vec::new();
    let mut skipped_single_disc = 0;

    for (base_name, member_indices) in &groups {
        if member_indices.len() == 1 && !options.include_single_disc {
            skipped_single_disc += 1;
            continue;
        }

        let target_folder = folder.join(format!("{}.m3u", base_name));

        // Skip if target folder already exists
        if target_folder.exists() {
            continue;
        }

        // Build entry points sorted by disc number
        let mut entry_points: Vec<OrganizeEntry> = member_indices
            .iter()
            .map(|&idx| {
                let m = &matched[idx];
                let disc_number = extract_disc_number(&m.dat_game_name);
                OrganizeEntry {
                    source_path: m.source_path.clone(),
                    dat_game_name: m.dat_game_name.clone(),
                    target_filename: m.original_filename.clone(),
                    disc_number,
                }
            })
            .collect();

        // Sort by disc number if present, else alphabetically
        if entry_points.iter().any(|e| e.disc_number.is_some()) {
            entry_points.sort_by_key(|e| e.disc_number.unwrap_or(u32::MAX));
        } else {
            entry_points.sort_by(|a, b| a.target_filename.cmp(&b.target_filename));
        }

        // Collect companion files for CUE entry points
        let mut companion_files: Vec<PathBuf> = Vec::new();
        for entry in &entry_points {
            if entry
                .source_path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("cue"))
                .unwrap_or(false)
            {
                companion_files.extend(collect_cue_companions(&entry.source_path));
            }
        }

        jobs.push(OrganizeJob {
            target_folder,
            game_name: base_name.clone(),
            entry_points,
            companion_files,
        });
    }

    // Sort jobs by game name for deterministic output
    jobs.sort_by(|a, b| a.game_name.cmp(&b.game_name));

    progress(OrganizeProgress::Done);

    Ok(OrganizePlan {
        jobs,
        unmatched,
        already_organized: Vec::new(),
        skipped_single_disc,
    })
}

/// Execute a single organize job: create .m3u folder, move files, write playlist.
pub fn execute_organize_job(job: &OrganizeJob) -> OrganizeResult {
    let mut result = OrganizeResult {
        folder_created: false,
        files_moved: 0,
        playlist_written: false,
        errors: Vec::new(),
        final_folder: job.target_folder.clone(),
    };

    // Create the .m3u folder
    if let Err(e) = fs::create_dir(&job.target_folder) {
        result.errors.push(format!(
            "Failed to create folder {}: {}",
            job.target_folder.display(),
            e,
        ));
        return result;
    }
    result.folder_created = true;

    // Move entry-point files
    for entry in &job.entry_points {
        let target = job.target_folder.join(&entry.target_filename);
        match fs::rename(&entry.source_path, &target) {
            Ok(()) => result.files_moved += 1,
            Err(e) => {
                result.errors.push(format!(
                    "Failed to move {}: {}",
                    entry.source_path.display(),
                    e,
                ));
            }
        }
    }

    // Move companion files (BIN tracks, etc.)
    for companion in &job.companion_files {
        let filename = companion.file_name().unwrap_or_default();
        let target = job.target_folder.join(filename);
        match fs::rename(companion, &target) {
            Ok(()) => result.files_moved += 1,
            Err(e) => {
                result
                    .errors
                    .push(format!("Failed to move {}: {}", companion.display(), e,));
            }
        }
    }

    // Write the M3U playlist
    let playlist_entries: Vec<String> = job
        .entry_points
        .iter()
        .map(|e| e.target_filename.clone())
        .collect();

    match write_m3u_playlist(&job.target_folder, &job.game_name, &playlist_entries) {
        Ok(()) => result.playlist_written = true,
        Err(e) => {
            result
                .errors
                .push(format!("Failed to write playlist: {}", e));
        }
    }

    result
}

/// Internal: a matched disc file before grouping.
struct MatchedDiscFile {
    source_path: PathBuf,
    dat_game_name: String,
    original_filename: String,
}

/// Try hash-based fallback matching for a file.
fn try_hash_fallback(
    file_path: &Path,
    analyzer: &dyn RomAnalyzer,
    index: &DatIndex,
    progress: &dyn Fn(OrganizeProgress),
) -> Option<retro_junk_dat::matcher::MatchResult> {
    let file_name = file_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let mut file = fs::File::open(file_path).ok()?;
    let hashes = crate::hasher::compute_crc32_sha1(&mut file, analyzer, Some(file_path)).ok()?;

    progress(OrganizeProgress::Hashing {
        file_name,
        bytes_done: hashes.data_size,
        bytes_total: hashes.data_size,
    });

    index
        .match_by_hash(hashes.data_size, &hashes)
        .into_iter()
        .next()
}

#[cfg(test)]
#[path = "tests/organize_tests.rs"]
mod tests;
