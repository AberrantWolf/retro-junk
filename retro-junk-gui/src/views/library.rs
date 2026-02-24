use crate::app::RetroJunkApp;
use crate::backend;
use crate::widgets;

/// Render the three-pane library view.
pub fn show(ui: &mut egui::Ui, app: &mut RetroJunkApp, ctx: &egui::Context) {
    if app.root_path.is_none() {
        // No folder open yet — show welcome screen
        show_welcome(ui, app, ctx);
        return;
    }

    // Three-pane layout: Console Tree | Game Table | Detail Panel

    // Console tree (left)
    egui::SidePanel::left("console_tree")
        .resizable(true)
        .default_width(200.0)
        .width_range(150.0..=300.0)
        .show_inside(ui, |ui| {
            ui.add_space(4.0);

            // Open folder button at top of tree
            if ui.button("Open Folder...").clicked() {
                open_folder(app, ctx);
            }
            ui.separator();

            egui::ScrollArea::vertical().show(ui, |ui| {
                widgets::console_tree::show(ui, app, ctx);
            });
        });

    // Detail panel (right, collapsible)
    if app.detail_panel_open {
        egui::SidePanel::right("detail_panel")
            .resizable(true)
            .default_width(280.0)
            .width_range(200.0..=500.0)
            .show_inside(ui, |ui| {
                widgets::detail_panel::show(ui, app);
            });
    }

    // Game table (center, fills remaining space)
    egui::CentralPanel::default().show_inside(ui, |ui| {
        // Toolbar
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut app.filter_text)
                .hint_text("Filter...")
                .desired_width(200.0));

            ui.separator();

            let has_selection = !app.selected_entries.is_empty();
            if ui.add_enabled(has_selection, egui::Button::new("Calculate Hashes")).clicked() {
                if let Some(ci) = app.selected_console {
                    backend::hash::compute_hashes_for_selection(app, ci);
                }
            }

            ui.separator();

            // Toggle detail panel
            let label = if app.detail_panel_open { "Hide Detail" } else { "Show Detail" };
            if ui.button(label).clicked() {
                app.detail_panel_open = !app.detail_panel_open;
            }
        });

        ui.separator();

        // Game table
        if app.selected_console.is_some() {
            widgets::game_table::show(ui, app);
        } else {
            ui.centered_and_justified(|ui| {
                ui.label("Select a console from the tree to view its games.");
            });
        }
    });
}

fn show_welcome(ui: &mut egui::Ui, app: &mut RetroJunkApp, ctx: &egui::Context) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() / 3.0);
        ui.heading("retro-junk Library Manager");
        ui.add_space(16.0);
        ui.label("Open a folder containing your ROM collection to get started.");
        ui.label("The folder should contain subfolders named after consoles (e.g., snes, n64, ps1).");
        ui.add_space(16.0);
        if ui.button("Open Folder...").clicked() {
            open_folder(app, ctx);
        }
    });
}

fn open_folder(app: &mut RetroJunkApp, ctx: &egui::Context) {
    if let Some(path) = rfd::FileDialog::new().pick_folder() {
        app.root_path = Some(path.clone());
        app.library = crate::state::Library::default();
        app.selected_console = None;
        app.focused_entry = None;
        app.selected_entries.clear();
        backend::scan::scan_root_folder(app, path, ctx);
    }
}
