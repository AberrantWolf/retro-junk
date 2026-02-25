use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use retro_junk_dat::{DatIndex, FileHashes, MatchMethod, SerialLookupResult};
use retro_junk_frontend::MediaType;
use retro_junk_lib::scanner::GameEntry;
use retro_junk_lib::{AnalysisError, Platform, RomIdentification};

use crate::app::RetroJunkApp;

// -- Navigation --

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Library,
    Settings,
    Tools,
}

// -- Library state --

#[derive(Default)]
pub struct Library {
    pub consoles: Vec<ConsoleState>,
}

impl Library {
    /// Find a console by folder_name. Returns the index.
    pub fn find_by_folder(&self, folder_name: &str) -> Option<usize> {
        self.consoles.iter().position(|c| c.folder_name == folder_name)
    }
}

pub struct ConsoleState {
    pub platform: Platform,
    pub folder_name: String,
    pub folder_path: PathBuf,
    pub manufacturer: &'static str,
    pub platform_name: &'static str,
    pub short_name: &'static str,
    pub scan_status: ScanStatus,
    pub entries: Vec<LibraryEntry>,
    pub dat_status: DatStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScanStatus {
    NotScanned,
    Scanning,
    Scanned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DatStatus {
    NotLoaded,
    Loading,
    Loaded { game_count: usize },
    Unavailable { reason: String },
}

impl Default for DatStatus {
    fn default() -> Self {
        Self::NotLoaded
    }
}

/// Derive the media directory for a console from the root path and folder name.
///
/// Convention: `root.parent() / "{root_name}-media" / folder_name`
/// Example: root=`/roms`, folder=`n64` → `/roms-media/n64/`
pub fn media_dir_for_console(root_path: &Path, folder_name: &str) -> Option<PathBuf> {
    let parent = root_path.parent()?;
    let root_name = root_path.file_name()?.to_str()?;
    Some(parent.join(format!("{}-media", root_name)).join(folder_name))
}

/// Subdirectory name for a media type (matches ES-DE layout).
fn media_subdir(mt: MediaType) -> &'static str {
    match mt {
        MediaType::Cover => "covers",
        MediaType::Cover3D => "3dboxes",
        MediaType::Screenshot => "screenshots",
        MediaType::TitleScreen => "titlescreens",
        MediaType::Marquee => "marquees",
        MediaType::Video => "videos",
        MediaType::Fanart => "fanart",
        MediaType::PhysicalMedia => "physicalmedia",
        MediaType::Miximage => "miximages",
    }
}

/// All displayable media types in preferred display order.
pub const DISPLAY_MEDIA_TYPES: &[MediaType] = &[
    MediaType::Cover,
    MediaType::Cover3D,
    MediaType::Screenshot,
    MediaType::TitleScreen,
    MediaType::Marquee,
    MediaType::PhysicalMedia,
    MediaType::Fanart,
    MediaType::Miximage,
];

/// Discover media files on disk for a given ROM entry.
///
/// Checks each media type subdirectory for a file matching `rom_stem.ext`.
pub fn collect_existing_media(media_dir: &Path, rom_stem: &str) -> HashMap<MediaType, PathBuf> {
    let mut found = HashMap::new();
    for &mt in DISPLAY_MEDIA_TYPES {
        if mt == MediaType::Video {
            continue;
        }
        let subdir = media_dir.join(media_subdir(mt));
        let ext = mt.default_extension();
        let path = subdir.join(format!("{}.{}", rom_stem, ext));
        if path.exists() {
            found.insert(mt, path);
        }
    }
    found
}

pub struct LibraryEntry {
    pub game_entry: GameEntry,
    pub identification: Option<RomIdentification>,
    pub hashes: Option<FileHashes>,
    pub dat_match: Option<DatMatchInfo>,
    pub status: EntryStatus,
    /// When status is Ambiguous, holds the candidate game names from the DAT.
    pub ambiguous_candidates: Vec<String>,
    /// Discovered media files on disk. `None` = not yet scanned, `Some(empty)` = scanned but none found.
    pub media_paths: Option<HashMap<MediaType, PathBuf>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatMatchInfo {
    pub game_name: String,
    pub method: MatchMethod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryStatus {
    /// Not yet analyzed / DAT not loaded
    Unknown,
    /// Analyzed but no serial and no hash match
    Unrecognized,
    /// Serial found but no DAT confirmation (or ambiguous)
    Ambiguous,
    /// DAT-matched (hash or serial confirmed)
    Matched,
}

impl EntryStatus {
    pub fn color(&self) -> egui::Color32 {
        match self {
            EntryStatus::Unknown => egui::Color32::GRAY,
            EntryStatus::Unrecognized => egui::Color32::from_rgb(220, 50, 50),
            EntryStatus::Ambiguous => egui::Color32::from_rgb(220, 180, 30),
            EntryStatus::Matched => egui::Color32::from_rgb(50, 180, 50),
        }
    }
}

// -- Background operations --

pub struct BackgroundOperation {
    pub id: u64,
    pub description: String,
    pub progress_current: u64,
    pub progress_total: u64,
    pub cancel_token: Arc<AtomicBool>,
}

impl BackgroundOperation {
    pub fn new(id: u64, description: String, cancel_token: Arc<AtomicBool>) -> Self {
        Self {
            id,
            description,
            progress_current: 0,
            progress_total: 0,
            cancel_token,
        }
    }

    pub fn progress_fraction(&self) -> f32 {
        if self.progress_total == 0 {
            0.0
        } else {
            self.progress_current as f32 / self.progress_total as f32
        }
    }
}

// -- Messages --

static NEXT_OP_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub fn next_operation_id() -> u64 {
    NEXT_OP_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Messages sent from background threads to the UI thread.
///
/// All messages use `folder_name: String` (not `Platform`) to identify which
/// console they target. This is critical because multiple folders can map to
/// the same platform (e.g., "gb" and "gbc" both map to `Platform::GameBoy`).
pub enum AppMessage {
    // -- Folder scan --
    ConsoleFolderFound {
        platform: Platform,
        folder_name: String,
        folder_path: PathBuf,
        manufacturer: &'static str,
        platform_name: &'static str,
        short_name: &'static str,
    },
    FolderScanComplete,

    // -- Quick scan --
    ConsoleScanComplete {
        folder_name: String,
        entries: Vec<GameEntry>,
    },
    EntryAnalyzed {
        folder_name: String,
        index: usize,
        result: Result<RomIdentification, AnalysisError>,
    },
    ConsoleScanDone {
        folder_name: String,
    },

    // -- DAT --
    DatLoaded {
        folder_name: String,
        platform: Platform,
        index: DatIndex,
    },
    DatLoadFailed {
        folder_name: String,
        error: String,
    },

    // -- Hashing --
    HashComplete {
        folder_name: String,
        index: usize,
        hashes: FileHashes,
    },
    HashFailed {
        folder_name: String,
        index: usize,
        error: String,
    },

    // -- Media --
    MediaLoaded {
        folder_name: String,
        entry_index: usize,
        media: HashMap<MediaType, PathBuf>,
    },

    // -- Operations --
    OperationProgress {
        op_id: u64,
        current: u64,
        total: u64,
    },
    OperationComplete {
        op_id: u64,
    },
    OperationFailed {
        op_id: u64,
        error: String,
    },
}

// -- Message handler --

pub fn handle_message(app: &mut RetroJunkApp, msg: AppMessage, ctx: &egui::Context) {
    match msg {
        AppMessage::ConsoleFolderFound {
            platform,
            folder_name,
            folder_path,
            manufacturer,
            platform_name,
            short_name,
        } => {
            // Avoid duplicates (keyed on folder_name, which is unique per directory)
            if app.library.consoles.iter().any(|c| c.folder_name == folder_name) {
                return;
            }
            app.library.consoles.push(ConsoleState {
                platform,
                folder_name,
                folder_path,
                manufacturer,
                platform_name,
                short_name,
                scan_status: ScanStatus::NotScanned,
                entries: Vec::new(),
                dat_status: DatStatus::NotLoaded,
            });
            // Sort by manufacturer then platform name then folder name
            app.library.consoles.sort_by(|a, b| {
                a.manufacturer.cmp(&b.manufacturer)
                    .then(a.platform_name.cmp(&b.platform_name))
                    .then(a.folder_name.cmp(&b.folder_name))
            });
        }

        AppMessage::FolderScanComplete => {
            app.operations.retain(|op| op.description != "Scanning folders...");

            // Auto-scan all unscanned consoles if setting is enabled
            if app.settings.general.auto_scan_on_open {
                let unscanned: Vec<usize> = app.library.consoles.iter()
                    .enumerate()
                    .filter(|(_, c)| c.scan_status == ScanStatus::NotScanned)
                    .map(|(i, _)| i)
                    .collect();
                for i in unscanned {
                    crate::backend::scan::quick_scan_console(app, i, ctx);
                }
            }
        }

        AppMessage::ConsoleScanComplete { folder_name, entries } => {
            if let Some(ci) = app.library.find_by_folder(&folder_name) {
                let console = &mut app.library.consoles[ci];
                console.entries = entries
                    .into_iter()
                    .map(|ge| LibraryEntry {
                        game_entry: ge,
                        identification: None,
                        hashes: None,
                        dat_match: None,
                        status: EntryStatus::Unknown,
                        ambiguous_candidates: Vec::new(),
                        media_paths: None,
                    })
                    .collect();
                console.scan_status = ScanStatus::Scanning;
            }
        }

        AppMessage::EntryAnalyzed { folder_name, index, result } => {
            if let Some(ci) = app.library.find_by_folder(&folder_name) {
                if let Some(entry) = app.library.consoles[ci].entries.get_mut(index) {
                    match result {
                        Ok(id) => {
                            let has_serial = id.serial_number.is_some();
                            entry.identification = Some(id);
                            entry.status = if has_serial {
                                EntryStatus::Ambiguous
                            } else {
                                EntryStatus::Unrecognized
                            };
                        }
                        Err(_) => {
                            entry.status = EntryStatus::Unrecognized;
                        }
                    }
                }
            }
        }

        AppMessage::ConsoleScanDone { folder_name } => {
            if let Some(ci) = app.library.find_by_folder(&folder_name) {
                app.library.consoles[ci].scan_status = ScanStatus::Scanned;
            }
            let desc_match = format!("Scanning ");
            app.operations.retain(|op| {
                // Match operations like "Scanning Super Nintendo..."
                !(op.description.starts_with(&desc_match))
                || !app.library.find_by_folder(&folder_name)
                    .and_then(|ci| Some(op.description.contains(app.library.consoles[ci].platform_name)))
                    .unwrap_or(false)
            });
            app.save_library_cache();
        }

        AppMessage::DatLoaded { folder_name, platform, index } => {
            let game_count = index.game_count();

            // Run serial matching for this specific console's entries
            if let Some(ci) = app.library.find_by_folder(&folder_name) {
                app.library.consoles[ci].dat_status = DatStatus::Loaded { game_count };

                let context = app.context.clone();
                if let Some(registered) = context.get_by_platform(platform) {
                    for entry in app.library.consoles[ci].entries.iter_mut() {
                        if let Some(ref id) = entry.identification {
                            if let Some(ref serial) = id.serial_number {
                                let game_code = registered.analyzer.extract_dat_game_code(serial);
                                match index.match_by_serial(serial, game_code.as_deref()) {
                                    SerialLookupResult::Match(m) => {
                                        let game_name = index.games[m.game_index].name.clone();
                                        entry.dat_match = Some(DatMatchInfo {
                                            game_name,
                                            method: m.method,
                                        });
                                        entry.status = EntryStatus::Matched;
                                        entry.ambiguous_candidates.clear();
                                    }
                                    SerialLookupResult::Ambiguous { candidates } => {
                                        entry.status = EntryStatus::Ambiguous;
                                        entry.ambiguous_candidates = candidates;
                                    }
                                    SerialLookupResult::NotFound => {
                                        // Keep current status
                                    }
                                }
                            }
                        }
                    }
                }

                // Re-check hash matches for entries that have cached hashes
                // but weren't resolved by serial alone (e.g. Ambiguous or Unrecognized)
                for entry in app.library.consoles[ci].entries.iter_mut() {
                    if entry.status != EntryStatus::Matched {
                        if let Some(ref hashes) = entry.hashes {
                            if let Some(m) = index.match_by_hash(hashes.data_size, hashes) {
                                let game_name = index.games[m.game_index].name.clone();
                                entry.dat_match = Some(DatMatchInfo {
                                    game_name,
                                    method: m.method,
                                });
                                entry.status = EntryStatus::Matched;
                                entry.ambiguous_candidates.clear();
                            }
                        }
                    }
                }
            }

            // Store the DatIndex for later hash matching
            app.dat_indices.insert(folder_name.clone(), index);

            app.operations.retain(|op| !op.description.contains("Loading DAT"));
            app.save_library_cache();
        }

        AppMessage::DatLoadFailed { folder_name, error } => {
            if let Some(ci) = app.library.find_by_folder(&folder_name) {
                app.library.consoles[ci].dat_status = DatStatus::Unavailable { reason: error };
            }
            app.operations.retain(|op| !op.description.contains("Loading DAT"));
        }

        AppMessage::HashComplete { folder_name, index, hashes } => {
            if let Some(ci) = app.library.find_by_folder(&folder_name) {
                if let Some(entry) = app.library.consoles[ci].entries.get_mut(index) {
                    // Try hash matching against the loaded DAT
                    if let Some(dat_index) = app.dat_indices.get(&folder_name) {
                        if let Some(m) = dat_index.match_by_hash(hashes.data_size, &hashes) {
                            let game_name = dat_index.games[m.game_index].name.clone();
                            entry.dat_match = Some(DatMatchInfo {
                                game_name,
                                method: m.method,
                            });
                            entry.status = EntryStatus::Matched;
                            entry.ambiguous_candidates.clear();
                        }
                    }
                    entry.hashes = Some(hashes);
                }
            }
            app.save_library_cache();
        }

        AppMessage::HashFailed { folder_name, index, error } => {
            log::warn!("Hash failed for {} entry {}: {}", folder_name, index, error);
        }

        AppMessage::MediaLoaded { folder_name, entry_index, media } => {
            if let Some(ci) = app.library.find_by_folder(&folder_name) {
                if let Some(entry) = app.library.consoles[ci].entries.get_mut(entry_index) {
                    entry.media_paths = Some(media);
                }
            }
        }

        AppMessage::OperationProgress { op_id, current, total } => {
            if let Some(op) = app.operations.iter_mut().find(|op| op.id == op_id) {
                op.progress_current = current;
                op.progress_total = total;
            }
        }

        AppMessage::OperationComplete { op_id } => {
            app.operations.retain(|op| op.id != op_id);
        }

        AppMessage::OperationFailed { op_id, error } => {
            log::warn!("Operation {op_id} failed: {error}");
            app.operations.retain(|op| op.id != op_id);
        }
    }
}
