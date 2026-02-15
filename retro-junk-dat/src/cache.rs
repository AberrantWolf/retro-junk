use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::dat::{self, DatFile};
use crate::error::DatError;
use crate::systems;

const GITHUB_MIRROR_BASE: &str =
    "https://raw.githubusercontent.com/libretro/libretro-database/master/metadat/no-intro/";

/// Cache format version. Bump this when changing DAT sources or format to
/// invalidate stale cached DATs automatically.
const CACHE_VERSION: u32 = 2;

/// Metadata about a cached DAT file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedDat {
    pub source: String,
    pub downloaded: String,
    pub dat_version: String,
    pub file_size: u64,
}

/// Metadata file tracking all cached DATs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CacheMeta {
    /// Cache format version — mismatched versions trigger automatic invalidation.
    #[serde(default)]
    pub version: u32,
    pub dats: HashMap<String, CachedDat>,
}

/// Information about a cached DAT for display purposes.
#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub short_name: String,
    pub dat_name: String,
    pub file_size: u64,
    pub downloaded: String,
    pub dat_version: String,
}

/// Get the cache directory for retro-junk DAT files.
pub fn cache_dir() -> Result<PathBuf, DatError> {
    let base = dirs::cache_dir().ok_or_else(|| DatError::cache("Could not determine cache directory"))?;
    Ok(base.join("retro-junk").join("dats"))
}

/// Get the path to the meta.json file.
fn meta_path() -> Result<PathBuf, DatError> {
    let base = dirs::cache_dir().ok_or_else(|| DatError::cache("Could not determine cache directory"))?;
    Ok(base.join("retro-junk").join("meta.json"))
}

/// Load cache metadata. If the cache version doesn't match, clears stale data.
fn load_meta() -> Result<CacheMeta, DatError> {
    let path = meta_path()?;
    if !path.exists() {
        return Ok(CacheMeta {
            version: CACHE_VERSION,
            ..Default::default()
        });
    }
    let contents = fs::read_to_string(&path)?;
    let meta: CacheMeta = serde_json::from_str(&contents)?;
    if meta.version != CACHE_VERSION {
        // Stale cache from a different DAT source — wipe it
        let _ = clear();
        return Ok(CacheMeta {
            version: CACHE_VERSION,
            ..Default::default()
        });
    }
    Ok(meta)
}

/// Save cache metadata.
fn save_meta(meta: &CacheMeta) -> Result<(), DatError> {
    let path = meta_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let contents = serde_json::to_string_pretty(meta)?;
    fs::write(&path, contents)?;
    Ok(())
}

/// Get the cached DAT file path for a system.
fn dat_file_path(short_name: &str) -> Result<PathBuf, DatError> {
    Ok(cache_dir()?.join(format!("{short_name}.dat")))
}

/// Construct the download URL for a system's DAT file.
fn download_url(system: &systems::SystemMapping) -> String {
    // GitHub mirror uses the DAT name as the filename with .dat extension
    let encoded = system.dat_name.replace(' ', "%20");
    format!("{GITHUB_MIRROR_BASE}{encoded}.dat")
}

/// Download and cache a DAT file for a system.
pub fn fetch(short_name: &str) -> Result<PathBuf, DatError> {
    let system = systems::find_system(short_name)
        .ok_or_else(|| DatError::UnknownSystem(short_name.to_string()))?;

    let url = download_url(system);
    let dat_path = dat_file_path(short_name)?;

    // Ensure cache directory exists
    if let Some(parent) = dat_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Download
    let response = reqwest::blocking::get(&url)
        .map_err(|e| DatError::download(format!("Failed to download {url}: {e}")))?;

    if !response.status().is_success() {
        return Err(DatError::download(format!(
            "HTTP {} for {url}",
            response.status()
        )));
    }

    let bytes = response
        .bytes()
        .map_err(|e| DatError::download(format!("Failed to read response: {e}")))?;

    fs::write(&dat_path, &bytes)?;

    // Parse to get version info
    let dat = dat::parse_dat_file(&dat_path)?;

    // Update metadata
    let mut meta = load_meta()?;
    meta.version = CACHE_VERSION;
    meta.dats.insert(
        short_name.to_string(),
        CachedDat {
            source: url,
            downloaded: chrono_now(),
            dat_version: dat.version.clone(),
            file_size: bytes.len() as u64,
        },
    );
    save_meta(&meta)?;

    Ok(dat_path)
}

/// Load a DAT file, either from a custom directory or from the cache.
/// If not cached and no custom dir is provided, downloads it automatically.
pub fn load_dat(short_name: &str, dat_dir: Option<&Path>) -> Result<DatFile, DatError> {
    if let Some(dir) = dat_dir {
        // Try to find a matching DAT in the custom directory
        let path = find_dat_in_dir(short_name, dir)?;
        return dat::parse_dat_file(&path);
    }

    // Try cache first
    let dat_path = dat_file_path(short_name)?;
    if dat_path.exists() {
        return dat::parse_dat_file(&dat_path);
    }

    // Download and cache
    let path = fetch(short_name)?;
    dat::parse_dat_file(&path)
}

/// Find a DAT file in a user-provided directory.
/// Looks for `{short_name}.dat` or matches by DAT name in the file.
fn find_dat_in_dir(short_name: &str, dir: &Path) -> Result<PathBuf, DatError> {
    // Try direct match: short_name.dat
    let direct = dir.join(format!("{short_name}.dat"));
    if direct.exists() {
        return Ok(direct);
    }

    // Try matching by NoIntro DAT name
    let system = systems::find_system(short_name)
        .ok_or_else(|| DatError::UnknownSystem(short_name.to_string()))?;

    // Look for files containing the DAT name
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("dat") {
                let name = path
                    .file_stem()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                // Check if the filename contains the NoIntro system name
                if name.contains(system.dat_name)
                    || system.dat_name.contains(name)
                {
                    return Ok(path);
                }
            }
        }
    }

    Err(DatError::cache(format!(
        "No DAT file found for '{short_name}' in {}",
        dir.display()
    )))
}

/// List all cached DAT files.
pub fn list() -> Result<Vec<CacheEntry>, DatError> {
    let meta = load_meta()?;
    let mut entries = Vec::new();

    for (short_name, cached) in &meta.dats {
        let system = systems::find_system(short_name);
        let dat_name = system
            .map(|s| s.dat_name.to_string())
            .unwrap_or_else(|| short_name.clone());

        entries.push(CacheEntry {
            short_name: short_name.clone(),
            dat_name,
            file_size: cached.file_size,
            downloaded: cached.downloaded.clone(),
            dat_version: cached.dat_version.clone(),
        });
    }

    entries.sort_by(|a, b| a.short_name.cmp(&b.short_name));
    Ok(entries)
}

/// Clear all cached DAT files.
pub fn clear() -> Result<u64, DatError> {
    let dir = cache_dir()?;
    let mut total_size = 0u64;

    if dir.exists() {
        for entry in fs::read_dir(&dir)?.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Ok(meta) = fs::metadata(&path) {
                    total_size += meta.len();
                }
                fs::remove_file(&path)?;
            }
        }
    }

    // Also remove meta.json
    let meta = meta_path()?;
    if meta.exists() {
        if let Ok(m) = fs::metadata(&meta) {
            total_size += m.len();
        }
        fs::remove_file(&meta)?;
    }

    Ok(total_size)
}

/// Get the total size of cached DAT files.
pub fn total_cache_size() -> Result<u64, DatError> {
    let meta = load_meta()?;
    Ok(meta.dats.values().map(|c| c.file_size).sum())
}

/// Simple ISO-8601-ish timestamp without pulling in a chrono dependency.
fn chrono_now() -> String {
    use std::time::SystemTime;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    // Return Unix timestamp as string — good enough without chrono
    let secs = now.as_secs();
    // Format as YYYY-MM-DD using basic arithmetic
    let days = secs / 86400;
    let years = 1970 + days / 365; // approximate
    format!("{years}-xx-xx (unix: {secs})")
}
