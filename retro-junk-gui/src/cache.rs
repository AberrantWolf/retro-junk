use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

use retro_junk_lib::{AnalysisContext, Platform, RomIdentification};
use retro_junk_dat::FileHashes;
use retro_junk_lib::scanner::GameEntry;

use crate::state::{
    ConsoleState, DatMatchInfo, DatStatus, EntryStatus, Library, LibraryEntry, ScanStatus,
};

const LIBRARY_CACHE_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
pub struct LibraryCache {
    pub version: u32,
    pub root_path: PathBuf,
    pub saved_at: String,
    pub consoles: Vec<CachedConsole>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CachedConsole {
    pub platform: Platform,
    pub folder_name: String,
    pub folder_path: PathBuf,
    pub fingerprint: FolderFingerprint,
    pub entries: Vec<CachedEntry>,
    pub dat_game_count: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FolderFingerprint {
    pub file_count: usize,
    pub total_size: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CachedEntry {
    pub game_entry: GameEntry,
    pub identification: Option<RomIdentification>,
    pub hashes: Option<FileHashes>,
    pub dat_match: Option<DatMatchInfo>,
    pub status: EntryStatus,
    pub ambiguous_candidates: Vec<String>,
}

/// Returns `~/.cache/retro-junk/library/`.
pub fn cache_dir() -> PathBuf {
    let cache = dirs::cache_dir().unwrap_or_else(|| PathBuf::from("."));
    cache.join("retro-junk").join("library")
}

/// SHA-256 of canonical path, truncated to 16 hex chars.
pub fn cache_key(root: &Path) -> String {
    let canonical = root
        .canonicalize()
        .unwrap_or_else(|_| root.to_path_buf());
    let hash = Sha256::digest(canonical.to_string_lossy().as_bytes());
    hex_encode(&hash[..8])
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn cache_path(root: &Path) -> PathBuf {
    cache_dir().join(format!("{}.json", cache_key(root)))
}

/// Save the current library state to a cache file.
pub fn save_library(root: &Path, library: &Library) -> std::io::Result<()> {
    let dir = cache_dir();
    std::fs::create_dir_all(&dir)?;

    let cached = LibraryCache {
        version: LIBRARY_CACHE_VERSION,
        root_path: root.to_path_buf(),
        saved_at: chrono::Utc::now().to_rfc3339(),
        consoles: library
            .consoles
            .iter()
            .filter(|c| c.scan_status == ScanStatus::Scanned)
            .map(|c| CachedConsole {
                platform: c.platform,
                folder_name: c.folder_name.clone(),
                folder_path: c.folder_path.clone(),
                fingerprint: compute_fingerprint(&c.folder_path),
                entries: c
                    .entries
                    .iter()
                    .map(|e| CachedEntry {
                        game_entry: e.game_entry.clone(),
                        identification: e.identification.clone(),
                        hashes: e.hashes.clone(),
                        dat_match: e.dat_match.clone(),
                        status: e.status,
                        ambiguous_candidates: e.ambiguous_candidates.clone(),
                    })
                    .collect(),
                dat_game_count: match &c.dat_status {
                    DatStatus::Loaded { game_count } => Some(*game_count),
                    _ => None,
                },
            })
            .collect(),
    };

    let contents = serde_json::to_string_pretty(&cached)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    let path = cache_path(root);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Load a cached library. Returns the library and a list of stale folder names
/// that need re-scanning due to changed fingerprints.
pub fn load_library(
    root: &Path,
    context: &AnalysisContext,
) -> Option<(Library, Vec<String>)> {
    let path = cache_path(root);
    let contents = std::fs::read_to_string(&path).ok()?;
    let cached: LibraryCache = serde_json::from_str(&contents).ok()?;

    if cached.version != LIBRARY_CACHE_VERSION {
        log::info!("Cache version mismatch, ignoring");
        return None;
    }

    let mut consoles = Vec::new();
    let mut stale_folders = Vec::new();

    for cc in cached.consoles {
        let registered = context.get_by_platform(cc.platform);
        let (manufacturer, platform_name, short_name) = match registered {
            Some(r) => (
                r.metadata.manufacturer,
                r.metadata.platform_name,
                r.metadata.short_name,
            ),
            None => {
                log::warn!(
                    "Platform {:?} not registered, skipping cached console {}",
                    cc.platform,
                    cc.folder_name
                );
                continue;
            }
        };

        let current_fp = compute_fingerprint(&cc.folder_path);
        let is_stale = current_fp.file_count != cc.fingerprint.file_count
            || current_fp.total_size != cc.fingerprint.total_size;

        if is_stale {
            stale_folders.push(cc.folder_name.clone());
            // Still add the console but mark as needing re-scan
            consoles.push(ConsoleState {
                platform: cc.platform,
                folder_name: cc.folder_name,
                folder_path: cc.folder_path,
                manufacturer,
                platform_name,
                short_name,
                scan_status: ScanStatus::NotScanned,
                entries: Vec::new(),
                dat_status: DatStatus::NotLoaded,
            });
        } else {
            let entries = cc
                .entries
                .into_iter()
                .map(|ce| LibraryEntry {
                    game_entry: ce.game_entry,
                    identification: ce.identification,
                    hashes: ce.hashes,
                    dat_match: ce.dat_match,
                    status: ce.status,
                    ambiguous_candidates: ce.ambiguous_candidates,
                    media_paths: None, // re-discovered lazily
                })
                .collect();

            let dat_status = match cc.dat_game_count {
                Some(game_count) => DatStatus::Loaded { game_count },
                None => DatStatus::NotLoaded,
            };

            consoles.push(ConsoleState {
                platform: cc.platform,
                folder_name: cc.folder_name,
                folder_path: cc.folder_path,
                manufacturer,
                platform_name,
                short_name,
                scan_status: ScanStatus::Scanned,
                entries,
                dat_status,
            });
        }
    }

    // Sort same as handle_message does
    consoles.sort_by(|a, b| {
        a.manufacturer
            .cmp(&b.manufacturer)
            .then(a.platform_name.cmp(&b.platform_name))
            .then(a.folder_name.cmp(&b.folder_name))
    });

    Some((Library { consoles }, stale_folders))
}

/// Delete the cache file for a given root path.
pub fn delete_cache(root: &Path) -> std::io::Result<()> {
    let path = cache_path(root);
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

/// List all cached roots with their file sizes.
pub fn list_caches() -> Vec<(PathBuf, u64)> {
    let dir = cache_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                let contents = std::fs::read_to_string(&path).ok()?;
                let cached: LibraryCache = serde_json::from_str(&contents).ok()?;
                let size = e.metadata().ok()?.len();
                Some((cached.root_path, size))
            } else {
                None
            }
        })
        .collect()
}

/// Compute a quick fingerprint of a folder (file count + total size).
pub fn compute_fingerprint(path: &Path) -> FolderFingerprint {
    let mut file_count = 0usize;
    let mut total_size = 0u64;

    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if meta.is_file() {
                    file_count += 1;
                    total_size += meta.len();
                } else if meta.is_dir() {
                    // Count files in subdirectories (for .m3u folders)
                    if let Ok(sub_entries) = std::fs::read_dir(entry.path()) {
                        for sub in sub_entries.flatten() {
                            if let Ok(sub_meta) = sub.metadata() {
                                if sub_meta.is_file() {
                                    file_count += 1;
                                    total_size += sub_meta.len();
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    FolderFingerprint {
        file_count,
        total_size,
    }
}

/// Total size of all library cache files in bytes.
pub fn total_cache_size() -> u64 {
    let dir = cache_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };

    entries
        .flatten()
        .filter_map(|e| e.metadata().ok().map(|m| m.len()))
        .sum()
}

/// Delete all library cache files.
pub fn clear_all_caches() -> std::io::Result<()> {
    let dir = cache_dir();
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    Ok(())
}
