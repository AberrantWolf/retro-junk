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
            ui.add(
                egui::TextEdit::singleline(&mut app.filter_text)
                    .hint_text("Filter...")
                    .desired_width(200.0),
            );

            ui.separator();

            let has_selection = !app.selected_entries.is_empty();
            if ui
                .add_enabled(has_selection, egui::Button::new("Calculate Hashes"))
                .clicked()
                && let Some(ci) = app.selected_console
            {
                backend::hash::compute_hashes_for_selection(app, ci);
            }

            ui.separator();

            // Toggle detail panel
            let label = if app.detail_panel_open {
                "Hide Detail"
            } else {
                "Show Detail"
            };
            if ui.button(label).clicked() {
                app.detail_panel_open = !app.detail_panel_open;
            }
        });

        ui.separator();

        // Game table
        if app.selected_console.is_some() {
            widgets::game_table::show(ui, app, ctx);
        } else {
            ui.centered_and_justified(|ui| {
                ui.label("Select a console from the tree to view its games.");
            });
        }
    });
}

fn show_welcome(ui: &mut egui::Ui, app: &mut RetroJunkApp, ctx: &egui::Context) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() / 4.0);
        ui.heading("retro-junk Library Manager");
        ui.add_space(16.0);
        ui.label("Open a folder containing your ROM collection to get started.");
        ui.label(
            "The folder should contain subfolders named after consoles (e.g., snes, n64, ps1).",
        );
        ui.add_space(16.0);
        if ui.button("Open Folder...").clicked() {
            open_folder(app, ctx);
        }

        // Show recent roots
        if !app.settings.library.recent_roots.is_empty() {
            ui.add_space(24.0);
            ui.separator();
            ui.add_space(8.0);
            ui.label("Recent Libraries:");
            ui.add_space(4.0);

            let mut open_root = None;
            for recent in &app.settings.library.recent_roots {
                ui.horizontal(|ui| {
                    let label = format!(
                        "{}  ({} consoles)",
                        recent.path.display(),
                        recent.console_count
                    );
                    if ui.button(&label).clicked() {
                        open_root = Some(recent.path.clone());
                    }
                });
            }

            if let Some(root) = open_root {
                switch_to_root(app, root, ctx);
            }
        }
    });
}

fn open_folder(app: &mut RetroJunkApp, ctx: &egui::Context) {
    if let Some(path) = rfd::FileDialog::new().pick_folder() {
        switch_to_root(app, path, ctx);
    }
}

/// Switch to a new root path, saving the current library and loading the new one.
pub fn switch_to_root(app: &mut RetroJunkApp, new_root: std::path::PathBuf, ctx: &egui::Context) {
    // Save current library cache before switching
    app.save_library_cache();

    // Reset UI state
    app.selected_console = None;
    app.focused_entry = None;
    app.selected_entries.clear();
    app.dat_indices.clear();

    // Set new root
    app.root_path = Some(new_root.clone());

    // Load cache to restore previously computed work (hashes, DAT matches, etc.)
    if let Some((library, stale)) = crate::cache::load_library(&new_root, &app.context) {
        log::info!(
            "Restored {} consoles from cache ({} stale)",
            library.consoles.len(),
            stale.len()
        );
        app.library = library;

        // Auto-trigger DAT loads for consoles that had DATs loaded
        for console in &app.library.consoles {
            if matches!(console.dat_status, crate::state::DatStatus::Loaded { .. }) {
                crate::backend::dat::load_dat_for_console(
                    app.message_tx.clone(),
                    app.context.clone(),
                    console.platform,
                    console.folder_name.clone(),
                    ctx.clone(),
                );
            }
        }
    } else {
        app.library = crate::state::Library::default();
    }

    // Always scan disk to discover new/removed console folders.
    // ConsoleFolderFound handler deduplicates, so cached consoles keep their data.
    backend::scan::scan_root_folder(app, new_root.clone(), ctx);

    // Update settings: add/move to front of recent roots
    update_recent_roots(app, &new_root);

    // Save settings immediately
    app.settings.library.current_root = Some(new_root);
    if let Err(e) = crate::settings::save_settings(&app.settings) {
        log::warn!("Failed to save settings: {}", e);
    }
}

fn update_recent_roots(app: &mut RetroJunkApp, root: &std::path::Path) {
    let recent = &mut app.settings.library.recent_roots;

    // Remove existing entry for this path
    recent.retain(|r| r.path != root);

    // Add to front
    recent.insert(
        0,
        crate::settings::RecentRoot {
            path: root.to_path_buf(),
            last_opened: chrono::Utc::now().to_rfc3339(),
            console_count: app.library.consoles.len(),
        },
    );

    // Cap at 10 entries
    recent.truncate(10);
}
