#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "id_stability_tests.rs"]
mod id_stability_tests;

#[cfg(test)]
#[path = "dirty_tick_tests.rs"]
mod dirty_tick_tests;

use std::collections::HashMap;
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use retro_junk_lib::AnalysisContext;

use crate::settings::AppSettings;
use crate::state::{
    AppMessage, AppMessageEnvelope, AppMessageSender, BackgroundOperation, CueFixOutcome,
    CueFixResult, FocusedPanel, LibraryBrowserState, RenameOutcome, RenameResult, ToolsState, View,
};
use crate::util;
use crate::views;
use crate::widgets;
use crate::widgets::icons;

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

#[derive(Clone, Copy)]
enum ConsoleDetailsAction {
    HashAll,
    ScrapeAll,
    ScrapeMissing,
    CompressAll,
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
    /// Durable logical archive-release identities for multi-select.
    pub selected_archive_releases: HashSet<String>,
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
    /// Automation policy being edited in Settings; loaded on first show,
    /// saved surgically on change.
    pub automation_policy: Option<retro_junk_work::AutomationPolicy>,
    /// Open-suggestion count for the status bar; refreshed with the
    /// archive projection.
    pub open_suggestion_count: u64,
    /// Transient state for the Tools (catalog) view.
    pub tools_state: ToolsState,
    /// Which panel currently has keyboard focus for arrow-key navigation.
    pub focused_panel: FocusedPanel,
    /// One-shot request to scroll the game table to a filtered row index.
    pub scroll_to_row: Option<usize>,
    /// Logical archive release focused in the unified Library table. This is
    /// mutually exclusive with `focused_entry`.
    pub focused_archive_release: Option<String>,
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
    /// Selected physical item and playable-policy editor in Collection.
    pub collection_editor: Option<crate::state::PhysicalCopyEditor>,
    /// In-memory Collection projection; populated off the render thread.
    pub collection_summaries: Arc<Vec<retro_junk_db::ArchiveReleaseSummary>>,
    pub collection_profile_id: Option<String>,
    pub collection_summaries_loading: bool,
    pub collection_editor_loading: Option<String>,
    pub collection_selected_release: Option<String>,
    /// Blocking discovery/review/progress flow for archive dump imports.
    pub dump_import_dialog: Option<crate::state::DumpImportDialogState>,
    /// Reconcile archive bindings after this playable console finishes its
    /// forced post-build content scan.
    pub refresh_archive_after_console_scan: Option<String>,
    /// Coalesces binding refresh requests while another archive projection
    /// rebuild is already running.
    pub archive_refresh_pending: bool,
    /// Set when the menubar's Find command asks for the Library filter
    /// field; the Library view consumes it on its next render.
    pub pending_filter_focus: bool,
    /// Derived convergence backlog for the scope currently on screen, plus
    /// the open-error set the per-row badges read.
    pub backlog: crate::backend::convergence::Backlog,
    /// Scope the loaded backlog was derived for; a change triggers a reload.
    pub backlog_scope: Option<retro_junk_db::convergence::Scope>,
    /// True while a backlog derivation is in flight.
    pub backlog_loading: bool,
    /// Last observed `runtime_state.dirty_tick`. `None` until the first
    /// poll, so startup records the current value instead of treating every
    /// write since the database was created as news.
    pub dirty_tick: Option<i64>,
    /// When the tick was last read, throttling the poll to ~1 Hz.
    pub last_dirty_poll: Option<std::time::Instant>,
    /// Loaded review-inbox contents.
    pub inbox: crate::backend::inbox::InboxContents,
    /// True while an inbox load is in flight.
    pub inbox_loading: bool,
    /// Set when something changed that the inbox reflects; the view reloads
    /// on its next render rather than every writer racing to reload it.
    pub inbox_dirty: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            current_view: View::Library,
            selected_console: None,
            scroll_to_console: None,
            focused_entry: None,
            selected_entries: HashSet::new(),
            selected_archive_releases: HashSet::new(),
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
            automation_policy: None,
            open_suggestion_count: 0,
            tools_state: ToolsState::default(),
            focused_panel: FocusedPanel::default(),
            scroll_to_row: None,
            focused_archive_release: None,
            tag_dialog: crate::state::TagDialog::None,
            fragile_mount_prompt: None,
            log_viewer: crate::widgets::log_viewer::LogViewerState::default(),
            error_list: Vec::new(),
            pending_organize_plan: None,
            pending_auto_scans: VecDeque::new(),
            auto_scan_in_flight: None,
            auto_scan_op_id: None,
            collection_editor: None,
            collection_summaries: Arc::new(Vec::new()),
            collection_profile_id: None,
            collection_summaries_loading: false,
            collection_editor_loading: None,
            collection_selected_release: None,
            dump_import_dialog: None,
            refresh_archive_after_console_scan: None,
            archive_refresh_pending: false,
            pending_filter_focus: false,
            backlog: crate::backend::convergence::Backlog::default(),
            backlog_scope: None,
            backlog_loading: false,
            dirty_tick: None,
            last_dirty_poll: None,
            inbox: crate::backend::inbox::InboxContents::default(),
            inbox_loading: false,
            inbox_dirty: true,
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
    pub message_rx: mpsc::Receiver<AppMessageEnvelope>,

    /// Sender cloned into background threads.
    pub message_tx: AppMessageSender,

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
    store_requests_in_flight: HashSet<u64>,
    pending_page_request: Option<u64>,
    pending_details_request: Option<(u64, retro_junk_db::LibraryEntryId)>,
    pending_selection_details_request: Option<u64>,
    pending_console_details_request:
        Option<(u64, retro_junk_db::LibraryConsoleId, ConsoleDetailsAction)>,
    pending_scan_commits: HashMap<u64, String>,

    /// Set while a startup root restore is waiting for its `RootOpened`
    /// reply. If that reply carries no projected consoles the root has never
    /// been scanned, and only then does startup fall back to a filesystem
    /// walk — a projected root paints straight from the database.
    pub pending_first_open_scan: bool,

    /// `JoinHandle`s for every spawned background-operation thread, keyed by
    /// `op_id`. Joined (and removed) when the operation completes, or all at
    /// once in `on_exit` so the process never dies mid-write (D2).
    pub op_threads: HashMap<u64, std::thread::JoinHandle<()>>,

    /// Native menubar. Held for the life of the app so the menu and its
    /// items are not dropped out from under the OS. `None` in headless
    /// test instances, which never build one.
    _app_menu: Option<crate::menu::AppMenu>,

    /// Menu-item identities, copied out of `_app_menu` so dispatch does not
    /// re-borrow it.
    menu_ids: Option<crate::menu::AppMenuIds>,

    /// Transient success feedback for background work that has no other
    /// surface. Failures keep going to the error modal via `push_error`.
    pub toasts: crate::widgets::toasts::Toasts,
}

impl RetroJunkApp {
    /// Keep decoded media bounded to the one entry that can currently render
    /// in the detail panel. `forget_image` clears file bytes, decoded pixels,
    /// and GPU textures from every installed egui loader.
    pub fn reconcile_detail_assets(&mut self, ctx: &egui::Context) {
        let target = if self.ui_state.current_view == View::Library
            && self.ui_state.detail_panel_open
            && self.selected_library_row_count() <= 1
        {
            self.ui_state.focused_entry.or_else(|| {
                let release_id = self.ui_state.focused_archive_release.as_deref()?;
                self.browser
                    .active_page
                    .as_ref()?
                    .archived_releases
                    .iter()
                    .find(|release| release.summary.archive_release_id == release_id)?
                    .playable_library_entries
                    .first()
                    .map(|entry| entry.id)
            })
        } else {
            None
        };
        if self.browser.detail_asset_entry == target {
            return;
        }
        self.release_detail_assets(ctx);
        self.browser.detail_asset_entry = target;
    }

    pub fn release_detail_assets(&mut self, ctx: &egui::Context) {
        for console in &mut self.browser.consoles {
            for entry in &mut console.entries {
                if let Some(paths) = entry.asset_paths.take() {
                    for path in paths.values() {
                        ctx.forget_image(&crate::state::asset_image_uri(path));
                    }
                }
            }
        }
        self.browser.detail_asset_entry = None;
    }

    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let construct_started = std::time::Instant::now();
        let settings = crate::settings::load_settings();
        let target_db_path = retro_junk_lib::settings::catalog_database_path();
        let mut app = Self::with_parts(&cc.egui_ctx, settings, None, None);
        app.db_path = Some(target_db_path.clone());
        // Built here rather than in `with_parts` so headless test instances
        // never touch the platform menubar. eframe has already initialized
        // the platform UI thread before this closure runs.
        let app_menu = crate::menu::build();
        app_menu.install();
        app.menu_ids = Some(app_menu.ids.clone());
        app._app_menu = Some(app_menu);
        log::info!(
            "startup: settings + UI construction took {:?}",
            construct_started.elapsed()
        );

        // Every database probe, migration, archive check, and saved-root
        // probe runs off the UI thread so the first frame paints immediately.
        // The blocking modal appears only if a real location/schema migration
        // turns out to be required.
        let tx = app.message_tx.clone();
        let ctx = cc.egui_ctx.clone();
        let analysis_context = app.context.clone();
        let configured_root = app.settings.library.current_root.clone();
        let configured_profile = app.settings.library.active_profile().cloned();
        std::thread::spawn(move || {
            let thread_started = std::time::Instant::now();
            let location_migration =
                retro_junk_lib::settings::catalog_database_needs_location_migration()
                    .unwrap_or(true);
            let schema_migration = target_db_path.is_file()
                && retro_junk_db::database_needs_migration(&target_db_path).unwrap_or(true);
            if location_migration || schema_migration {
                let status = if location_migration {
                    "Moving and validating the catalog database…".to_owned()
                } else {
                    "Migrating the catalog database schema…".to_owned()
                };
                let _ = tx.send(crate::state::AppMessage::StartupStatus {
                    status: Some(status),
                });
                ctx.request_repaint();
            }

            let stage = std::time::Instant::now();
            let database = retro_junk_lib::settings::ensure_catalog_database_location()
                .map_err(|error| error.to_string())
                .and_then(|path| {
                    retro_junk_db::open_database(&path).map_err(|error| error.to_string())
                });
            let database_ready = database.is_ok();
            log::info!("startup: catalog database ready in {:?}", stage.elapsed());

            // Refresh the archive projection at startup only when it has
            // never been committed. The archive changes only through this
            // tool and every mutation reconciles, so an indexed profile
            // paints from the committed projection immediately; deep rescans
            // stay behind explicit Refresh/reindex.
            let mut refresh_profile = None;
            if let (Ok(connection), Some(profile)) = (&database, &configured_profile)
                && retro_junk_archive::root_manifest_path(&profile.archive_root).is_file()
            {
                let profile_id = profile.profile_id.to_string();
                match retro_junk_db::archive_profile_indexed_at(connection, &profile_id) {
                    Ok(Some(indexed_at)) => {
                        log::info!(
                            "startup: archive projection committed at {indexed_at}; \
                             skipping startup refresh"
                        );
                    }
                    Ok(None) => refresh_profile = Some(profile.clone()),
                    Err(error) => {
                        log::warn!("startup: archive projection probe failed: {error}");
                        refresh_profile = Some(profile.clone());
                    }
                }
            }

            let _ = tx.send(crate::state::AppMessage::StartupReady { database });
            ctx.request_repaint();

            if let Some(profile) = refresh_profile {
                let _ = tx.send(crate::state::AppMessage::StartArchiveRefresh { profile });
                ctx.request_repaint();
            }

            let mut restored_root = None;
            let mut fragile_mount_kind = None;
            if let Some(root) = configured_root {
                log::info!("Settings current_root: {}", root.display());
                if root.is_dir() {
                    fragile_mount_kind = crate::util::fragile_mount_kind(&root);
                    if fragile_mount_kind.is_none()
                        && database_ready
                        && crate::cache::has_legacy_cache(&root)
                        && let Ok(mut connection) = retro_junk_db::open_database(
                            &retro_junk_lib::settings::catalog_database_path(),
                        )
                    {
                        crate::cache::migrate_json_cache(
                            &mut connection,
                            &root,
                            analysis_context.as_ref(),
                        );
                    }
                    restored_root = Some(root);
                } else {
                    log::warn!("current_root is not a directory, skipping auto-load");
                }
            }
            let _ = tx.send(crate::state::AppMessage::StartupRootReady {
                restored_root,
                fragile_mount_kind,
            });
            ctx.request_repaint();
            log::info!(
                "startup: background preparation finished in {:?}",
                thread_started.elapsed()
            );
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
        // `TableBody::rows` virtualizes large lists by removing off-screen
        // widget IDs. At certain scroll offsets a different row then occupies
        // an identical screen rectangle, which the debug-only cross-frame
        // heuristic misdiagnoses as ID instability and outlines in red. Keep
        // true same-frame ID-clash detection enabled; disable only this
        // virtualization-incompatible heuristic. ID-stability tests explicitly
        // re-enable it while exercising non-virtualized layout transitions.
        #[cfg(debug_assertions)]
        egui_ctx.all_styles_mut(|style| style.debug.warn_if_rect_changes_id = false);
        let (raw_tx, rx) = mpsc::channel();
        let tx = AppMessageSender::new(raw_tx);
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
            store_requests_in_flight: HashSet::new(),
            pending_page_request: None,
            pending_details_request: None,
            pending_selection_details_request: None,
            pending_console_details_request: None,
            pending_scan_commits: HashMap::new(),
            pending_first_open_scan: false,
            op_threads: HashMap::new(),
            _app_menu: None,
            menu_ids: None,
            toasts: crate::widgets::toasts::Toasts::default(),
        }
    }

    /// Transient success feedback for work whose only other surface is the
    /// log. Anything the user must act on stays an error-modal entry.
    pub fn notify(&mut self, message: impl Into<String>) {
        self.toasts.success(message);
    }

    /// Notice writes another process made to the same database.
    ///
    /// Every mutating commit in the coordination store bumps
    /// `runtime_state.dirty_tick`, so one indexed singleton read per second
    /// is enough to know the daemon or a `retro-junk sync` changed
    /// something — no second update channel, no notification service, and
    /// it degrades to "the GUI is a little behind" rather than to
    /// corruption if a poll is missed.
    ///
    /// On a change the projection is re-requested rather than cleared: the
    /// store replies replace the page atomically, so the table keeps
    /// showing current rows instead of flashing an empty loading state
    /// every time the daemon finishes a build.
    fn poll_dirty_tick(&mut self, ctx: &egui::Context) {
        const POLL_INTERVAL: Duration = Duration::from_secs(1);
        if self
            .ui_state
            .last_dirty_poll
            .is_some_and(|at| at.elapsed() < POLL_INTERVAL)
        {
            return;
        }
        self.ui_state.last_dirty_poll = Some(std::time::Instant::now());
        ctx.request_repaint_after(POLL_INTERVAL);

        let Some(connection) = self.catalog_db.as_ref() else {
            return;
        };
        let Ok(runtime) = retro_junk_db::work::read_runtime_state(connection) else {
            return;
        };
        let previous = self.ui_state.dirty_tick.replace(runtime.dirty_tick);
        if previous.is_none_or(|previous| previous == runtime.dirty_tick) {
            return;
        }
        log::debug!(
            "another process wrote (tick {}); refreshing",
            runtime.dirty_tick
        );
        self.library_controller.invalidate_lists();
        self.refresh_console_summaries(ctx);
        if let Some(console_id) = self.ui_state.selected_console {
            self.request_console_page(console_id, ctx);
        }
        if let Some(scope) = self.ui_state.backlog_scope.clone() {
            crate::backend::convergence::load_backlog(self, scope, ctx);
        }
        self.ui_state.inbox_dirty = true;
    }

    /// Drain pending native-menu events and dispatch them.
    ///
    /// Predefined items (Quit, Close Window, Cut/Copy/Paste, About, Hide,
    /// Minimize, Zoom) are handled by the OS — no event arrives for those.
    fn process_menu_events(&mut self, ctx: &egui::Context) {
        let Some(ids) = self.menu_ids.clone() else {
            return;
        };
        while let Ok(event) = muda::MenuEvent::receiver().try_recv() {
            if event.id == ids.open_library {
                crate::views::library::open_folder(self, ctx);
            } else if event.id == ids.preferences {
                self.ui_state.current_view = View::Settings;
            } else if event.id == ids.find {
                self.ui_state.current_view = View::Library;
                self.ui_state.pending_filter_focus = true;
            } else if event.id == ids.view_collection {
                self.ui_state.current_view = View::Collection;
            } else if event.id == ids.view_library {
                self.ui_state.current_view = View::Library;
            } else if event.id == ids.view_inbox {
                self.ui_state.current_view = View::Inbox;
            } else if event.id == ids.view_settings {
                self.ui_state.current_view = View::Settings;
            } else if event.id == ids.view_tools {
                self.ui_state.current_view = View::Tools;
            } else if event.id == ids.toggle_log_viewer {
                self.ui_state.log_viewer.open = !self.ui_state.log_viewer.open;
            } else {
                log::debug!("unhandled menu event: {:?}", event.id);
            }
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
            let Ok(message) = self.message_rx.try_recv() else {
                return;
            };
            if message.payload.is_root_scoped()
                && message.session_generation != self.library_controller.session_generation
            {
                processed += 1;
                continue;
            }
            crate::state::handle_message(self, message.payload, ctx);
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
        if let Err(error) = store.submit(crate::backend::library_store::StoreEnvelope {
            session_generation: self.library_controller.session_generation,
            request_id,
            payload,
        }) {
            self.push_error(
                "Library database",
                format!("The database worker is not running: {error}"),
            );
            return None;
        }
        self.store_requests_in_flight.insert(request_id);
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
            self.store_requests_in_flight.remove(&reply.request_id);
            if reply.session_generation != self.library_controller.session_generation {
                continue;
            }
            let request_id = reply.request_id;
            let scan_folder = self.pending_scan_commits.remove(&request_id);
            match reply.payload {
                Err(error) => {
                    if self
                        .pending_console_details_request
                        .is_some_and(|(pending, _, _)| pending == request_id)
                    {
                        self.pending_console_details_request = None;
                    }
                    self.push_error("Library database", error);
                    if let Some(folder_name) = scan_folder {
                        crate::state::finish_auto_scan(self, &folder_name, false, ctx);
                    }
                }
                Ok(crate::backend::library_store::LibraryStoreValue::RootOpened {
                    root_id,
                    summaries,
                }) => {
                    let unprojected = summaries.is_empty();
                    self.browser.root_id = Some(root_id);
                    self.merge_console_summaries(summaries);
                    if std::mem::take(&mut self.pending_first_open_scan) && unprojected {
                        log::info!("startup: root has no committed projection; scanning");
                        let _ = self
                            .message_tx
                            .send(crate::state::AppMessage::StartFolderScan);
                    }
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
                    let bindings_may_have_changed =
                        !changes.affected_entries.is_empty() || !changes.removed_entries.is_empty();
                    self.library_controller.apply_change_set(&changes);
                    self.browser.entry_counts.insert(console_id, entry_count);
                    if let Some(index) = self.browser.find_by_folder(&folder_name) {
                        self.browser.consoles[index].id = Some(console_id);
                        self.browser.consoles[index].scan_status =
                            crate::state::ScanStatus::Scanned;
                    }
                    if self.ui_state.selected_console == Some(console_id) {
                        self.request_console_page(console_id, ctx);
                    }
                    self.refresh_console_summaries(ctx);
                    crate::state::finish_auto_scan(self, &folder_name, true, ctx);
                    if self.ui_state.refresh_archive_after_console_scan.as_deref()
                        == Some(folder_name.as_str())
                    {
                        self.ui_state.refresh_archive_after_console_scan = None;
                        if let Some(profile) = self.settings.library.active_profile().cloned() {
                            let _ = self
                                .message_tx
                                .send(crate::state::AppMessage::StartArchiveRefresh { profile });
                        }
                    } else if bindings_may_have_changed
                        && let Some(profile) = self.settings.library.active_profile().cloned()
                    {
                        let _ = self
                            .message_tx
                            .send(crate::state::AppMessage::StartArchiveRefresh { profile });
                    }
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
                        .insert(page.console_id, page.logical_count);
                    let ids: Vec<_> = page.rows.iter().map(|row| row.id).collect();
                    let archive_ids = page
                        .archived_releases
                        .iter()
                        .map(|release| release.summary.archive_release_id.clone())
                        .collect::<HashSet<_>>();
                    self.ui_state
                        .selected_archive_releases
                        .retain(|id| archive_ids.contains(id));
                    if self
                        .ui_state
                        .focused_archive_release
                        .as_ref()
                        .is_some_and(|id| !archive_ids.contains(id))
                    {
                        self.ui_state.focused_archive_release = self
                            .ui_state
                            .selected_archive_releases
                            .iter()
                            .next()
                            .cloned();
                    }
                    let asset_rows: Vec<_> = page
                        .rows
                        .iter()
                        .map(|row| (row.id, row.display_name.clone()))
                        .chain(page.archived_releases.iter().flat_map(|release| {
                            release
                                .playable_library_entries
                                .iter()
                                .map(|entry| (entry.id, entry.display_name.clone()))
                        }))
                        .collect();
                    let page_console_id = page.console_id;
                    self.browser.active_page = Some(page);
                    self.browser.asset_statuses.clear();
                    self.browser.entries_with_miximages.clear();
                    let focused = self.ui_state.focused_entry;
                    if let Some(console_index) = self
                        .browser
                        .find_by_id(self.browser.active_page.as_ref().unwrap().console_id)
                    {
                        let selected = &self.ui_state.selected_entries;
                        self.browser.consoles[console_index]
                            .entries
                            .retain(|entry| {
                                entry.id == focused
                                    || entry.id.is_some_and(|id| selected.contains(&id))
                            });
                    }
                    if let Some(id) = focused.filter(|id| ids.contains(id)) {
                        self.request_entry_detail(id, ctx);
                    }
                    if let (Some(root_path), Some(console_index)) = (
                        self.root_path.clone(),
                        self.browser.find_by_id(page_console_id),
                    ) {
                        crate::backend::assets::load_asset_statuses_for_page(
                            self.message_tx.clone(),
                            ctx.clone(),
                            root_path,
                            page_console_id,
                            self.browser.consoles[console_index].folder_name.clone(),
                            asset_rows,
                            self.settings.general.assets_dir.clone(),
                        );
                    }
                }
                Ok(crate::backend::library_store::LibraryStoreValue::EntryDetail(detail)) => {
                    let Some((pending_request, entry_id)) = self.pending_details_request else {
                        continue;
                    };
                    if pending_request != request_id {
                        continue;
                    }
                    self.pending_details_request = None;
                    let Some(detail) = detail.filter(|detail| detail.id == entry_id) else {
                        continue;
                    };
                    let console_id = detail.console_id;
                    let Some(entry) = detail_to_entry(detail) else {
                        self.push_error(
                            "Library database",
                            "An entry contains invalid persisted data",
                        );
                        continue;
                    };
                    let Some(console_index) = self.browser.find_by_id(console_id) else {
                        continue;
                    };
                    if let Some(existing) =
                        self.browser.consoles[console_index].entry_index(entry_id)
                    {
                        self.browser.consoles[console_index].entries[existing] = entry;
                    } else {
                        self.browser.consoles[console_index].entries.push(entry);
                    }
                }
                Ok(crate::backend::library_store::LibraryStoreValue::ConsoleDetails {
                    entries,
                    archived_releases,
                }) => {
                    let Some((pending_request, console_id, action)) =
                        self.pending_console_details_request
                    else {
                        continue;
                    };
                    if pending_request != request_id {
                        continue;
                    }
                    self.pending_console_details_request = None;
                    let Some(console_index) = self.browser.find_by_id(console_id) else {
                        continue;
                    };
                    self.browser.consoles[console_index].entries = entries
                        .into_iter()
                        .filter(|detail| detail.console_id == console_id)
                        .filter_map(detail_to_entry)
                        .collect();
                    self.ui_state.selected_entries = self.browser.consoles[console_index]
                        .entries
                        .iter()
                        .filter_map(|entry| entry.id)
                        .collect();
                    match action {
                        ConsoleDetailsAction::HashAll => {
                            crate::backend::hash::compute_hashes_for_selection(self, console_index);
                        }
                        ConsoleDetailsAction::ScrapeAll => {
                            crate::backend::assets::rescrape_media_for_console(
                                self,
                                console_index,
                                ctx,
                                archived_releases,
                            );
                        }
                        ConsoleDetailsAction::ScrapeMissing => {
                            crate::backend::assets::scrape_missing_artwork_for_console(
                                self,
                                console_index,
                                ctx,
                                archived_releases,
                            );
                        }
                        ConsoleDetailsAction::CompressAll => {
                            let indices: Vec<_> =
                                (0..self.browser.consoles[console_index].entries.len()).collect();
                            crate::backend::chd_compress::open_compress_dialog(
                                self,
                                console_index,
                                &indices,
                            );
                        }
                    }
                }
                Ok(crate::backend::library_store::LibraryStoreValue::EntryDetails(details)) => {
                    if self.pending_selection_details_request != Some(request_id) {
                        continue;
                    }
                    self.pending_selection_details_request = None;
                    for detail in details {
                        let console_id = detail.console_id;
                        let entry_id = detail.id;
                        let Some(entry) = detail_to_entry(detail) else {
                            continue;
                        };
                        let Some(console_index) = self.browser.find_by_id(console_id) else {
                            continue;
                        };
                        if let Some(existing) =
                            self.browser.consoles[console_index].entry_index(entry_id)
                        {
                            self.browser.consoles[console_index].entries[existing] = entry;
                        } else {
                            self.browser.consoles[console_index].entries.push(entry);
                        }
                    }
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
            } else if summary.likely_count > 0 {
                Some(crate::state::EntryStatus::LikelyMatched)
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

    pub(crate) fn refresh_console_summaries(&mut self, ctx: &egui::Context) {
        if let Some(root_id) = self.browser.root_id {
            self.submit_store(
                crate::backend::library_store::LibraryStoreRequest::ConsoleSummaries(root_id),
                ctx,
            );
        }
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

    pub fn commit_completed_scan(
        &mut self,
        snapshot: crate::backend::library_store::CompletedConsoleScan,
        ctx: &egui::Context,
    ) {
        let folder_name = snapshot.folder_name.clone();
        if let Some(request_id) = self.queue_store(
            crate::backend::library_store::LibraryStoreRequest::CommitConsoleScan(snapshot),
        ) {
            self.pending_scan_commits.insert(request_id, folder_name);
            ctx.request_repaint_after(Duration::from_millis(20));
        } else {
            crate::state::finish_auto_scan(self, &folder_name, false, ctx);
        }
    }

    /// Request the one rich row needed by the detail panel or an explicit
    /// entry action. Page navigation itself only loads the compact projection.
    pub fn request_entry_detail(
        &mut self,
        entry_id: retro_junk_db::LibraryEntryId,
        ctx: &egui::Context,
    ) {
        if self
            .selected_console_index()
            .is_some_and(|index| self.browser.consoles[index].entry_by_id(entry_id).is_some())
            || self
                .pending_details_request
                .is_some_and(|(_, pending)| pending == entry_id)
        {
            return;
        }
        if let Some(request_id) = self
            .queue_store(crate::backend::library_store::LibraryStoreRequest::EntryDetail(entry_id))
        {
            self.pending_details_request = Some((request_id, entry_id));
            ctx.request_repaint_after(Duration::from_millis(20));
        }
    }

    pub fn request_selected_entry_details(&mut self, ctx: &egui::Context) {
        let Some(console_index) = self.selected_console_index() else {
            return;
        };
        if self.pending_selection_details_request.is_some() {
            return;
        }
        let console = &self.browser.consoles[console_index];
        let missing: Vec<_> = self
            .ui_state
            .selected_entries
            .iter()
            .copied()
            .filter(|id| console.entry_by_id(*id).is_none())
            .collect();
        if missing.is_empty() {
            return;
        }
        if let Some(request_id) = self
            .queue_store(crate::backend::library_store::LibraryStoreRequest::EntryDetails(missing))
        {
            self.pending_selection_details_request = Some(request_id);
            ctx.request_repaint_after(Duration::from_millis(20));
        }
    }

    pub fn selected_entry_details_loaded(&self) -> bool {
        self.selected_console_index().is_some_and(|console_index| {
            let console = &self.browser.consoles[console_index];
            self.ui_state
                .selected_entries
                .iter()
                .all(|id| console.entry_by_id(*id).is_some())
        })
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
        self.load_console_details_for_action(console_idx, ConsoleDetailsAction::HashAll, ctx);
    }

    pub fn scrape_all_media(&mut self, console_idx: usize, ctx: &egui::Context) {
        self.load_console_details_for_action(console_idx, ConsoleDetailsAction::ScrapeAll, ctx);
    }

    pub fn scrape_missing_artwork(&mut self, console_idx: usize, ctx: &egui::Context) {
        self.load_console_details_for_action(console_idx, ConsoleDetailsAction::ScrapeMissing, ctx);
    }

    pub fn compress_all_to_chd(&mut self, console_idx: usize, ctx: &egui::Context) {
        self.load_console_details_for_action(console_idx, ConsoleDetailsAction::CompressAll, ctx);
    }

    fn load_console_details_for_action(
        &mut self,
        console_idx: usize,
        action: ConsoleDetailsAction,
        ctx: &egui::Context,
    ) {
        let Some(console_id) = self
            .browser
            .consoles
            .get(console_idx)
            .and_then(|console| console.id)
        else {
            return;
        };
        if self.pending_console_details_request.is_some() {
            return;
        }
        if let Some(request_id) = self.queue_store(
            crate::backend::library_store::LibraryStoreRequest::ConsoleEntryDetails(console_id),
        ) {
            self.pending_console_details_request = Some((request_id, console_id, action));
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

    /// Number of logical rows selected in the unified Library table.
    ///
    /// Playable entries grouped under a selected archive release are action
    /// targets for that release, not additional selected rows.
    pub fn selected_library_row_count(&self) -> usize {
        let grouped_entry_ids = self
            .browser
            .active_page
            .as_ref()
            .into_iter()
            .flat_map(|page| &page.archived_releases)
            .filter(|release| {
                self.ui_state
                    .selected_archive_releases
                    .contains(&release.summary.archive_release_id)
            })
            .flat_map(|release| {
                release
                    .playable_library_entries
                    .iter()
                    .map(|entry| entry.id)
            })
            .collect::<HashSet<_>>();
        self.ui_state.selected_archive_releases.len()
            + self
                .ui_state
                .selected_entries
                .iter()
                .filter(|id| !grouped_entry_ids.contains(id))
                .count()
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

    /// Cancel only work whose output exists to populate the previous UI
    /// projection. Durable jobs keep running across navigation/root changes.
    pub fn cancel_root_scoped_operations(&mut self) {
        for operation in &self.operations {
            if operation.kind == crate::state::OperationKind::UiFetch {
                operation
                    .cancel_token
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        self.operations
            .retain(|operation| operation.kind != crate::state::OperationKind::UiFetch);
    }

    pub fn reset_root_session_requests(&mut self) {
        self.store_requests_in_flight.clear();
        self.pending_page_request = None;
        self.pending_details_request = None;
        self.pending_selection_details_request = None;
        self.pending_console_details_request = None;
        self.pending_scan_commits.clear();
        self.pending_first_open_scan = false;
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

    /// Persist derived analysis from stable job-owned state. The entry need
    /// not exist in any currently loaded UI projection.
    pub fn publish_entry_analysis_snapshots(
        &mut self,
        entries: &[crate::state::LibraryEntry],
        ctx: &egui::Context,
    ) {
        let commands = entries
            .iter()
            .filter_map(|entry| {
                let entry_id = entry.id?;
                match crate::cache::entry_analysis_update(entry) {
                    Ok(update) => Some(retro_junk_db::EntryAnalysisCommand {
                        entry_id,
                        expected_source_revision: entry.source_revision,
                        update,
                    }),
                    Err(error) => {
                        log::warn!("Could not serialize analysis for entry {entry_id:?}: {error}");
                        None
                    }
                }
            })
            .collect::<Vec<_>>();
        if !commands.is_empty() {
            self.submit_store(
                crate::backend::library_store::LibraryStoreRequest::ApplyAnalysisBatch(commands),
                ctx,
            );
        }
    }

    /// Persist an explicit hash job from its stable entry snapshot. Unlike
    /// `publish_entry_analysis`, this does not depend on the entry still being
    /// present in the active UI projection.
    pub fn publish_entry_hash_update(
        &mut self,
        entry: &crate::state::LibraryEntry,
        ctx: &egui::Context,
    ) {
        let Some(entry_id) = entry.id else {
            return;
        };
        match crate::cache::entry_hash_update(entry) {
            Ok(update) => self.submit_store(
                crate::backend::library_store::LibraryStoreRequest::ApplyHashUpdate {
                    entry_id,
                    expected_source_revision: entry.source_revision,
                    update,
                },
                ctx,
            ),
            Err(error) => self.push_error("Library hash update", error.to_string()),
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
        self.message_tx = self
            .message_tx
            .for_generation(self.library_controller.session_generation);
        self.cancel_root_scoped_operations();
        self.reset_root_session_requests();
        self.release_detail_assets(ctx);
        self.browser = LibraryBrowserState::default();
        self.ui_state.selected_console = None;
        self.ui_state.focused_entry = None;
        self.ui_state.selected_entries.clear();
        self.ui_state.selected_archive_releases.clear();
        self.ui_state.page_offset = 0;
        self.ui_state.loading_library = true;
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
    crate::cache::detail_to_entry(detail)
}

impl eframe::App for RetroJunkApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = &ui.ctx().clone();

        // Drain background messages
        self.process_pending_messages(ctx);
        self.reconcile_detail_assets(ctx);

        // View switching, Find, and the log-viewer toggle arrive as menubar
        // accelerators rather than ad-hoc `consume_key` calls, so every
        // shortcut the app answers to is discoverable in the menu.
        self.process_menu_events(ctx);

        // Pick up work the daemon or a CLI run committed since the last frame.
        self.poll_dirty_tick(ctx);

        // Fallback polling for workers which only send channel messages and
        // cannot wake egui directly. Progress producers request immediate
        // repaints, so this need not run at animation cadence.
        if self.has_active_operations() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        if !self.store_requests_in_flight.is_empty() {
            // The store owns no UI handle, so keep polling until every request
            // has produced a reply. This closes the one-shot wakeup race.
            ctx.request_repaint_after(Duration::from_millis(20));
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

                let pending_reviews = self.ui_state.inbox.pending_count();
                let view = &mut self.ui_state.current_view;
                ui.selectable_value(
                    view,
                    View::Collection,
                    icons::labeled(icons::COLLECTION, "Collection"),
                );
                ui.selectable_value(
                    view,
                    View::Library,
                    icons::labeled(icons::LIBRARY, "Library"),
                );
                // The count sits in the label rather than a separate badge
                // so a reviewable item is visible without opening the view.
                let inbox_label = if pending_reviews > 0 {
                    format!("{}  Inbox ({pending_reviews})", icons::INBOX)
                } else {
                    icons::labeled(icons::INBOX, "Inbox")
                };
                ui.selectable_value(view, View::Inbox, inbox_label);
                ui.selectable_value(
                    view,
                    View::Settings,
                    icons::labeled(icons::SETTINGS, "Settings"),
                );
                ui.selectable_value(view, View::Tools, icons::labeled(icons::TOOLS, "Tools"));
            });

        // Trigger refresh when switching to Tools view
        if self.ui_state.current_view == View::Tools && prev_view != View::Tools {
            self.ui_state.tools_state.needs_refresh = true;
        }

        // Bottom panels render in order: status bar (bottommost), log viewer, activity bar.
        // egui stacks bottom panels upward, so the first one rendered sits at the very bottom.
        if widgets::status_bar::show(ui, self.ui_state.open_suggestion_count) {
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
            View::Collection => views::collection::show(ui, self),
            View::Library => views::library::show(ui, self, ctx),
            View::Inbox => views::inbox::show(ui, self, ctx),
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

        views::collection::show_import_modal(ctx, self);

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

        // Toasts render last so they overlay every panel and dialog.
        self.toasts.show(ctx);
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
            crate::state::handle_message(self, message.payload, &ctx);
        }

        // Commands already queued by completed producers are committed before
        // the store worker acknowledges shutdown.
        if let Some(store) = &mut self.library_store {
            store.shutdown_and_join(self.library_controller.session_generation);
        }

        // Save settings
        if let Some(root) = self.root_path.clone() {
            self.settings.library.ensure_profile_for_root(&root);
        } else {
            self.settings.library.current_root = None;
        }
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

    let outcome = widgets::modal::show(
        ctx,
        "organize_preview_dialog",
        "Organize Disc Files",
        550.0,
        |ui| {
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

            ui.add_space(4.0);

            egui::ScrollArea::vertical()
                .max_height(350.0)
                .show(ui, |ui| {
                    for job in &plan.jobs {
                        ui.horizontal(|ui| {
                            ui.colored_label(crate::theme::STATUS_OK, widgets::icons::ARROW_RIGHT);
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
                                ui.colored_label(
                                    crate::theme::STATUS_WARN,
                                    widgets::icons::WARNING,
                                );
                                ui.label(format!("{} \u{2014} {}", name, uf.reason));
                            });
                        }
                    }
                });

            widgets::modal::footer(ui, |ui| {
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
        },
    );

    if execute {
        let (folder_name, plan) = app.ui_state.pending_organize_plan.take().unwrap();
        crate::backend::organize::execute_organize_plan(app, folder_name, plan, ctx);
    } else if dismiss || outcome.dismissed {
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
