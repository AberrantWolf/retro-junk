use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use retro_junk_dat::{DatIndex, FileHashes, MatchMethod, SerialLookupResult};
use retro_junk_frontend::MediaType;
use retro_junk_lib::scanner::GameEntry;
use retro_junk_lib::{AnalysisError, Platform, Region, RomIdentification};

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
        self.consoles
            .iter()
            .position(|c| c.folder_name == folder_name)
    }
}

pub struct ConsoleState {
    pub platform: Platform,
    pub folder_name: String,
    pub folder_path: PathBuf,
    pub manufacturer: &'static str,
    pub platform_name: &'static str,
    pub scan_status: ScanStatus,
    pub entries: Vec<LibraryEntry>,
    pub dat_status: DatStatus,
    /// Cached folder fingerprint (avoids recomputing on every save).
    pub fingerprint: Option<crate::cache::FolderFingerprint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScanStatus {
    NotScanned,
    Scanning,
    Scanned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DatStatus {
    #[default]
    NotLoaded,
    Loading,
    Loaded {
        game_count: usize,
    },
    Unavailable {
        reason: String,
    },
}

/// Derive the media directory for a console from the root path, folder name, and user setting.
///
/// If `setting` is empty, uses the legacy `{root}-media` sibling convention.
/// Otherwise, the setting is treated as a path (absolute or relative to `root_path`).
pub fn media_dir_for_console(
    root_path: &Path,
    folder_name: &str,
    setting: &str,
) -> Option<PathBuf> {
    if setting.is_empty() {
        // Legacy default: {root}-media sibling
        let parent = root_path.parent()?;
        let root_name = root_path.file_name()?.to_str()?;
        Some(
            parent
                .join(format!("{}-media", root_name))
                .join(folder_name),
        )
    } else {
        Some(resolve_dir(root_path, setting).join(folder_name))
    }
}

/// Derive the metadata directory for a console from the root path, folder name, and user setting.
///
/// The setting is treated as a path (absolute or relative to `root_path`).
/// Default setting `"."` places metadata inline with ROMs (ES-DE legacy mode).
pub fn metadata_dir_for_console(
    root_path: &Path,
    folder_name: &str,
    setting: &str,
) -> Option<PathBuf> {
    Some(resolve_dir(root_path, setting).join(folder_name))
}

/// Resolve a directory setting to an absolute path.
///
/// - Absolute paths are used as-is.
/// - Relative paths are resolved from `root_path`.
fn resolve_dir(root_path: &Path, setting: &str) -> PathBuf {
    let p = Path::new(setting);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        root_path.join(p)
    }
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
    /// User-set region override. When set, takes precedence over detected regions.
    pub region_override: Option<Region>,
    /// Box/cover title from catalog DB (e.g., the title printed on the game box).
    pub cover_title: Option<String>,
    /// Screen title from catalog DB (e.g., the title shown on the title screen).
    pub screen_title: Option<String>,
}

impl LibraryEntry {
    /// Returns the effective region list: the override if set, otherwise the detected regions.
    pub fn effective_regions(&self) -> Vec<Region> {
        if let Some(r) = self.region_override {
            vec![r]
        } else if let Some(ref id) = self.identification {
            id.regions.clone()
        } else {
            Vec::new()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatMatchInfo {
    pub game_name: String,
    /// Individual ROM filename from the DAT (e.g., "Game Name (USA).chd").
    #[serde(default)]
    pub rom_name: String,
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

    /// Human-readable tooltip explaining this status.
    pub fn tooltip(&self) -> &'static str {
        match self {
            EntryStatus::Unknown => "Not yet analyzed",
            EntryStatus::Unrecognized => "Not recognized \u{2013} no serial or hash match found",
            EntryStatus::Ambiguous => "Possible match \u{2013} hash verification needed to confirm",
            EntryStatus::Matched => "Verified match in database",
        }
    }

    /// Severity ranking (higher = worse). Used to find the worst status in a folder.
    pub fn severity(&self) -> u8 {
        match self {
            EntryStatus::Matched => 0,
            EntryStatus::Ambiguous => 1,
            EntryStatus::Unknown => 2,
            EntryStatus::Unrecognized => 3,
        }
    }
}

// -- Rename results --

pub struct RenameResult {
    pub entry_index: usize,
    pub outcome: RenameOutcome,
}

pub enum RenameOutcome {
    Renamed {
        source: PathBuf,
        target: PathBuf,
    },
    AlreadyCorrect,
    NoMatch {
        reason: String,
    },
    Error {
        message: String,
    },
    M3uRenamed {
        target_folder: PathBuf,
        discs_renamed: usize,
        playlist_written: bool,
        folder_renamed: bool,
        errors: Vec<String>,
    },
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

// -- Catalog enrichment --

/// Try to enrich a library entry with cover/screen titles from the catalog DB.
///
/// Skips if the entry already has a cover_title or has no SHA1 hash.
/// SQLite indexed lookups are sub-millisecond, safe for the main thread.
fn try_catalog_enrich(entry: &mut LibraryEntry, conn: &retro_junk_db::Connection) {
    if entry.cover_title.is_some() {
        return;
    }
    let sha1 = match entry.hashes.as_ref().and_then(|h| h.sha1.as_deref()) {
        Some(s) => s,
        None => return,
    };
    let media_list = match retro_junk_db::find_media_by_sha1(conn, sha1) {
        Ok(m) => m,
        Err(_) => return,
    };
    let release_id = match media_list.first() {
        Some(m) => &m.release_id,
        None => return,
    };
    if let Ok(Some(release)) = retro_junk_db::get_release_by_id(conn, release_id) {
        entry.cover_title = release.cover_title;
        entry.screen_title = release.screen_title;
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

    // -- Media / Scraping --
    MediaLoaded {
        folder_name: String,
        entry_index: usize,
        media: HashMap<MediaType, PathBuf>,
    },
    ScrapeEntryFailed {
        folder_name: String,
        entry_index: usize,
        error: String,
    },
    ScrapeFatalError {
        message: String,
        op_id: u64,
    },

    // -- Cache --
    CacheLoaded {
        library: Library,
    },

    /// Sent after the cache load attempt finishes (whether cache existed or not)
    /// to kick off the folder scan. This ensures the cache is merged before
    /// any scan can overwrite it.
    StartFolderScan,

    // -- Export --
    ExportComplete {
        folder_name: String,
        result: Result<String, String>,
    },

    // -- Rename --
    RenameComplete {
        folder_name: String,
        results: Vec<RenameResult>,
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
        } => {
            // Avoid duplicates (keyed on folder_name, which is unique per directory)
            if app
                .library
                .consoles
                .iter()
                .any(|c| c.folder_name == folder_name)
            {
                return;
            }
            app.library.consoles.push(ConsoleState {
                platform,
                folder_name,
                folder_path,
                manufacturer,
                platform_name,
                scan_status: ScanStatus::NotScanned,
                entries: Vec::new(),
                dat_status: DatStatus::NotLoaded,
                fingerprint: None,
            });
            // Sort by manufacturer then platform name then folder name
            app.library.consoles.sort_by(|a, b| {
                a.manufacturer
                    .cmp(b.manufacturer)
                    .then(a.platform_name.cmp(b.platform_name))
                    .then(a.folder_name.cmp(&b.folder_name))
            });
        }

        AppMessage::FolderScanComplete => {
            app.operations
                .retain(|op| op.description != "Scanning folders...");

            log::info!(
                "Folder scan complete: {} consoles discovered",
                app.library.consoles.len()
            );

            // Auto-scan all unscanned consoles if setting is enabled
            if app.settings.general.auto_scan_on_open {
                let unscanned: Vec<usize> = app
                    .library
                    .consoles
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| c.scan_status == ScanStatus::NotScanned)
                    .map(|(i, _)| i)
                    .collect();
                for i in unscanned {
                    crate::backend::scan::quick_scan_console(app, i, ctx);
                }
            }
        }

        AppMessage::ConsoleScanComplete {
            folder_name,
            entries,
        } => {
            if let Some(ci) = app.library.find_by_folder(&folder_name) {
                let console = &mut app.library.consoles[ci];

                // Build a lookup from display_name to existing entry so we can
                // preserve cached data (hashes, status, dat_match, etc.) across
                // re-scans instead of starting from scratch.
                let existing: HashMap<String, LibraryEntry> = console
                    .entries
                    .drain(..)
                    .map(|e| (e.game_entry.display_name().to_owned(), e))
                    .collect();

                console.entries = entries
                    .into_iter()
                    .map(|ge| {
                        if let Some(cached) = existing.get(ge.display_name()) {
                            // File still exists — keep cached analysis data
                            LibraryEntry {
                                game_entry: ge,
                                identification: cached.identification.clone(),
                                hashes: cached.hashes.clone(),
                                dat_match: cached.dat_match.clone(),
                                status: cached.status,
                                ambiguous_candidates: cached.ambiguous_candidates.clone(),
                                media_paths: cached.media_paths.clone(),
                                region_override: cached.region_override,
                                cover_title: cached.cover_title.clone(),
                                screen_title: cached.screen_title.clone(),
                            }
                        } else {
                            // New file — start fresh
                            LibraryEntry {
                                game_entry: ge,
                                identification: None,
                                hashes: None,
                                dat_match: None,
                                status: EntryStatus::Unknown,
                                ambiguous_candidates: Vec::new(),
                                media_paths: None,
                                region_override: None,
                                cover_title: None,
                                screen_title: None,
                            }
                        }
                    })
                    .collect();
                console.scan_status = ScanStatus::Scanning;
            }
        }

        AppMessage::EntryAnalyzed {
            folder_name,
            index,
            result,
        } => {
            if let Some(ci) = app.library.find_by_folder(&folder_name)
                && let Some(entry) = app.library.consoles[ci].entries.get_mut(index)
            {
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

        AppMessage::ConsoleScanDone { folder_name } => {
            if let Some(ci) = app.library.find_by_folder(&folder_name) {
                let console = &mut app.library.consoles[ci];
                console.scan_status = ScanStatus::Scanned;
                // Cache fingerprint so save_library doesn't need to recompute
                console.fingerprint = Some(crate::cache::compute_fingerprint(&console.folder_path));
            }
            let desc_match = "Scanning ".to_string();
            app.operations.retain(|op| {
                // Match operations like "Scanning Super Nintendo..."
                !(op.description.starts_with(&desc_match))
                    || !app
                        .library
                        .find_by_folder(&folder_name)
                        .map(|ci| {
                            op.description
                                .contains(app.library.consoles[ci].platform_name)
                        })
                        .unwrap_or(false)
            });
            app.save_library_cache();
        }

        AppMessage::DatLoaded {
            folder_name,
            platform,
            index,
        } => {
            let game_count = index.game_count();

            // Run serial matching for this specific console's entries
            if let Some(ci) = app.library.find_by_folder(&folder_name) {
                app.library.consoles[ci].dat_status = DatStatus::Loaded { game_count };

                let context = app.context.clone();
                if let Some(registered) = context.get_by_platform(platform) {
                    for entry in app.library.consoles[ci].entries.iter_mut() {
                        if let Some(ref id) = entry.identification
                            && let Some(ref serial) = id.serial_number
                        {
                            let game_code = registered.analyzer.extract_dat_game_code(serial);
                            match index.match_by_serial(serial, game_code.as_deref()) {
                                SerialLookupResult::Match(m) => {
                                    let game_name = index.games[m.game_index].name.clone();
                                    let rom_name =
                                        index.games[m.game_index].roms[m.rom_index].name.clone();
                                    entry.dat_match = Some(DatMatchInfo {
                                        game_name,
                                        rom_name,
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

                // Re-check hash matches for entries that have cached hashes
                // but weren't resolved by serial alone (e.g. Ambiguous or Unrecognized)
                for entry in app.library.consoles[ci].entries.iter_mut() {
                    if entry.status != EntryStatus::Matched
                        && let Some(ref hashes) = entry.hashes
                        && let Some(m) = index.match_by_hash(hashes.data_size, hashes)
                    {
                        let game_name = index.games[m.game_index].name.clone();
                        let rom_name = index.games[m.game_index].roms[m.rom_index].name.clone();
                        entry.dat_match = Some(DatMatchInfo {
                            game_name,
                            rom_name,
                            method: m.method,
                        });
                        entry.status = EntryStatus::Matched;
                        entry.ambiguous_candidates.clear();
                    }
                }

                // Enrich entries that have hashes with catalog titles
                if let Some(ref conn) = app.catalog_db {
                    for entry in app.library.consoles[ci].entries.iter_mut() {
                        if entry.hashes.is_some() {
                            try_catalog_enrich(entry, conn);
                        }
                    }
                }
            }

            // Store the DatIndex for later hash matching
            app.dat_indices.insert(folder_name.clone(), Arc::new(index));

            app.operations
                .retain(|op| !op.description.contains("Loading DAT"));
            app.save_library_cache();
        }

        AppMessage::DatLoadFailed { folder_name, error } => {
            if let Some(ci) = app.library.find_by_folder(&folder_name) {
                app.library.consoles[ci].dat_status = DatStatus::Unavailable { reason: error };
            }
            app.operations
                .retain(|op| !op.description.contains("Loading DAT"));
        }

        AppMessage::HashComplete {
            folder_name,
            index,
            hashes,
        } => {
            if let Some(ci) = app.library.find_by_folder(&folder_name)
                && let Some(entry) = app.library.consoles[ci].entries.get_mut(index)
            {
                // Try hash matching against the loaded DAT
                if let Some(dat_index) = app.dat_indices.get(&folder_name)
                    && let Some(m) = dat_index.match_by_hash(hashes.data_size, &hashes)
                {
                    let game_name = dat_index.games[m.game_index].name.clone();
                    let rom_name = dat_index.games[m.game_index].roms[m.rom_index].name.clone();
                    entry.dat_match = Some(DatMatchInfo {
                        game_name,
                        rom_name,
                        method: m.method,
                    });
                    entry.status = EntryStatus::Matched;
                    entry.ambiguous_candidates.clear();
                }
                entry.hashes = Some(hashes);

                // Enrich with catalog titles
                if let Some(ref conn) = app.catalog_db {
                    try_catalog_enrich(entry, conn);
                }
            }
            app.save_library_cache();
        }

        AppMessage::HashFailed {
            folder_name,
            index,
            error,
        } => {
            log::warn!("Hash failed for {} entry {}: {}", folder_name, index, error);
        }

        AppMessage::MediaLoaded {
            folder_name,
            entry_index,
            media,
        } => {
            if let Some(ci) = app.library.find_by_folder(&folder_name)
                && let Some(entry) = app.library.consoles[ci].entries.get_mut(entry_index)
            {
                // Invalidate stale cached textures when a media path changes
                if let Some(ref old_media) = entry.media_paths {
                    for (mt, old_path) in old_media {
                        let new_path = media.get(mt);
                        if new_path != Some(old_path) {
                            let old_uri = format!("bytes://media/{}", old_path.display());
                            ctx.forget_image(&old_uri);
                        }
                    }
                }
                entry.media_paths = Some(media);
            }
        }

        AppMessage::ScrapeEntryFailed {
            folder_name,
            entry_index,
            error,
        } => {
            log::warn!(
                "Scrape failed for {} entry {}: {}",
                folder_name,
                entry_index,
                error
            );
            if let Some(ci) = app.library.find_by_folder(&folder_name)
                && let Some(entry) = app.library.consoles[ci].entries.get_mut(entry_index)
            {
                entry.media_paths = Some(HashMap::new());
            }
        }

        AppMessage::ScrapeFatalError { message, op_id } => {
            log::error!("Scrape fatal error: {}", message);
            app.operations.retain(|op| op.id != op_id);
        }

        AppMessage::CacheLoaded { library } => {
            // Merge cached consoles with any already discovered from folder scan.
            // Consoles that have already started scanning are not replaced.
            for cached_console in library.consoles {
                if let Some(ci) = app.library.find_by_folder(&cached_console.folder_name) {
                    if app.library.consoles[ci].scan_status == ScanStatus::NotScanned {
                        app.library.consoles[ci] = cached_console;
                    }
                } else {
                    app.library.consoles.push(cached_console);
                }
            }
            app.library.consoles.sort_by(|a, b| {
                a.manufacturer
                    .cmp(b.manufacturer)
                    .then(a.platform_name.cmp(b.platform_name))
                    .then(a.folder_name.cmp(&b.folder_name))
            });

            // Enrich cached entries that have hashes but no catalog titles yet
            if let Some(ref conn) = app.catalog_db {
                for console in &mut app.library.consoles {
                    for entry in &mut console.entries {
                        if entry.hashes.is_some() {
                            try_catalog_enrich(entry, conn);
                        }
                    }
                }
            }

            // Trigger DAT loads for consoles that previously had DATs loaded
            for console in &app.library.consoles {
                if matches!(console.dat_status, DatStatus::Loaded { .. }) {
                    crate::backend::dat::load_dat_for_console(
                        app.message_tx.clone(),
                        app.context.clone(),
                        console.platform,
                        console.folder_name.clone(),
                        ctx.clone(),
                    );
                }
            }
        }

        AppMessage::StartFolderScan => {
            if let Some(ref root) = app.root_path.clone() {
                crate::backend::scan::scan_root_folder(app, root.clone(), ctx);
            }
        }

        AppMessage::ExportComplete {
            folder_name,
            result,
        } => match result {
            Ok(path) => {
                log::info!("Exported gamelist.xml for {}: {}", folder_name, path);
            }
            Err(error) => {
                log::warn!("Export failed for {}: {}", folder_name, error);
            }
        },

        AppMessage::RenameComplete {
            folder_name,
            results,
        } => {
            let mut renamed = 0usize;
            let mut already = 0usize;
            let mut failed = 0usize;

            if let Some(ci) = app.library.find_by_folder(&folder_name) {
                for r in &results {
                    match &r.outcome {
                        RenameOutcome::Renamed { target, .. } => {
                            renamed += 1;
                            // Update the GameEntry path to reflect the new filename
                            if let Some(entry) =
                                app.library.consoles[ci].entries.get_mut(r.entry_index)
                            {
                                entry.game_entry =
                                    retro_junk_lib::scanner::GameEntry::SingleFile(target.clone());
                            }
                        }
                        RenameOutcome::M3uRenamed {
                            target_folder,
                            discs_renamed,
                            errors: m3u_errors,
                            ..
                        } => {
                            renamed += discs_renamed;
                            failed += m3u_errors.len();
                            // Update the MultiDisc GameEntry to reflect the new folder
                            if let Some(entry) =
                                app.library.consoles[ci].entries.get_mut(r.entry_index)
                            {
                                if let retro_junk_lib::scanner::GameEntry::MultiDisc {
                                    ref mut name,
                                    ref mut files,
                                } = entry.game_entry
                                {
                                    // Update folder name from the target folder
                                    if let Some(folder_stem) =
                                        target_folder.file_name().and_then(|n| n.to_str())
                                    {
                                        *name = folder_stem.to_string();
                                    }
                                    // Re-enumerate disc files in the new folder
                                    if let Ok(entries) = std::fs::read_dir(target_folder) {
                                        let mut new_files: Vec<PathBuf> = entries
                                            .flatten()
                                            .map(|e| e.path())
                                            .filter(|p| {
                                                p.is_file()
                                                    && p.extension()
                                                        .and_then(|e| e.to_str())
                                                        .map(|e| !e.eq_ignore_ascii_case("m3u"))
                                                        .unwrap_or(true)
                                            })
                                            .collect();
                                        new_files.sort();
                                        *files = new_files;
                                    }
                                }
                            }
                        }
                        RenameOutcome::AlreadyCorrect => already += 1,
                        RenameOutcome::NoMatch { .. } | RenameOutcome::Error { .. } => {
                            failed += 1;
                        }
                    }
                }
            }
            log::info!(
                "Rename {}: {} renamed, {} already correct, {} failed",
                folder_name,
                renamed,
                already,
                failed
            );
            app.rename_results = Some(results);
            app.save_library_cache();
        }

        AppMessage::OperationProgress {
            op_id,
            current,
            total,
        } => {
            if let Some(op) = app.operations.iter_mut().find(|op| op.id == op_id) {
                op.progress_current = current;
                op.progress_total = total;
            }
        }

        AppMessage::OperationComplete { op_id } => {
            app.operations.retain(|op| op.id != op_id);
        }
    }
}
