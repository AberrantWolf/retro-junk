use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::dat::{self, DatFile};
use crate::error::DatError;
use retro_junk_core::DatSource;

/// Cache format version. Bump this when changing DAT sources or format to
/// invalidate stale cached DATs automatically.
const CACHE_VERSION: u32 = 6;

/// Metadata about a cached DAT file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedDat {
    pub source: String,
    pub downloaded: String,
    pub dat_version: String,
    pub file_size: u64,
    /// DAT name (e.g., "Nintendo - Nintendo 64" or "Sony - PlayStation")
    #[serde(default)]
    pub dat_name: String,
}

/// Metadata file tracking all cached DATs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CacheMeta {
    /// Cache format version — mismatched versions trigger automatic invalidation.
    #[serde(default)]
    pub version: u32,
    /// Per-console list of cached DATs (keyed by short_name).
    pub dats: HashMap<String, Vec<CachedDat>>,
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
    let base =
        dirs::cache_dir().ok_or_else(|| DatError::cache("Could not determine cache directory"))?;
    Ok(base.join("retro-junk").join("dats"))
}

/// Get the path to the meta.json file.
fn meta_path() -> Result<PathBuf, DatError> {
    let base =
        dirs::cache_dir().ok_or_else(|| DatError::cache("Could not determine cache directory"))?;
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
/// When a console has multiple DATs, they are stored as `{short_name}_{index}.dat`.
fn dat_file_path(short_name: &str, index: usize) -> Result<PathBuf, DatError> {
    if index == 0 {
        Ok(cache_dir()?.join(format!("{short_name}.dat")))
    } else {
        Ok(cache_dir()?.join(format!("{short_name}_{index}.dat")))
    }
}

/// Construct the download URL for a DAT file.
///
/// Both No-Intro and Redump DATs are hosted on the libretro-database GitHub
/// repo as raw `.dat` files. The download ID is the DAT name used as the
/// filename (e.g., "Sony - PlayStation" → "Sony%20-%20PlayStation.dat").
fn download_url(download_id: &str, dat_source: DatSource) -> String {
    let base = dat_source.base_url();
    let encoded = download_id.replace(' ', "%20");
    format!("{base}{encoded}.dat")
}

/// Download and cache all DAT files for a system.
///
/// `short_name` is used as the cache key. `dat_names` are the display names
/// for cache metadata. `download_ids` are the identifiers used to construct
/// download URLs (same as `dat_names` for No-Intro; system slugs for Redump).
/// `dat_source` determines the URL scheme and download format.
///
/// Returns paths to all successfully downloaded DAT files. Partial failures
/// are warned but don't fail the entire operation — partial coverage is better
/// than none.
pub fn fetch(
    short_name: &str,
    dat_names: &[&str],
    download_ids: &[&str],
    dat_source: DatSource,
) -> Result<Vec<PathBuf>, DatError> {
    let mut paths = Vec::new();
    let mut cached_entries = Vec::new();

    for (i, (dat_name, download_id)) in dat_names.iter().zip(download_ids.iter()).enumerate() {
        let url = download_url(download_id, dat_source);
        let dat_path = dat_file_path(short_name, i)?;

        // Ensure cache directory exists
        if let Some(parent) = dat_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Download
        let response = match reqwest::blocking::get(&url) {
            Ok(r) => r,
            Err(e) => {
                log::warn!("Failed to download {dat_name}: {e}");
                continue;
            }
        };

        if !response.status().is_success() {
            log::warn!("HTTP {} for {dat_name} ({url})", response.status());
            continue;
        }

        let bytes = match response.bytes() {
            Ok(b) => b,
            Err(e) => {
                log::warn!("Failed to read response for {dat_name}: {e}");
                continue;
            }
        };

        fs::write(&dat_path, &bytes)?;
        let dat_bytes = &bytes;

        // Parse to get version info
        let dat = dat::parse_dat_file(&dat_path)?;

        cached_entries.push(CachedDat {
            source: url,
            downloaded: chrono_now(),
            dat_version: dat.version.clone(),
            file_size: dat_bytes.len() as u64,
            dat_name: dat_name.to_string(),
        });

        paths.push(dat_path);
    }

    if paths.is_empty() {
        return Err(DatError::download(format!(
            "Failed to download any DATs for '{short_name}'"
        )));
    }

    // Update metadata
    let mut meta = load_meta()?;
    meta.version = CACHE_VERSION;
    meta.dats.insert(short_name.to_string(), cached_entries);
    save_meta(&meta)?;

    Ok(paths)
}

/// Load all DAT files for a system, either from a custom directory or from the cache.
/// If not cached and no custom dir is provided, downloads them automatically.
///
/// `short_name` is used as the cache key. `dat_names` are the display names.
/// `download_ids` are the identifiers used for URL construction.
/// `dat_source` determines the URL scheme and download format.
pub fn load_dats(
    short_name: &str,
    dat_names: &[&str],
    download_ids: &[&str],
    dat_dir: Option<&Path>,
    dat_source: DatSource,
) -> Result<Vec<DatFile>, DatError> {
    if let Some(dir) = dat_dir {
        // Try to find matching DATs in the custom directory
        let mut dats = Vec::new();
        for dat_name in dat_names {
            match find_dat_in_dir(short_name, dat_name, dir) {
                Ok(path) => dats.push(dat::parse_dat_file(&path)?),
                Err(e) => log::warn!("{e}"),
            }
        }
        if dats.is_empty() {
            return Err(DatError::cache(format!(
                "No DAT files found for '{short_name}' in {}",
                dir.display()
            )));
        }
        return Ok(dats);
    }

    // Check cache version before trusting cached files — a version mismatch
    // means the DAT source or format changed and we need to re-download.
    let meta = load_meta()?;
    let cache_valid = meta.version == CACHE_VERSION;

    // Try cache first — check if all indexed DATs exist and cache version is current
    if cache_valid {
        let mut cached_paths = Vec::new();
        for i in 0..dat_names.len() {
            let dat_path = dat_file_path(short_name, i)?;
            if dat_path.exists() {
                cached_paths.push(dat_path);
            }
        }

        if cached_paths.len() == dat_names.len() {
            let mut dats = Vec::new();
            for path in &cached_paths {
                dats.push(dat::parse_dat_file(path)?);
            }
            return Ok(dats);
        }
    }

    // Download and cache
    let paths = fetch(short_name, dat_names, download_ids, dat_source)?;
    let mut dats = Vec::new();
    for path in &paths {
        dats.push(dat::parse_dat_file(path)?);
    }
    Ok(dats)
}

/// Find a DAT file in a user-provided directory.
/// Looks for `{short_name}.dat` or matches by DAT name in the file.
fn find_dat_in_dir(short_name: &str, dat_name: &str, dir: &Path) -> Result<PathBuf, DatError> {
    // Try direct match: short_name.dat
    let direct = dir.join(format!("{short_name}.dat"));
    if direct.exists() {
        return Ok(direct);
    }

    // Look for files containing the DAT name
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("dat") {
                let name = path.file_stem().and_then(|n| n.to_str()).unwrap_or("");
                // Check if the filename contains the NoIntro system name
                if name.contains(dat_name) || dat_name.contains(name) {
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

    for (short_name, cached_list) in &meta.dats {
        for cached in cached_list {
            let dat_name = if cached.dat_name.is_empty() {
                short_name.clone()
            } else {
                cached.dat_name.clone()
            };

            entries.push(CacheEntry {
                short_name: short_name.clone(),
                dat_name,
                file_size: cached.file_size,
                downloaded: cached.downloaded.clone(),
                dat_version: cached.dat_version.clone(),
            });
        }
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
    Ok(meta
        .dats
        .values()
        .flat_map(|list| list.iter())
        .map(|c| c.file_size)
        .sum())
}

use crate::util::chrono_now;
