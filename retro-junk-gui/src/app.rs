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

use retro_junk_dat::DatIndex;
use retro_junk_lib::AnalysisContext;

use crate::settings::AppSettings;
use crate::state::{
    AppMessage, BackgroundOperation, CueFixOutcome, CueFixResult, FocusedPanel, LibraryState,
    RenameOutcome, RenameResult, ToolsState, View,
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
    /// Index of the currently selected console in `library.consoles`.
    pub selected_console: Option<usize>,
    /// One-shot request to scroll the console tree to an index.
    pub scroll_to_console: Option<usize>,
    /// Index of the focused entry in the selected console's entries list.
    pub focused_entry: Option<usize>,
    /// Selected entry indices for multi-select.
    pub selected_entries: HashSet<usize>,
    /// Text filter for the game table.
    pub filter_text: String,
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
            detail_panel_open: true,
            results_dialog: ResultsDialog::None,
            chd_compress_prompt: None,
            chdman_probe: ChdmanProbe::Idle,
            credential_status: None,
            credential_info_popup: None,
            loading_library: false,
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
    pub library: LibraryState,

    /// Loaded DAT indices, keyed by `folder_name`.
    /// Stored separately from `ConsoleState` because hash matching needs
    /// immutable access to the index while mutating entries.
    pub dat_indices: HashMap<String, Arc<DatIndex>>,

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

    /// Connection to the catalog database (for enrichment + library cache).
    /// `None` only if the database file could not be opened.
    pub catalog_db: Option<retro_junk_db::Connection>,

    /// Path to the catalog database file (for opening separate connections in background threads).
    pub db_path: Option<std::path::PathBuf>,

    /// Serialized owner of all revisioned library-table reads and writes.
    pub library_store: Option<crate::backend::library_store::LibraryStore>,

    /// Rejects stale/superseded projections and tracks durable UI identity.
    pub library_controller: crate::backend::library_store::LibraryProjectionController,

    /// `JoinHandle`s for every spawned background-operation thread, keyed by
    /// `op_id`. Joined (and removed) when the operation completes, or all at
    /// once in `on_exit` so the process never dies mid-write (D2).
    pub op_threads: HashMap<u64, std::thread::JoinHandle<()>>,
}

impl RetroJunkApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings = crate::settings::load_settings();

        // Always open (or create) the catalog DB — used for enrichment + library cache
        let db_path = retro_junk_dat::cache::cache_dir()
            .ok()
            .map(|p| p.join("catalog.db"));
        let catalog_db = db_path
            .as_ref()
            .and_then(|p| retro_junk_db::open_database(p).ok());

        let mut app = Self::with_parts(&cc.egui_ctx, settings, catalog_db, db_path);

        // Restore last open root from settings
        if let Some(ref root) = app.settings.library.current_root.clone() {
            log::info!("Settings current_root: {}", root.display());
            if !root.is_dir() {
                log::warn!("current_root is not a directory, skipping auto-load");
            }
        }
        if let Some(ref root) = app.settings.library.current_root.clone()
            && root.is_dir()
        {
            if let Some(kind) = crate::util::fragile_mount_kind(root) {
                // Don't auto-load a fragile network mount at startup; show the
                // warning dialog instead. Confirming resumes the load via
                // switch_to_root_unchecked.
                app.ui_state.fragile_mount_prompt = Some(crate::state::FragileMountPrompt {
                    root: root.clone(),
                    kind,
                });
                return app;
            }
            app.root_path = Some(root.clone());
            app.ui_state.loading_library = true;

            // Load cache first, then scan. The cache thread sends CacheLoaded
            // (if a cache exists) followed by StartFolderScan. This ordering
            // ensures cached data (hashes, status, dat_match) is fully merged
            // before any scan can overwrite it.
            let tx = app.message_tx.clone();
            let context = app.context.clone();
            let root_bg = root.clone();
            let db_path_bg = app.db_path.clone();
            let ctx_bg = cc.egui_ctx.clone();
            std::thread::spawn(move || {
                // Open a separate DB connection for this thread (WAL allows concurrent readers)
                let bg_conn = db_path_bg
                    .as_ref()
                    .and_then(|p| retro_junk_db::open_database(p).ok());

                if let Some(ref conn) = bg_conn {
                    // Migrate legacy JSON cache if it exists
                    crate::cache::migrate_json_cache(conn, &root_bg, &context);

                    if let Some((library, stale)) =
                        crate::cache::load_library(conn, &root_bg, &context)
                    {
                        log::info!(
                            "Restored {} consoles from cache ({} stale)",
                            library.consoles.len(),
                            stale.len()
                        );
                        let _ = tx.send(crate::state::AppMessage::CacheLoaded { library });
                        // Repaint immediately so cached entries are visible before
                        // the folder scan starts (which may take a moment).
                        ctx_bg.request_repaint();
                    }
                }
                // Always trigger a folder scan to discover new/removed consoles.
                let _ = tx.send(crate::state::AppMessage::StartFolderScan);
                ctx_bg.request_repaint();
            });
        }

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
            library: LibraryState::default(),
            dat_indices: HashMap::new(),
            operations: Vec::new(),
            message_rx: rx,
            message_tx: tx,
            ui_state: UiState::default(),
            settings,
            catalog_db,
            db_path,
            library_store,
            library_controller: Default::default(),
            op_threads: HashMap::new(),
        }
    }

    /// Drain all pending messages from background threads.
    fn process_pending_messages(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.message_rx.try_recv() {
            crate::state::handle_message(self, msg, ctx);
        }
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

    /// Save one console's entries to the database.
    pub fn save_console_cache(&mut self, console_idx: usize) {
        let error = self
            .root_path
            .as_ref()
            .zip(self.catalog_db.as_ref())
            .zip(self.library.consoles.get(console_idx))
            .and_then(|((root, conn), console)| {
                crate::cache::save_console(conn, root, console).err()
            });
        if let Some(error) = error {
            log::warn!("Failed to publish console scan: {error}");
            self.push_error("Library scan", error.to_string());
        }
    }

    /// Save specific entries within a console to the database.
    pub fn save_entry_cache(&mut self, console_idx: usize, entry_indices: &[usize]) {
        let error = self
            .root_path
            .as_ref()
            .zip(self.catalog_db.as_ref())
            .zip(self.library.consoles.get(console_idx))
            .and_then(|((root, conn), console)| {
                crate::cache::save_entries(conn, root, console, entry_indices).err()
            });
        if let Some(error) = error {
            log::warn!("Failed to publish entry edit: {error}");
            self.push_error("Library update", error.to_string());
        }
    }

    /// Push an error that will be shown to the user in a modal dialog.
    pub fn push_error(&mut self, category: impl Into<String>, message: impl Into<String>) {
        self.ui_state.error_list.push(crate::state::UserError {
            category: category.into(),
            message: message.into(),
        });
    }
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
        widgets::error_dialog::show(ctx, &mut self.ui_state.error_list);
    }

    fn on_exit(&mut self) {
        log::info!("on_exit: stopping background work");

        // Cancel every in-flight background operation and join its thread
        // before saving (D2).
        self.cancel_and_join_all_operations();
        // Apply whatever completion messages those threads sent (e.g.
        // ChdCompressComplete) before saving, so the persisted cache
        // reflects the final post-cancellation state.
        let ctx = egui::Context::default();
        self.process_pending_messages(&ctx);

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
