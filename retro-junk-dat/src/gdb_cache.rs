//! GDB CSV cache management.
//!
//! Downloads and caches GameDataBase CSV files from GitHub. Uses a separate
//! cache namespace (`~/.cache/retro-junk/gdb/`) from the DAT cache to allow
//! independent versioning and lifecycle management.
//!
//! Data source: <https://github.com/PigSaint/GameDataBase>
//! License: CC BY 4.0 — Attribution to PigSaint required.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::DatError;
use crate::gdb::{self, GdbFile};
use crate::gdb_index::GdbIndex;

/// Cache format version for GDB files. Bump when changing download URLs or parse format.
const GDB_CACHE_VERSION: u32 = 1;

/// Base URL for downloading GDB CSV files from GitHub.
const GDB_BASE_URL: &str =
    "https://raw.githubusercontent.com/PigSaint/GameDataBase/main/";

/// Metadata about a cached GDB CSV file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedGdb {
    pub source: String,
    pub downloaded: String,
    pub file_size: u64,
    pub csv_name: String,
    pub game_count: usize,
}

/// Metadata file tracking all cached GDB CSVs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GdbCacheMeta {
    #[serde(default)]
    pub version: u32,
    /// Keyed by CSV name (e.g., "console_nintendo_famicom_nes")
    pub csvs: HashMap<String, CachedGdb>,
}

/// Information about a cached GDB CSV for display purposes.
#[derive(Debug, Clone)]
pub struct GdbCacheEntry {
    pub csv_name: String,
    pub file_size: u64,
    pub downloaded: String,
    pub game_count: usize,
}

/// Get the GDB cache directory.
pub fn gdb_cache_dir() -> Result<PathBuf, DatError> {
    let base =
        dirs::cache_dir().ok_or_else(|| DatError::cache("Could not determine cache directory"))?;
    Ok(base.join("retro-junk").join("gdb"))
}

/// Get the path to the GDB meta.json file.
fn gdb_meta_path() -> Result<PathBuf, DatError> {
    let base =
        dirs::cache_dir().ok_or_else(|| DatError::cache("Could not determine cache directory"))?;
    Ok(base.join("retro-junk").join("gdb-meta.json"))
}

/// Load GDB cache metadata, clearing if version mismatches.
fn load_meta() -> Result<GdbCacheMeta, DatError> {
    let path = gdb_meta_path()?;
    if !path.exists() {
        return Ok(GdbCacheMeta {
            version: GDB_CACHE_VERSION,
            ..Default::default()
        });
    }
    let contents = fs::read_to_string(&path)?;
    let meta: GdbCacheMeta = serde_json::from_str(&contents)?;
    if meta.version != GDB_CACHE_VERSION {
        let _ = clear();
        return Ok(GdbCacheMeta {
            version: GDB_CACHE_VERSION,
            ..Default::default()
        });
    }
    Ok(meta)
}

/// Save GDB cache metadata.
fn save_meta(meta: &GdbCacheMeta) -> Result<(), DatError> {
    let path = gdb_meta_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let contents = serde_json::to_string_pretty(meta)?;
    fs::write(&path, contents)?;
    Ok(())
}

/// Get the cached file path for a GDB CSV.
fn csv_file_path(csv_name: &str) -> Result<PathBuf, DatError> {
    Ok(gdb_cache_dir()?.join(format!("{csv_name}.csv")))
}

/// Construct the download URL for a GDB CSV file.
fn download_url(csv_name: &str) -> String {
    format!("{GDB_BASE_URL}{csv_name}.csv")
}

/// Download and cache a single GDB CSV file. Returns the local path.
pub fn fetch_gdb(csv_name: &str) -> Result<PathBuf, DatError> {
    let url = download_url(csv_name);
    let csv_path = csv_file_path(csv_name)?;

    if let Some(parent) = csv_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let response = reqwest::blocking::get(&url).map_err(|e| {
        DatError::download(format!("Failed to download GDB CSV '{csv_name}': {e}"))
    })?;

    if !response.status().is_success() {
        return Err(DatError::download(format!(
            "HTTP {} for GDB CSV '{csv_name}' ({url})",
            response.status()
        )));
    }

    let bytes = response.bytes().map_err(|e| {
        DatError::download(format!("Failed to read GDB response for '{csv_name}': {e}"))
    })?;

    fs::write(&csv_path, &bytes)?;

    // Parse to get game count for metadata
    let content = String::from_utf8_lossy(&bytes);
    let games = gdb::parse_gdb_csv(&content)?;
    let game_count = games.len();

    // Update metadata
    let mut meta = load_meta()?;
    meta.version = GDB_CACHE_VERSION;
    meta.csvs.insert(
        csv_name.to_string(),
        CachedGdb {
            source: url,
            downloaded: chrono_now(),
            file_size: bytes.len() as u64,
            csv_name: csv_name.to_string(),
            game_count,
        },
    );
    save_meta(&meta)?;

    Ok(csv_path)
}

/// Load a GDB CSV file, downloading if not cached. Returns parsed GdbFile.
pub fn load_gdb(csv_name: &str) -> Result<GdbFile, DatError> {
    let csv_path = csv_file_path(csv_name)?;

    // Use cached file if available and cache version is current
    let meta = load_meta()?;
    if meta.version == GDB_CACHE_VERSION && csv_path.exists() {
        return gdb::parse_gdb_file(&csv_path);
    }

    // Download and cache
    let path = fetch_gdb(csv_name)?;
    gdb::parse_gdb_file(&path)
}

/// Load a GDB CSV from a local directory (for --gdb-dir override).
pub fn load_gdb_from_dir(csv_name: &str, dir: &Path) -> Result<GdbFile, DatError> {
    let path = dir.join(format!("{csv_name}.csv"));
    if !path.exists() {
        return Err(DatError::cache(format!(
            "GDB CSV '{csv_name}.csv' not found in {}",
            dir.display()
        )));
    }
    gdb::parse_gdb_file(&path)
}

/// Load multiple GDB CSVs and merge into a single index.
pub fn load_gdb_index(csv_names: &[&str]) -> Result<GdbIndex, DatError> {
    let mut all_games = Vec::new();

    for csv_name in csv_names {
        let gdb_file = load_gdb(csv_name)?;
        all_games.extend(gdb_file.games);
    }

    Ok(GdbIndex::from_games(all_games))
}

/// Load multiple GDB CSVs from a local directory and merge into a single index.
pub fn load_gdb_index_from_dir(csv_names: &[&str], dir: &Path) -> Result<GdbIndex, DatError> {
    let mut all_games = Vec::new();

    for csv_name in csv_names {
        let gdb_file = load_gdb_from_dir(csv_name, dir)?;
        all_games.extend(gdb_file.games);
    }

    Ok(GdbIndex::from_games(all_games))
}

/// List all cached GDB CSV files.
pub fn list() -> Result<Vec<GdbCacheEntry>, DatError> {
    let meta = load_meta()?;
    let mut entries: Vec<GdbCacheEntry> = meta
        .csvs
        .values()
        .map(|c| GdbCacheEntry {
            csv_name: c.csv_name.clone(),
            file_size: c.file_size,
            downloaded: c.downloaded.clone(),
            game_count: c.game_count,
        })
        .collect();

    entries.sort_by(|a, b| a.csv_name.cmp(&b.csv_name));
    Ok(entries)
}

/// Clear all cached GDB files.
pub fn clear() -> Result<u64, DatError> {
    let dir = gdb_cache_dir()?;
    let mut total_size = 0u64;

    if dir.exists() {
        for entry in fs::read_dir(&dir)?.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Ok(m) = fs::metadata(&path) {
                    total_size += m.len();
                }
                fs::remove_file(&path)?;
            }
        }
    }

    let meta_path = gdb_meta_path()?;
    if meta_path.exists() {
        if let Ok(m) = fs::metadata(&meta_path) {
            total_size += m.len();
        }
        fs::remove_file(&meta_path)?;
    }

    Ok(total_size)
}

/// Get the total size of cached GDB files.
pub fn total_cache_size() -> Result<u64, DatError> {
    let meta = load_meta()?;
    Ok(meta.csvs.values().map(|c| c.file_size).sum())
}

/// Simple timestamp (same pattern as DAT cache).
fn chrono_now() -> String {
    use std::time::SystemTime;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let days = secs / 86400;
    let years = 1970 + days / 365;
    format!("{years}-xx-xx (unix: {secs})")
}
