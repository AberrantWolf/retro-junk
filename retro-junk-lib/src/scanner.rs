//! Directory scanner for ROM collections.
//!
//! Handles both flat file layouts and ES-DE `.m3u` multi-disc directories.
//! Used by both the CLI analyze and scraper commands.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// A logical game entry — either a single file or a multi-disc set from an .m3u folder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GameEntry {
    /// A single ROM file at the top level.
    SingleFile(PathBuf),
    /// An .m3u directory containing one or more disc images.
    MultiDisc {
        /// The .m3u directory name (e.g., "Final Fantasy 7 (US).m3u").
        name: String,
        /// All matching ROM files inside the directory, sorted.
        files: Vec<PathBuf>,
    },
}

impl GameEntry {
    /// Sort key for ordering entries alphabetically.
    pub fn sort_key(&self) -> &OsStr {
        match self {
            GameEntry::SingleFile(p) => p.file_name().unwrap_or_default(),
            GameEntry::MultiDisc { name, .. } => OsStr::new(name),
        }
    }

    /// The display name for this entry (filename or .m3u dir name).
    pub fn display_name(&self) -> &str {
        match self {
            GameEntry::SingleFile(p) => p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("?"),
            GameEntry::MultiDisc { name, .. } => name,
        }
    }

    /// Stem used for media file naming: filename stem for single files,
    /// full `.m3u` directory name for multi-disc (ES-DE matches media by
    /// the full entry name, e.g. `game.m3u.png` for `./game.m3u`).
    pub fn rom_stem(&self) -> &str {
        match self {
            GameEntry::SingleFile(p) => p
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or("?"),
            GameEntry::MultiDisc { name, .. } => name,
        }
    }

    /// The best file to use for analysis (serial extraction, identification).
    ///
    /// For single files, returns that file. For multi-disc sets, returns
    /// the first `.cue` file (preferred) or the first matching file.
    pub fn analysis_path(&self) -> &Path {
        match self {
            GameEntry::SingleFile(p) => p,
            GameEntry::MultiDisc { files, .. } => {
                // Prefer .cue over raw data files
                files
                    .iter()
                    .find(|p| {
                        p.extension()
                            .and_then(|e| e.to_str())
                            .map(|e| e.eq_ignore_ascii_case("cue"))
                            .unwrap_or(false)
                    })
                    .unwrap_or(&files[0])
            }
        }
    }

    /// All ROM file paths in this entry (1 for single, N for multi-disc).
    pub fn all_files(&self) -> &[PathBuf] {
        match self {
            GameEntry::SingleFile(p) => std::slice::from_ref(p),
            GameEntry::MultiDisc { files, .. } => files,
        }
    }
}

/// Scan a console folder and return logical game entries.
///
/// Handles:
/// - Top-level ROM files matching the given extensions
/// - `.m3u` subdirectories containing disc images (ES-DE convention)
/// - CUE/BIN deduplication (`.bin`/`.img`/`.iso` files paired with a `.cue` are filtered)
pub fn scan_game_entries(
    folder: &Path,
    extensions: &HashSet<String>,
) -> std::io::Result<Vec<GameEntry>> {
    let mut game_entries: Vec<GameEntry> = Vec::new();
    let mut dir_entries: Vec<std::fs::DirEntry> = std::fs::read_dir(folder)?
        .flatten()
        .collect();
    dir_entries.sort_by_key(|e| e.path());

    for entry in &dir_entries {
        let path = entry.path();
        if path.is_file() {
            if has_matching_extension(&path, extensions) {
                game_entries.push(GameEntry::SingleFile(path));
            }
        } else if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.ends_with(".m3u") {
                    let disc_files = collect_matching_files(&path, extensions);
                    if !disc_files.is_empty() {
                        game_entries.push(GameEntry::MultiDisc {
                            name: name.to_string(),
                            files: disc_files,
                        });
                    }
                }
            }
        }
    }

    // Dedup: filter out .bin/.img/.iso files that share a stem with a .cue
    let root_files: Vec<PathBuf> = game_entries
        .iter()
        .filter_map(|e| match e {
            GameEntry::SingleFile(p) => Some(p.clone()),
            _ => None,
        })
        .collect();
    let cue_stems = collect_cue_stems(&root_files);
    if !cue_stems.is_empty() {
        game_entries.retain(|e| match e {
            GameEntry::SingleFile(p) => !is_data_file_covered_by_cue(p, &cue_stems),
            GameEntry::MultiDisc { .. } => true,
        });
    }

    game_entries.sort_by(|a, b| a.sort_key().cmp(b.sort_key()));

    Ok(game_entries)
}

/// Build the extension set from an analyzer's file_extensions().
pub fn extension_set(extensions: &[&str]) -> HashSet<String> {
    extensions.iter().map(|e| e.to_lowercase()).collect()
}

/// Check if a path has an extension in the allowed set.
fn has_matching_extension(path: &Path, extensions: &HashSet<String>) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| extensions.contains(&e.to_lowercase()))
        .unwrap_or(false)
}

/// Collect all files with matching extensions from a directory (sorted).
fn collect_matching_files(dir: &Path, extensions: &HashSet<String>) -> Vec<PathBuf> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut files: Vec<PathBuf> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.is_file() && has_matching_extension(&path, extensions) {
                Some(path)
            } else {
                None
            }
        })
        .collect();

    files.sort();
    files
}

/// Collect the lowercase stems of all .cue files in a list of paths.
fn collect_cue_stems(files: &[PathBuf]) -> HashSet<String> {
    files
        .iter()
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("cue"))
                .unwrap_or(false)
        })
        .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(|s| s.to_lowercase()))
        .collect()
}

/// Returns true if this path is a disc data file whose stem matches a known CUE file.
fn is_data_file_covered_by_cue(path: &Path, cue_stems: &HashSet<String>) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    if !matches!(ext.as_str(), "bin" | "img" | "iso") {
        return false;
    }
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_lowercase())
        .unwrap_or_default();
    cue_stems.contains(&stem)
}
