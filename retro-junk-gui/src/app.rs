#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "id_stability_tests.rs"]
mod id_stability_tests;

use std::collections::HashMap;
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use retro_junk_lib::AnalysisContext;

use crate::settings::AppSettings;
use crate::state::{
    AppMessage, BackgroundOperation, CueFixOutcome, CueFixResult, FocusedPanel,
    LibraryBrowserState, RenameOutcome, RenameResult, ToolsState, View,
};
use crate::util;
use crate::views;
use crate::widgets;

/// Which batch-results dialog is open, if any. The three result kinds are
/// mutually exclusive by construction: at most one dialog at a time.
pub enum ResultsDialog {
    /// No results dialog is open.
    None,
    /// Results from the last rename operation.
    Rename(Vec<RenameResult>),
    /// Results from the last CUE fix operation.
    CueFix(Vec<CueFixResult>),
    /// Results from the last CHD compression.
    ChdCompress(Vec<crate::state::ChdCompressResult>),
}

/// State of the Settings-view chdman detection probe.
///
/// `Chdman::detect` spawns a subprocess with no timeout, so it runs on a
/// background thread (D1); `Probing` guards against launching a second probe
/// while one is already in flight and drives the Settings-view spinner.
pub enum ChdmanProbe {
    /// No probe has run for the current setting (or it was invalidated).
    Idle,
    /// A probe is running on a background thread.
    Probing,
    /// A probe finished for `path` (the configured chdman path it ran against).
    Done {
        path: String,
        result: Result<
            retro_junk_lib::chd_convert::Chdman,
            retro_junk_lib::chd_convert::ChdmanUnavailable,
        >,
    },
}

/// Ephemeral state used to render and interact with the UI.
///
/// Keeping this separate from [`RetroJunkApp`]'s library, database, and
/// background-operation state makes the boundary between application data and
/// per-session UI state explicit.
pub struct UiState {
    /// Current sidebar navigation selection.
    pub current_view: View,
    /// Durable identity of the currently selected console.
    pub selected_console: Option<retro_junk_db::LibraryConsoleId>,
    /// One-shot request to scroll the console tree to an index.
    pub scroll_to_console: Option<usize>,
    /// Durable identity of the focused entry.
    pub focused_entry: Option<retro_junk_db::LibraryEntryId>,
    /// Durable entry identities for multi-select.
    pub selected_entries: HashSet<retro_junk_db::LibraryEntryId>,
    /// Text filter for the game table.
    pub filter_text: String,
    /// Offset of the active 300-row SQL page.
    pub page_offset: u64,
    /// Whether the detail panel is visible.
    pub detail_panel_open: bool,
    /// Which batch-results dialog is open, if any.
    pub results_dialog: ResultsDialog,
    /// Pending CHD compression awaiting user confirmation.
    pub chd_compress_prompt: Option<crate::state::ChdCompressPrompt>,
    /// Cached chdman detection for the Settings view.
    pub chdman_probe: ChdmanProbe,
    /// Cached ScreenScraper credential provenance for the Settings view.
    pub credential_status: Option<(std::time::Instant, retro_junk_scraper::CredentialSources)>,
    /// Credential field whose explanation popup is open, if any.
    pub credential_info_popup: Option<&'static retro_junk_scraper::CredentialFieldMeta>,
    /// True while the initial cache load is in flight on startup.
    pub loading_library: bool,
    /// Present while startup database work blocks reliable application use.
    pub startup_status: Option<String>,
    /// Transient state for the Tools (catalog) view.
    pub tools_state: ToolsState,
    /// Which panel currently has keyboard focus for arrow-key navigation.
    pub focused_panel: FocusedPanel,
    /// One-shot request to scroll the game table to a filtered row index.
    pub scroll_to_row: Option<usize>,
    /// State for the homebrew/modded tagging dialog.
    pub tag_dialog: crate::state::TagDialog,
    /// Pending root switch awaiting fragile-mount confirmation.
    pub fragile_mount_prompt: Option<crate::state::FragileMountPrompt>,
    /// State for the log viewer panel.
    pub log_viewer: crate::widgets::log_viewer::LogViewerState,
    /// Accumulated errors to show in the error dialog.
    pub error_list: Vec<crate::state::UserError>,
    /// Pending organize plan awaiting user confirmation.
    pub pending_organize_plan: Option<(String, retro_junk_lib::organize::OrganizePlan)>,
    /// Folder names queued for serialized auto-scan.
    pub pending_auto_scans: VecDeque<String>,
    /// Folder name of the queued auto-scan currently in flight.
    pub auto_scan_in_flight: Option<String>,
    /// Activity-bar operation id for the auto-scan batch.
    pub auto_scan_op_id: Option<u64>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            current_view: View::Library,
            selected_console: None,
            scroll_to_console: None,
            focused_entry: None,
            selected_entries: HashSet::new(),
            filter_text: String::new(),
            page_offset: 0,
            detail_panel_open: true,
            results_dialog: ResultsDialog::None,
            chd_compress_prompt: None,
            chdman_probe: ChdmanProbe::Idle,
            credential_status: None,
            credential_info_popup: None,
            loading_library: false,
            startup_status: None,
            tools_state: ToolsState::default(),
            focused_panel: FocusedPanel::default(),
            scroll_to_row: None,
            tag_dialog: crate::state::TagDialog::None,
            fragile_mount_prompt: None,
            log_viewer: crate::widgets::log_viewer::LogViewerState::default(),
            error_list: Vec::new(),
            pending_organize_plan: None,
            pending_auto_scans: VecDeque::new(),
            auto_scan_in_flight: None,
            auto_scan_op_id: None,
        }
    }
}

/// Main application state.
pub struct RetroJunkApp {
    /// Analysis context with all registered console analyzers.
    pub context: Arc<AnalysisContext>,

    /// Root path for the ROM library.
    pub root_path: Option<std::path::PathBuf>,

    /// Per-run ROM library state used by the GUI between frames.
    pub browser: LibraryBrowserState,

    /// Active background operations (shown in activity bar).
    pub operations: Vec<BackgroundOperation>,

    /// Receiver for messages from background threads.
    pub message_rx: mpsc::Receiver<AppMessage>,

    /// Sender cloned into background threads.
    pub message_tx: mpsc::Sender<AppMessage>,

    /// Ephemeral state used only by the UI.
    pub ui_state: UiState,

    /// Persistent settings (library roots, preferences).
    pub settings: AppSettings,

    /// Catalog connection used for enrichment and catalog-management screens.
    /// `None` only if the database file could not be opened.
    pub catalog_db: Option<retro_junk_db::Connection>,

    /// Path to the catalog database file (for opening separate connections in background threads).
    pub db_path: Option<std::path::PathBuf>,

    /// Serialized owner of all revisioned library-table reads and writes.
    pub library_store: Option<crate::backend::library_store::LibraryStore>,

    /// Rejects stale/superseded projections and tracks durable UI identity.
    pub library_controller: crate::backend::library_store::LibraryProjectionController,

    next_store_request_id: u64,
    pending_page_request: Option<u64>,
    pending_details_request: Option<u64>,
    pending_all_hashes_request: Option<(u64, retro_junk_db::LibraryConsoleId)>,
    pending_filesystem_writes: HashMap<u64, retro_junk_db::LibraryConsoleId>,

    /// `JoinHandle`s for every spawned background-operation thread, keyed by
    /// `op_id`. Joined (and removed) when the operation completes, or all at
    /// once in `on_exit` so the process never dies mid-write (D2).
    pub op_threads: HashMap<u64, std::thread::JoinHandle<()>>,
}

impl RetroJunkApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings = crate::settings::load_settings();

        let db_path = retro_junk_dat::cache::cache_dir()
            .ok()
            .map(|p| p.join("catalog.db"));
        let mut app = Self::with_parts(&cc.egui_ctx, settings, None, None);
        app.db_path = db_path.clone();
        app.ui_state.startup_status = Some("Opening and migrating the catalog…".to_owned());

        // Schema migration, legacy import, and saved-root probes can all be
        // slow. Run them after constructing the app so the first frame is not
        // held hostage by database size or a disconnected network mount.
        let tx = app.message_tx.clone();
        let ctx = cc.egui_ctx.clone();
        let analysis_context = app.context.clone();
        let configured_root = app.settings.library.current_root.clone();
        std::thread::spawn(move || {
            let mut database = db_path
                .as_ref()
                .ok_or_else(|| "Could not determine the catalog database path".to_owned())
                .and_then(|path| retro_junk_db::open_database(path).map_err(|e| e.to_string()));
            let mut restored_root = None;
            let mut fragile_mount_kind = None;
            if let Some(root) = configured_root {
                log::info!("Settings current_root: {}", root.display());
                if root.is_dir() {
                    fragile_mount_kind = crate::util::fragile_mount_kind(&root);
                    if fragile_mount_kind.is_none()
                        && let Ok(conn) = database.as_mut()
                    {
                        crate::cache::migrate_json_cache(conn, &root, analysis_context.as_ref());
                    }
                    restored_root = Some(root);
                } else {
                    log::warn!("current_root is not a directory, skipping auto-load");
                }
            }
            let _ = tx.send(crate::state::AppMessage::StartupReady {
                database,
                restored_root,
                fragile_mount_kind,
            });
            ctx.request_repaint();
        });

        app
    }

    /// Build the app from explicit parts, touching neither settings on disk
    /// nor the catalog database. `new` layers the disk I/O and library
    /// restore on top; GUI tests call this directly for a hermetic instance.
    pub(crate) fn with_parts(
        egui_ctx: &egui::Context,
        settings: AppSettings,
        catalog_db: Option<retro_junk_db::Connection>,
        db_path: Option<std::path::PathBuf>,
    ) -> Self {
        egui_extras::install_image_loaders(egui_ctx);
        crate::fonts::configure_fonts(egui_ctx);
        let (tx, rx) = mpsc::channel();
        let context = Arc::new(retro_junk_lib::create_default_context());
        let library_store =
            db_path.clone().and_then(
                |path| match crate::backend::library_store::LibraryStore::start(path) {
                    Ok(store) => Some(store),
                    Err(error) => {
                        log::warn!("Failed to start library store: {error}");
                        None
                    }
                },
            );

        Self {
            context,
            root_path: None,
            browser: LibraryBrowserState::default(),
            operations: Vec::new(),
            message_rx: rx,
            message_tx: tx,
            ui_state: UiState::default(),
            settings,
            catalog_db,
            db_path,
            library_store,
            library_controller: Default::default(),
            next_store_request_id: 0,
            pending_page_request: None,
            pending_details_request: None,
            pending_all_hashes_request: None,
            pending_filesystem_writes: HashMap::new(),
            op_threads: HashMap::new(),
        }
    }

    /// Drain all pending messages from background threads.
    fn process_pending_messages(&mut self, ctx: &egui::Context) {
        const MAX_MESSAGES_PER_FRAME: usize = 64;
        const MESSAGE_BUDGET: Duration = Duration::from_millis(4);
        self.process_store_replies(ctx, MAX_MESSAGES_PER_FRAME);
        let started = std::time::Instant::now();
        let mut processed = 0;
        while processed < MAX_MESSAGES_PER_FRAME && started.elapsed() < MESSAGE_BUDGET {
            let Ok(msg) = self.message_rx.try_recv() else {
                return;
            };
            crate::state::handle_message(self, msg, ctx);
            processed += 1;
        }
        // There may be more work. Yield to layout/paint first, then promptly
        // schedule another frame instead of starving the event loop.
        ctx.request_repaint();
    }

    pub fn submit_store(
        &mut self,
        payload: crate::backend::library_store::LibraryStoreRequest,
        ctx: &egui::Context,
    ) {
        self.queue_store(payload);
        ctx.request_repaint_after(Duration::from_millis(20));
    }

    fn queue_store(
        &mut self,
        payload: crate::backend::library_store::LibraryStoreRequest,
    ) -> Option<u64> {
        let Some(store) = self.library_store.as_ref() else {
            return None;
        };
        self.next_store_request_id = self.next_store_request_id.wrapping_add(1);
        let request_id = self.next_store_request_id;
        let _ = store.submit(crate::backend::library_store::StoreEnvelope {
            session_generation: self.library_controller.session_generation,
            request_id,
            payload,
        });
        Some(request_id)
    }

    pub fn open_browser_root(&mut self, root: &std::path::Path, ctx: &egui::Context) {
        self.submit_store(
            crate::backend::library_store::LibraryStoreRequest::OpenRoot(
                root.to_string_lossy().into_owned(),
            ),
            ctx,
        );
    }

    fn process_store_replies(&mut self, ctx: &egui::Context, limit: usize) {
        let mut replies = Vec::new();
        if let Some(store) = self.library_store.as_ref() {
            while replies.len() < limit
                && let Ok(reply) = store.try_recv()
            {
                replies.push(reply);
            }
        }
        for reply in replies {
            if reply.session_generation != self.library_controller.session_generation {
                continue;
            }
            let request_id = reply.request_id;
            let filesystem_console = self.pending_filesystem_writes.remove(&request_id);
            match reply.payload {
                Err(error) => {
                    if self
                        .pending_all_hashes_request
                        .is_some_and(|(pending, _)| pending == request_id)
                    {
                        self.pending_all_hashes_request = None;
                    }
                    if filesystem_console.is_some() {
                        self.push_error(
                            "Library database",
                            format!(
                                "The filesystem changed but its database transition failed: {error}. The console was marked for rescan."
                            ),
                        );
                    } else {
                        self.push_error("Library database", error);
                    }
                }
                Ok(crate::backend::library_store::LibraryStoreValue::RootOpened {
                    root_id,
                    summaries,
                }) => {
                    self.browser.root_id = Some(root_id);
                    self.merge_console_summaries(summaries);
                    let missing: Vec<_> = self
                        .browser
                        .consoles
                        .iter()
                        .filter(|console| console.id.is_none())
                        .filter_map(|console| {
                            let platform = serde_json::to_string(&console.platform).ok()?;
                            Some(retro_junk_db::LibraryConsoleDescriptor {
                                root_id,
                                platform: platform.trim_matches('"').to_owned(),
                                folder_name: console.folder_name.clone(),
                                folder_path: console.folder_path.to_string_lossy().into_owned(),
                            })
                        })
                        .collect();
                    for descriptor in missing {
                        self.submit_store(
                            crate::backend::library_store::LibraryStoreRequest::EnsureConsole(
                                descriptor,
                            ),
                            ctx,
                        );
                    }
                    self.ui_state.loading_library = false;
                }
                Ok(crate::backend::library_store::LibraryStoreValue::ConsoleEnsured {
                    folder_name,
                    console_id,
                }) => {
                    if let Some(index) = self.browser.find_by_folder(&folder_name) {
                        self.browser.consoles[index].id = Some(console_id);
                        if self.ui_state.selected_console != Some(console_id) {
                            self.browser.consoles[index].entries.clear();
                        }
                    }
                }
                Ok(crate::backend::library_store::LibraryStoreValue::ConsoleScanCommitted {
                    folder_name,
                    console_id,
                    entry_count,
                    changes,
                }) => {
                    self.library_controller.apply_change_set(&changes);
                    self.browser.entry_counts.insert(console_id, entry_count);
                    if let Some(index) = self.browser.find_by_folder(&folder_name) {
                        self.browser.consoles[index].id = Some(console_id);
                    }
                    if self.ui_state.selected_console == Some(console_id) {
                        self.request_console_page(console_id, ctx);
                    }
                    self.refresh_console_summaries(ctx);
                }
                Ok(crate::backend::library_store::LibraryStoreValue::ConsoleSummaries(
                    summaries,
                )) => self.merge_console_summaries(summaries),
                Ok(crate::backend::library_store::LibraryStoreValue::EntryList(page)) => {
                    if self.pending_page_request != Some(request_id)
                        || self.ui_state.selected_console != Some(page.console_id)
                    {
                        continue;
                    }
                    self.pending_page_request = None;
                    self.browser
                        .entry_counts
                        .insert(page.console_id, page.total_count);
                    let ids = page.rows.iter().map(|row| row.id).collect();
                    self.browser.active_page = Some(page);
                    self.pending_details_request = self.queue_store(
                        crate::backend::library_store::LibraryStoreRequest::EntryDetails(ids),
                    );
                    ctx.request_repaint_after(Duration::from_millis(20));
                }
                Ok(crate::backend::library_store::LibraryStoreValue::EntryDetails(details)) => {
                    if let Some((pending_request, console_id)) = self.pending_all_hashes_request
                        && pending_request == request_id
                    {
                        self.pending_all_hashes_request = None;
                        let Some(console_index) = self.browser.find_by_id(console_id) else {
                            continue;
                        };
                        self.browser.consoles[console_index].entries = details
                            .into_iter()
                            .filter(|detail| detail.console_id == console_id)
                            .filter_map(detail_to_entry)
                            .collect();
                        self.ui_state.selected_entries = self.browser.consoles[console_index]
                            .entries
                            .iter()
                            .filter_map(|entry| entry.id)
                            .collect();
                        crate::backend::hash::compute_hashes_for_selection(self, console_index);
                        continue;
                    }
                    if self.pending_details_request != Some(request_id) {
                        continue;
                    }
                    self.pending_details_request = None;
                    let Some(console_id) = self.ui_state.selected_console else {
                        continue;
                    };
                    let Some(console_index) = self.browser.find_by_id(console_id) else {
                        continue;
                    };
                    self.browser.consoles[console_index].entries = details
                        .into_iter()
                        .filter(|detail| detail.console_id == console_id)
                        .filter_map(detail_to_entry)
                        .collect();
                    self.discover_assets_for_page(console_index, ctx);
                    crate::backend::hash::compute_missing_hashes(self, console_index);
                }
                Ok(crate::backend::library_store::LibraryStoreValue::ChangeSet(changes)) => {
                    self.library_controller.apply_change_set(&changes);
                    self.ui_state
                        .selected_entries
                        .retain(|id| !changes.removed_entries.contains(id));
                    if self
                        .ui_state
                        .focused_entry
                        .is_some_and(|id| changes.removed_entries.contains(&id))
                    {
                        self.ui_state.focused_entry = None;
                    }
                    if let (Some(selected), Some((changed, _))) =
                        (self.ui_state.selected_console, changes.console_revision)
                        && selected == changed
                    {
                        self.request_console_page(selected, ctx);
                    }
                    if changes.console_revision.is_some() || changes.root_revision.is_some() {
                        self.refresh_console_summaries(ctx);
                    }
                }
                _ => {}
            }
        }
    }

    fn merge_console_summaries(&mut self, summaries: Vec<retro_junk_db::LibraryConsoleSummary>) {
        for summary in summaries {
            self.browser
                .entry_counts
                .insert(summary.id, summary.entry_count);
            let worst_status = if summary.unrecognized_count > 0 {
                Some(crate::state::EntryStatus::Unrecognized)
            } else if summary.unknown_count > 0 {
                Some(crate::state::EntryStatus::Unknown)
            } else if summary.ambiguous_count > 0 {
                Some(crate::state::EntryStatus::Ambiguous)
            } else if summary.matched_count > 0 {
                Some(crate::state::EntryStatus::Matched)
            } else if summary.tagged_count > 0 {
                // Tag variants share their console-badge severity and color.
                Some(crate::state::EntryStatus::Tagged(
                    retro_junk_catalog::CatalogTag::Homebrew,
                ))
            } else {
                None
            };
            if let Some(status) = worst_status {
                self.browser.console_statuses.insert(summary.id, status);
            } else {
                self.browser.console_statuses.remove(&summary.id);
            }
            if summary.scan_state == retro_junk_db::LibraryScanState::Stale {
                self.browser.stale_consoles.insert(summary.id);
            } else {
                self.browser.stale_consoles.remove(&summary.id);
            }
            if let Some(index) = self
                .browser
                .find_by_id(summary.id)
                .or_else(|| self.browser.find_by_folder(&summary.folder_name))
            {
                self.browser.consoles[index].id = Some(summary.id);
                self.browser.consoles[index].revision = summary.revision;
                self.browser.consoles[index].scan_status = match summary.scan_state {
                    retro_junk_db::LibraryScanState::Ready => crate::state::ScanStatus::Scanned,
                    _ => crate::state::ScanStatus::NotScanned,
                };
                continue;
            }
            let Ok(platform) = serde_json::from_str(&format!("\"{}\"", summary.platform)) else {
                continue;
            };
            let Some(registered) = self.context.get_by_platform(platform) else {
                continue;
            };
            self.browser.consoles.push(crate::state::ConsoleState {
                id: Some(summary.id),
                revision: summary.revision,
                platform,
                folder_name: summary.folder_name,
                folder_path: summary.folder_path.into(),
                manufacturer: registered.metadata.manufacturer,
                platform_name: registered.metadata.platform_name,
                scan_status: match summary.scan_state {
                    retro_junk_db::LibraryScanState::Ready => crate::state::ScanStatus::Scanned,
                    _ => crate::state::ScanStatus::NotScanned,
                },
                entries: Vec::new(),
                fingerprint: None,
                loose_disc_files: Vec::new(),
            });
        }
    }

    fn refresh_console_summaries(&mut self, ctx: &egui::Context) {
        if let Some(root_id) = self.browser.root_id {
            self.submit_store(
                crate::backend::library_store::LibraryStoreRequest::ConsoleSummaries(root_id),
                ctx,
            );
        }
    }

    fn discover_assets_for_page(&mut self, console_index: usize, ctx: &egui::Context) {
        let Some(root_path) = self.root_path.clone() else {
            return;
        };
        let Some(console) = self.browser.consoles.get(console_index) else {
            return;
        };
        let folder_name = console.folder_name.clone();
        let entries: Vec<_> = console
            .entries
            .iter()
            .filter(|entry| entry.asset_paths.is_none())
            .filter_map(|entry| {
                let id = entry.id?;
                (!self.browser.asset_discovery_in_flight.contains(&id)).then(|| {
                    (
                        id,
                        entry.game_entry.display_name().to_owned(),
                        entry.game_entry.rom_stem().to_owned(),
                    )
                })
            })
            .collect();
        if entries.is_empty() {
            return;
        }
        self.browser
            .asset_discovery_in_flight
            .extend(entries.iter().map(|(id, _, _)| *id));
        crate::backend::assets::discover_assets_for_console(
            self.message_tx.clone(),
            ctx.clone(),
            root_path,
            folder_name,
            self.settings.general.assets_dir.clone(),
            entries,
        );
    }

    pub fn request_console_page(
        &mut self,
        console_id: retro_junk_db::LibraryConsoleId,
        ctx: &egui::Context,
    ) {
        self.browser.evict_inactive_entries(Some(console_id));
        self.queue_console_page(console_id);
        ctx.request_repaint_after(Duration::from_millis(20));
    }

    fn queue_console_page(&mut self, console_id: retro_junk_db::LibraryConsoleId) {
        self.pending_page_request = self.queue_store(
            crate::backend::library_store::LibraryStoreRequest::EntryList(
                retro_junk_db::LibraryEntryListQuery {
                    console_id,
                    search: self.ui_state.filter_text.clone(),
                    filter: retro_junk_db::LibraryEntryFilter::All,
                    sort: retro_junk_db::LibraryEntrySortField::DisplayName,
                    direction: retro_junk_db::SortDirection::Ascending,
                    offset: self.ui_state.page_offset,
                    limit: retro_junk_db::LibraryEntryListQuery::DEFAULT_PAGE_SIZE,
                },
            ),
        );
        self.pending_details_request = None;
    }

    pub fn selected_console_index(&self) -> Option<usize> {
        self.ui_state
            .selected_console
            .and_then(|id| self.browser.find_by_id(id))
    }

    pub fn calculate_all_hashes(&mut self, console_idx: usize, ctx: &egui::Context) {
        let Some(console_id) = self
            .browser
            .consoles
            .get(console_idx)
            .and_then(|console| console.id)
        else {
            return;
        };
        if self.pending_all_hashes_request.is_some() {
            return;
        }
        if let Some(request_id) = self.queue_store(
            crate::backend::library_store::LibraryStoreRequest::ConsoleEntryDetails(console_id),
        ) {
            self.pending_all_hashes_request = Some((request_id, console_id));
            ctx.request_repaint_after(Duration::from_millis(20));
        }
    }

    pub fn selected_entry_indices(&self) -> Vec<usize> {
        let Some(console_index) = self.selected_console_index() else {
            return Vec::new();
        };
        self.browser.consoles[console_index]
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                entry
                    .id
                    .is_some_and(|id| self.ui_state.selected_entries.contains(&id))
            })
            .map(|(index, _)| index)
            .collect()
    }

    /// Cancel every in-flight background operation and join its thread (D2).
    ///
    /// Without this, closing the app mid-`delete_job_sources` (or any other
    /// multi-step write) could kill the process between steps — a
    /// half-deleted disc set, a stale cache, an orphaned chdman child. With
    /// B2's responsive cancellation, a cancelled chdman run stops within
    /// ~100ms, so these joins complete promptly. Split out from `on_exit` so
    /// it's testable without touching real settings/cache files on disk.
    fn cancel_and_join_all_operations(&mut self) {
        for op in &self.operations {
            op.cancel_token
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        for (_, handle) in self.op_threads.drain() {
            let _ = handle.join();
        }
    }

    /// Whether a CHD-compression operation (planning or compressing) is
    /// already running for the given console folder. Used to gate the
    /// context-menu items (advisory) and guard `start_compression` /
    /// the D1 planning op (the actual guarantee) against launching a second
    /// overlapping run against the same inputs/outputs.
    pub fn chd_compress_busy(&self, folder_name: &str) -> bool {
        self.operations.iter().any(|op| {
            op.kind == crate::state::OperationKind::ChdCompress
                && !op.scope.is_empty()
                && op.scope == folder_name
        })
    }

    /// Returns true if any background operations are active.
    fn has_active_operations(&self) -> bool {
        !self.operations.is_empty()
    }

    /// Prepare filesystem fingerprints and serialized scan rows off the UI thread.
    pub fn prepare_completed_scan(&mut self, console_idx: usize, ctx: &egui::Context) {
        let Some(root) = self.root_path.clone() else {
            return;
        };
        let Some(console) = self.browser.consoles.get(console_idx).cloned() else {
            return;
        };
        let tx = self.message_tx.clone();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let folder_name = console.folder_name.clone();
            let console_id = console.id;
            let result = crate::cache::completed_console_scan(&root, &console)
                .map_err(|error| error.to_string());
            let _ = tx.send(crate::state::AppMessage::ScanSnapshotPrepared {
                folder_name,
                console_id,
                result,
            });
            repaint.request_repaint();
        });
    }

    pub fn set_entry_regions(
        &mut self,
        entry_ids: impl IntoIterator<Item = retro_junk_db::LibraryEntryId>,
        value: Option<retro_junk_core::Region>,
        ctx: &egui::Context,
    ) {
        let value = value.map(|region| region.name().to_owned());
        for entry_id in entry_ids {
            self.submit_store(
                crate::backend::library_store::LibraryStoreRequest::SetRegionOverride {
                    entry_id,
                    value: value.clone(),
                },
                ctx,
            );
        }
    }

    pub fn set_entry_tags(
        &mut self,
        entry_ids: impl IntoIterator<Item = retro_junk_db::LibraryEntryId>,
        value: Option<retro_junk_catalog::CatalogTag>,
        ctx: &egui::Context,
    ) {
        let value = value.map(|tag| match tag {
            retro_junk_catalog::CatalogTag::Homebrew => "homebrew".to_owned(),
            retro_junk_catalog::CatalogTag::Modded => "modded".to_owned(),
        });
        for entry_id in entry_ids {
            self.submit_store(
                crate::backend::library_store::LibraryStoreRequest::SetTag {
                    entry_id,
                    value: value.clone(),
                },
                ctx,
            );
        }
    }

    /// Publish derived analysis for one durable entry without reconciling or
    /// rewriting the rest of its console.
    pub fn publish_entry_analysis(
        &mut self,
        entry_id: retro_junk_db::LibraryEntryId,
        ctx: &egui::Context,
    ) {
        let Some(entry) = self
            .browser
            .consoles
            .iter()
            .find_map(|console| console.entry_by_id(entry_id))
        else {
            return;
        };
        let expected_source_revision = entry.source_revision;
        match crate::cache::entry_analysis_update(entry) {
            Ok(update) => self.submit_store(
                crate::backend::library_store::LibraryStoreRequest::ApplyAnalysis {
                    entry_id,
                    expected_source_revision,
                    update,
                },
                ctx,
            ),
            Err(error) => self.push_error("Library analysis", error.to_string()),
        }
    }

    pub fn publish_filesystem_entries(
        &mut self,
        console_idx: usize,
        entry_ids: impl IntoIterator<Item = retro_junk_db::LibraryEntryId>,
        ctx: &egui::Context,
    ) {
        let commands: Vec<_> = entry_ids
            .into_iter()
            .filter_map(|entry_id| {
                let console = self.browser.consoles.get(console_idx)?;
                let entry = console.entry_by_id(entry_id)?;
                let scanned = crate::cache::scanned_entry(console, entry).ok()?;
                Some((entry_id, entry.source_revision, scanned))
            })
            .collect();
        let Some(console_id) = self
            .browser
            .consoles
            .get(console_idx)
            .and_then(|console| console.id)
        else {
            return;
        };
        for (entry_id, expected_source_revision, entry) in commands {
            if let Some(request_id) = self.queue_store(
                crate::backend::library_store::LibraryStoreRequest::ApplyFilesystemTransition {
                    console_id,
                    entry_id,
                    expected_source_revision,
                    entry,
                },
            ) {
                self.pending_filesystem_writes
                    .insert(request_id, console_id);
            }
        }
        ctx.request_repaint_after(Duration::from_millis(20));
    }

    pub fn mark_console_stale(&mut self, console_idx: usize, ctx: &egui::Context) {
        if let Some(id) = self
            .browser
            .consoles
            .get(console_idx)
            .and_then(|console| console.id)
        {
            self.submit_store(
                crate::backend::library_store::LibraryStoreRequest::MarkConsoleStale(id),
                ctx,
            );
        }
    }

    pub fn delete_library_cache(&mut self, root: &std::path::Path, ctx: &egui::Context) {
        let reopen_current = self.root_path.as_deref() == Some(root);
        self.submit_store(
            crate::backend::library_store::LibraryStoreRequest::DeleteRootPath(
                root.to_string_lossy().into_owned(),
            ),
            ctx,
        );
        if reopen_current {
            self.reopen_current_root_after_cache_clear(ctx);
        }
    }

    pub fn clear_library_caches(&mut self, ctx: &egui::Context) {
        self.submit_store(
            crate::backend::library_store::LibraryStoreRequest::ClearCache,
            ctx,
        );
        self.reopen_current_root_after_cache_clear(ctx);
    }

    fn reopen_current_root_after_cache_clear(&mut self, ctx: &egui::Context) {
        let Some(root) = self.root_path.clone() else {
            return;
        };
        self.library_controller.switch_root();
        self.browser = LibraryBrowserState::default();
        self.ui_state.selected_console = None;
        self.ui_state.focused_entry = None;
        self.ui_state.selected_entries.clear();
        self.ui_state.page_offset = 0;
        self.ui_state.loading_library = true;
        self.pending_page_request = None;
        self.pending_details_request = None;
        self.open_browser_root(&root, ctx);
        let _ = self.message_tx.send(AppMessage::StartFolderScan);
    }

    /// Push an error that will be shown to the user in a modal dialog.
    pub fn push_error(&mut self, category: impl Into<String>, message: impl Into<String>) {
        self.ui_state.error_list.push(crate::state::UserError {
            category: category.into(),
            message: message.into(),
        });
    }
}

fn detail_to_entry(
    detail: retro_junk_db::LibraryEntryDetail,
) -> Option<crate::state::LibraryEntry> {
    let mut entry = crate::cache::row_to_entry(detail.row)?;
    entry.id = Some(detail.id);
    entry.revision = detail.revision;
    entry.source_revision = detail.source_revision;
    Some(entry)
}

impl eframe::App for RetroJunkApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = &ui.ctx().clone();

        // Drain background messages
        self.process_pending_messages(ctx);

        // Global view switching: Ctrl+1/2/3
        ctx.input_mut(|i| {
            if i.consume_key(egui::Modifiers::CTRL, egui::Key::Num1) {
                self.ui_state.current_view = View::Library;
            } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::Num2) {
                self.ui_state.current_view = View::Settings;
            } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::Num3) {
                self.ui_state.current_view = View::Tools;
            }
        });

        // Schedule repaint while operations are running
        if self.has_active_operations() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        // Sidebar
        let prev_view = self.ui_state.current_view;
        egui::Panel::left("sidebar")
            .resizable(false)
            .exact_size(120.0)
            .show(ui, |ui| {
                ui.add_space(8.0);
                ui.heading("retro-junk");
                ui.separator();
                ui.add_space(4.0);

                let view = &mut self.ui_state.current_view;
                ui.selectable_value(view, View::Library, "Library");
                ui.selectable_value(view, View::Settings, "Settings");
                ui.selectable_value(view, View::Tools, "Tools");
            });

        // Trigger refresh when switching to Tools view
        if self.ui_state.current_view == View::Tools && prev_view != View::Tools {
            self.ui_state.tools_state.needs_refresh = true;
        }

        // Bottom panels render in order: status bar (bottommost), log viewer, activity bar.
        // egui stacks bottom panels upward, so the first one rendered sits at the very bottom.
        if widgets::status_bar::show(ui) {
            self.ui_state.log_viewer.open = !self.ui_state.log_viewer.open;
        }
        widgets::log_viewer::show(ui, &mut self.ui_state.log_viewer);

        // Activity bar (bottom, only when operations active)
        if self.has_active_operations() {
            egui::Panel::bottom("activity_bar").show(ui, |ui| {
                widgets::activity_bar::show(ui, &mut self.operations);
            });
        }

        // Main content. Uses a stable-id central panel so toggling the
        // conditional log viewer / activity bar panels above doesn't re-id every
        // widget in the view (see `util::stable_central_panel`).
        util::stable_central_panel(ui, "main_view", |ui| match self.ui_state.current_view {
            View::Library => views::library::show(ui, self, ctx),
            View::Settings => views::settings::show(ui, self),
            View::Tools => views::tools::show(ui, self),
        });

        // Batch-results modal dialog (rename / CUE fix / CHD compression)
        let dismissed = match &self.ui_state.results_dialog {
            ResultsDialog::None => false,
            ResultsDialog::Rename(items) => widgets::results_dialog::show_results_dialog(
                ctx,
                "Rename Results",
                items,
                rename_results_summary,
                rename_results_row,
            ),
            ResultsDialog::CueFix(items) => widgets::results_dialog::show_results_dialog(
                ctx,
                "Fix CUE Results",
                items,
                cue_fix_results_summary,
                cue_fix_results_row,
            ),
            ResultsDialog::ChdCompress(items) => widgets::results_dialog::show_results_dialog(
                ctx,
                "Compress to CHD Results",
                items,
                chd_compress_results_summary,
                chd_compress_results_row,
            ),
        };
        if dismissed {
            self.ui_state.results_dialog = ResultsDialog::None;
        }

        // Compress-to-CHD confirmation dialog
        widgets::chd_compress_dialog::show(ctx, self);

        // Organize preview dialog
        if self.ui_state.pending_organize_plan.is_some() {
            show_organize_preview_dialog(ctx, self);
        }

        // Fragile network mount confirmation
        widgets::fragile_mount_dialog::show(ctx, self);

        // Tag dialog
        widgets::tag_dialog::show(ctx, self);

        // Error dialog
        widgets::startup_dialog::show(ctx, self.ui_state.startup_status.as_deref());
        widgets::error_dialog::show(ctx, &mut self.ui_state.error_list);
    }

    fn on_exit(&mut self) {
        log::info!("on_exit: stopping background work");

        // Cancel every in-flight background operation and join its thread.
        self.cancel_and_join_all_operations();
        // Apply whatever completion messages those threads sent (e.g.
        // ChdCompressComplete), queuing their final store commands.
        let ctx = egui::Context::default();
        // Producers are joined, so drain their finite completion queue before
        // shutting down the serialized store.
        while let Ok(message) = self.message_rx.try_recv() {
            crate::state::handle_message(self, message, &ctx);
        }

        // Commands already queued by completed producers are committed before
        // the store worker acknowledges shutdown.
        if let Some(store) = &mut self.library_store {
            store.shutdown_and_join(self.library_controller.session_generation);
        }

        // Save settings
        self.settings.library.current_root = self.root_path.clone();
        if let Err(e) = crate::settings::save_settings(&self.settings) {
            log::warn!("Failed to save settings on exit: {e}");
        }
    }
}

/// Summary line for the rename-results dialog ([`widgets::results_dialog`]).
fn rename_results_summary(items: &[RenameResult]) -> String {
    let renamed = items
        .iter()
        .filter(|r| {
            matches!(
                r.outcome,
                RenameOutcome::Renamed { .. } | RenameOutcome::M3uRenamed { .. }
            )
        })
        .count();
    let already = items
        .iter()
        .filter(|r| matches!(r.outcome, RenameOutcome::AlreadyCorrect))
        .count();
    let failed = items
        .iter()
        .filter(|r| {
            matches!(
                r.outcome,
                RenameOutcome::NoMatch { .. } | RenameOutcome::Error { .. }
            )
        })
        .count();
    format!("{renamed} renamed, {already} already correct, {failed} failed")
}

/// One row of the rename-results dialog.
fn rename_results_row(ui: &mut egui::Ui, item: &RenameResult) {
    use widgets::results_dialog::{STATUS_ERR, STATUS_OK, STATUS_WARN};

    match &item.outcome {
        RenameOutcome::Renamed { source, target } => {
            ui.colored_label(STATUS_OK, "Renamed");
            ui.label(format!(
                "{} -> {}",
                source.file_name().unwrap_or_default().to_string_lossy(),
                target.file_name().unwrap_or_default().to_string_lossy()
            ));
        }
        RenameOutcome::AlreadyCorrect => {
            ui.colored_label(egui::Color32::GRAY, "OK");
            ui.label(format!("{} already correct", item.entry_name));
        }
        RenameOutcome::NoMatch { reason } => {
            ui.colored_label(STATUS_WARN, "No match");
            ui.label(reason);
        }
        RenameOutcome::Error { message } => {
            ui.colored_label(STATUS_ERR, "Error");
            ui.label(message);
        }
        RenameOutcome::M3uRenamed {
            target_folder,
            discs_renamed,
            playlist_written,
            folder_renamed,
            errors,
            ..
        } => {
            ui.colored_label(STATUS_OK, "M3U");
            let folder_name = target_folder
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            let mut parts = Vec::new();
            parts.push(format!("{discs_renamed} discs"));
            if *playlist_written {
                parts.push("playlist written".to_string());
            }
            if *folder_renamed {
                parts.push(format!("folder -> {folder_name}"));
            }
            if !errors.is_empty() {
                parts.push(format!("{} errors", errors.len()));
            }
            ui.label(parts.join(", "));
        }
    }
}

/// Modal dialog previewing an organize plan and letting the user confirm or cancel.
fn show_organize_preview_dialog(ctx: &egui::Context, app: &mut RetroJunkApp) {
    let Some((ref _folder_name, ref plan)) = app.ui_state.pending_organize_plan else {
        return;
    };

    let job_count = plan.jobs.len();
    let unmatched_count = plan.unmatched.len();
    let total_files: usize = plan
        .jobs
        .iter()
        .map(|j| j.entry_points.len() + j.companion_files.len())
        .sum();

    let mut execute = false;
    let mut dismiss = false;
    let mut open = true;

    egui::Window::new("Organize Disc Files")
        .collapsible(false)
        .resizable(true)
        .open(&mut open)
        .default_width(550.0)
        .show(ctx, |ui| {
            ui.label(format!(
                "{job_count} folders to create ({total_files} files to move)"
            ));
            if unmatched_count > 0 {
                ui.colored_label(
                    crate::theme::STATUS_WARN,
                    format!("{unmatched_count} files could not be matched"),
                );
            }
            if plan.skipped_single_disc > 0 {
                ui.colored_label(
                    egui::Color32::GRAY,
                    format!("{} single-disc games skipped", plan.skipped_single_disc),
                );
            }

            ui.separator();

            egui::ScrollArea::vertical()
                .max_height(350.0)
                .show(ui, |ui| {
                    for job in &plan.jobs {
                        ui.horizontal(|ui| {
                            ui.colored_label(crate::theme::STATUS_OK, "\u{2192}");
                            ui.label(format!(
                                "{}.m3u ({} discs)",
                                job.game_name,
                                job.entry_points.len()
                            ));
                        });
                        for entry in &job.entry_points {
                            ui.horizontal(|ui| {
                                ui.add_space(20.0);
                                ui.colored_label(egui::Color32::GRAY, &entry.target_filename);
                            });
                        }
                    }

                    if !plan.unmatched.is_empty() {
                        ui.separator();
                        ui.label("Unmatched:");
                        for uf in &plan.unmatched {
                            let name = uf.path.file_name().unwrap_or_default().to_string_lossy();
                            ui.horizontal(|ui| {
                                ui.colored_label(crate::theme::STATUS_WARN, "\u{26A0}");
                                ui.label(format!("{} \u{2014} {}", name, uf.reason));
                            });
                        }
                    }
                });

            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(job_count > 0, egui::Button::new("Execute"))
                    .clicked()
                {
                    execute = true;
                }
                if ui.button("Cancel").clicked() {
                    dismiss = true;
                }
            });
        });

    if execute {
        let (folder_name, plan) = app.ui_state.pending_organize_plan.take().unwrap();
        crate::backend::organize::execute_organize_plan(app, folder_name, plan, ctx);
    } else if dismiss || !open {
        app.ui_state.pending_organize_plan = None;
    }
}

/// Summary line for the CUE-fix results dialog ([`widgets::results_dialog`]).
fn cue_fix_results_summary(items: &[CueFixResult]) -> String {
    let fixed = items
        .iter()
        .filter(|r| matches!(r.outcome, CueFixOutcome::Fixed { .. }))
        .count();
    let already = items
        .iter()
        .filter(|r| matches!(r.outcome, CueFixOutcome::AlreadyStandard))
        .count();
    let failed = items
        .iter()
        .filter(|r| {
            matches!(
                r.outcome,
                CueFixOutcome::Unfixable { .. } | CueFixOutcome::Error { .. }
            )
        })
        .count();
    format!("{fixed} fixed, {already} already standard, {failed} failed")
}

/// One row of the CUE-fix results dialog.
fn cue_fix_results_row(ui: &mut egui::Ui, item: &CueFixResult) {
    use widgets::results_dialog::{STATUS_ERR, STATUS_OK, STATUS_WARN};

    match &item.outcome {
        CueFixOutcome::Fixed { summary } => {
            ui.colored_label(STATUS_OK, "Fixed");
            ui.label(format!("{} ({})", item.file_name, summary));
        }
        CueFixOutcome::AlreadyStandard => {
            ui.colored_label(egui::Color32::GRAY, "OK");
            ui.label(format!("{} already standard", item.file_name));
        }
        CueFixOutcome::Unfixable { reason } => {
            ui.colored_label(STATUS_WARN, "Unfixable");
            ui.label(format!("{}: {}", item.file_name, reason));
        }
        CueFixOutcome::Error { message } => {
            ui.colored_label(STATUS_ERR, "Error");
            ui.label(format!("{}: {}", item.file_name, message));
        }
    }
}

/// Summary line for the CHD-compression results dialog ([`widgets::results_dialog`]).
fn chd_compress_results_summary(items: &[crate::state::ChdCompressResult]) -> String {
    use crate::state::ChdCompressOutcome;

    let compressed = items
        .iter()
        .filter(|r| matches!(r.outcome, ChdCompressOutcome::Compressed { .. }))
        .count();
    let failed = items
        .iter()
        .filter(|r| {
            matches!(
                r.outcome,
                ChdCompressOutcome::VerifyFailed { .. } | ChdCompressOutcome::Error { .. }
            )
        })
        .count();
    let cancelled = items
        .iter()
        .filter(|r| matches!(r.outcome, ChdCompressOutcome::Cancelled))
        .count();

    let mut summary = format!("{compressed} compressed, {failed} failed");
    if cancelled > 0 {
        use std::fmt::Write;
        let _ = write!(summary, ", {cancelled} cancelled");
    }
    summary
}

/// One row of the CHD-compression results dialog.
fn chd_compress_results_row(ui: &mut egui::Ui, item: &crate::state::ChdCompressResult) {
    use crate::state::ChdCompressOutcome;
    use retro_junk_lib::util::format_bytes_approx;
    use widgets::results_dialog::{STATUS_ERR, STATUS_OK};

    match &item.outcome {
        ChdCompressOutcome::Compressed {
            input_bytes,
            output_bytes,
            tracks,
            sources_deleted,
            delete_failures,
            ..
        } => {
            ui.colored_label(STATUS_OK, "Compressed");
            let ratio = if *input_bytes > 0 {
                format!(
                    " ({:.0}%)",
                    *output_bytes as f64 / *input_bytes as f64 * 100.0
                )
            } else {
                String::new()
            };
            let originals = if !delete_failures.is_empty() {
                format!(
                    "some originals could not be deleted: {}",
                    delete_failures.join(", ")
                )
            } else if *sources_deleted {
                "originals deleted".to_string()
            } else {
                "originals kept".to_string()
            };
            ui.label(format!(
                "{}: {} → {}{ratio}, {tracks} track(s) verified, {originals}",
                item.input_name,
                format_bytes_approx(*input_bytes),
                format_bytes_approx(*output_bytes),
            ));
        }
        ChdCompressOutcome::VerifyFailed { detail } => {
            ui.colored_label(STATUS_ERR, "Verify failed");
            ui.label(format!(
                "{}: {detail} — the .chd was discarded, originals kept",
                item.input_name
            ));
        }
        ChdCompressOutcome::Error { message } => {
            ui.colored_label(STATUS_ERR, "Error");
            ui.label(format!("{}: {message}", item.input_name));
        }
        ChdCompressOutcome::Cancelled => {
            ui.colored_label(egui::Color32::GRAY, "Cancelled");
            ui.label(&item.input_name);
        }
    }
}
