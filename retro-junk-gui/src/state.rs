#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use retro_junk_catalog::CatalogTag;
use retro_junk_dat::{DatIndex, FileHashes, MatchMethod, SerialLookupResult};
use retro_junk_frontend::AssetType;
use retro_junk_lib::rename::BrokenReference;

// -- Asset status --

/// The 5 scrapeable asset types that `rescrape_media_for_selection` downloads
/// (matches `AssetSelection::default()` minus Video, which `collect_existing_assets` skips).
pub const SCRAPEABLE_ASSET_TYPES: &[AssetType] = &[
    AssetType::Cover,
    AssetType::Cover3D,
    AssetType::Screenshot,
    AssetType::Marquee,
    AssetType::PhysicalMedia,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetStatus {
    /// `asset_paths` is None — not yet discovered
    Unknown,
    /// Discovered, no scrapeable assets found
    None,
    /// Some but not all scrapeable asset types present
    Partial { found: u8, total: u8 },
    /// All scrapeable asset types present
    Complete,
}
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

/// Which panel currently has keyboard focus for arrow-key navigation.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum FocusedPanel {
    #[default]
    GameTable,
    ConsoleTree,
}

// -- In-memory library state --

#[derive(Default)]
/// Bounded browser read model. Console rows are lightweight shells and only
/// the active console's current page may contain entries.
pub struct LibraryBrowserState {
    pub consoles: Vec<ConsoleState>,
    pub root_id: Option<retro_junk_db::LibraryRootId>,
    pub active_page: Option<retro_junk_db::LibraryEntryListPage>,
    pub entry_counts: HashMap<retro_junk_db::LibraryConsoleId, u64>,
}

impl LibraryBrowserState {
    /// Find a console by `folder_name`. Returns the index.
    pub fn find_by_folder(&self, folder_name: &str) -> Option<usize> {
        self.consoles
            .iter()
            .position(|c| c.folder_name == folder_name)
    }

    pub fn find_by_id(&self, id: retro_junk_db::LibraryConsoleId) -> Option<usize> {
        self.consoles
            .iter()
            .position(|console| console.id == Some(id))
    }

    pub fn evict_inactive_entries(&mut self, active: Option<retro_junk_db::LibraryConsoleId>) {
        for console in &mut self.consoles {
            if console.id != active {
                console.entries.clear();
            }
        }
    }

    pub fn entry_count(&self, console: &ConsoleState) -> usize {
        console
            .id
            .and_then(|id| self.entry_counts.get(&id).copied())
            .unwrap_or(console.entries.len() as u64) as usize
    }
}

pub struct ConsoleState {
    /// Durable database identity; absent only before the first reconciliation.
    #[allow(dead_code)]
    pub id: Option<retro_junk_db::LibraryConsoleId>,
    #[allow(dead_code)]
    pub revision: u64,
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
    /// Loose disc entry-point files at the top level (not inside .m3u folders).
    /// Populated during scan for disc-based consoles. Non-empty means the user
    /// may want to run the Organize command.
    pub loose_disc_files: Vec<PathBuf>,
}

impl ConsoleState {
    pub fn entry_index(&self, id: retro_junk_db::LibraryEntryId) -> Option<usize> {
        self.entries.iter().position(|entry| entry.id == Some(id))
    }

    pub fn entry_by_id(&self, id: retro_junk_db::LibraryEntryId) -> Option<&LibraryEntry> {
        self.entry_index(id)
            .and_then(|index| self.entries.get(index))
    }

    /// Compatibility lookup for background messages not yet converted to IDs.
    pub fn find_entry_mut(&mut self, name: &str) -> Option<&mut LibraryEntry> {
        self.entries
            .iter_mut()
            .find(|e| e.game_entry.display_name() == name)
    }

    /// Look up an entry by one of its constituent files.
    ///
    /// Unlike [`Self::find_entry_mut`] (keyed on the display name, which can
    /// go stale across a rename or CHD compression within the same batch),
    /// this matches by path — the entry still holds its pre-compression path
    /// even after the file is deleted, so matching a job's `input` against
    /// `all_files()` is exact and rename-proof for the batch that just ran.
    pub fn find_entry_by_file_mut(&mut self, file: &Path) -> Option<&mut LibraryEntry> {
        self.entries
            .iter_mut()
            .find(|e| e.game_entry.all_files().iter().any(|f| f == file))
    }
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
pub fn asset_dir_for_console(
    root_path: &Path,
    folder_name: &str,
    setting: &str,
) -> Option<PathBuf> {
    if setting.is_empty() {
        // Legacy default: {root}-media sibling
        let parent = root_path.parent()?;
        let root_name = root_path.file_name()?.to_str()?;
        Some(parent.join(format!("{root_name}-media")).join(folder_name))
    } else {
        Some(resolve_dir(root_path, setting).join(folder_name))
    }
}

/// Derive the metadata directory for a console from the root path, folder name, and user setting.
///
/// The setting is treated as a path (absolute or relative to `root_path`).
/// Default setting `"."` places metadata inline with ROMs (ES-DE legacy mode).
pub fn metadata_dir_for_console(root_path: &Path, folder_name: &str, setting: &str) -> PathBuf {
    resolve_dir(root_path, setting).join(folder_name)
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
fn asset_subdir(mt: AssetType) -> &'static str {
    match mt {
        AssetType::Cover => "covers",
        AssetType::Cover3D => "3dboxes",
        AssetType::Screenshot => "screenshots",
        AssetType::TitleScreen => "titlescreens",
        AssetType::Marquee => "marquees",
        AssetType::Video => "videos",
        AssetType::Fanart => "fanart",
        AssetType::PhysicalMedia => "physicalmedia",
        AssetType::Miximage => "miximages",
    }
}

/// All displayable media types in preferred display order.
pub const DISPLAY_ASSET_TYPES: &[AssetType] = &[
    AssetType::Cover,
    AssetType::Cover3D,
    AssetType::Screenshot,
    AssetType::TitleScreen,
    AssetType::Marquee,
    AssetType::PhysicalMedia,
    AssetType::Fanart,
    AssetType::Miximage,
];

/// Discover media files on disk for a given ROM entry.
///
/// Checks each media type subdirectory for a file matching `rom_stem.ext`.
pub fn collect_existing_assets(media_dir: &Path, rom_stem: &str) -> HashMap<AssetType, PathBuf> {
    let mut found = HashMap::new();
    for &mt in DISPLAY_ASSET_TYPES {
        if mt == AssetType::Video {
            continue;
        }
        let subdir = media_dir.join(asset_subdir(mt));
        // Check all plausible extensions — ScreenScraper may return JPG instead of PNG.
        for ext in mt.discovery_extensions() {
            let path = subdir.join(format!("{rom_stem}.{ext}"));
            if path.exists() {
                found.insert(mt, path);
                break;
            }
        }
    }
    found
}

/// Per-disc identification data for multi-disc entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscIdentification {
    pub path: PathBuf,
    pub identification: RomIdentification,
    #[serde(default)]
    pub hashes: Option<FileHashes>,
    #[serde(default)]
    pub dat_match: Option<DatMatchInfo>,
}

pub struct LibraryEntry {
    /// Durable database identity; absent only for not-yet-reconciled scan rows.
    pub id: Option<retro_junk_db::LibraryEntryId>,
    pub revision: u64,
    pub source_revision: u64,
    pub game_entry: GameEntry,
    pub identification: Option<RomIdentification>,
    pub hashes: Option<FileHashes>,
    pub dat_match: Option<DatMatchInfo>,
    pub status: EntryStatus,
    /// When status is Ambiguous, holds the candidate game names from the DAT.
    pub ambiguous_candidates: Vec<String>,
    /// Discovered media files on disk. `None` = not yet scanned, `Some(empty)` = scanned but none found.
    pub asset_paths: Option<HashMap<AssetType, PathBuf>>,
    /// User-set region override. When set, takes precedence over detected regions.
    pub region_override: Option<Region>,
    /// Box/cover title from catalog DB (e.g., the title printed on the game box).
    /// Empty = absent.
    pub cover_title: String,
    /// Screen title from catalog DB (e.g., the title shown on the title screen).
    /// Empty = absent.
    pub screen_title: String,
    /// Per-disc identification data for multi-disc entries. `None` for single-file entries.
    pub disc_identifications: Option<Vec<DiscIdentification>>,
    /// Broken CUE/M3U references. `None` = not yet checked, `Some(empty)` = checked and clean.
    pub broken_references: Option<Vec<BrokenReference>>,
    /// CUE sheet compatibility issues. `None` = not yet checked, `Some(empty)` = checked and clean.
    pub cue_compat_issues: Option<Vec<CueCompatIssue>>,
    /// User-applied tag (homebrew or modded).
    pub tag: Option<CatalogTag>,
}

/// A CUE sheet compatibility issue detected during scan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CueCompatIssue {
    pub file_name: String,
    pub summary: String,
    pub can_auto_fix: bool,
}

impl LibraryEntry {
    /// Returns the effective status, accounting for user-applied tags.
    ///
    /// When a tag is set, the entry always shows as `Tagged` regardless
    /// of DAT matching status.
    pub fn effective_status(&self) -> EntryStatus {
        match self.tag {
            Some(tag) => EntryStatus::Tagged(tag),
            None => self.status,
        }
    }

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

    /// Compute the media status by checking `asset_paths` against the scrapeable types.
    pub fn asset_status(&self) -> AssetStatus {
        let Some(media) = self.asset_paths.as_ref() else {
            return AssetStatus::Unknown;
        };
        let total = SCRAPEABLE_ASSET_TYPES.len() as u8;
        let found = SCRAPEABLE_ASSET_TYPES
            .iter()
            .filter(|mt| media.contains_key(mt))
            .count() as u8;
        match found {
            0 => AssetStatus::None,
            n if n == total => AssetStatus::Complete,
            n => AssetStatus::Partial { found: n, total },
        }
    }

    /// Whether this entry has detected CUE sheet compatibility issues.
    pub fn has_cue_compat_issues(&self) -> bool {
        self.cue_compat_issues
            .as_ref()
            .is_some_and(|issues| !issues.is_empty())
    }

    /// Whether this entry has broken CUE/M3U file references.
    pub fn has_broken_refs(&self) -> bool {
        self.broken_references
            .as_ref()
            .is_some_and(|refs| !refs.is_empty())
    }

    /// Whether this entry has a generated miximage on disk.
    pub fn has_miximage(&self) -> bool {
        self.asset_paths
            .as_ref()
            .is_some_and(|m| m.contains_key(&AssetType::Miximage))
    }
}

/// Deserialize a JSON `null` (or missing field) as the type's default.
///
/// Cached `DiscIdentification` JSON written before the empty-string
/// convention serializes absent fields as `null`; this keeps those caches
/// loadable. (Mirrors the private helper in `retro-junk-core`.)
fn null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + serde::Deserialize<'de>,
{
    use serde::Deserialize;
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatMatchInfo {
    pub game_name: String,
    /// Individual ROM filename from the DAT (e.g., "Game Name (USA).chd").
    #[serde(default)]
    pub rom_name: String,
    pub method: MatchMethod,
    /// Region string from the DAT entry (e.g., "USA", "Japan"). Empty = unknown.
    #[serde(default, deserialize_with = "null_default")]
    pub region: String,
    /// True when the DAT match's region differs from the file's detected region.
    #[serde(default)]
    pub cross_region: bool,
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
    /// User-tagged as homebrew or modded
    Tagged(CatalogTag),
}

impl EntryStatus {
    pub fn color(self) -> egui::Color32 {
        match self {
            EntryStatus::Unknown => egui::Color32::GRAY,
            EntryStatus::Unrecognized => crate::theme::STATUS_ERR,
            EntryStatus::Ambiguous => crate::theme::STATUS_WARN,
            EntryStatus::Matched => crate::theme::STATUS_OK,
            EntryStatus::Tagged(_) => egui::Color32::from_rgb(100, 150, 220),
        }
    }

    /// Human-readable tooltip explaining this status.
    pub fn tooltip(self) -> &'static str {
        match self {
            EntryStatus::Unknown => "Not yet analyzed",
            EntryStatus::Unrecognized => "Not recognized \u{2013} no serial or hash match found",
            EntryStatus::Ambiguous => "Possible match \u{2013} hash verification needed to confirm",
            EntryStatus::Matched => "Verified match in database",
            EntryStatus::Tagged(CatalogTag::Homebrew) => "Homebrew game",
            EntryStatus::Tagged(CatalogTag::Modded) => "Modded ROM",
        }
    }

    /// Severity ranking (higher = worse). Used to find the worst status in a folder.
    pub fn severity(self) -> u8 {
        match self {
            EntryStatus::Matched | EntryStatus::Tagged(_) => 0,
            EntryStatus::Ambiguous => 1,
            EntryStatus::Unknown => 2,
            EntryStatus::Unrecognized => 3,
        }
    }
}

// -- Fragile mount prompt --

/// A pending library-root switch awaiting user confirmation because the path
/// lives on a fragile userspace network mount (GVFS/KIO-FUSE).
pub struct FragileMountPrompt {
    /// The root the user asked to open.
    pub root: std::path::PathBuf,
    /// Short label for the mount kind ("GVFS", "KIO-FUSE").
    pub kind: &'static str,
}

// -- Tag dialog --

/// State for the homebrew/modded tagging dialogs.
#[derive(Default)]
pub enum TagDialog {
    #[default]
    None,
    Homebrew {
        name: String,
        console_id: retro_junk_db::LibraryConsoleId,
        entry_id: retro_junk_db::LibraryEntryId,
    },
    ModSearch {
        query: String,
        results: Vec<retro_junk_db::WorkRow>,
        selected: Option<usize>,
        console_id: retro_junk_db::LibraryConsoleId,
        entry_id: retro_junk_db::LibraryEntryId,
    },
}

// -- Rename results --

pub struct RenameResult {
    pub entry_name: String,
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

// -- CUE fix results --

pub struct CueFixResult {
    pub file_name: String,
    pub outcome: CueFixOutcome,
}

pub enum CueFixOutcome {
    Fixed { summary: String },
    AlreadyStandard,
    Unfixable { reason: String },
    Error { message: String },
}

// -- CHD compression --

/// One disc queued in the CHD-compression confirmation dialog.
pub struct ChdCompressItem {
    pub job: retro_junk_lib::chd_convert::CompressionJob,
    /// Precomputed at plan time (D8): "{input} ({size}) -> {output}". Avoids
    /// re-deriving file-name strings and byte formatting every frame the
    /// confirmation dialog is open.
    pub display_line: String,
}

/// A selected disc that cannot be compressed, with the reason shown to the user.
pub struct ChdCompressSkip {
    pub entry_name: String,
    pub reason: String,
}

/// State backing the "Compress to CHD" confirmation dialog.
///
/// Also carries the chdman probe result: when chdman is missing the dialog
/// becomes the explanation of why compression is unavailable and how to
/// install it, instead of silently hiding the feature.
pub struct ChdCompressPrompt {
    pub folder_name: String,
    pub items: Vec<ChdCompressItem>,
    pub skipped: Vec<ChdCompressSkip>,
    pub chdman:
        Result<retro_junk_lib::chd_convert::Chdman, retro_junk_lib::chd_convert::ChdmanUnavailable>,
    /// Delete original files after a verified compression.
    pub delete_sources: bool,
}

pub struct ChdCompressResult {
    /// The chdman input this result is for (one entry can hold several discs).
    pub input_name: String,
    /// The planned job this result came from — carries `input`/`output`/
    /// `source_files` so the completion handler can look up the affected
    /// library entry by path (rename-proof) instead of by display name, and
    /// know exactly which files were deleted.
    pub job: retro_junk_lib::chd_convert::CompressionJob,
    pub outcome: ChdCompressOutcome,
}

pub enum ChdCompressOutcome {
    /// Compressed and round-trip verified. The output path is available via
    /// the sibling `ChdCompressResult::job.output` — not duplicated here.
    Compressed {
        input_bytes: u64,
        output_bytes: u64,
        tracks: usize,
        sources_deleted: bool,
        delete_failures: Vec<String>,
    },
    /// Round-trip verification failed; the .chd was removed and the original
    /// files were kept.
    VerifyFailed {
        detail: String,
    },
    Error {
        message: String,
    },
    Cancelled,
}

// -- Background operations --

/// What kind of work a `BackgroundOperation` represents. Used by
/// [`crate::app::RetroJunkApp::chd_compress_busy`] to guard against
/// overlapping CHD-compression runs on the same console folder, and can grow
/// further overlap-guards as needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind {
    Scan,
    Hash,
    Rename,
    CueFix,
    ChdCompress,
    Other,
}

/// How a `BackgroundOperation`'s `progress_current/progress_total` pair should
/// be rendered. Replaces two mutually-exclusive bools with a type that makes
/// the "count vs. bytes vs. percent" choice unrepresentable as invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProgressDisplay {
    /// "3/10"
    #[default]
    Count,
    /// "234.5 MB / 4.7 GB"
    Bytes,
    /// "42%" (`progress_current/progress_total` is an abstract unit scale)
    Percent,
}

pub struct BackgroundOperation {
    pub id: u64,
    pub description: String,
    pub progress_current: u64,
    pub progress_total: u64,
    pub cancel_token: Arc<AtomicBool>,
    /// How to render `progress_current/progress_total`.
    pub display: ProgressDisplay,
    /// What kind of work this operation represents.
    pub kind: OperationKind,
    /// Console folder this operation is scoped to (used by the
    /// overlapping-operation guard). Empty = unscoped.
    pub scope: String,
}

impl BackgroundOperation {
    pub fn new(
        id: u64,
        description: String,
        cancel_token: Arc<AtomicBool>,
        kind: OperationKind,
        scope: String,
        display: ProgressDisplay,
    ) -> Self {
        Self {
            id,
            description,
            progress_current: 0,
            progress_total: 0,
            cancel_token,
            display,
            kind,
            scope,
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

/// Describe the format of an M3U folder, e.g. "M3U folder (3x CHD)" or "M3U folder (2x CHD, 1x CUE)".
fn describe_m3u_format(files: &[PathBuf]) -> String {
    let mut ext_counts: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    for f in files {
        let ext = f
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("bin")
            .to_uppercase();
        *ext_counts.entry(ext).or_default() += 1;
    }
    let parts: Vec<String> = ext_counts
        .iter()
        .map(|(ext, count)| format!("{count}x {ext}"))
        .collect();
    format!("M3U folder ({})", parts.join(", "))
}

/// Try to enrich a library entry with cover/screen titles from the catalog DB
/// using disc serial numbers.
///
/// Used as a fallback for multi-disc entries that have per-disc serials but no SHA1.
fn try_catalog_enrich_by_serial(entry: &mut LibraryEntry, conn: &retro_junk_db::Connection) {
    if !entry.cover_title.is_empty() {
        return;
    }
    let discs = match entry.disc_identifications.as_ref() {
        Some(d) if !d.is_empty() => d,
        _ => return,
    };
    for disc in discs {
        let serial = &disc.identification.serial_number;
        if serial.is_empty() {
            continue;
        }
        let Ok(media_list) = retro_junk_db::find_media_by_serial(conn, serial) else {
            continue;
        };
        let release_id = match media_list.first() {
            Some(m) => &m.release_id,
            None => continue,
        };
        if let Ok(Some(release)) = retro_junk_db::get_release_by_id(conn, release_id) {
            entry.cover_title = release.cover_title;
            entry.screen_title = release.screen_title;
            return;
        }
    }
}

/// Try to resolve a canonical game name for a multi-disc set from the catalog DB.
///
/// Lookup priority per disc:
/// 1. Serial → `find_media_by_serial()` (always available for PS1)
/// 2. SHA1 → `find_media_by_sha1()` (available after hashing)
///
/// If all matched discs share the same work, returns `"{canonical_name} ({region_display})"`.
/// Returns `None` if no discs found or discs belong to different works.
fn try_catalog_game_name(
    discs: &[DiscIdentification],
    conn: &retro_junk_db::Connection,
) -> Option<String> {
    let mut work_id: Option<String> = None;
    let mut release_id_for_region: Option<String> = None;

    for disc in discs {
        // Try serial first, then SHA1
        let serial = &disc.identification.serial_number;
        let media = (!serial.is_empty())
            .then(|| retro_junk_db::find_media_by_serial(conn, serial).ok())
            .flatten()
            .and_then(|v| if v.is_empty() { None } else { Some(v) })
            .or_else(|| {
                disc.hashes
                    .as_ref()
                    .and_then(|h| h.sha1.as_deref())
                    .and_then(|s| retro_junk_db::find_media_by_sha1(conn, s).ok())
                    .and_then(|v| if v.is_empty() { None } else { Some(v) })
            });

        let Some(media_list) = media else {
            continue;
        };

        let first = &media_list[0];
        let release = retro_junk_db::get_release_by_id(conn, &first.release_id).ok()??;

        match &work_id {
            None => {
                work_id = Some(release.work_id.clone());
                release_id_for_region = Some(release.id.clone());
            }
            Some(existing) if *existing == release.work_id => {
                // Same work — consistent
            }
            Some(_) => {
                // Different works — can't determine a single game name
                return None;
            }
        }
    }

    let wid = work_id?;
    let rid = release_id_for_region?;
    let work = retro_junk_db::get_work_by_id(conn, &wid).ok()??;
    let release = retro_junk_db::get_release_by_id(conn, &rid).ok()??;

    let region_display = retro_junk_catalog::region_slug_to_display(&release.region);
    Some(format!("{} ({})", work.canonical_name, region_display))
}

/// Try to enrich a library entry with cover/screen titles from the catalog DB.
///
/// Skips if the entry already has a `cover_title` or has no SHA1 hash.
/// `SQLite` indexed lookups are sub-millisecond, safe for the main thread.
fn try_catalog_enrich(entry: &mut LibraryEntry, conn: &retro_junk_db::Connection) {
    if !entry.cover_title.is_empty() {
        return;
    }
    let Some(sha1) = entry.hashes.as_ref().and_then(|h| h.sha1.as_deref()) else {
        return;
    };
    let Ok(media_list) = retro_junk_db::find_media_by_sha1(conn, sha1) else {
        return;
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

/// An error that should be shown to the user in a modal dialog.
#[derive(Debug, Clone)]
pub struct UserError {
    pub category: String,
    pub message: String,
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
        loose_disc_files: Vec<PathBuf>,
    },
    EntryAnalyzed {
        folder_name: String,
        entry_name: String,
        result: Result<RomIdentification, AnalysisError>,
    },
    MultiDiscAnalyzed {
        folder_name: String,
        entry_name: String,
        disc_results: Vec<(PathBuf, Result<RomIdentification, AnalysisError>)>,
    },
    BrokenRefsChecked {
        folder_name: String,
        entry_name: String,
        broken_refs: Vec<BrokenReference>,
    },
    CueCompatChecked {
        folder_name: String,
        entry_name: String,
        issues: Vec<CueCompatIssue>,
    },
    ConsoleScanDone {
        folder_name: String,
        fingerprint: crate::cache::FolderFingerprint,
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
        entry_name: String,
        hashes: FileHashes,
    },
    DiscHashComplete {
        folder_name: String,
        entry_name: String,
        disc_path: PathBuf,
        hashes: FileHashes,
    },
    HashFailed {
        folder_name: String,
        entry_name: String,
        error: String,
    },

    // -- Media / Scraping --
    AssetsLoaded {
        folder_name: String,
        entry_name: String,
        assets: HashMap<AssetType, PathBuf>,
    },
    ScrapeEntryFailed {
        folder_name: String,
        entry_name: String,
        error: String,
    },
    ScrapeFatalError {
        message: String,
        op_id: u64,
    },

    // -- Library discovery --
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

    // -- Organize --
    OrganizePlanReady {
        folder_name: String,
        plan: retro_junk_lib::organize::OrganizePlan,
    },
    OrganizeComplete {
        folder_name: String,
        jobs_executed: usize,
        files_moved: usize,
        unmatched: usize,
        errors: Vec<String>,
    },

    // -- CUE fix --
    CueFixComplete {
        folder_name: String,
        results: Vec<CueFixResult>,
    },

    // -- CHD compression --
    /// Background planning (D1) finished: chdman probed + `plan_batch` run for
    /// every selected entry, off the UI thread. Stores the prompt so the
    /// dialog appears once everything is ready.
    ChdCompressPromptReady {
        prompt: ChdCompressPrompt,
    },
    ChdCompressComplete {
        folder_name: String,
        results: Vec<ChdCompressResult>,
    },

    // -- Settings: chdman probe (D1) --
    /// Result of probing the configured chdman path on a background thread.
    /// `key` is the probed setting string (trimmed), matched against the
    /// current setting when applied so a stale in-flight probe from a path
    /// the user has since changed doesn't clobber a fresher result.
    ChdmanProbeResult {
        key: String,
        result: Result<
            retro_junk_lib::chd_convert::Chdman,
            retro_junk_lib::chd_convert::ChdmanUnavailable,
        >,
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

    // -- Catalog data operations --
    /// A catalog data-gathering operation (import/enrich) mutated the DB.
    /// Triggers a refresh of the Dashboard stats and Browse tables and clears
    /// the Data-tab in-flight guard.
    CatalogDataChanged,
    /// Refreshed DAT/GDB cache listings loaded on a background thread.
    CacheListsLoaded {
        dat: Vec<retro_junk_dat::cache::CacheEntry>,
        gdb: Vec<retro_junk_dat::gdb_cache::GdbCacheEntry>,
    },
}

// -- Helpers --

/// Create the activity-bar batch operation that tracks overall auto-scan
/// progress, then kick off the first queued scan.
pub fn start_auto_scan_batch(app: &mut crate::app::RetroJunkApp, ctx: &egui::Context) {
    if app.ui_state.pending_auto_scans.is_empty() {
        return;
    }
    let total = app.ui_state.pending_auto_scans.len() as u64;
    let op_id = next_operation_id();
    let cancel = Arc::new(AtomicBool::new(false));
    let mut op = BackgroundOperation::new(
        op_id,
        "Auto-scanning library".to_string(),
        cancel,
        OperationKind::Scan,
        String::new(),
        ProgressDisplay::Count,
    );
    op.progress_total = total;
    app.operations.push(op);
    app.ui_state.auto_scan_op_id = Some(op_id);
    start_next_auto_scan(app, ctx);
}

/// Start the next queued auto-scan, if any. Does nothing if a queued scan is
/// already in flight (so manual user-initiated scans don't accidentally
/// double-advance the queue). If the batch operation was cancelled or the
/// queue is empty, removes the batch op from the activity bar.
fn start_next_auto_scan(app: &mut crate::app::RetroJunkApp, ctx: &egui::Context) {
    if app.ui_state.auto_scan_in_flight.is_some() {
        return;
    }
    // If the user clicked Cancel on the batch op, drop the rest of the queue.
    if let Some(op_id) = app.ui_state.auto_scan_op_id
        && let Some(op) = app.operations.iter().find(|o| o.id == op_id)
        && op.cancel_token.load(std::sync::atomic::Ordering::Relaxed)
    {
        app.ui_state.pending_auto_scans.clear();
    }
    while let Some(folder_name) = app.ui_state.pending_auto_scans.pop_front() {
        if let Some(ci) = app.browser.find_by_folder(&folder_name)
            && app.browser.consoles[ci].scan_status == ScanStatus::NotScanned
        {
            app.ui_state.auto_scan_in_flight = Some(folder_name);
            crate::backend::scan::quick_scan_console(app, ci, ctx);
            return;
        }
    }
    // Queue drained — remove the batch op from the activity bar.
    if let Some(op_id) = app.ui_state.auto_scan_op_id.take() {
        app.operations.retain(|o| o.id != op_id);
    }
}

/// Re-check broken references and CUE compat for invalidated entries in a console.
///
/// Collects entries whose `broken_references` is `None` (i.e., were just
/// invalidated) and spawns `check_broken_refs_background` to re-scan them.
fn recheck_invalidated_entries(
    app: &crate::app::RetroJunkApp,
    console_idx: usize,
    folder_name: &str,
    ctx: &egui::Context,
) {
    let unchecked: Vec<_> = app.browser.consoles[console_idx]
        .entries
        .iter()
        .filter(|e| e.broken_references.is_none())
        .map(|e| {
            (
                folder_name.to_string(),
                e.game_entry.display_name().to_string(),
                e.game_entry.clone(),
            )
        })
        .collect();
    if !unchecked.is_empty() {
        crate::backend::scan::check_broken_refs_background(
            app.message_tx.clone(),
            unchecked,
            ctx.clone(),
        );
    }
}

/// Re-enumerate a `MultiDisc` entry's folder after its files changed on disk
/// (rename, CHD compression): refreshes the entry's `files` list and remaps
/// `disc_identifications` paths to the new files (matching by filename stem,
/// then by extension). No-op for single-file entries.
fn refresh_multidisc_files(
    entry: &mut LibraryEntry,
    folder: &std::path::Path,
    extensions: &std::collections::HashSet<String>,
) {
    let retro_junk_lib::scanner::GameEntry::MultiDisc { ref mut files, .. } = entry.game_entry
    else {
        return;
    };

    // Reuse the scanner's playlist-driven collection (D5): reads the .m3u
    // (rewritten by B5 to point at the new files before this runs) and falls
    // back to extension-filtered + CUE-deduped scanning only if no playlist
    // is found. This avoids counting `.sbi`/other non-disc companions as
    // "discs" and — because the playlist already names the post-compression
    // `.chd`s — avoids picking up leftover `.bin`s from a failed delete.
    let new_files = retro_junk_lib::scanner::collect_m3u_disc_files(folder, extensions);
    if new_files.is_empty() {
        // A transient read failure (or a genuinely empty folder, which
        // shouldn't happen for a live entry) must not wipe the entry's file
        // list; the next full rescan will reconcile it properly.
        return;
    }

    if let Some(ref mut discs) = entry.disc_identifications {
        // Claim tracking: each new file can be assigned to at most one disc,
        // so two stem-unmatched discs never both collapse onto the same
        // leftover/renamed path.
        let mut claimed: std::collections::HashSet<&PathBuf> = std::collections::HashSet::new();
        for disc in discs.iter_mut() {
            let old_stem = disc.path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if let Some(new_path) = new_files.iter().find(|p| {
                !claimed.contains(p)
                    && p.file_stem().and_then(|s| s.to_str()).unwrap_or("") == old_stem
            }) {
                claimed.insert(new_path);
                disc.path = new_path.clone();
            } else {
                log::warn!(
                    "multi-disc refresh: no new file matches disc {}",
                    disc.path.display()
                );
                // Keep the stale path; the next full rescan reconciles it.
            }
        }
    }

    *files = new_files;
}

/// Check whether detected regions match the DAT entry's region string.
/// Returns `true` if any detected region name appears in the DAT region string.
fn regions_match_dat(detected: &[Region], dat_region: &str) -> bool {
    if detected.is_empty() {
        return true; // No detected regions — can't flag a mismatch
    }
    detected.iter().any(|r| {
        let name = r.name();
        dat_region.contains(name) || name.contains(dat_region)
    })
}

/// Build a `DatMatchInfo` from a single DAT match result, including cross-region detection.
fn build_dat_match_info(
    dat_index: &DatIndex,
    game_index: usize,
    rom_index: usize,
    method: MatchMethod,
    detected_regions: &[Region],
) -> DatMatchInfo {
    let game = &dat_index.games[game_index];
    let region = game.region.clone().unwrap_or_default();
    let cross_region = !region.is_empty() && !regions_match_dat(detected_regions, &region);
    DatMatchInfo {
        game_name: game.name.clone(),
        rom_name: game.roms[rom_index].name.clone(),
        method,
        region,
        cross_region,
    }
}

/// Pick the best match from multiple hash matches, preferring the one whose
/// DAT region matches the file's detected regions.
fn pick_best_hash_match(
    dat_index: &DatIndex,
    matches: &[retro_junk_dat::MatchResult],
    detected_regions: &[Region],
) -> Option<DatMatchInfo> {
    if matches.is_empty() {
        return None;
    }
    // If we have detected regions and multiple matches, prefer region-matching entry
    if !detected_regions.is_empty() && matches.len() > 1 {
        for m in matches {
            let game = &dat_index.games[m.game_index];
            if let Some(ref dat_region) = game.region
                && regions_match_dat(detected_regions, dat_region)
            {
                return Some(build_dat_match_info(
                    dat_index,
                    m.game_index,
                    m.rom_index,
                    m.method.clone(),
                    detected_regions,
                ));
            }
        }
    }
    // Fall back to first match (with cross-region flag if applicable)
    let m = &matches[0];
    Some(build_dat_match_info(
        dat_index,
        m.game_index,
        m.rom_index,
        m.method.clone(),
        detected_regions,
    ))
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
                .browser
                .consoles
                .iter()
                .any(|c| c.folder_name == folder_name)
            {
                return;
            }
            app.browser.consoles.push(ConsoleState {
                id: None,
                revision: 0,
                platform,
                folder_name: folder_name.clone(),
                folder_path: folder_path.clone(),
                manufacturer,
                platform_name,
                scan_status: ScanStatus::NotScanned,
                entries: Vec::new(),
                dat_status: DatStatus::NotLoaded,
                fingerprint: None,
                loose_disc_files: Vec::new(),
            });
            if let Some(root_id) = app.browser.root_id {
                let platform = serde_json::to_string(&platform)
                    .unwrap_or_default()
                    .trim_matches('"')
                    .to_owned();
                app.submit_store(
                    crate::backend::library_store::LibraryStoreRequest::EnsureConsole(
                        retro_junk_db::LibraryConsoleDescriptor {
                            root_id,
                            platform,
                            folder_name: folder_name.clone(),
                            folder_path: folder_path.to_string_lossy().into_owned(),
                        },
                    ),
                    ctx,
                );
            }
            // Sort by manufacturer then platform name then folder name
            app.browser.consoles.sort_by(|a, b| {
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
                app.browser.consoles.len()
            );

            // Auto-scan all unscanned consoles if setting is enabled.
            // Scans run sequentially via the pending_auto_scans queue to avoid
            // stampeding slow filesystems (e.g., network shares).
            if app.settings.general.auto_scan_on_open {
                for c in &app.browser.consoles {
                    if c.scan_status == ScanStatus::NotScanned {
                        app.ui_state
                            .pending_auto_scans
                            .push_back(c.folder_name.clone());
                    }
                }
                start_auto_scan_batch(app, ctx);
            }
        }

        AppMessage::ConsoleScanComplete {
            folder_name,
            entries,
            loose_disc_files,
        } => {
            if let Some(ci) = app.browser.find_by_folder(&folder_name) {
                let console = &mut app.browser.consoles[ci];
                console.loose_disc_files = loose_disc_files;

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
                                id: cached.id,
                                revision: cached.revision,
                                source_revision: cached.source_revision,
                                game_entry: ge,
                                identification: cached.identification.clone(),
                                hashes: cached.hashes.clone(),
                                dat_match: cached.dat_match.clone(),
                                status: cached.status,
                                ambiguous_candidates: cached.ambiguous_candidates.clone(),
                                asset_paths: cached.asset_paths.clone(),
                                region_override: cached.region_override,
                                cover_title: cached.cover_title.clone(),
                                screen_title: cached.screen_title.clone(),
                                disc_identifications: cached.disc_identifications.clone(),
                                broken_references: cached.broken_references.clone(),
                                cue_compat_issues: cached.cue_compat_issues.clone(),
                                tag: cached.tag,
                            }
                        } else {
                            // New file — start fresh
                            LibraryEntry {
                                id: None,
                                revision: 0,
                                source_revision: 0,
                                game_entry: ge,
                                identification: None,
                                hashes: None,
                                dat_match: None,
                                status: EntryStatus::Unknown,
                                ambiguous_candidates: Vec::new(),
                                asset_paths: None,
                                region_override: None,
                                cover_title: String::new(),
                                screen_title: String::new(),
                                disc_identifications: None,
                                broken_references: None,
                                cue_compat_issues: None,
                                tag: None,
                            }
                        }
                    })
                    .collect();
                console.scan_status = ScanStatus::Scanning;
            }
        }

        AppMessage::EntryAnalyzed {
            folder_name,
            entry_name,
            result,
        } => {
            // Pre-fetch DAT context before mutably borrowing library entries.
            // Cloning avoids borrow conflicts when we mutate app.browser below.
            let platform = app
                .browser
                .find_by_folder(&folder_name)
                .map(|ci| app.browser.consoles[ci].platform);
            let dat_index = app.dat_indices.get(&folder_name).cloned();
            let context = app.context.clone();

            if let Some(ci) = app.browser.find_by_folder(&folder_name)
                && let Some(entry) = app.browser.consoles[ci].find_entry_mut(&entry_name)
            {
                let prior_status = entry.status;
                let had_prior_dat_match = entry.dat_match.is_some();

                match result {
                    Ok(id) => {
                        let has_serial = !id.serial_number.is_empty();
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

                // If the DAT is already loaded (e.g. during a rescan), re-run matching
                // immediately so we don't temporarily drop a previously-Matched status.
                if entry.disc_identifications.is_none()
                    && let Some(ref dat) = dat_index
                {
                    // Try serial matching.
                    if let Some(ref id) = entry.identification
                        && !id.serial_number.is_empty()
                        && let Some(p) = platform
                        && let Some(registered) = context.get_by_platform(p)
                    {
                        let serial = &id.serial_number;
                        let game_code = registered.analyzer.extract_dat_game_code(serial);
                        match dat.match_by_serial(serial, game_code.as_deref()) {
                            SerialLookupResult::Match(m) => {
                                let game_name = dat.games[m.game_index].name.clone();
                                let rom_name =
                                    dat.games[m.game_index].roms[m.rom_index].name.clone();
                                entry.dat_match = Some(DatMatchInfo {
                                    game_name,
                                    rom_name,
                                    method: m.method,
                                    region: String::new(),
                                    cross_region: false,
                                });
                                entry.status = EntryStatus::Matched;
                                entry.ambiguous_candidates.clear();
                            }
                            SerialLookupResult::Ambiguous { candidates } => {
                                entry.status = EntryStatus::Ambiguous;
                                entry.ambiguous_candidates = candidates;
                            }
                            SerialLookupResult::NotFound => {}
                        }
                    }

                    // If serial didn't resolve it, try hash matching against
                    // hashes cached from a prior scan.
                    if entry.status != EntryStatus::Matched
                        && let Some(ref hashes) = entry.hashes
                    {
                        let matches = dat.match_by_hash(hashes.data_size, hashes);
                        let detected = entry
                            .identification
                            .as_ref()
                            .map(|id| id.regions.as_slice())
                            .unwrap_or_default();
                        if let Some(dm) = pick_best_hash_match(dat, &matches, detected) {
                            entry.dat_match = Some(dm);
                            entry.status = EntryStatus::Matched;
                            entry.ambiguous_candidates.clear();
                        }
                    }
                }

                // If re-matching couldn't run (DATs not loaded yet) or didn't
                // find anything, but the entry already had a valid DAT match
                // from a prior cycle, restore its old status. The cached
                // dat_match is still valid — only the identification was refreshed.
                if entry.status != EntryStatus::Matched && had_prior_dat_match {
                    entry.status = prior_status;
                }
            }
        }

        AppMessage::MultiDiscAnalyzed {
            folder_name,
            entry_name,
            disc_results,
        } => {
            // Pre-fetch DAT context before mutably borrowing library entries.
            let console_platform = app
                .browser
                .find_by_folder(&folder_name)
                .map(|ci| app.browser.consoles[ci].platform);
            let dat_index = app.dat_indices.get(&folder_name).cloned();
            let context = app.context.clone();

            if let Some(ci) = app.browser.find_by_folder(&folder_name)
                && let Some(entry) = app.browser.consoles[ci].find_entry_mut(&entry_name)
            {
                // Save old per-disc hashes so we can carry them forward.
                // Analysis doesn't produce hashes, so without this a rescan
                // would wipe previously-computed per-disc hashes and DAT matches.
                let old_disc_data: HashMap<PathBuf, (Option<FileHashes>, Option<DatMatchInfo>)> =
                    entry
                        .disc_identifications
                        .as_ref()
                        .map(|discs| {
                            discs
                                .iter()
                                .map(|d| (d.path.clone(), (d.hashes.clone(), d.dat_match.clone())))
                                .collect()
                        })
                        .unwrap_or_default();

                // Collect successful disc identifications, carrying forward cached data.
                for (path, result) in &disc_results {
                    match result {
                        Ok(_) => log::debug!("MultiDiscAnalyzed: disc {} => Ok", path.display()),
                        Err(e) => {
                            log::warn!("MultiDiscAnalyzed: disc {} => Err({})", path.display(), e);
                        }
                    }
                }
                let disc_ids: Vec<DiscIdentification> = disc_results
                    .iter()
                    .filter_map(|(path, result)| {
                        result.as_ref().ok().map(|id| {
                            let (cached_hashes, cached_dat_match) =
                                old_disc_data.get(path).cloned().unwrap_or_default();
                            DiscIdentification {
                                path: path.clone(),
                                identification: id.clone(),
                                hashes: cached_hashes,
                                dat_match: cached_dat_match,
                            }
                        })
                    })
                    .collect();

                // Build game-level identification by merging disc data
                let mut regions = Vec::new();
                let mut has_serial = false;
                for disc in &disc_ids {
                    for r in &disc.identification.regions {
                        if !regions.contains(r) {
                            regions.push(*r);
                        }
                    }
                    if !disc.identification.serial_number.is_empty() {
                        has_serial = true;
                    }
                }

                let disc_files: Vec<PathBuf> =
                    disc_results.iter().map(|(p, _)| p.clone()).collect();
                let format_str = describe_m3u_format(&disc_files);

                let mut game_id = RomIdentification::new();
                game_id.regions = regions;
                game_id.extra.insert("format".to_string(), format_str);
                game_id
                    .extra
                    .insert("disc_count".to_string(), disc_results.len().to_string());

                entry.identification = Some(game_id);
                entry.disc_identifications = Some(disc_ids);

                // Remember whether the entry had a prior DAT match so we can
                // preserve its status if DAT re-matching can't run (e.g. DATs
                // still loading asynchronously after app restart).
                let prior_status = entry.status;
                let had_prior_dat_match = entry.dat_match.is_some();

                entry.status = if has_serial {
                    EntryStatus::Ambiguous
                } else {
                    EntryStatus::Unrecognized
                };

                // If the DAT is already loaded (e.g. during a rescan), re-run
                // multi-disc serial matching immediately to restore Matched status.
                if let Some(ref dat) = dat_index
                    && let Some(p) = console_platform
                    && let Some(registered) = context.get_by_platform(p)
                {
                    let mut matched_names: Vec<String> = Vec::new();
                    let mut first_rom_name = String::new();
                    let mut any_ambiguous = false;
                    let mut all_candidates: Vec<String> = Vec::new();

                    if let Some(ref mut discs) = entry.disc_identifications {
                        for disc in discs.iter_mut() {
                            let serial = &disc.identification.serial_number;
                            if !serial.is_empty() {
                                let game_code = registered.analyzer.extract_dat_game_code(serial);
                                match dat.match_by_serial(serial, game_code.as_deref()) {
                                    SerialLookupResult::Match(m) => {
                                        let name = dat.games[m.game_index].name.clone();
                                        let rom_name =
                                            dat.games[m.game_index].roms[m.rom_index].name.clone();
                                        if first_rom_name.is_empty() {
                                            first_rom_name.clone_from(&rom_name);
                                        }
                                        disc.dat_match = Some(DatMatchInfo {
                                            game_name: name.clone(),
                                            rom_name,
                                            method: MatchMethod::Serial,
                                            region: String::new(),
                                            cross_region: false,
                                        });
                                        matched_names.push(name);
                                    }
                                    SerialLookupResult::Ambiguous { candidates } => {
                                        any_ambiguous = true;
                                        for c in candidates {
                                            if !all_candidates.contains(&c) {
                                                all_candidates.push(c);
                                            }
                                        }
                                    }
                                    SerialLookupResult::NotFound => {}
                                }
                            }
                        }
                    }

                    if !matched_names.is_empty() {
                        let combined = app
                            .catalog_db
                            .as_ref()
                            .and_then(|conn| {
                                entry
                                    .disc_identifications
                                    .as_ref()
                                    .and_then(|discs| try_catalog_game_name(discs, conn))
                            })
                            .unwrap_or_else(|| {
                                let name_refs: Vec<&str> = matched_names
                                    .iter()
                                    .map(std::string::String::as_str)
                                    .collect();
                                retro_junk_core::disc::derive_base_game_name(&name_refs)
                            });
                        entry.dat_match = Some(DatMatchInfo {
                            game_name: combined,
                            rom_name: first_rom_name.clone(),
                            method: MatchMethod::Serial,
                            region: String::new(),
                            cross_region: false,
                        });
                        if any_ambiguous {
                            // Check if "ambiguous" candidates are just different discs of the same game
                            if retro_junk_core::disc::candidates_are_same_game(&all_candidates)
                                .is_some()
                            {
                                entry.status = EntryStatus::Matched;
                                entry.ambiguous_candidates.clear();
                            } else {
                                entry.status = EntryStatus::Ambiguous;
                                entry.ambiguous_candidates = all_candidates;
                            }
                        } else {
                            entry.status = EntryStatus::Matched;
                            entry.ambiguous_candidates.clear();
                        }
                    } else if any_ambiguous {
                        if let Some(base_name) =
                            retro_junk_core::disc::candidates_are_same_game(&all_candidates)
                        {
                            entry.status = EntryStatus::Matched;
                            entry.ambiguous_candidates.clear();
                            entry.dat_match = Some(DatMatchInfo {
                                game_name: base_name,
                                rom_name: first_rom_name,
                                method: MatchMethod::Serial,
                                region: String::new(),
                                cross_region: false,
                            });
                        } else {
                            entry.status = EntryStatus::Ambiguous;
                            entry.ambiguous_candidates = all_candidates;
                        }
                    }
                }

                // If re-matching couldn't run (DATs not loaded yet) or didn't
                // find anything, but the entry already had a valid DAT match
                // from a prior cycle, restore its old status. The cached
                // dat_match is still valid — only the identification was refreshed.
                if entry.status != EntryStatus::Matched && had_prior_dat_match {
                    entry.status = prior_status;
                }
            }
        }

        AppMessage::BrokenRefsChecked {
            folder_name,
            entry_name,
            broken_refs,
        } => {
            if let Some(ci) = app.browser.find_by_folder(&folder_name)
                && let Some(entry) = app.browser.consoles[ci].find_entry_mut(&entry_name)
            {
                entry.broken_references = Some(broken_refs);
            }
        }

        AppMessage::CueCompatChecked {
            folder_name,
            entry_name,
            issues,
        } => {
            if let Some(ci) = app.browser.find_by_folder(&folder_name)
                && let Some(entry) = app.browser.consoles[ci].find_entry_mut(&entry_name)
            {
                entry.cue_compat_issues = Some(issues);
            }
        }

        AppMessage::ConsoleScanDone {
            folder_name,
            fingerprint,
        } => {
            if let Some(ci) = app.browser.find_by_folder(&folder_name) {
                let console = &mut app.browser.consoles[ci];
                console.scan_status = ScanStatus::Scanned;
                // Cache the quick folder fingerprint for stale-state checks.
                // Fingerprint is computed in the scan worker to keep network I/O off the UI thread.
                console.fingerprint = Some(fingerprint);
            }
            let desc_match = "Scanning ".to_string();
            app.operations.retain(|op| {
                // Match operations like "Scanning Super Nintendo..."
                !(op.description.starts_with(&desc_match))
                    || !app.browser.find_by_folder(&folder_name).is_some_and(|ci| {
                        op.description
                            .contains(app.browser.consoles[ci].platform_name)
                    })
            });
            if let Some(ci) = app.browser.find_by_folder(&folder_name) {
                app.save_console_cache(ci, ctx);
            }

            // Discover media for newly scanned entries so the table shows media indicators.
            if let Some(ci) = app.browser.find_by_folder(&folder_name)
                && let Some(ref root) = app.root_path.clone()
            {
                let entries: Vec<(String, String)> = app.browser.consoles[ci]
                    .entries
                    .iter()
                    .filter(|e| e.asset_paths.is_none())
                    .map(|e| {
                        (
                            e.game_entry.display_name().to_string(),
                            e.game_entry.rom_stem().to_string(),
                        )
                    })
                    .collect();
                if !entries.is_empty() {
                    crate::backend::assets::discover_assets_for_console(
                        app.message_tx.clone(),
                        ctx.clone(),
                        root.clone(),
                        folder_name.clone(),
                        app.settings.general.assets_dir.clone(),
                        entries,
                    );
                }
            }

            // Advance the auto-scan queue only when the in-flight queued scan
            // finishes — manual scans run by the user don't pop the queue.
            if app.ui_state.auto_scan_in_flight.as_deref() == Some(folder_name.as_str()) {
                app.ui_state.auto_scan_in_flight = None;
                if let Some(op_id) = app.ui_state.auto_scan_op_id
                    && let Some(op) = app.operations.iter_mut().find(|o| o.id == op_id)
                {
                    op.progress_current = (op.progress_current + 1).min(op.progress_total);
                }
                start_next_auto_scan(app, ctx);
            }
        }

        AppMessage::DatLoaded {
            folder_name,
            platform,
            index,
        } => {
            let game_count = index.game_count();

            // Run serial matching for this specific console's entries
            if let Some(ci) = app.browser.find_by_folder(&folder_name) {
                app.browser.consoles[ci].dat_status = DatStatus::Loaded { game_count };

                let context = app.context.clone();
                if let Some(registered) = context.get_by_platform(platform) {
                    // Single-entry serial matching (skip multi-disc entries)
                    for entry in &mut app.browser.consoles[ci].entries {
                        if entry.disc_identifications.is_some() {
                            continue;
                        }
                        if let Some(ref id) = entry.identification
                            && !id.serial_number.is_empty()
                        {
                            let serial = &id.serial_number;
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
                                        region: String::new(),
                                        cross_region: false,
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

                    // Multi-disc serial+hash matching: resolve each disc, derive game name
                    for entry in &mut app.browser.consoles[ci].entries {
                        let discs = match entry.disc_identifications.as_mut() {
                            Some(d) if !d.is_empty() => d,
                            _ => continue,
                        };

                        let prior_status = entry.status;
                        let had_prior_dat_match = entry.dat_match.is_some();

                        let mut matched_names: Vec<String> = Vec::new();
                        let mut first_rom_name = String::new();
                        let mut any_ambiguous = false;
                        let mut all_candidates: Vec<String> = Vec::new();

                        for disc in discs.iter_mut() {
                            let serial = &disc.identification.serial_number;
                            if !serial.is_empty() {
                                let game_code = registered.analyzer.extract_dat_game_code(serial);
                                match index.match_by_serial(serial, game_code.as_deref()) {
                                    SerialLookupResult::Match(m) => {
                                        let name = index.games[m.game_index].name.clone();
                                        let rom_name = index.games[m.game_index].roms[m.rom_index]
                                            .name
                                            .clone();
                                        if first_rom_name.is_empty() {
                                            first_rom_name.clone_from(&rom_name);
                                        }
                                        // Cache per-disc DAT match for rename
                                        disc.dat_match = Some(DatMatchInfo {
                                            game_name: name.clone(),
                                            rom_name,
                                            method: MatchMethod::Serial,
                                            region: String::new(),
                                            cross_region: false,
                                        });
                                        matched_names.push(name);
                                        continue;
                                    }
                                    SerialLookupResult::Ambiguous { candidates } => {
                                        any_ambiguous = true;
                                        for c in candidates {
                                            if !all_candidates.contains(&c) {
                                                all_candidates.push(c);
                                            }
                                        }
                                    }
                                    SerialLookupResult::NotFound => {}
                                }
                            }

                            // Serial didn't resolve — try hash matching with cached hashes
                            if let Some(ref hashes) = disc.hashes {
                                let matches = index.match_by_hash(hashes.data_size, hashes);
                                let detected = disc.identification.regions.as_slice();
                                if let Some(dm) = pick_best_hash_match(&index, &matches, detected) {
                                    if first_rom_name.is_empty() {
                                        first_rom_name.clone_from(&dm.rom_name);
                                    }
                                    matched_names.push(dm.game_name.clone());
                                    disc.dat_match = Some(dm);
                                } else {
                                    any_ambiguous = true;
                                }
                            } else {
                                any_ambiguous = true;
                            }
                        }
                        if !matched_names.is_empty() {
                            // Try catalog DB for canonical game name, fall back to DAT name derivation
                            let combined = app
                                .catalog_db
                                .as_ref()
                                .and_then(|conn| {
                                    entry
                                        .disc_identifications
                                        .as_ref()
                                        .and_then(|discs| try_catalog_game_name(discs, conn))
                                })
                                .unwrap_or_else(|| {
                                    let name_refs: Vec<&str> = matched_names
                                        .iter()
                                        .map(std::string::String::as_str)
                                        .collect();
                                    retro_junk_core::disc::derive_base_game_name(&name_refs)
                                });
                            // Cross-region if any disc has a cross-region hash match
                            let cross_region =
                                entry.disc_identifications.as_ref().is_some_and(|ds| {
                                    ds.iter().any(|d| {
                                        d.dat_match.as_ref().is_some_and(|dm| dm.cross_region)
                                    })
                                });
                            entry.dat_match = Some(DatMatchInfo {
                                game_name: combined,
                                rom_name: first_rom_name.clone(),
                                method: MatchMethod::Serial,
                                region: String::new(),
                                cross_region,
                            });
                            if any_ambiguous {
                                // Check if "ambiguous" candidates are just different discs of the same game
                                if retro_junk_core::disc::candidates_are_same_game(&all_candidates)
                                    .is_some()
                                {
                                    entry.status = EntryStatus::Matched;
                                    entry.ambiguous_candidates.clear();
                                } else {
                                    entry.status = EntryStatus::Ambiguous;
                                    entry.ambiguous_candidates = all_candidates;
                                }
                            } else {
                                entry.status = EntryStatus::Matched;
                                entry.ambiguous_candidates.clear();
                            }
                        } else if any_ambiguous {
                            if let Some(base_name) =
                                retro_junk_core::disc::candidates_are_same_game(&all_candidates)
                            {
                                entry.status = EntryStatus::Matched;
                                entry.ambiguous_candidates.clear();
                                entry.dat_match = Some(DatMatchInfo {
                                    game_name: base_name,
                                    rom_name: first_rom_name,
                                    method: MatchMethod::Serial,
                                    region: String::new(),
                                    cross_region: false,
                                });
                            } else {
                                entry.status = EntryStatus::Ambiguous;
                                entry.ambiguous_candidates = all_candidates;
                            }
                        }

                        // If re-matching couldn't fully resolve but the entry
                        // already had a valid DAT match from a prior cycle,
                        // restore its old status.
                        if entry.status != EntryStatus::Matched && had_prior_dat_match {
                            entry.status = prior_status;
                        }
                    }
                }

                // Re-check hash matches for entries that have cached hashes
                // but weren't resolved by serial alone (e.g. Ambiguous or Unrecognized).
                // Skip multi-disc entries — their game-level match is handled above.
                for entry in &mut app.browser.consoles[ci].entries {
                    if entry.disc_identifications.is_some() {
                        continue;
                    }
                    if entry.status != EntryStatus::Matched
                        && let Some(ref hashes) = entry.hashes
                    {
                        let matches = index.match_by_hash(hashes.data_size, hashes);
                        let detected = entry
                            .identification
                            .as_ref()
                            .map(|id| id.regions.as_slice())
                            .unwrap_or_default();
                        if let Some(dm) = pick_best_hash_match(&index, &matches, detected) {
                            entry.dat_match = Some(dm);
                            entry.status = EntryStatus::Matched;
                            entry.ambiguous_candidates.clear();
                        }
                    }
                }

                // Enrich entries with catalog titles
                if let Some(ref conn) = app.catalog_db {
                    for entry in &mut app.browser.consoles[ci].entries {
                        if entry.hashes.is_some() {
                            try_catalog_enrich(entry, conn);
                        }
                        try_catalog_enrich_by_serial(entry, conn);
                    }
                }
            }

            // Store the DatIndex for later hash matching
            app.dat_indices.insert(folder_name.clone(), Arc::new(index));

            app.operations
                .retain(|op| !op.description.contains("Loading DAT"));
            if let Some(ci) = app.browser.find_by_folder(&folder_name) {
                app.save_console_cache(ci, ctx);
            }
        }

        AppMessage::DatLoadFailed { folder_name, error } => {
            app.push_error("DAT Load Failed", format!("{folder_name}: {error}"));
            if let Some(ci) = app.browser.find_by_folder(&folder_name) {
                app.browser.consoles[ci].dat_status = DatStatus::Unavailable { reason: error };
            }
            app.operations
                .retain(|op| !op.description.contains("Loading DAT"));
        }

        AppMessage::HashComplete {
            folder_name,
            entry_name,
            hashes,
        } => {
            if let Some(ci) = app.browser.find_by_folder(&folder_name)
                && let Some(entry) = app.browser.consoles[ci].find_entry_mut(&entry_name)
            {
                // Try hash matching against the loaded DAT.
                // Skip multi-disc entries — their game-level match is handled by
                // multi-disc serial matching; this hash only covers one disc.
                if entry.disc_identifications.is_none()
                    && let Some(dat_index) = app.dat_indices.get(&folder_name)
                {
                    let matches = dat_index.match_by_hash(hashes.data_size, &hashes);
                    let detected = entry
                        .identification
                        .as_ref()
                        .map(|id| id.regions.as_slice())
                        .unwrap_or_default();
                    if let Some(dm) = pick_best_hash_match(dat_index, &matches, detected) {
                        entry.dat_match = Some(dm);
                        entry.status = EntryStatus::Matched;
                        entry.ambiguous_candidates.clear();
                    }
                }
                entry.hashes = Some(hashes);

                // Enrich with catalog titles
                if let Some(ref conn) = app.catalog_db {
                    try_catalog_enrich(entry, conn);
                    try_catalog_enrich_by_serial(entry, conn);
                }
            }
            if let Some(ci) = app.browser.find_by_folder(&folder_name) {
                app.save_console_cache(ci, ctx);
            }
        }

        AppMessage::DiscHashComplete {
            folder_name,
            entry_name,
            disc_path,
            hashes,
        } => {
            if let Some(ci) = app.browser.find_by_folder(&folder_name)
                && let Some(entry) = app.browser.consoles[ci].find_entry_mut(&entry_name)
                && let Some(ref mut discs) = entry.disc_identifications
            {
                // Find the matching disc and store its hashes
                if let Some(disc) = discs.iter_mut().find(|d| d.path == disc_path) {
                    // Try per-disc DAT matching
                    if let Some(dat_index) = app.dat_indices.get(&folder_name) {
                        let matches = dat_index.match_by_hash(hashes.data_size, &hashes);
                        let detected = disc.identification.regions.as_slice();
                        if let Some(dm) = pick_best_hash_match(dat_index, &matches, detected) {
                            log::info!(
                                "Disc hash match: {} -> {} ({})",
                                disc_path.display(),
                                dm.game_name,
                                dm.rom_name
                            );
                            disc.dat_match = Some(dm);
                        }
                    }
                    disc.hashes = Some(hashes);
                }

                // Check if ALL discs now have hashes and at least one has a DAT match
                let all_hashed = discs.iter().all(|d| d.hashes.is_some());
                if all_hashed && discs.iter().any(|d| d.dat_match.is_some()) {
                    // Derive game-level name: try catalog DB, then fall back to DAT name derivation
                    let game_name = app
                        .catalog_db
                        .as_ref()
                        .and_then(|conn| try_catalog_game_name(discs, conn))
                        .unwrap_or_else(|| {
                            let names: Vec<&str> = discs
                                .iter()
                                .filter_map(|d| {
                                    d.dat_match.as_ref().map(|dm| dm.game_name.as_str())
                                })
                                .collect();
                            retro_junk_core::disc::derive_base_game_name(&names)
                        });
                    let first_match = discs.iter().find_map(|d| d.dat_match.as_ref());
                    let first_rom_name = first_match
                        .map(|dm| dm.rom_name.clone())
                        .unwrap_or_default();
                    let method = first_match.map_or(MatchMethod::Crc32, |dm| dm.method.clone());
                    let region = first_match.map(|dm| dm.region.clone()).unwrap_or_default();
                    // Cross-region if any disc has a cross-region match
                    let cross_region = discs
                        .iter()
                        .any(|d| d.dat_match.as_ref().is_some_and(|dm| dm.cross_region));
                    entry.dat_match = Some(DatMatchInfo {
                        game_name,
                        rom_name: first_rom_name,
                        method,
                        region,
                        cross_region,
                    });
                    entry.status = EntryStatus::Matched;
                    entry.ambiguous_candidates.clear();
                }

                // Enrich with catalog titles
                if let Some(ref conn) = app.catalog_db {
                    try_catalog_enrich(entry, conn);
                    try_catalog_enrich_by_serial(entry, conn);
                }
            }
            if let Some(ci) = app.browser.find_by_folder(&folder_name) {
                app.save_console_cache(ci, ctx);
            }
        }

        AppMessage::HashFailed {
            folder_name,
            entry_name,
            error,
        } => {
            log::warn!("Hash failed for {folder_name} entry {entry_name}: {error}");
            app.push_error(
                "Hash Failed",
                format!("{entry_name} ({folder_name}): {error}"),
            );
        }

        AppMessage::AssetsLoaded {
            folder_name,
            entry_name,
            assets,
        } => {
            if let Some(ci) = app.browser.find_by_folder(&folder_name)
                && let Some(entry) = app.browser.consoles[ci].find_entry_mut(&entry_name)
            {
                // Invalidate stale cached textures when an asset path changes
                if let Some(ref old_assets) = entry.asset_paths {
                    for (at, old_path) in old_assets {
                        let new_path = assets.get(at);
                        if new_path != Some(old_path) {
                            let old_uri = format!("bytes://media/{}", old_path.display());
                            ctx.forget_image(&old_uri);
                        }
                    }
                }
                entry.asset_paths = Some(assets);
            }
        }

        AppMessage::ScrapeEntryFailed {
            folder_name,
            entry_name,
            error,
        } => {
            log::warn!("Scrape failed for {folder_name} entry {entry_name}: {error}");
            app.push_error(
                "Scrape Failed",
                format!("{entry_name} ({folder_name}): {error}"),
            );
            if let Some(ci) = app.browser.find_by_folder(&folder_name)
                && let Some(entry) = app.browser.consoles[ci].find_entry_mut(&entry_name)
            {
                // Only clear asset_paths if they haven't been discovered yet.
                // If media was already known (e.g. scraping missing media for a game
                // that fails lookup), preserve the existing paths so the UI doesn't
                // lose track of files that are still on disk.
                if entry.asset_paths.is_none() {
                    entry.asset_paths = Some(HashMap::new());
                }
            }
        }

        AppMessage::ScrapeFatalError { message, op_id } => {
            log::error!("Scrape fatal error: {message}");
            app.push_error("Scrape Failed", &message);
            app.operations.retain(|op| op.id != op_id);
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
                log::info!("Exported gamelist.xml for {folder_name}: {path}");
            }
            Err(error) => {
                log::warn!("Export failed for {folder_name}: {error}");
                app.push_error("Export Failed", format!("{folder_name}: {error}"));
            }
        },

        AppMessage::RenameComplete {
            folder_name,
            results,
        } => {
            let mut renamed = 0usize;
            let mut already = 0usize;
            let mut failed = 0usize;

            // Extension set for the console's platform, used by
            // `refresh_multidisc_files` (D5) to re-collect a renamed
            // multi-disc folder's contents via the scanner's playlist logic.
            let extensions = app
                .browser
                .find_by_folder(&folder_name)
                .and_then(|ci| {
                    app.context
                        .get_by_platform(app.browser.consoles[ci].platform)
                })
                .map(|r| retro_junk_lib::scanner::extension_set(r.analyzer.file_extensions()))
                .unwrap_or_default();

            if let Some(ci) = app.browser.find_by_folder(&folder_name) {
                for r in &results {
                    match &r.outcome {
                        RenameOutcome::Renamed { target, .. } => {
                            renamed += 1;
                            // Update the GameEntry path to reflect the new filename
                            if let Some(entry) =
                                app.browser.consoles[ci].find_entry_mut(&r.entry_name)
                            {
                                entry.game_entry =
                                    retro_junk_lib::scanner::GameEntry::SingleFile(target.clone());
                                // Invalidate cached checks so background re-check picks them up
                                entry.broken_references = None;
                                entry.cue_compat_issues = None;
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
                                app.browser.consoles[ci].find_entry_mut(&r.entry_name)
                            {
                                // Update folder name from the target folder
                                if let retro_junk_lib::scanner::GameEntry::MultiDisc {
                                    ref mut name,
                                    ..
                                } = entry.game_entry
                                    && let Some(folder_stem) =
                                        target_folder.file_name().and_then(|n| n.to_str())
                                {
                                    *name = folder_stem.to_string();
                                }
                                refresh_multidisc_files(entry, target_folder, &extensions);
                                // Invalidate cached checks so background re-check picks them up
                                entry.broken_references = None;
                                entry.cue_compat_issues = None;
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
                "Rename {folder_name}: {renamed} renamed, {already} already correct, {failed} failed"
            );
            app.ui_state.results_dialog = crate::app::ResultsDialog::Rename(results);

            // Invalidate fingerprint so the next save recomputes it from the
            // actual (post-rename) file names on disk.
            if renamed > 0
                && let Some(ci) = app.browser.find_by_folder(&folder_name)
            {
                app.browser.consoles[ci].fingerprint = None;
            }
            if let Some(ci) = app.browser.find_by_folder(&folder_name) {
                app.save_console_cache(ci, ctx);
                if renamed > 0 {
                    recheck_invalidated_entries(app, ci, &folder_name, ctx);
                }
            }
        }

        AppMessage::OrganizePlanReady { folder_name, plan } => {
            log::info!(
                "Organize plan ready for {}: {} folders to create, {} unmatched",
                folder_name,
                plan.jobs.len(),
                plan.unmatched.len(),
            );
            app.ui_state.pending_organize_plan = Some((folder_name, plan));
        }

        AppMessage::OrganizeComplete {
            folder_name,
            jobs_executed,
            files_moved,
            unmatched,
            errors,
        } => {
            if jobs_executed > 0 {
                log::info!(
                    "Organized {folder_name}: created {jobs_executed} folders, moved {files_moved} files",
                );
                // Trigger rescan since folder structure changed
                if let Some(ci) = app.browser.find_by_folder(&folder_name) {
                    let console = &mut app.browser.consoles[ci];
                    console.scan_status = ScanStatus::NotScanned;
                    console.loose_disc_files.clear();
                    crate::backend::scan::quick_scan_console(app, ci, ctx);
                }
            }
            if unmatched > 0 {
                log::warn!("{unmatched} files could not be matched in {folder_name}");
            }
            for err in &errors {
                log::error!("Organize error: {err}");
            }
        }

        AppMessage::CueFixComplete {
            folder_name,
            results,
        } => {
            let fixed = results
                .iter()
                .filter(|r| matches!(r.outcome, CueFixOutcome::Fixed { .. }))
                .count();
            let already = results
                .iter()
                .filter(|r| matches!(r.outcome, CueFixOutcome::AlreadyStandard))
                .count();
            let failed = results
                .iter()
                .filter(|r| {
                    matches!(
                        r.outcome,
                        CueFixOutcome::Unfixable { .. } | CueFixOutcome::Error { .. }
                    )
                })
                .count();
            log::info!(
                "CUE fix {folder_name}: {fixed} fixed, {already} already standard, {failed} failed"
            );
            // Invalidate cached checks on affected entries and re-check
            if fixed > 0
                && let Some(ci) = app.browser.find_by_folder(&folder_name)
            {
                let fixed_files: std::collections::HashSet<&str> = results
                    .iter()
                    .filter(|r| matches!(r.outcome, CueFixOutcome::Fixed { .. }))
                    .map(|r| r.file_name.as_str())
                    .collect();
                for entry in &mut app.browser.consoles[ci].entries {
                    let dominated = entry.cue_compat_issues.as_ref().is_some_and(|issues| {
                        issues
                            .iter()
                            .any(|i| fixed_files.contains(i.file_name.as_str()))
                    }) || entry.broken_references.as_ref().is_some_and(|refs| {
                        refs.iter().any(|r| {
                            r.ref_file
                                .file_name()
                                .and_then(|n| n.to_str())
                                .is_some_and(|n| fixed_files.contains(n))
                        })
                    });
                    if dominated {
                        entry.broken_references = None;
                        entry.cue_compat_issues = None;
                    }
                }
                app.save_console_cache(ci, ctx);
                recheck_invalidated_entries(app, ci, &folder_name, ctx);
            }
            app.ui_state.results_dialog = crate::app::ResultsDialog::CueFix(results);
        }

        AppMessage::ChdCompressComplete {
            folder_name,
            results,
        } => {
            let compressed = results
                .iter()
                .filter(|r| matches!(r.outcome, ChdCompressOutcome::Compressed { .. }))
                .count();
            let failed = results.len() - compressed;
            log::info!(
                "CHD compression {folder_name}: {compressed} compressed, {failed} failed/skipped"
            );

            // Extension set for the console's platform, used by
            // `refresh_multidisc_files` (D5) when a compressed disc belongs
            // to a MultiDisc entry.
            let extensions = app
                .browser
                .find_by_folder(&folder_name)
                .and_then(|ci| {
                    app.context
                        .get_by_platform(app.browser.consoles[ci].platform)
                })
                .map(|r| retro_junk_lib::scanner::extension_set(r.analyzer.file_extensions()))
                .unwrap_or_default();

            if let Some(ci) = app.browser.find_by_folder(&folder_name) {
                let mut changed = false;
                let mut deleted_files: std::collections::HashSet<PathBuf> =
                    std::collections::HashSet::new();
                let mut any_sources_deleted = false;

                // Process every Compressed outcome, not just ones with
                // sources_deleted: with the dialog default (keep originals),
                // a verified compression still needs the fingerprint cleared
                // so the next scan reconciles the new sibling .chd instead of
                // silently going stale.
                for r in &results {
                    let ChdCompressOutcome::Compressed {
                        sources_deleted, ..
                    } = &r.outcome
                    else {
                        continue;
                    };
                    changed = true;

                    // Path-keyed lookup (not display-name): the entry still
                    // holds its pre-compression path even after the file is
                    // deleted, so matching this job's input against
                    // `all_files()` is exact and rename-proof for this batch.
                    let Some(entry) = app.browser.consoles[ci].find_entry_by_file_mut(&r.job.input)
                    else {
                        log::warn!(
                            "CHD compress complete: no library entry found for input {}",
                            r.job.input.display()
                        );
                        continue;
                    };

                    if *sources_deleted {
                        any_sources_deleted = true;
                        deleted_files.extend(r.job.source_files.iter().cloned());
                        match entry.game_entry {
                            retro_junk_lib::scanner::GameEntry::SingleFile(ref mut path) => {
                                path.clone_from(&r.job.output);
                            }
                            retro_junk_lib::scanner::GameEntry::MultiDisc { .. } => {
                                if let Some(folder) = r.job.output.parent() {
                                    let folder = folder.to_path_buf();
                                    refresh_multidisc_files(entry, &folder, &extensions);
                                }
                            }
                        }
                        // The cached hashes describe the old container; the
                        // cue checks no longer apply to a .chd.
                        entry.hashes = None;
                        entry.broken_references = None;
                        entry.cue_compat_issues = None;
                    }
                    // Else: sources kept — the entry stays on the cue/iso and
                    // its hashes remain valid. The fingerprint invalidation
                    // below lets the next scan pick up the new sibling .chd
                    // (deduped onto this same entry by the scanner's stem
                    // dedup, which now covers ".chd" — see D4).
                }

                if any_sources_deleted {
                    // Drop entries whose every file was deleted: removes
                    // dangling "(Track N).bin" SingleFile ghosts the scanner
                    // kept as separate entries (stem-only dedup). Entries
                    // updated above now point at the .chd, so they survive
                    // this filter.
                    app.browser.consoles[ci].entries.retain(|e| {
                        !e.game_entry
                            .all_files()
                            .iter()
                            .all(|f| deleted_files.contains(f))
                    });
                }

                if changed {
                    app.browser.consoles[ci].fingerprint = None;
                    app.save_console_cache(ci, ctx);
                    recheck_invalidated_entries(app, ci, &folder_name, ctx);
                }
            }
            app.ui_state.results_dialog = crate::app::ResultsDialog::ChdCompress(results);
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
            // The worker thread is at (or immediately reaching) its end
            // after sending this message, so the join is effectively
            // instant. Reclaiming the handle here (rather than only in
            // `on_exit`) keeps `op_threads` from growing unboundedly over a
            // long session.
            if let Some(handle) = app.op_threads.remove(&op_id) {
                let _ = handle.join();
            }
        }

        AppMessage::CatalogDataChanged => {
            // A background catalog op finished writing to the DB. Reload the
            // dashboard/browse data and re-enable the Data-tab buttons.
            app.ui_state.tools_state.data.op_in_flight = false;
            app.ui_state.tools_state.needs_refresh = true;
            app.ui_state.tools_state.browse.table_state.needs_query = true;
            app.ui_state.tools_state.data.needs_cache_refresh = true;
        }

        AppMessage::CacheListsLoaded { dat, gdb } => {
            app.ui_state.tools_state.data.dat_cache_entries = dat;
            app.ui_state.tools_state.data.gdb_cache_entries = gdb;
        }

        AppMessage::ChdCompressPromptReady { prompt } => {
            app.ui_state.chd_compress_prompt = Some(prompt);
        }

        AppMessage::ChdmanProbeResult { key, result } => {
            // Only apply if the setting hasn't changed since this probe was
            // kicked off — otherwise a slow probe for a since-abandoned path
            // could clobber a fresher result.
            let current_key = app.settings.general.chdman_path.trim().to_string();
            app.ui_state.chdman_probe = if key == current_key {
                crate::app::ChdmanProbe::Done { path: key, result }
            } else {
                crate::app::ChdmanProbe::Idle
            };
        }
    }
}

// -- Tools state --

/// Fields that can appear in disagreements (for the filter dropdown).
pub const DISAGREEMENT_FIELDS: &[&str] = &[
    "title",
    "alt_title",
    "release_date",
    "game_serial",
    "genre",
    "players",
    "description",
    "media_serial",
    "revision",
    "status",
];

/// Context about the entity referenced by the selected disagreement.
pub struct DisagreementContext {
    pub entity_title: String,
    pub platform_name: String,
}

/// Active tab in the Tools view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolsTab {
    #[default]
    Dashboard,
    Browse,
    Data,
}

/// Which table is being viewed in the Browse tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BrowseTable {
    #[default]
    Releases,
    Media,
    Works,
    Companies,
    Collection,
    ImportLog,
}

impl BrowseTable {
    pub const ALL: &[BrowseTable] = &[
        BrowseTable::Releases,
        BrowseTable::Media,
        BrowseTable::Works,
        BrowseTable::Companies,
        BrowseTable::Collection,
        BrowseTable::ImportLog,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Releases => "Releases",
            Self::Media => "Media",
            Self::Works => "Works",
            Self::Companies => "Companies",
            Self::Collection => "Collection",
            Self::ImportLog => "Import Log",
        }
    }

    /// Whether this table supports the platform filter dropdown.
    pub fn has_platform_filter(self) -> bool {
        matches!(self, Self::Releases | Self::Media | Self::Collection)
    }

    /// Whether this table supports a text search box.
    pub fn has_search(self) -> bool {
        !matches!(self, Self::ImportLog)
    }
}

/// Shared pagination + search state used by every browse table.
pub struct TableViewState {
    pub search_text: String,
    pub platform_filter: Option<String>,
    pub page: u32,
    pub page_size: u32,
    pub total_count: i64,
    pub page_input: String,
    /// Set to true to trigger a data reload.
    pub needs_query: bool,
}

impl Default for TableViewState {
    fn default() -> Self {
        Self {
            search_text: String::new(),
            platform_filter: None,
            page: 0,
            page_size: 50,
            total_count: 0,
            page_input: "1".to_string(),
            needs_query: true,
        }
    }
}

impl TableViewState {
    pub fn total_pages(&self) -> u32 {
        ((self.total_count as u32).saturating_add(self.page_size - 1)) / self.page_size
    }

    pub fn offset(&self) -> u32 {
        self.page * self.page_size
    }

    /// Reset to page 0 and flag for reload.
    pub fn reset_query(&mut self) {
        self.page = 0;
        self.page_input = "1".to_string();
        self.needs_query = true;
    }
}

/// State for the database browser in the Browse tab.
#[derive(Default)]
pub struct BrowseState {
    pub active_table: BrowseTable,
    pub table_state: TableViewState,
    /// Cached rows for the current table view.
    pub releases: Vec<retro_junk_catalog::types::Release>,
    pub media_rows: Vec<retro_junk_catalog::types::Media>,
    pub works: Vec<retro_junk_db::WorkWithCount>,
    pub companies: Vec<retro_junk_db::CompanyRow>,
    pub collection: Vec<retro_junk_db::CollectionRow>,
    pub import_logs: Vec<retro_junk_catalog::types::ImportLog>,
}

/// Transient UI state for the Tools (catalog) view.
pub struct ToolsState {
    pub stats: Option<retro_junk_db::CatalogStats>,
    pub platforms: Vec<retro_junk_db::PlatformRow>,
    pub disagreements: Vec<retro_junk_catalog::types::Disagreement>,
    pub selected_idx: Option<usize>,
    pub filter_platform: Option<String>,
    pub filter_field: Option<String>,
    pub selected_context: Option<DisagreementContext>,
    pub needs_refresh: bool,
    pub active_tab: ToolsTab,
    pub browse: BrowseState,
    pub data: DataToolsState,
}

impl Default for ToolsState {
    fn default() -> Self {
        Self {
            stats: None,
            platforms: Vec::new(),
            disagreements: Vec::new(),
            selected_idx: None,
            filter_platform: None,
            filter_field: None,
            selected_context: None,
            needs_refresh: true,
            active_tab: ToolsTab::default(),
            browse: BrowseState::default(),
            data: DataToolsState::default(),
        }
    }
}

/// Transient UI state for the Data tab (catalog data-gathering operations).
///
/// The system selection is a set of `Platform`s; an empty set means "all
/// capable systems" (mirroring the CLI's `all` keyword). Per-operation option
/// fields hold text/bool inputs bound to the UI widgets.
// The bools are independent checkbox/guard flags bound to UI widgets, not an encoded state machine.
#[allow(clippy::struct_excessive_bools)]
pub struct DataToolsState {
    /// Selected systems to operate on. Empty = all capable systems.
    pub selected_systems: std::collections::HashSet<Platform>,
    /// True while a catalog-mutating operation (import/enrich) is running.
    /// Guards against launching concurrent DB writers.
    pub op_in_flight: bool,
    /// GDB enrichment: max releases per system (text input; empty = no limit).
    pub gdb_limit: String,
    /// `ScreenScraper` enrichment options.
    pub ss_force: bool,
    pub ss_download_assets: bool,
    pub ss_region: String,
    pub ss_language: String,
    pub ss_limit: String,
    pub ss_reconcile: bool,
    /// Cached DAT cache listing (for display).
    pub dat_cache_entries: Vec<retro_junk_dat::cache::CacheEntry>,
    /// Cached GDB cache listing (for display).
    pub gdb_cache_entries: Vec<retro_junk_dat::gdb_cache::GdbCacheEntry>,
    /// Set true to reload the cache listings on next frame.
    pub needs_cache_refresh: bool,
}

impl Default for DataToolsState {
    fn default() -> Self {
        Self {
            selected_systems: std::collections::HashSet::new(),
            op_in_flight: false,
            gdb_limit: String::new(),
            ss_force: false,
            ss_download_assets: false,
            ss_region: "us".to_string(),
            ss_language: "en".to_string(),
            ss_limit: String::new(),
            ss_reconcile: true,
            dat_cache_entries: Vec::new(),
            gdb_cache_entries: Vec::new(),
            needs_cache_refresh: true,
        }
    }
}
