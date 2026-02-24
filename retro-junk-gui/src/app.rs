use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use retro_junk_dat::DatIndex;
use retro_junk_lib::AnalysisContext;

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
}

impl RetroJunkApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            context: Arc::new(retro_junk_lib::create_default_context()),
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
        }
    }

    /// Drain all pending messages from background threads.
    fn process_messages(&mut self) {
        while let Ok(msg) = self.message_rx.try_recv() {
            crate::state::handle_message(self, msg);
        }
    }

    /// Returns true if any background operations are active.
    fn has_active_operations(&self) -> bool {
        !self.operations.is_empty()
    }
}

impl eframe::App for RetroJunkApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Drain background messages
        self.process_messages();

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
        egui::CentralPanel::default().show(ctx, |ui| {
            match self.current_view {
                View::Library => views::library::show(ui, self, ctx),
                View::Settings => views::settings::show(ui),
                View::Tools => views::tools::show(ui),
            }
        });
    }
}
