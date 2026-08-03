#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use retro_junk_frontend::AssetType;

// -- Asset status --

pub use retro_junk_backend::assets::{AssetStatus, SCRAPEABLE_ASSET_TYPES, asset_availability};

/// URI used by egui's file loader. Unlike `include_bytes`, this lets us evict
/// all decoded data and textures when the detail selection changes.
pub fn asset_image_uri(path: &Path) -> String {
    format!("file://{}", path.display())
}
use retro_junk_lib::Platform;
#[cfg(test)]
use retro_junk_lib::Region;

use crate::app::RetroJunkApp;

// -- Navigation --

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Library,
    Collection,
    Inbox,
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
    /// Consoles explicitly marked stale by `SQLite` and requiring a correctness rebuild.
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

// The library-entry domain model — `LibraryEntry`, its statuses, catalog
// resolution, and hash-result application — lives in the backend so scan and
// hash operations share one implementation. Re-exported here so existing
// `crate::state::` paths keep working.
pub use retro_junk_backend::library::{
    DatMatchInfo, DiscIdentification, DiscVerification, EntryHashResult, EntryStatus, LibraryEntry,
    apply_entry_hash_results,
};
#[cfg(test)]
pub(crate) use retro_junk_backend::library::{apply_catalog_resolution, regions_match_dat};

/// Badge color for an entry status.
///
/// An extension trait because [`EntryStatus`] lives in the backend crate,
/// which must stay free of egui; only the presentation of a status belongs
/// here.
pub trait EntryStatusColor {
    fn color(self) -> egui::Color32;
}

impl EntryStatusColor for EntryStatus {
    fn color(self) -> egui::Color32 {
        match self {
            EntryStatus::Unknown => egui::Color32::GRAY,
            EntryStatus::Unrecognized => crate::theme::STATUS_ERR,
            EntryStatus::Ambiguous => crate::theme::STATUS_WARN,
            EntryStatus::LikelyMatched => crate::theme::STATUS_INFO,
            EntryStatus::Matched => crate::theme::STATUS_OK,
            EntryStatus::Tagged(_) => egui::Color32::from_rgb(100, 150, 220),
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

// -- Review inbox --

/// How the inbox is currently being filtered, sorted, and browsed.
///
/// Kept apart from the loaded rows so a reload — which happens after every
/// decision — does not throw away the filter someone typed or collapse the
/// group they were working through.
#[derive(Default)]
pub struct InboxUiState {
    /// The path pattern in the filter box.
    pub filter_text: String,
    /// The kind chip currently selected, if any.
    pub filter_kind: Option<String>,
    pub sort: crate::backend::inbox::InboxSort,
    /// Groups the user has folded away. Collapsed state is remembered by
    /// group name so it survives reloads and re-sorts.
    pub collapsed: std::collections::HashSet<String>,
    /// Rows showing their full detail.
    pub expanded: std::collections::HashSet<i64>,
    /// The row the keyboard is on.
    pub cursor: Option<i64>,
    /// Scroll the cursor into view on the next frame, after a key moved it.
    pub scroll_to_cursor: bool,
    /// What the last bulk dismissal closed, so it can be put back. Cleared
    /// when the user acts again — an undo that reaches back through several
    /// decisions would be a worse promise than none.
    pub undo: Option<InboxUndo>,
    /// A bulk action waiting for confirmation.
    pub confirm: Option<InboxConfirm>,
    /// An ignore rule being written.
    pub ignore_draft: Option<InboxIgnoreDraft>,
    /// A review whose candidates are being chosen between.
    pub choice: Option<InboxChoice>,
    /// Whether the ignore-rule list is open.
    pub show_ignore_rules: bool,
}

/// What a bulk dismissal closed.
pub struct InboxUndo {
    pub ids: Vec<i64>,
    pub label: String,
}

/// A bulk action the user has asked for but not yet confirmed.
pub struct InboxConfirm {
    pub kind: InboxConfirmKind,
    pub ids: Vec<i64>,
    /// What the filter said, for the confirmation to quote back.
    pub description: String,
}

pub enum InboxConfirmKind {
    Dismiss,
    Apply,
}

/// An ignore rule being written, pre-filled from the current filter.
///
/// How many reviews it covers is deliberately not stored: the pattern is
/// editable in the dialog, so a count captured when it opened would go stale
/// the moment someone typed — and this is the one dialog whose whole job is to
/// say truthfully what the button is about to do.
pub struct InboxIgnoreDraft {
    pub pattern: String,
    pub note: String,
}

/// A review with several candidates, waiting for one to be chosen.
pub struct InboxChoice {
    pub id: i64,
    pub label: String,
    pub candidates: Vec<retro_junk_backend::AdoptionCandidate>,
    pub selected: Option<usize>,
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

// Re-export shim: these moved to the backend; keep them visible at the old
// path for the app-level results dialog and completion handlers.
pub use retro_junk_backend::ops::rename::{RenameOutcome, RenameResult};

// -- CUE fix results --

pub use retro_junk_backend::ops::fix_cue::{CueFixOutcome, CueFixResult};

// -- CHD compression --

// Re-export shim: these moved to the backend; keep the whole set visible at
// the old path (`state` is a private module, so members only tests or dialogs
// touch would otherwise warn as unused).
#[allow(unused_imports)]
pub use retro_junk_backend::ops::chd_compress::{
    ChdCompressItem, ChdCompressOutcome, ChdCompressPrompt, ChdCompressResult, ChdCompressSkip,
};

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

impl ProgressDisplay {
    /// How to render what a running operation just reported.
    ///
    /// The operation says whether its numbers are bytes or work items, so this
    /// never has to guess. Guessing was a real bug: identifying a single disc
    /// reports "0 of 1 dumps", and reading that as bytes rendered "0 B / 1 B"
    /// beside a progress bar that then sat still for the several minutes the
    /// disc actually took.
    #[must_use]
    pub fn for_report(unit: retro_junk_io::ProgressUnit, total: u64) -> Self {
        match (total, unit) {
            // A byte count with a real total is the only case that renders as
            // "412 MB / 1.1 GB"; everything else, zero total included, reads
            // better as a plain fraction.
            (0, _) | (_, retro_junk_io::ProgressUnit::Items) => Self::Count,
            (_, retro_junk_io::ProgressUnit::Bytes) => Self::Bytes,
        }
    }
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
    /// Backlog summary + open errors for one scope (B5/B4). The scope rides
    /// along because the user may have moved on while the query ran; the
    /// handler must not file one console's backlog under another's name.
    BacklogReady {
        scope: retro_junk_db::convergence::Scope,
        result: Result<crate::backend::convergence::Backlog, String>,
    },
    /// Loaded review-inbox contents.
    InboxReady {
        result: Result<crate::backend::inbox::InboxContents, String>,
    },
    /// Something resolved or created a reviewable item; reload the inbox.
    InboxChanged,
    /// A dismissal closed exactly these rows, so the view can offer to put
    /// exactly them back.
    InboxDismissed {
        ids: Vec<i64>,
    },
    /// Result of a `ScreenScraper` "Test login" attempt.
    ScraperLoginTested {
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

/// Move each renamed row's library identity to its new path.
///
/// Library entries are keyed by path, so a rename plus a rescan looks exactly
/// like "one file vanished, another appeared" — and the new row starts with no
/// digests, no DAT match, and no identification. A rename cannot change
/// content, so the identity follows the file instead of being re-derived.
///
/// Best-effort: a row that cannot be re-keyed (its destination already exists,
/// or the path is outside the console) simply gets re-read by the rescan, which
/// is the old behaviour.
fn carry_identity_across_renames(
    app: &crate::app::RetroJunkApp,
    target: &crate::backend::scan::ConsoleScanTarget,
    results: &[RenameResult],
) {
    let (Some(console_id), Some(conn)) = (target.console_id, app.catalog_db.as_ref()) else {
        return;
    };
    let key = |path: &std::path::Path, directory: bool| {
        let relative = path.strip_prefix(&target.folder_path).ok()?;
        let key = if directory {
            retro_junk_db::set_source_key(relative)
        } else {
            retro_junk_db::file_source_key(relative)
        };
        key.ok().map(|value| value.as_str().to_owned())
    };
    for result in results {
        let (from, to, directory) = match &result.outcome {
            RenameOutcome::Renamed { source, target } => (source, target, false),
            RenameOutcome::M3uRenamed {
                source_folder,
                target_folder,
                ..
            } => (source_folder, target_folder, true),
            _ => continue,
        };
        let (Some(old_key), Some(new_key)) = (key(from, directory), key(to, directory)) else {
            continue;
        };
        match retro_junk_db::rekey_library_entry(conn, console_id, &old_key, &new_key) {
            Ok(true) => log::debug!("Carried library identity {old_key} -> {new_key}"),
            Ok(false) => {}
            Err(error) => log::warn!("Could not carry library identity for {old_key}: {error}"),
        }
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
                    app.notify(message);
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
                    app.notify(format!(
                        "Built {}",
                        output.file_name().unwrap_or_default().to_string_lossy()
                    ));
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
                    app.notify("Catalog-verified archived release");
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
                    if report.copied > 0 {
                        app.notify(format!("Restored {} archived media file(s)", report.copied));
                    }
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
                app.notify(format!("Generated {generated} miximage(s)"));
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
                app.notify(format!("Exported gamelist.xml for {folder_name}"));
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
            // Carry each renamed row's identity to its new path *before* the
            // rescan. Entries are keyed by path, so without this the rescan
            // meets the new name as a file it has never seen and the row comes
            // back with no digests and no DAT match — asking the user to
            // re-read bytes a rename cannot have changed.
            if let Some(target) = rescan_target.as_ref() {
                carry_identity_across_renames(app, target, &results);
            }
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
                // Organize is the one batch operation with no results
                // dialog, so the toast is its only completion surface.
                app.notify(format!(
                    "Organized {folder_name}: {jobs_executed} folders, {files_moved} files moved"
                ));
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

        AppMessage::BacklogReady { scope, result } => {
            app.ui_state.backlog_loading = false;
            // The user may have switched consoles while this query ran; the
            // load for the new scope was skipped because this one was in
            // flight. Storing the reply anyway would label one console's
            // backlog with another's name — discard it and run the load the
            // current scope is still owed.
            if app.ui_state.backlog_scope.as_ref() != Some(&scope) {
                if let Some(current) = app.ui_state.backlog_scope.clone() {
                    crate::backend::convergence::load_backlog(app, current, ctx);
                }
                return;
            }
            match result {
                Ok(backlog) => {
                    app.ui_state.open_suggestion_count = backlog.summary.open_suggestions;
                    app.ui_state.backlog = backlog;
                }
                // A failed derivation is a projection problem, not a user
                // action: log it and keep the last good backlog rather than
                // interrupting with a modal.
                Err(error) => log::warn!("convergence backlog unavailable: {error}"),
            }
        }

        AppMessage::InboxReady { result } => {
            app.ui_state.inbox_loading = false;
            match result {
                Ok(contents) => {
                    app.ui_state.open_suggestion_count = contents.items.len() as u64;
                    // A row that resolved while the cursor was on it must not
                    // leave the keyboard pointing at nothing.
                    if let Some(cursor) = app.ui_state.inbox_ui.cursor
                        && !contents
                            .items
                            .iter()
                            .any(|item| item.suggestion.id == cursor)
                    {
                        app.ui_state.inbox_ui.cursor = None;
                    }
                    app.ui_state.inbox = contents;
                }
                Err(error) => log::warn!("inbox unavailable: {error}"),
            }
        }

        AppMessage::InboxDismissed { ids } => {
            if ids.is_empty() {
                app.ui_state.inbox_ui.undo = None;
            } else {
                let label = if ids.len() == 1 {
                    "Dismissed 1 review".to_owned()
                } else {
                    format!("Dismissed {} reviews", ids.len())
                };
                app.ui_state.inbox_ui.undo = Some(crate::state::InboxUndo { ids, label });
            }
        }

        AppMessage::InboxChanged => {
            app.ui_state.inbox_dirty = true;
        }

        AppMessage::ScraperLoginTested { result } => {
            if let Some(account) = app.ui_state.scraper_account.as_mut() {
                account.test = match result {
                    Ok(summary) => LoginTest::Ok(summary),
                    Err(error) => LoginTest::Failed(error),
                };
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
    if let Some(connection) = app.catalog_db.as_ref() {
        app.ui_state.open_suggestion_count =
            retro_junk_db::work::list_open_suggestions(connection, None)
                .map_or(0, |open| open.len() as u64);
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

/// `ScreenScraper` account fields being edited in Settings, plus the state of
/// the last login test.
///
/// The values start from whatever `Credentials::load` resolves, so the
/// fields show what scraping would actually use — including a value coming
/// from an environment variable rather than the config file.
pub struct ScraperAccount {
    pub user_id: String,
    pub user_password: String,
    pub test: LoginTest,
}

impl ScraperAccount {
    #[must_use]
    pub fn load() -> Self {
        let credentials = retro_junk_scraper::Credentials::load().ok();
        Self {
            user_id: credentials
                .as_ref()
                .map(|credentials| credentials.user_id.clone())
                .unwrap_or_default(),
            user_password: credentials
                .map(|credentials| credentials.user_password)
                .unwrap_or_default(),
            test: LoginTest::Idle,
        }
    }
}

/// Outcome of the most recent "Test login".
pub enum LoginTest {
    Idle,
    Running,
    Ok(String),
    Failed(String),
}
