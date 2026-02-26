use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

use retro_junk_dat::FileHashes;
use retro_junk_lib::scanner::GameEntry;
use retro_junk_lib::{AnalysisContext, Platform, Region, RomIdentification};

use crate::state::{
    ConsoleState, DatMatchInfo, DatStatus, EntryStatus, Library, LibraryEntry, ScanStatus,
};

const LIBRARY_CACHE_VERSION: u32 = 3;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderFingerprint {
    /// Hash of sorted filenames in the folder (no metadata/stat calls needed).
    pub name_hash: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CachedEntry {
    pub game_entry: GameEntry,
    pub identification: Option<RomIdentification>,
    pub hashes: Option<FileHashes>,
    pub dat_match: Option<DatMatchInfo>,
    pub status: EntryStatus,
    pub ambiguous_candidates: Vec<String>,
    #[serde(default)]
    pub region_override: Option<Region>,
    #[serde(default)]
    pub cover_title: Option<String>,
    #[serde(default)]
    pub screen_title: Option<String>,
}

/// Returns `~/.cache/retro-junk/library/`.
pub fn cache_dir() -> PathBuf {
    let cache = dirs::cache_dir().unwrap_or_else(|| PathBuf::from("."));
    cache.join("retro-junk").join("library")
}

/// SHA-256 of canonical path, truncated to 16 hex chars.
pub fn cache_key(root: &Path) -> String {
    let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
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
                fingerprint: c
                    .fingerprint
                    .clone()
                    .unwrap_or_else(|| compute_fingerprint(&c.folder_path)),
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
                        region_override: e.region_override,
                        cover_title: e.cover_title.clone(),
                        screen_title: e.screen_title.clone(),
                    })
                    .collect(),
                dat_game_count: match &c.dat_status {
                    DatStatus::Loaded { game_count } => Some(*game_count),
                    _ => None,
                },
            })
            .collect(),
    };

    let contents = serde_json::to_string_pretty(&cached).map_err(std::io::Error::other)?;
    let path = cache_path(root);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Load a cached library. Returns the library and a list of stale folder names
/// that need re-scanning due to changed fingerprints.
pub fn load_library(root: &Path, context: &AnalysisContext) -> Option<(Library, Vec<String>)> {
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
        let (manufacturer, platform_name) = match registered {
            Some(r) => (r.metadata.manufacturer, r.metadata.platform_name),
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
        let is_stale = current_fp.name_hash != cc.fingerprint.name_hash;

        if is_stale {
            stale_folders.push(cc.folder_name.clone());
            // Still add the console but mark as needing re-scan
            consoles.push(ConsoleState {
                platform: cc.platform,
                folder_name: cc.folder_name,
                folder_path: cc.folder_path,
                manufacturer,
                platform_name,
                scan_status: ScanStatus::NotScanned,
                entries: Vec::new(),
                dat_status: DatStatus::NotLoaded,
                fingerprint: None,
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
                    region_override: ce.region_override,
                    cover_title: ce.cover_title,
                    screen_title: ce.screen_title,
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
                scan_status: ScanStatus::Scanned,
                entries,
                dat_status,
                fingerprint: Some(current_fp),
            });
        }
    }

    // Sort same as handle_message does
    consoles.sort_by(|a, b| {
        a.manufacturer
            .cmp(b.manufacturer)
            .then(a.platform_name.cmp(b.platform_name))
            .then(a.folder_name.cmp(&b.folder_name))
    });

    if consoles.is_empty() {
        return None;
    }

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

/// Compute a quick fingerprint of a folder by hashing sorted filenames.
///
/// Only uses `read_dir` + `file_type()` (no `metadata()`/`stat()` calls),
/// so this is fast even for folders with thousands of entries.
pub fn compute_fingerprint(path: &Path) -> FolderFingerprint {
    let mut names = Vec::new();

    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            // Check subdirectories using file_type() (free on macOS/Linux via d_type)
            if let Ok(ft) = entry.file_type()
                && ft.is_dir()
                && let Ok(sub_entries) = std::fs::read_dir(entry.path())
            {
                for sub in sub_entries.flatten() {
                    names.push(format!("{}/{}", name, sub.file_name().to_string_lossy()));
                }
            }
            names.push(name);
        }
    }

    names.sort();
    let hash = Sha256::digest(names.join("\n").as_bytes());
    FolderFingerprint {
        name_hash: hex_encode(&hash[..8]),
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
