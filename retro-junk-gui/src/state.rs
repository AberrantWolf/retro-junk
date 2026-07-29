#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use retro_junk_catalog::CatalogTag;
use retro_junk_dat::{FileHashes, MatchMethod};
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
    /// Filesystem availability has not been discovered yet.
    Unknown,
    /// Discovered, no scrapeable assets found
    None,
    /// Some but not all scrapeable asset types present
    Partial { found: u8, total: u8 },
    /// All scrapeable asset types present
    Complete,
}

fn asset_status_from_paths(media: &HashMap<AssetType, PathBuf>) -> AssetStatus {
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

pub fn asset_availability(media: &HashMap<AssetType, PathBuf>) -> (AssetStatus, bool) {
    (
        asset_status_from_paths(media),
        media.contains_key(&AssetType::Miximage),
    )
}

/// URI used by egui's file loader. Unlike `include_bytes`, this lets us evict
/// all decoded data and textures when the detail selection changes.
pub fn asset_image_uri(path: &Path) -> String {
    format!("file://{}", path.display())
}
use retro_junk_lib::scanner::GameEntry;
use retro_junk_lib::{AnalysisError, Platform, Region, RomIdentification};

use crate::app::RetroJunkApp;

// -- Navigation --

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Library,
    Collection,
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
    /// Worst effective entry status for each console, retained when pages are evicted.
    pub console_statuses: HashMap<retro_junk_db::LibraryConsoleId, EntryStatus>,
    /// Consoles explicitly marked stale by SQLite and requiring a correctness rebuild.
    pub stale_consoles: HashSet<retro_junk_db::LibraryConsoleId>,
    /// Entry IDs with filesystem media discovery currently in flight.
    pub asset_discovery_in_flight: HashSet<retro_junk_db::LibraryEntryId>,
    /// Lightweight filesystem-derived availability for rows in the active page.
    /// This deliberately contains no paths or image bytes.
    pub asset_statuses: HashMap<retro_junk_db::LibraryEntryId, AssetStatus>,
    pub entries_with_miximages: HashSet<retro_junk_db::LibraryEntryId>,
    /// The sole entry whose paths/images may be retained for the detail panel.
    pub detail_asset_entry: Option<retro_junk_db::LibraryEntryId>,
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

#[derive(Clone)]
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
    /// Cached folder fingerprint (avoids recomputing on every save).
    pub fingerprint: Option<crate::fingerprint::FolderFingerprint>,
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

    pub fn entry_by_id_mut(
        &mut self,
        id: retro_junk_db::LibraryEntryId,
    ) -> Option<&mut LibraryEntry> {
        self.entry_index(id)
            .and_then(|index| self.entries.get_mut(index))
    }

    #[cfg(test)]
    pub fn find_entry_by_file_mut(&mut self, file: &Path) -> Option<&mut LibraryEntry> {
        self.entries
            .iter_mut()
            .find(|entry| entry.game_entry.all_files().iter().any(|path| path == file))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScanStatus {
    NotScanned,
    Scanning,
    Scanned,
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
    #[serde(default)]
    pub ambiguous_candidates: Vec<String>,
    /// Whether a disc container was verified as a complete DAT track set.
    #[serde(default)]
    pub disc_verification: DiscVerification,
}

/// One successfully completed file/disc checksum from a batched entry job.
#[derive(Clone)]
pub struct EntryHashResult {
    pub disc_path: Option<PathBuf>,
    pub hashes: FileHashes,
    pub catalog_matches: Vec<retro_junk_db::CatalogMediaMatch>,
    pub disc_verification: DiscVerification,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiscVerification {
    /// Ordinary flat-file hashing, or legacy data created before disc-set
    /// verification existed.
    #[default]
    NotApplicable,
    /// Every logical track matched one catalog media entry.
    Complete,
    /// The disc was identified, but one or more tracks were absent or wrong.
    Incomplete,
    /// The descriptor did not define a safe, coherent logical track layout.
    InvalidLayout,
}

impl DiscVerification {
    fn permits_verified_status(self) -> bool {
        matches!(self, Self::NotApplicable | Self::Complete)
    }
}

#[derive(Clone)]
pub struct LibraryEntry {
    /// Durable database identity; absent only for not-yet-reconciled scan rows.
    pub id: Option<retro_junk_db::LibraryEntryId>,
    pub revision: u64,
    pub source_revision: u64,
    pub game_entry: GameEntry,
    pub identification: Option<RomIdentification>,
    pub hashes: Option<FileHashes>,
    /// Complete-disc verification for a standalone disc container. Multi-disc
    /// entries store this on each [`DiscIdentification`].
    pub disc_verification: DiscVerification,
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

    /// Whether this entry has detected CUE sheet compatibility issues.
    pub fn has_cue_compat_issues(&self) -> bool {
        self.cue_compat_issues
            .as_ref()
            .is_some_and(|issues| !issues.is_empty())
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
    /// A unique DAT entry was identified, but it is not completely verified
    /// (for example, serial-only or a disc with missing/mismatched tracks).
    LikelyMatched,
    /// Definitively matched to a DAT fingerprint by hash.
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
            EntryStatus::LikelyMatched => crate::theme::STATUS_INFO,
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
            EntryStatus::LikelyMatched => {
                "Likely match \u{2013} identity is known, but the complete content is not hash-verified"
            }
            EntryStatus::Matched => "Verified match in database",
            EntryStatus::Tagged(CatalogTag::Homebrew) => "Homebrew game",
            EntryStatus::Tagged(CatalogTag::Modded) => "Modded ROM",
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
        platform_id: String,
        disc_number_required: bool,
        disc_number: String,
        console_id: retro_junk_db::LibraryConsoleId,
        entry_id: retro_junk_db::LibraryEntryId,
    },
}

// -- Rename results --

pub struct RenameResult {
    pub entry_id: retro_junk_db::LibraryEntryId,
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
    /// Work whose result exists only to populate the current UI projection.
    UiFetch,
    Scan,
    Hash,
    Rename,
    CueFix,
    ChdCompress,
    ArchiveImport,
    Other,
}

#[derive(Debug)]
pub enum DumpImportDialogState {
    Planning {
        op_id: u64,
        source: PathBuf,
    },
    Review {
        plan: retro_junk_archive_import::DumpImportPlan,
        consume: bool,
        new_physical_copy: bool,
        make_playable: bool,
        discard_redundant_bin_cue: bool,
    },
    Importing {
        op_id: u64,
    },
    Complete {
        result: retro_junk_archive_import::DumpImportBatchResult,
    },
}

#[derive(Debug, Clone, Default)]
pub struct PhysicalCopyEditor {
    pub archive_release_id: String,
    pub platform_id: String,
    pub title: String,
    pub region: String,
    pub revision: String,
    pub variant: String,
    pub catalog_release_id: String,
    pub catalog_source: String,
    pub release_binding_state: String,
    pub carrier_kind: String,
    pub carrier_serial: String,
    pub carrier_binding_state: String,
    pub physical_copy_id: String,
    pub physical_copy_manifest_path: PathBuf,
    pub carrier_manifest_path: PathBuf,
    pub label: String,
    pub condition: String,
    pub notes: String,
    pub date_acquired: String,
    pub provenance: String,
    pub desired_format: String,
    pub retain_intermediate: bool,
    pub allow_unverified: bool,
    pub ingest_format: String,
    pub release_asset_type: String,
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

/// A background-to-UI message stamped with the root session that created it.
/// Old workers retain their original sender when the user switches roots, so
/// their late results can be rejected without relying on folder-name matches.
pub struct AppMessageEnvelope {
    pub session_generation: crate::backend::library_store::UiSessionGeneration,
    pub payload: AppMessage,
}

#[derive(Clone)]
pub struct AppMessageSender {
    sender: std::sync::mpsc::Sender<AppMessageEnvelope>,
    session_generation: crate::backend::library_store::UiSessionGeneration,
}

impl AppMessageSender {
    pub fn new(sender: std::sync::mpsc::Sender<AppMessageEnvelope>) -> Self {
        Self {
            sender,
            session_generation: 0,
        }
    }

    pub fn for_generation(
        &self,
        session_generation: crate::backend::library_store::UiSessionGeneration,
    ) -> Self {
        Self {
            sender: self.sender.clone(),
            session_generation,
        }
    }

    pub fn send(
        &self,
        payload: AppMessage,
    ) -> Result<(), std::sync::mpsc::SendError<AppMessageEnvelope>> {
        self.sender.send(AppMessageEnvelope {
            session_generation: self.session_generation,
            payload,
        })
    }
}

/// Messages sent from background threads to the UI thread.
///
/// All messages use `folder_name: String` (not `Platform`) to identify which
/// console they target. This is critical because multiple folders can map to
/// the same platform (e.g., "gb" and "gbc" both map to `Platform::GameBoy`).
pub enum AppMessage {
    /// Set or clear the blocking startup modal. Sent by the startup thread
    /// only when a real location/schema migration is required, so the first
    /// frame never waits on database probes.
    StartupStatus {
        status: Option<String>,
    },
    StartupReady {
        database: Result<retro_junk_db::Connection, String>,
    },
    StartupRootReady {
        restored_root: Option<PathBuf>,
        fragile_mount_kind: Option<&'static str>,
    },
    StartArchiveRefresh {
        profile: retro_junk_archive::CollectionProfile,
    },
    ArchiveOperationComplete {
        op_id: u64,
        result: Result<String, String>,
    },
    CollectionSummariesReady {
        profile_id: String,
        result: Result<Vec<retro_junk_db::ArchiveReleaseSummary>, String>,
    },
    CollectionEditorReady {
        release_id: String,
        result: Result<PhysicalCopyEditor, String>,
    },
    PlayablePolicyUpdated {
        op_id: u64,
        result: Result<retro_junk_archive::ArchiveRootManifest, String>,
    },
    PlayableBuildComplete {
        op_id: u64,
        result: Result<Option<std::path::PathBuf>, String>,
    },
    AssetProjectionComplete {
        op_id: u64,
        result: Result<retro_junk_lib::archive_assets::AssetProjectionReport, String>,
    },
    ArchiveImportPlanReady {
        op_id: u64,
        result: Result<retro_junk_archive_import::DumpImportPlan, String>,
    },
    ArchiveImportComplete {
        op_id: u64,
        result: Result<retro_junk_archive_import::DumpImportBatchResult, String>,
    },
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
    ScanProjectionInfo {
        folder_name: String,
        loose_disc_files: Vec<PathBuf>,
        fingerprint: crate::fingerprint::FolderFingerprint,
    },
    EntryAnalysisSnapshotsComplete {
        folder_name: String,
        entries: Vec<LibraryEntry>,
    },
    ConsoleScanFailed {
        folder_name: String,
        /// None means user cancellation; a concrete error is surfaced.
        error: Option<String>,
    },
    ScanSnapshotPrepared {
        folder_name: String,
        console_id: Option<retro_junk_db::LibraryConsoleId>,
        result: Result<crate::backend::library_store::CompletedConsoleScan, String>,
    },

    // -- DAT --
    // -- Hashing --
    EntryHashBatchComplete {
        folder_name: String,
        /// Stable start-of-job state used for the durable write even when its
        /// console has since been evicted from the visible projection.
        entry: Box<LibraryEntry>,
        results: Vec<EntryHashResult>,
    },
    HashFailed {
        folder_name: String,
        entry_id: retro_junk_db::LibraryEntryId,
        entry_name: String,
        error: String,
    },

    // -- Media / Scraping --
    AssetsLoaded {
        folder_name: String,
        entry_id: retro_junk_db::LibraryEntryId,
        assets: HashMap<AssetType, PathBuf>,
    },
    AssetStatusesLoaded {
        console_id: retro_junk_db::LibraryConsoleId,
        statuses: Vec<(retro_junk_db::LibraryEntryId, AssetStatus, bool)>,
    },
    ScrapeEntryFailed {
        folder_name: String,
        entry_id: retro_junk_db::LibraryEntryId,
        entry_name: String,
        error: String,
    },
    ScrapeFatalError {
        message: String,
        op_id: u64,
    },
    MiximageComplete {
        generated: usize,
        failures: Vec<String>,
    },
    /// Authoritative archive artwork changed; rebuild the Library projection.
    ArchiveAssetsChanged,
    ModSearchResults {
        query: String,
        result: Result<Vec<retro_junk_db::WorkRow>, String>,
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
        rescan_target: Option<crate::backend::scan::ConsoleScanTarget>,
        results: Vec<RenameResult>,
    },

    // -- Organize --
    OrganizePlanReady {
        folder_name: String,
        plan: retro_junk_lib::organize::OrganizePlan,
    },
    OrganizeComplete {
        folder_name: String,
        rescan_target: Option<crate::backend::scan::ConsoleScanTarget>,
        jobs_executed: usize,
        files_moved: usize,
        unmatched: usize,
        errors: Vec<String>,
    },

    // -- CUE fix --
    CueFixComplete {
        folder_name: String,
        rescan_target: Option<crate::backend::scan::ConsoleScanTarget>,
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
        rescan_target: Option<crate::backend::scan::ConsoleScanTarget>,
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
    OperationPhase {
        op_id: u64,
        description: String,
        display: ProgressDisplay,
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

impl AppMessage {
    /// Messages that refer to the active filesystem/library projection. These
    /// must never cross a root-session boundary. Process-global catalog and
    /// settings work, plus operation lifecycle bookkeeping, remains valid.
    pub fn is_root_scoped(&self) -> bool {
        !matches!(
            self,
            Self::StartupStatus { .. }
                | Self::StartupReady { .. }
                | Self::StartupRootReady { .. }
                | Self::StartArchiveRefresh { .. }
                | Self::ArchiveOperationComplete { .. }
                | Self::PlayablePolicyUpdated { .. }
                | Self::PlayableBuildComplete { .. }
                | Self::AssetProjectionComplete { .. }
                | Self::ArchiveImportPlanReady { .. }
                | Self::ArchiveImportComplete { .. }
                | Self::ChdmanProbeResult { .. }
                | Self::OperationProgress { .. }
                | Self::OperationPhase { .. }
                | Self::OperationComplete { .. }
                | Self::CatalogDataChanged
                | Self::CacheListsLoaded { .. }
                | Self::ScanSnapshotPrepared { .. }
                | Self::EntryAnalysisSnapshotsComplete { .. }
                | Self::EntryHashBatchComplete { .. }
                | Self::RenameComplete { .. }
                | Self::OrganizeComplete { .. }
                | Self::CueFixComplete { .. }
                | Self::ChdCompressComplete { .. }
        )
    }
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

pub fn finish_auto_scan(
    app: &mut crate::app::RetroJunkApp,
    folder_name: &str,
    succeeded: bool,
    ctx: &egui::Context,
) {
    if app.ui_state.auto_scan_in_flight.as_deref() != Some(folder_name) {
        return;
    }
    app.ui_state.auto_scan_in_flight = None;
    if succeeded
        && let Some(op_id) = app.ui_state.auto_scan_op_id
        && let Some(operation) = app
            .operations
            .iter_mut()
            .find(|operation| operation.id == op_id)
    {
        operation.progress_current = (operation.progress_current + 1).min(operation.progress_total);
    }
    start_next_auto_scan(app, ctx);
}

/// Re-enumerate a `MultiDisc` entry's folder after its files changed on disk
/// (rename, CHD compression): refreshes the entry's `files` list and remaps
/// `disc_identifications` paths to the new files (matching by filename stem,
/// then by extension). No-op for single-file entries.
#[cfg(test)]
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
    retro_junk_lib::catalog_match::regions_match_catalog(detected, dat_region)
}

enum CatalogUiResolution {
    Match {
        info: DatMatchInfo,
        cover_title: String,
        screen_title: String,
    },
    Ambiguous(Vec<String>),
    NotFound,
}

fn resolve_catalog_candidates(
    matches: &[retro_junk_db::CatalogMediaMatch],
    identification: Option<&RomIdentification>,
    hashes: Option<&FileHashes>,
    disc_verification: DiscVerification,
) -> CatalogUiResolution {
    let resolution =
        retro_junk_lib::catalog_match::resolve_catalog_match(matches, identification, hashes);
    match resolution {
        retro_junk_lib::catalog_match::CatalogMatchResolution::Match {
            candidate,
            mut method,
        } => {
            if disc_verification == DiscVerification::Complete
                && matches!(method, MatchMethod::Serial)
            {
                // Complete per-track verification is definitive even when the
                // catalog's representative media hash is not the data track.
                method = MatchMethod::Crc32;
            }
            let detected_regions = identification.map_or(&[][..], |id| id.regions.as_slice());
            CatalogUiResolution::Match {
                info: DatMatchInfo {
                    game_name: candidate.media.dat_name.clone(),
                    rom_name: candidate.media.rom_name.clone(),
                    method,
                    region: candidate.region.clone(),
                    cross_region: !candidate.region.is_empty()
                        && !regions_match_dat(detected_regions, &candidate.region),
                },
                cover_title: candidate.cover_title.clone(),
                screen_title: candidate.screen_title.clone(),
            }
        }
        retro_junk_lib::catalog_match::CatalogMatchResolution::Ambiguous { candidates } => {
            CatalogUiResolution::Ambiguous(candidates)
        }
        retro_junk_lib::catalog_match::CatalogMatchResolution::NotFound
            if disc_verification == DiscVerification::Complete && matches.len() == 1 =>
        {
            let candidate = &matches[0];
            let detected_regions = identification.map_or(&[][..], |id| id.regions.as_slice());
            CatalogUiResolution::Match {
                info: DatMatchInfo {
                    game_name: candidate.media.dat_name.clone(),
                    rom_name: candidate.media.rom_name.clone(),
                    method: MatchMethod::Crc32,
                    region: candidate.region.clone(),
                    cross_region: !candidate.region.is_empty()
                        && !regions_match_dat(detected_regions, &candidate.region),
                },
                cover_title: candidate.cover_title.clone(),
                screen_title: candidate.screen_title.clone(),
            }
        }
        retro_junk_lib::catalog_match::CatalogMatchResolution::NotFound => {
            CatalogUiResolution::NotFound
        }
    }
}

fn apply_catalog_resolution(
    entry: &mut LibraryEntry,
    matches: &[retro_junk_db::CatalogMediaMatch],
) {
    match resolve_catalog_candidates(
        matches,
        entry.identification.as_ref(),
        entry.hashes.as_ref(),
        entry.disc_verification,
    ) {
        CatalogUiResolution::Match {
            info,
            cover_title,
            screen_title,
        } => {
            entry.status = match info.method {
                MatchMethod::Serial => EntryStatus::LikelyMatched,
                MatchMethod::Crc32 | MatchMethod::Sha1
                    if entry.disc_verification.permits_verified_status() =>
                {
                    EntryStatus::Matched
                }
                MatchMethod::Crc32 | MatchMethod::Sha1 => EntryStatus::LikelyMatched,
            };
            entry.dat_match = Some(info);
            entry.ambiguous_candidates.clear();
            if !cover_title.is_empty() {
                entry.cover_title = cover_title;
            }
            if !screen_title.is_empty() {
                entry.screen_title = screen_title;
            }
        }
        CatalogUiResolution::Ambiguous(candidates) => {
            entry.status = EntryStatus::Ambiguous;
            entry.dat_match = None;
            entry.ambiguous_candidates = candidates;
        }
        CatalogUiResolution::NotFound => {
            entry.status = if entry.identification.is_some() {
                EntryStatus::Unrecognized
            } else {
                EntryStatus::Unknown
            };
            entry.dat_match = None;
            entry.ambiguous_candidates.clear();
        }
    }
}

fn apply_multi_disc_resolution(entry: &mut LibraryEntry) {
    let Some(discs) = entry.disc_identifications.as_ref() else {
        return;
    };
    let mut ambiguous_candidates: Vec<String> = discs
        .iter()
        .flat_map(|disc| disc.ambiguous_candidates.iter().cloned())
        .collect();
    ambiguous_candidates.sort();
    ambiguous_candidates.dedup();

    let matched: Vec<_> = discs
        .iter()
        .filter_map(|disc| disc.dat_match.as_ref())
        .collect();
    if matched.is_empty() {
        entry.dat_match = retro_junk_core::disc::candidates_are_same_game(&ambiguous_candidates)
            .map(|game_name| DatMatchInfo {
                game_name,
                rom_name: String::new(),
                method: MatchMethod::Serial,
                region: String::new(),
                cross_region: false,
            });
        if entry.dat_match.is_some() {
            entry.status = EntryStatus::LikelyMatched;
            entry.ambiguous_candidates.clear();
        } else if ambiguous_candidates.is_empty() {
            entry.status = EntryStatus::Unrecognized;
            entry.ambiguous_candidates.clear();
        } else {
            entry.status = EntryStatus::Ambiguous;
            entry.ambiguous_candidates = ambiguous_candidates;
        }
        return;
    }

    if !ambiguous_candidates.is_empty()
        && retro_junk_core::disc::candidates_are_same_game(&ambiguous_candidates).is_none()
    {
        entry.dat_match = None;
        entry.status = EntryStatus::Ambiguous;
        entry.ambiguous_candidates = ambiguous_candidates;
        return;
    }

    let names: Vec<_> = matched
        .iter()
        .map(|matched| matched.game_name.as_str())
        .collect();
    let first = matched[0];
    let all_hash_verified = !discs.is_empty()
        && discs.iter().all(|disc| {
            disc.disc_verification.permits_verified_status()
                && disc
                    .dat_match
                    .as_ref()
                    .is_some_and(|matched| !matches!(matched.method, MatchMethod::Serial))
        });
    entry.dat_match = Some(DatMatchInfo {
        game_name: retro_junk_core::disc::derive_base_game_name(&names),
        rom_name: first.rom_name.clone(),
        method: if all_hash_verified {
            first.method.clone()
        } else {
            MatchMethod::Serial
        },
        region: first.region.clone(),
        cross_region: matched.iter().any(|matched| matched.cross_region),
    });
    entry.status = if all_hash_verified {
        EntryStatus::Matched
    } else {
        EntryStatus::LikelyMatched
    };
    entry.ambiguous_candidates.clear();
}

/// Apply every successful checksum for one library entry as a unit. This is
/// shared by the durable snapshot and the optional live UI projection, so
/// navigation cannot change matching behavior.
fn apply_entry_hash_results(entry: &mut LibraryEntry, results: &[EntryHashResult]) {
    if entry.disc_identifications.is_none() {
        if let Some(result) = results.iter().find(|result| result.disc_path.is_none()) {
            entry.hashes = Some(result.hashes.clone());
            entry.disc_verification = result.disc_verification;
            apply_catalog_resolution(entry, &result.catalog_matches);
        }
        return;
    }

    if let Some(discs) = entry.disc_identifications.as_mut() {
        for result in results {
            let Some(disc_path) = result.disc_path.as_ref() else {
                continue;
            };
            let Some(disc) = discs.iter_mut().find(|disc| &disc.path == disc_path) else {
                continue;
            };
            disc.hashes = Some(result.hashes.clone());
            disc.disc_verification = result.disc_verification;
            match resolve_catalog_candidates(
                &result.catalog_matches,
                Some(&disc.identification),
                disc.hashes.as_ref(),
                disc.disc_verification,
            ) {
                CatalogUiResolution::Match {
                    info,
                    cover_title,
                    screen_title,
                } => {
                    disc.dat_match = Some(info);
                    disc.ambiguous_candidates.clear();
                    if entry.cover_title.is_empty() && !cover_title.is_empty() {
                        entry.cover_title = cover_title;
                    }
                    if entry.screen_title.is_empty() && !screen_title.is_empty() {
                        entry.screen_title = screen_title;
                    }
                }
                CatalogUiResolution::Ambiguous(candidates) => {
                    disc.dat_match = None;
                    disc.ambiguous_candidates = candidates;
                }
                CatalogUiResolution::NotFound => {
                    disc.dat_match = None;
                    disc.ambiguous_candidates.clear();
                }
            }
        }
    }
    apply_multi_disc_resolution(entry);
}

pub(crate) fn apply_single_analysis_result(
    entry: &mut LibraryEntry,
    result: Result<RomIdentification, AnalysisError>,
    catalog_matches: &[retro_junk_db::CatalogMediaMatch],
) {
    match result {
        Ok(identification) => {
            entry.identification = Some(identification);
            entry.disc_identifications = None;
            apply_catalog_resolution(entry, catalog_matches);
        }
        Err(_) => {
            entry.identification = None;
            entry.disc_identifications = None;
            apply_catalog_resolution(entry, catalog_matches);
            if entry.status == EntryStatus::Unknown {
                entry.status = EntryStatus::Unrecognized;
            }
        }
    }
}

pub(crate) fn apply_multi_disc_analysis_results(
    entry: &mut LibraryEntry,
    disc_results: &[(PathBuf, Result<RomIdentification, AnalysisError>)],
    catalog_matches: &[Vec<retro_junk_db::CatalogMediaMatch>],
) {
    let old_disc_data: HashMap<
        PathBuf,
        (
            Option<FileHashes>,
            Option<DatMatchInfo>,
            Vec<String>,
            DiscVerification,
        ),
    > = entry
        .disc_identifications
        .as_ref()
        .map(|discs| {
            discs
                .iter()
                .map(|disc| {
                    (
                        disc.path.clone(),
                        (
                            disc.hashes.clone(),
                            disc.dat_match.clone(),
                            disc.ambiguous_candidates.clone(),
                            disc.disc_verification,
                        ),
                    )
                })
                .collect()
        })
        .unwrap_or_default();

    let mut disc_ids = Vec::new();
    for ((path, result), candidates) in disc_results.iter().zip(catalog_matches) {
        match result {
            Ok(identification) => {
                let (cached_hashes, cached_dat_match, cached_candidates, cached_disc_verification) =
                    old_disc_data.get(path).cloned().unwrap_or_default();
                let mut disc = DiscIdentification {
                    path: path.clone(),
                    identification: identification.clone(),
                    hashes: cached_hashes,
                    dat_match: cached_dat_match,
                    ambiguous_candidates: cached_candidates,
                    disc_verification: cached_disc_verification,
                };
                match resolve_catalog_candidates(
                    candidates,
                    Some(&disc.identification),
                    disc.hashes.as_ref(),
                    disc.disc_verification,
                ) {
                    CatalogUiResolution::Match { info, .. } => {
                        disc.dat_match = Some(info);
                        disc.ambiguous_candidates.clear();
                    }
                    CatalogUiResolution::Ambiguous(candidates) => {
                        disc.dat_match = None;
                        disc.ambiguous_candidates = candidates;
                    }
                    CatalogUiResolution::NotFound => {
                        disc.dat_match = None;
                        disc.ambiguous_candidates.clear();
                    }
                }
                disc_ids.push(disc);
            }
            Err(error) => log::warn!("Disc analysis failed for {}: {error}", path.display()),
        }
    }

    let mut regions = Vec::new();
    for disc in &disc_ids {
        for region in &disc.identification.regions {
            if !regions.contains(region) {
                regions.push(*region);
            }
        }
    }
    let disc_files: Vec<_> = disc_results.iter().map(|(path, _)| path.clone()).collect();
    let mut game_id = RomIdentification::new();
    game_id.regions = regions;
    game_id
        .extra
        .insert("format".to_owned(), describe_m3u_format(&disc_files));
    game_id
        .extra
        .insert("disc_count".to_owned(), disc_results.len().to_string());
    entry.identification = Some(game_id);
    entry.disc_identifications = Some(disc_ids);
    entry.status = EntryStatus::Unrecognized;
    entry.dat_match = None;
    entry.ambiguous_candidates.clear();
    apply_multi_disc_resolution(entry);
}

// -- Message handler --

pub fn handle_message(app: &mut RetroJunkApp, msg: AppMessage, ctx: &egui::Context) {
    match msg {
        AppMessage::StartupStatus { status } => {
            app.ui_state.startup_status = status;
        }
        AppMessage::StartupReady { database } => {
            app.ui_state.startup_status = None;
            match database {
                Ok(connection) => {
                    app.catalog_db = Some(connection);
                    if let Some(path) = app.db_path.clone() {
                        let store_started = std::time::Instant::now();
                        match crate::backend::library_store::LibraryStore::start(path) {
                            Ok(store) => {
                                log::info!(
                                    "startup: library store ready in {:?}",
                                    store_started.elapsed()
                                );
                                app.library_store = Some(store);
                            }
                            Err(error) => app.push_error("Library database", error),
                        }
                    }
                }
                Err(error) => app.push_error("Catalog database", error),
            }
        }
        AppMessage::StartupRootReady {
            restored_root,
            fragile_mount_kind,
        } => {
            if let Some(root) = restored_root
                && app.root_path.is_none()
                && app.settings.library.current_root.as_ref() == Some(&root)
            {
                if let Some(kind) = fragile_mount_kind {
                    app.ui_state.fragile_mount_prompt = Some(FragileMountPrompt { root, kind });
                } else {
                    // Paint the committed projection; the filesystem walk runs
                    // only when this root has never been projected (decided in
                    // the RootOpened reply). Explicit refresh and rescans keep
                    // their own StartFolderScan sends.
                    app.root_path = Some(root.clone());
                    app.ui_state.loading_library = true;
                    app.pending_first_open_scan = true;
                    app.open_browser_root(&root, ctx);
                }
            }
        }
        AppMessage::StartArchiveRefresh { profile } => {
            let is_current = app
                .settings
                .library
                .active_profile()
                .is_some_and(|active| active.profile_id == profile.profile_id);
            let archive_busy = app
                .operations
                .iter()
                .any(|operation| operation.scope == "archive");
            if is_current && !archive_busy && app.catalog_db.is_some() {
                app.ui_state.archive_refresh_pending = false;
                crate::backend::archive::start_archive_operation(app, &profile, false);
            } else if is_current && archive_busy {
                app.ui_state.archive_refresh_pending = true;
            }
        }

        AppMessage::ArchiveOperationComplete { op_id, result } => {
            app.operations.retain(|operation| operation.id != op_id);
            if let Some(handle) = app.op_threads.remove(&op_id) {
                let _ = handle.join();
            }
            match result {
                Ok(message) => {
                    log::info!("{message}");
                    app.ui_state.collection_profile_id = None;
                    app.ui_state.collection_summaries = std::sync::Arc::new(Vec::new());
                    refresh_library_availability(app, ctx);
                }
                Err(error) => app.push_error("Archive operation", error),
            }
            if app.ui_state.archive_refresh_pending
                && let Some(profile) = app.settings.library.active_profile().cloned()
            {
                app.ui_state.archive_refresh_pending = false;
                let _ = app
                    .message_tx
                    .send(AppMessage::StartArchiveRefresh { profile });
            }
        }
        AppMessage::CollectionSummariesReady { profile_id, result } => {
            app.ui_state.collection_summaries_loading = false;
            let current = app
                .settings
                .library
                .active_profile()
                .map(|profile| profile.profile_id.to_string());
            if current.as_deref() != Some(profile_id.as_str()) {
                return;
            }
            match result {
                Ok(summaries) => {
                    app.ui_state.collection_profile_id = Some(profile_id);
                    app.ui_state.collection_summaries = std::sync::Arc::new(summaries);
                }
                Err(error) => app.push_error("Collection", error),
            }
        }
        AppMessage::CollectionEditorReady { release_id, result } => {
            if app.ui_state.collection_editor_loading.as_deref() != Some(release_id.as_str()) {
                return;
            }
            app.ui_state.collection_editor_loading = None;
            match result {
                Ok(editor) if editor.archive_release_id == release_id => {
                    app.ui_state.collection_editor = Some(editor);
                }
                Ok(_) => app.push_error(
                    "Collection details",
                    "Loaded details did not match the selected release".to_owned(),
                ),
                Err(error) => app.push_error("Collection details", error),
            }
        }
        AppMessage::PlayablePolicyUpdated { op_id, result } => {
            app.operations.retain(|operation| operation.id != op_id);
            if let Some(handle) = app.op_threads.remove(&op_id) {
                let _ = handle.join();
            }
            match result {
                Ok(manifest) => {
                    if let Some(profile) = app.settings.library.active_profile_mut() {
                        profile.platform_defaults = manifest.platform_defaults;
                    }
                    if let Err(error) = crate::settings::save_settings(&app.settings) {
                        app.push_error("Save playable policy", error.to_string());
                    }
                    app.ui_state.collection_profile_id = None;
                    app.ui_state.collection_summaries = std::sync::Arc::new(Vec::new());
                    refresh_library_availability(app, ctx);
                }
                Err(error) => app.push_error("Playable policy", error),
            }
        }
        AppMessage::PlayableBuildComplete { op_id, result } => {
            app.operations.retain(|operation| operation.id != op_id);
            if let Some(handle) = app.op_threads.remove(&op_id) {
                let _ = handle.join();
            }
            let more_archive_work = app
                .operations
                .iter()
                .any(|operation| operation.scope == "archive");
            match result {
                Ok(Some(output)) => {
                    log::info!("Built playable copy {}", output.display());
                    if more_archive_work {
                        log::info!("Deferring library refresh until queued archive work completes");
                        return;
                    }
                    app.ui_state.collection_profile_id = None;
                    app.ui_state.collection_summaries = std::sync::Arc::new(Vec::new());
                    refresh_library_availability(app, ctx);
                    let target = app
                        .browser
                        .consoles
                        .iter()
                        .position(|console| output.starts_with(&console.folder_path))
                        .and_then(|index| {
                            crate::backend::scan::ConsoleScanTarget::durable(app, index)
                        });
                    if let Some(target) = target {
                        app.ui_state.refresh_archive_after_console_scan =
                            Some(target.folder_name.clone());
                        crate::backend::scan::restart_console_scan(app, target, ctx);
                    } else {
                        // The archive projection already records the new
                        // playable representation. Folder discovery is only a
                        // fallback for a newly-created console destination.
                        let _ = app.message_tx.send(AppMessage::StartFolderScan);
                    }
                }
                Ok(None) => {
                    log::info!("Catalog-verified archived release");
                    if more_archive_work {
                        log::info!("Deferring library refresh until queued archive work completes");
                        return;
                    }
                    app.ui_state.collection_profile_id = None;
                    app.ui_state.collection_summaries = std::sync::Arc::new(Vec::new());
                    refresh_library_availability(app, ctx);
                }
                Err(error) => {
                    if !more_archive_work {
                        app.ui_state.collection_profile_id = None;
                        app.ui_state.collection_summaries = std::sync::Arc::new(Vec::new());
                        refresh_library_availability(app, ctx);
                        let _ = app.message_tx.send(AppMessage::StartFolderScan);
                    }
                    app.push_error("Archive action", error);
                }
            }
        }
        AppMessage::AssetProjectionComplete { op_id, result } => {
            app.operations.retain(|operation| operation.id != op_id);
            if let Some(handle) = app.op_threads.remove(&op_id) {
                let _ = handle.join();
            }
            match result {
                Ok(report) => {
                    log::info!(
                        "Restored {} archived media file(s); {} already current",
                        report.copied,
                        report.current
                    );
                    if let Some(console_id) = app.ui_state.selected_console {
                        app.request_console_page(console_id, ctx);
                    }
                }
                Err(error) => app.push_error("Restore archived media", error),
            }
        }
        AppMessage::ArchiveImportPlanReady { op_id, result } => {
            app.operations.retain(|operation| operation.id != op_id);
            if let Some(handle) = app.op_threads.remove(&op_id) {
                let _ = handle.join();
            }
            match result {
                Ok(plan) => {
                    app.ui_state.dump_import_dialog = Some(DumpImportDialogState::Review {
                        plan,
                        consume: false,
                        new_physical_copy: false,
                        make_playable: false,
                        discard_redundant_bin_cue: false,
                    });
                }
                Err(error) => {
                    app.ui_state.dump_import_dialog = None;
                    if !error.to_ascii_lowercase().contains("cancelled") {
                        app.push_error("Archive import planning", error);
                    }
                }
            }
        }
        AppMessage::ArchiveImportComplete { op_id, result } => {
            app.operations.retain(|operation| operation.id != op_id);
            if let Some(handle) = app.op_threads.remove(&op_id) {
                let _ = handle.join();
            }
            match result {
                Ok(result) => {
                    app.ui_state.collection_profile_id = None;
                    app.ui_state.collection_summaries = std::sync::Arc::new(Vec::new());
                    app.ui_state.dump_import_dialog =
                        Some(DumpImportDialogState::Complete { result });
                }
                Err(error) => {
                    app.ui_state.dump_import_dialog = None;
                    app.push_error("Archive import", error);
                }
            }
        }
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

            let populated_aliases = app
                .browser
                .consoles
                .iter()
                .filter(|console| {
                    std::fs::read_dir(&console.folder_path)
                        .is_ok_and(|mut entries| entries.next().is_some())
                })
                .map(|console| {
                    (
                        crate::backend::scan::projection_alias_key(
                            console.platform,
                            &console.folder_name,
                        ),
                        console.folder_path.clone(),
                    )
                })
                .collect::<Vec<_>>();
            let stale_aliases = app
                .browser
                .consoles
                .iter()
                .filter(|console| {
                    let alias = crate::backend::scan::projection_alias_key(
                        console.platform,
                        &console.folder_name,
                    );
                    let folder_is_empty_or_missing = std::fs::read_dir(&console.folder_path)
                        .map_or(true, |mut entries| entries.next().is_none());
                    folder_is_empty_or_missing
                        && populated_aliases.iter().any(|(populated_alias, path)| {
                            *populated_alias == alias && *path != console.folder_path
                        })
                })
                .filter_map(|console| console.id)
                .collect::<Vec<_>>();
            if !stale_aliases.is_empty() {
                app.browser
                    .consoles
                    .retain(|console| !console.id.is_some_and(|id| stale_aliases.contains(&id)));
                for id in stale_aliases {
                    app.browser.entry_counts.remove(&id);
                    app.browser.console_statuses.remove(&id);
                    app.browser.stale_consoles.remove(&id);
                    app.submit_store(
                        crate::backend::library_store::LibraryStoreRequest::DeleteConsole(id),
                        ctx,
                    );
                    if app.ui_state.selected_console == Some(id) {
                        app.ui_state.selected_console = None;
                        app.browser.active_page = None;
                    }
                }
            }
            log::info!(
                "Folder scan complete: {} consoles discovered",
                app.browser.consoles.len()
            );

            // Database-stale consoles are always rebuilt: their projections are
            // known invalid, so this is correctness repair rather than the
            // optional scan-new-folders preference.
            // Scans run sequentially via the pending_auto_scans queue to avoid
            // stampeding slow filesystems (e.g., network shares).
            for console in &app.browser.consoles {
                let database_stale = console
                    .id
                    .is_some_and(|id| app.browser.stale_consoles.contains(&id));
                if should_queue_auto_scan(
                    console.scan_status,
                    app.settings.general.auto_scan_on_open,
                    database_stale,
                ) && !app
                    .ui_state
                    .pending_auto_scans
                    .contains(&console.folder_name)
                {
                    app.ui_state
                        .pending_auto_scans
                        .push_back(console.folder_name.clone());
                }
            }
            if !app.ui_state.pending_auto_scans.is_empty() {
                start_auto_scan_batch(app, ctx);
            }
        }

        AppMessage::ScanProjectionInfo {
            folder_name,
            loose_disc_files,
            fingerprint,
        } => {
            if let Some(ci) = app.browser.find_by_folder(&folder_name) {
                app.browser.consoles[ci].loose_disc_files = loose_disc_files;
                app.browser.consoles[ci].fingerprint = Some(fingerprint);
            }
        }

        AppMessage::EntryAnalysisSnapshotsComplete {
            folder_name,
            entries,
        } => {
            log::debug!(
                "Persisting {} background analysis result(s) for {folder_name}",
                entries.len()
            );
            app.publish_entry_analysis_snapshots(&entries, ctx);
        }

        AppMessage::ConsoleScanFailed { folder_name, error } => {
            if let Some(ci) = app.browser.find_by_folder(&folder_name) {
                app.browser.consoles[ci].scan_status = ScanStatus::NotScanned;
                app.browser.consoles[ci].entries.clear();
                if app.ui_state.selected_console == app.browser.consoles[ci].id
                    && let Some(console_id) = app.browser.consoles[ci].id
                {
                    // Throw away any uncommitted scan projection and restore
                    // the last authoritative database page.
                    app.request_console_page(console_id, ctx);
                }
            }
            if let Some(error) = error {
                app.push_error("Library scan", error);
            }
            finish_auto_scan(app, &folder_name, false, ctx);
        }

        AppMessage::ScanSnapshotPrepared {
            folder_name,
            console_id,
            result,
        } => match result {
            Ok(snapshot) => {
                let count = snapshot.entries.len() as u64;
                if let Some(console_id) = console_id {
                    app.browser.entry_counts.insert(console_id, count);
                }
                app.commit_completed_scan(snapshot, ctx);
            }
            Err(error) => {
                log::warn!("Failed to publish {folder_name} scan: {error}");
                app.push_error("Library scan", error);
                finish_auto_scan(app, &folder_name, false, ctx);
            }
        },

        AppMessage::EntryHashBatchComplete {
            folder_name,
            entry,
            results,
        } => {
            let mut durable_entry = *entry;
            let entry_id = durable_entry.id;
            let expected_source_revision = durable_entry.source_revision;
            let mut updated_live_projection = false;
            if let Some(ci) = app.browser.find_by_folder(&folder_name)
                && let Some(entry_id) = entry_id
                && let Some(live) = app.browser.consoles[ci].entry_by_id_mut(entry_id)
                && live.source_revision == expected_source_revision
            {
                apply_entry_hash_results(live, &results);
                durable_entry = live.clone();
                updated_live_projection = true;
            }
            if !updated_live_projection {
                apply_entry_hash_results(&mut durable_entry, &results);
            }
            app.publish_entry_hash_update(&durable_entry, ctx);
        }

        AppMessage::HashFailed {
            folder_name,
            entry_id,
            entry_name,
            error,
        } => {
            log::warn!("Hash failed for {folder_name} entry {entry_id:?} ({entry_name}): {error}");
            app.push_error(
                "Hash Failed",
                format!("{entry_name} ({folder_name}): {error}"),
            );
        }

        AppMessage::AssetsLoaded {
            folder_name,
            entry_id,
            assets,
        } => {
            let (status, has_miximage) = asset_availability(&assets);
            app.browser.asset_statuses.insert(entry_id, status);
            if has_miximage {
                app.browser.entries_with_miximages.insert(entry_id);
            } else {
                app.browser.entries_with_miximages.remove(&entry_id);
            }

            // Bulk scraping can report hundreds of entries. Only the active
            // detail entry is allowed to retain paths and feed egui's loaders.
            if app.browser.detail_asset_entry == Some(entry_id)
                && let Some(ci) = app.browser.find_by_folder(&folder_name)
                && let Some(entry) = app.browser.consoles[ci].entry_by_id_mut(entry_id)
            {
                if let Some(ref old_assets) = entry.asset_paths {
                    for (at, old_path) in old_assets {
                        if assets.get(at) != Some(old_path) {
                            ctx.forget_image(&asset_image_uri(old_path));
                        }
                    }
                }
                entry.asset_paths = Some(assets);
            }
            app.browser.asset_discovery_in_flight.remove(&entry_id);
        }

        AppMessage::AssetStatusesLoaded {
            console_id,
            statuses,
        } => {
            if app.ui_state.selected_console == Some(console_id) {
                let active_ids: HashSet<_> = app
                    .browser
                    .active_page
                    .as_ref()
                    .filter(|page| page.console_id == console_id)
                    .into_iter()
                    .flat_map(|page| page.rows.iter().map(|row| row.id))
                    .collect();
                for (entry_id, status, has_miximage) in statuses {
                    if !active_ids.contains(&entry_id) {
                        continue;
                    }
                    app.browser.asset_statuses.insert(entry_id, status);
                    if has_miximage {
                        app.browser.entries_with_miximages.insert(entry_id);
                    } else {
                        app.browser.entries_with_miximages.remove(&entry_id);
                    }
                }
            }
        }

        AppMessage::ScrapeEntryFailed {
            folder_name,
            entry_id,
            entry_name,
            error,
        } => {
            log::warn!("Scrape failed for {folder_name} entry {entry_name}: {error}");
            app.push_error(
                "Scrape Failed",
                format!("{entry_name} ({folder_name}): {error}"),
            );
            if app.browser.detail_asset_entry == Some(entry_id)
                && let Some(ci) = app.browser.find_by_folder(&folder_name)
                && let Some(entry) = app.browser.consoles[ci].entry_by_id_mut(entry_id)
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

        AppMessage::MiximageComplete {
            generated,
            failures,
        } => {
            if generated > 0 {
                log::info!("Generated {generated} miximage(s)");
            }
            if !failures.is_empty() {
                app.push_error("Generate miximage", failures.join("\n"));
            }
        }

        AppMessage::ArchiveAssetsChanged => {
            app.ui_state.collection_profile_id = None;
            app.ui_state.collection_summaries = std::sync::Arc::new(Vec::new());
            if let Some(profile) = app.settings.library.active_profile().cloned() {
                let _ = app
                    .message_tx
                    .send(AppMessage::StartArchiveRefresh { profile });
            } else {
                refresh_library_availability(app, ctx);
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
                log::info!("Exported gamelist.xml for {folder_name}: {path}");
            }
            Err(error) => {
                log::warn!("Export failed for {folder_name}: {error}");
                app.push_error("Export Failed", format!("{folder_name}: {error}"));
            }
        },

        AppMessage::RenameComplete {
            folder_name,
            rescan_target,
            results,
        } => {
            let mut renamed = 0usize;
            let mut already = 0usize;
            let mut failed = 0usize;
            for r in &results {
                log::debug!("Rename result for library entry {}", r.entry_id.0);
                match &r.outcome {
                    RenameOutcome::Renamed { .. } => renamed += 1,
                    RenameOutcome::M3uRenamed {
                        discs_renamed,
                        errors: m3u_errors,
                        ..
                    } => {
                        renamed += discs_renamed;
                        failed += m3u_errors.len();
                    }
                    RenameOutcome::AlreadyCorrect => already += 1,
                    RenameOutcome::NoMatch { .. } | RenameOutcome::Error { .. } => {
                        failed += 1;
                    }
                }
            }
            log::info!(
                "Rename {folder_name}: {renamed} renamed, {already} already correct, {failed} failed"
            );
            app.ui_state.results_dialog = crate::app::ResultsDialog::Rename(results);

            if renamed > 0 {
                if let Some(target) = rescan_target {
                    crate::backend::scan::restart_console_scan(app, target, ctx);
                } else {
                    app.push_error(
                        "Library rescan",
                        format!("No durable console ID for {folder_name}"),
                    );
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
            rescan_target,
            jobs_executed,
            files_moved,
            unmatched,
            errors,
        } => {
            if jobs_executed > 0 {
                log::info!(
                    "Organized {folder_name}: created {jobs_executed} folders, moved {files_moved} files",
                );
                if let Some(target) = rescan_target {
                    crate::backend::scan::restart_console_scan(app, target, ctx);
                } else {
                    app.push_error(
                        "Library rescan",
                        format!("No durable console ID for {folder_name}"),
                    );
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
            rescan_target,
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
            if fixed > 0 {
                if let Some(target) = rescan_target {
                    crate::backend::scan::restart_console_scan(app, target, ctx);
                } else {
                    app.push_error(
                        "Library rescan",
                        format!("No durable console ID for {folder_name}"),
                    );
                }
            }
            app.ui_state.results_dialog = crate::app::ResultsDialog::CueFix(results);
        }

        AppMessage::ChdCompressComplete {
            folder_name,
            rescan_target,
            results,
        } => {
            let compressed = results
                .iter()
                .filter(|r| matches!(r.outcome, ChdCompressOutcome::Compressed { .. }))
                .count();
            let failed = results.len() - compressed;
            for result in &results {
                log::debug!(
                    "CHD result for {} -> {}",
                    result.job.input.display(),
                    result.job.output.display()
                );
            }
            log::info!(
                "CHD compression {folder_name}: {compressed} compressed, {failed} failed/skipped"
            );

            if compressed > 0 {
                if let Some(target) = rescan_target {
                    crate::backend::scan::restart_console_scan(app, target, ctx);
                } else {
                    app.push_error(
                        "Library rescan",
                        format!("No durable console ID for {folder_name}"),
                    );
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

        AppMessage::OperationPhase {
            op_id,
            description,
            display,
            current,
            total,
        } => {
            if let Some(op) = app.operations.iter_mut().find(|op| op.id == op_id) {
                op.description = description;
                op.display = display;
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

        AppMessage::ModSearchResults { query, result } => {
            if let crate::state::TagDialog::ModSearch {
                query: current,
                results,
                selected,
                ..
            } = &mut app.ui_state.tag_dialog
                && *current == query
            {
                *selected = None;
                match result {
                    Ok(rows) => *results = rows,
                    Err(error) => app.push_error("Catalog search", error),
                }
            }
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

fn refresh_library_availability(app: &mut RetroJunkApp, ctx: &egui::Context) {
    app.library_controller.invalidate_lists();
    app.browser.active_page = None;
    app.refresh_console_summaries(ctx);
    if let Some(console_id) = app.ui_state.selected_console {
        app.request_console_page(console_id, ctx);
    }
}

fn should_queue_auto_scan(
    status: ScanStatus,
    auto_scan_on_open: bool,
    database_stale: bool,
) -> bool {
    status == ScanStatus::NotScanned && (auto_scan_on_open || database_stale)
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
    pub deduplication_report: Option<retro_junk_db::CatalogDeduplicationReport>,
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
            deduplication_report: None,
        }
    }
}
