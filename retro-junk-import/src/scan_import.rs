//! Scan ROM folders, match files to catalog media, and manage collection entries.
//!
//! This module scans ROM directories, hashes each file using the appropriate
//! analyzer (for header stripping and byte-order normalization), and matches
//! against the catalog's media table by CRC32/SHA1. Matched files are recorded
//! as owned in the collection table.

use std::collections::HashSet;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use retro_junk_catalog::types::*;
use retro_junk_core::{Platform, RomAnalyzer};
use retro_junk_db::{operations, queries};
use rusqlite::Connection;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ScanError {
    #[error("Database error: {0}")]
    Db(#[from] operations::OperationError),
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Hashing error: {0}")]
    Hash(#[from] retro_junk_dat::DatError),
}

/// Options for a collection scan.
pub struct ScanOptions {
    /// User ID for collection entries (default: "default").
    pub user_id: String,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            user_id: "default".to_string(),
        }
    }
}

/// Statistics from a scan operation.
#[derive(Debug, Default)]
pub struct ScanStats {
    pub files_scanned: u64,
    pub matched: u64,
    pub unmatched: u64,
    pub already_owned: u64,
    pub errors: u64,
}

/// An unmatched ROM file — not found in the catalog.
#[derive(Debug)]
pub struct UnmatchedFile {
    pub path: PathBuf,
    pub crc32: String,
    pub sha1: Option<String>,
    pub file_size: u64,
}

/// Result of scanning a single folder.
pub struct ScanResult {
    pub stats: ScanStats,
    pub unmatched: Vec<UnmatchedFile>,
}

/// Progress callbacks for scanning.
pub trait ScanProgress {
    fn on_file(&self, current: usize, total: usize, filename: &str);
    fn on_match(&self, filename: &str, title: &str);
    fn on_no_match(&self, filename: &str);
    fn on_error(&self, filename: &str, error: &str);
    fn on_complete(&self, stats: &ScanStats);
}

/// Silent progress — no output.
pub struct SilentScanProgress;

impl ScanProgress for SilentScanProgress {
    fn on_file(&self, _: usize, _: usize, _: &str) {}
    fn on_match(&self, _: &str, _: &str) {}
    fn on_no_match(&self, _: &str) {}
    fn on_error(&self, _: &str, _: &str) {}
    fn on_complete(&self, _: &ScanStats) {}
}

/// Scan a ROM folder and match files against the catalog.
///
/// Uses the provided analyzer's `dat_header_size()` and `dat_chunk_normalizer()`
/// for correct hash computation (header stripping, byte-order normalization).
/// Matched files are recorded in the collection table.
pub fn scan_folder(
    conn: &Connection,
    folder: &Path,
    analyzer: &dyn RomAnalyzer,
    platform: Platform,
    options: &ScanOptions,
    progress: Option<&dyn ScanProgress>,
) -> Result<ScanResult, ScanError> {
    let extensions: HashSet<String> = analyzer
        .file_extensions()
        .iter()
        .map(|e| e.to_lowercase())
        .collect();

    let entries = retro_junk_lib::scanner::scan_game_entries(folder, &extensions)?;
    let mut stats = ScanStats::default();
    let mut unmatched = Vec::new();

    // Collect all file paths from entries
    let all_files: Vec<PathBuf> = entries
        .iter()
        .flat_map(|entry| entry.all_files().iter().cloned())
        .collect();

    let total = all_files.len();

    for (i, file_path) in all_files.iter().enumerate() {
        let filename = file_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        if let Some(p) = progress {
            p.on_file(i + 1, total, &filename);
        }

        stats.files_scanned += 1;

        // Hash the file
        let hashes = match hash_file(file_path, analyzer) {
            Ok(h) => h,
            Err(e) => {
                if let Some(p) = progress {
                    p.on_error(&filename, &e.to_string());
                }
                stats.errors += 1;
                continue;
            }
        };

        // Try to match by CRC32 first, then SHA1
        let matched_media = find_matching_media(conn, &hashes, platform)?;

        match matched_media {
            Some((media, title)) => {
                // Check if already in collection
                let existing = queries::find_collection_entry(conn, &media.id, &options.user_id)?;
                if existing.is_some() {
                    stats.already_owned += 1;
                } else {
                    // Create collection entry
                    let now = chrono::Utc::now().to_rfc3339();
                    let entry = CollectionEntry {
                        id: 0,
                        media_id: media.id.clone(),
                        user_id: options.user_id.clone(),
                        owned: true,
                        condition: None,
                        notes: None,
                        date_acquired: None,
                        rom_path: Some(file_path.to_string_lossy().to_string()),
                        verified_at: Some(now),
                    };
                    operations::upsert_collection_entry(conn, &entry)?;
                    stats.matched += 1;
                }

                if let Some(p) = progress {
                    p.on_match(&filename, &title);
                }
            }
            None => {
                stats.unmatched += 1;
                unmatched.push(UnmatchedFile {
                    path: file_path.clone(),
                    crc32: hashes.crc32,
                    sha1: hashes.sha1,
                    file_size: hashes.data_size,
                });

                if let Some(p) = progress {
                    p.on_no_match(&filename);
                }
            }
        }
    }

    if let Some(p) = progress {
        p.on_complete(&stats);
    }

    Ok(ScanResult { stats, unmatched })
}

/// Re-verify existing collection entries against files on disk.
///
/// For each collection entry with a rom_path, re-hash the file and check
/// that it still matches the catalog. Returns the number of entries verified
/// and the number that no longer match or are missing.
pub fn verify_collection(
    conn: &Connection,
    analyzer: &dyn RomAnalyzer,
    platform: Platform,
    user_id: &str,
) -> Result<VerifyStats, ScanError> {
    let mut stats = VerifyStats::default();

    let entries = queries::list_collection(conn, Some(platform.short_name()), None)?;

    for entry in &entries {
        let rom_path = match &entry.rom_path {
            Some(p) => PathBuf::from(p),
            None => {
                stats.no_path += 1;
                continue;
            }
        };

        stats.checked += 1;

        if !rom_path.exists() {
            stats.missing += 1;
            log::warn!(
                "ROM file missing for '{}': {}",
                entry.title,
                rom_path.display()
            );
            continue;
        }

        // Re-hash and compare
        match hash_file(&rom_path, analyzer) {
            Ok(hashes) => {
                let crc_match = entry.crc32.as_deref() == Some(&hashes.crc32);
                let sha1_match = match (&entry.sha1, &hashes.sha1) {
                    (Some(expected), Some(actual)) => expected == actual,
                    _ => true, // If either is missing, don't fail on SHA1
                };

                if crc_match && sha1_match {
                    // Update verified_at timestamp
                    let now = chrono::Utc::now().to_rfc3339();
                    conn.execute(
                        "UPDATE collection SET verified_at = ?1 WHERE media_id = ?2 AND user_id = ?3",
                        rusqlite::params![now, entry.media_id, user_id],
                    )?;
                    stats.verified += 1;
                } else {
                    stats.hash_mismatch += 1;
                    log::warn!(
                        "Hash mismatch for '{}' at {}",
                        entry.title,
                        rom_path.display()
                    );
                }
            }
            Err(e) => {
                stats.errors += 1;
                log::warn!(
                    "Error hashing '{}' at {}: {}",
                    entry.title,
                    rom_path.display(),
                    e
                );
            }
        }
    }

    Ok(stats)
}

/// Statistics from a verification run.
#[derive(Debug, Default)]
pub struct VerifyStats {
    pub checked: u64,
    pub verified: u64,
    pub missing: u64,
    pub hash_mismatch: u64,
    pub no_path: u64,
    pub errors: u64,
}

// ── Internal Helpers ────────────────────────────────────────────────────────

/// Hash a ROM file using the analyzer's header stripping and normalization.
fn hash_file(
    path: &Path,
    analyzer: &dyn RomAnalyzer,
) -> Result<retro_junk_dat::matcher::FileHashes, ScanError> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let hashes = retro_junk_lib::hasher::compute_crc32_sha1(&mut reader, analyzer)?;
    Ok(hashes)
}

/// Find a media entry matching the given hashes, scoped to a platform.
///
/// Tries CRC32 first (fast index lookup), then validates with SHA1 if available.
fn find_matching_media(
    conn: &Connection,
    hashes: &retro_junk_dat::matcher::FileHashes,
    platform: Platform,
) -> Result<Option<(Media, String)>, ScanError> {
    let platform_id = platform.short_name();
    // Try CRC32 match
    let candidates = queries::find_media_by_crc32(conn, &hashes.crc32)?;
    for media in candidates {
        // Verify platform via release
        let release = match queries::get_release_by_id(conn, &media.release_id)? {
            Some(r) => r,
            None => continue,
        };
        if release.platform_id != platform_id {
            continue;
        }

        // If we have SHA1, verify it matches
        if let (Some(expected), Some(actual)) = (&media.sha1, &hashes.sha1)
            && expected != actual
        {
            continue;
        }

        return Ok(Some((media, release.title)));
    }

    // Try SHA1 match as fallback (if CRC32 had no results)
    if let Some(ref sha1) = hashes.sha1 {
        let candidates = queries::find_media_by_sha1(conn, sha1)?;
        for media in candidates {
            let release = match queries::get_release_by_id(conn, &media.release_id)? {
                Some(r) => r,
                None => continue,
            };
            if release.platform_id != platform_id {
                continue;
            }
            return Ok(Some((media, release.title)));
        }
    }

    Ok(None)
}
