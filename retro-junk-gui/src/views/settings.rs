use crate::app::RetroJunkApp;

/// Render the Settings view.
pub fn show(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    ui.heading("Settings");
    ui.separator();
    ui.add_space(8.0);

    egui::ScrollArea::vertical().show(ui, |ui| {
        show_library_section(ui, app);
        ui.add_space(16.0);
        show_cache_section(ui, app);
    });
}

fn show_library_section(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    ui.strong("Library");
    ui.add_space(4.0);

    // Current root
    ui.horizontal(|ui| {
        ui.label("Current root:");
        if let Some(ref root) = app.root_path {
            ui.monospace(root.display().to_string());
        } else {
            ui.weak("None");
        }
        if ui.button("Browse...").clicked() {
            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                // Use the library view's switch logic
                let ctx = ui.ctx().clone();
                crate::views::library::switch_to_root(app, path, &ctx);
            }
        }
    });

    ui.add_space(8.0);

    // Recent roots
    if !app.settings.library.recent_roots.is_empty() {
        ui.label("Recent Roots:");
        ui.add_space(4.0);

        let mut action = None;
        for (i, recent) in app.settings.library.recent_roots.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.monospace(recent.path.display().to_string());
                ui.weak(format!("{} consoles", recent.console_count));

                if ui.small_button("Open").clicked() {
                    action = Some(RecentAction::Open(recent.path.clone()));
                }
                if ui.small_button("Clear Cache").clicked() {
                    action = Some(RecentAction::ClearCache(recent.path.clone()));
                }
                if ui.small_button("Remove").clicked() {
                    action = Some(RecentAction::Remove(i));
                }
            });
        }

        if let Some(action) = action {
            match action {
                RecentAction::Open(path) => {
                    let ctx = ui.ctx().clone();
                    crate::views::library::switch_to_root(app, path, &ctx);
                }
                RecentAction::ClearCache(path) => {
                    if let Err(e) = crate::cache::delete_cache(&path) {
                        log::warn!("Failed to clear cache: {}", e);
                    }
                }
                RecentAction::Remove(idx) => {
                    let path = app.settings.library.recent_roots[idx].path.clone();
                    let _ = crate::cache::delete_cache(&path);
                    app.settings.library.recent_roots.remove(idx);
                    let _ = crate::settings::save_settings(&app.settings);
                }
            }
        }
    }

    ui.add_space(8.0);

    // Auto-scan toggle
    ui.checkbox(
        &mut app.settings.general.auto_scan_on_open,
        "Auto-scan consoles on open",
    );

    // Region override warning toggle
    ui.checkbox(
        &mut app.settings.general.warn_on_region_override,
        "Warn when overriding a specific detected region",
    );
}

fn show_cache_section(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    ui.strong("Cache Management");
    ui.add_space(4.0);

    // Library cache
    let lib_cache_size = crate::cache::total_cache_size();
    ui.horizontal(|ui| {
        ui.label(format!("Library cache: {}", format_size(lib_cache_size)));
        if ui.small_button("Clear All").clicked() {
            if let Err(e) = crate::cache::clear_all_caches() {
                log::warn!("Failed to clear library caches: {}", e);
            }
        }
    });

    // DAT cache
    let dat_cache_size = retro_junk_dat::cache::total_cache_size().unwrap_or(0);
    ui.horizontal(|ui| {
        ui.label(format!("DAT cache: {}", format_size(dat_cache_size)));
        if ui.small_button("Clear All").clicked() {
            if let Err(e) = retro_junk_dat::cache::clear() {
                log::warn!("Failed to clear DAT cache: {}", e);
            }
        }
    });
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

enum RecentAction {
    Open(std::path::PathBuf),
    ClearCache(std::path::PathBuf),
    Remove(usize),
}
