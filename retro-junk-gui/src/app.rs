use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use retro_junk_dat::DatIndex;
use retro_junk_lib::AnalysisContext;

use crate::settings::AppSettings;
use crate::state::{AppMessage, BackgroundOperation, Library, View};
use crate::views;
use crate::widgets;

/// Main application state.
pub struct RetroJunkApp {
    /// Analysis context with all registered console analyzers.
    pub context: Arc<AnalysisContext>,

    /// Current sidebar navigation selection.
    pub current_view: View,

    /// Root path for the ROM library.
    pub root_path: Option<std::path::PathBuf>,

    /// ROM library state.
    pub library: Library,

    /// Loaded DAT indices, keyed by folder_name.
    /// Stored separately from ConsoleState because hash matching needs
    /// immutable access to the index while mutating entries.
    pub dat_indices: HashMap<String, DatIndex>,

    /// Active background operations (shown in activity bar).
    pub operations: Vec<BackgroundOperation>,

    /// Receiver for messages from background threads.
    pub message_rx: mpsc::Receiver<AppMessage>,

    /// Sender cloned into background threads.
    pub message_tx: mpsc::Sender<AppMessage>,

    /// Index of the currently selected console in `library.consoles`.
    pub selected_console: Option<usize>,

    /// Index of the focused entry in the selected console's entries list.
    pub focused_entry: Option<usize>,

    /// Set of selected entry indices (for multi-select).
    pub selected_entries: std::collections::HashSet<usize>,

    /// Text filter for the game table.
    pub filter_text: String,

    /// Whether the detail panel is visible.
    pub detail_panel_open: bool,

    /// Persistent settings (library roots, preferences).
    pub settings: AppSettings,

    /// Connection to the catalog database (for cover/screen title enrichment).
    /// `None` if the catalog DB doesn't exist yet (user hasn't run catalog import).
    pub catalog_db: Option<retro_junk_db::Connection>,
}

impl RetroJunkApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        crate::fonts::configure_cjk_fonts(&cc.egui_ctx);
        let (tx, rx) = mpsc::channel();
        let context = Arc::new(retro_junk_lib::create_default_context());
        let settings = crate::settings::load_settings();

        // Try to open the catalog DB for title enrichment
        let catalog_db = retro_junk_dat::cache::cache_dir()
            .ok()
            .map(|p| p.join("catalog.db"))
            .filter(|p| p.exists())
            .and_then(|p| retro_junk_db::open_database(&p).ok());

        let mut app = Self {
            context,
            current_view: View::Library,
            root_path: None,
            library: Library::default(),
            dat_indices: HashMap::new(),
            operations: Vec::new(),
            message_rx: rx,
            message_tx: tx,
            selected_console: None,
            focused_entry: None,
            selected_entries: std::collections::HashSet::new(),
            filter_text: String::new(),
            detail_panel_open: true,
            settings,
            catalog_db,
        };

        // Restore last open root from settings
        if let Some(ref root) = app.settings.library.current_root.clone()
            && root.is_dir()
        {
            app.root_path = Some(root.clone());

            // Load cache on a background thread so the window appears immediately.
            // The CacheLoaded handler merges restored data with any consoles
            // already discovered by the folder scan.
            let tx = app.message_tx.clone();
            let context = app.context.clone();
            let root_bg = root.clone();
            let ctx_bg = cc.egui_ctx.clone();
            std::thread::spawn(move || {
                if let Some((library, stale)) = crate::cache::load_library(&root_bg, &context) {
                    log::info!(
                        "Restored {} consoles from cache ({} stale)",
                        library.consoles.len(),
                        stale.len()
                    );
                    let _ = tx.send(crate::state::AppMessage::CacheLoaded { library });
                    ctx_bg.request_repaint();
                }
            });

            // Always scan disk to discover new/removed console folders.
            // ConsoleFolderFound handler deduplicates, so cached consoles keep their data.
            crate::backend::scan::scan_root_folder(&mut app, root.clone(), &cc.egui_ctx);
        }

        app
    }

    /// Drain all pending messages from background threads.
    fn process_messages(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.message_rx.try_recv() {
            crate::state::handle_message(self, msg, ctx);
        }
    }

    /// Returns true if any background operations are active.
    fn has_active_operations(&self) -> bool {
        !self.operations.is_empty()
    }

    /// Save the current library state to disk cache.
    pub fn save_library_cache(&self) {
        if let Some(ref root) = self.root_path
            && let Err(e) = crate::cache::save_library(root, &self.library)
        {
            log::warn!("Failed to save library cache: {}", e);
        }
    }
}

impl eframe::App for RetroJunkApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Drain background messages
        self.process_messages(ctx);

        // Schedule repaint while operations are running
        if self.has_active_operations() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        // Sidebar
        egui::SidePanel::left("sidebar")
            .resizable(false)
            .exact_width(120.0)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.heading("retro-junk");
                ui.separator();
                ui.add_space(4.0);

                let view = &mut self.current_view;
                ui.selectable_value(view, View::Library, "Library");
                ui.selectable_value(view, View::Settings, "Settings");
                ui.selectable_value(view, View::Tools, "Tools");
            });

        // Activity bar (bottom, only when operations active)
        if self.has_active_operations() {
            egui::TopBottomPanel::bottom("activity_bar").show(ctx, |ui| {
                widgets::activity_bar::show(ui, &mut self.operations);
            });
        }

        // Main content
        egui::CentralPanel::default().show(ctx, |ui| match self.current_view {
            View::Library => views::library::show(ui, self, ctx),
            View::Settings => views::settings::show(ui, self),
            View::Tools => views::tools::show(ui),
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Save settings
        self.settings.library.current_root = self.root_path.clone();
        if let Err(e) = crate::settings::save_settings(&self.settings) {
            log::warn!("Failed to save settings on exit: {}", e);
        }

        // Save library cache
        self.save_library_cache();
    }
}
