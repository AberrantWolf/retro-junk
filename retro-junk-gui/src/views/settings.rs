#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;

use crate::app::RetroJunkApp;
use crate::widgets::results_dialog::{STATUS_ERR, STATUS_OK};

/// Render the Settings view.
pub fn show(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    ui.heading("Settings");
    ui.separator();
    ui.add_space(8.0);

    egui::ScrollArea::vertical().show(ui, |ui| {
        show_library_section(ui, app);
        ui.add_space(16.0);
        show_output_directories_section(ui, app);
        ui.add_space(16.0);
        show_external_tools_section(ui, app);
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
        if ui.button("Browse...").clicked()
            && let Some(path) = rfd::FileDialog::new().pick_folder()
        {
            // Use the library view's switch logic
            let ctx = ui.ctx().clone();
            crate::views::library::switch_to_root(app, path, &ctx);
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
                    if let Some(ref conn) = app.catalog_db
                        && let Err(e) = crate::cache::delete_cache(conn, &path)
                    {
                        log::warn!("Failed to clear cache: {}", e);
                    }
                }
                RecentAction::Remove(idx) => {
                    let path = app.settings.library.recent_roots[idx].path.clone();
                    if let Some(ref conn) = app.catalog_db {
                        let _ = crate::cache::delete_cache(conn, &path);
                    }
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

fn show_output_directories_section(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    ui.strong("Output Directories");
    ui.add_space(4.0);

    let mut changed = false;

    ui.horizontal(|ui| {
        ui.label("Metadata directory:");
        let response = ui.add(
            egui::TextEdit::singleline(&mut app.settings.general.metadata_dir).desired_width(200.0),
        );
        // D9: save on focus-loss (Enter or click-away), not per keystroke —
        // `changed()` would write settings.toml on every character typed.
        if response.lost_focus() {
            changed = true;
        }
    });
    ui.indent("metadata_hint", |ui| {
        ui.weak("Relative to ROM root. Use \".\" for inline with ROMs (ES-DE legacy mode).");
    });

    ui.add_space(4.0);

    ui.horizontal(|ui| {
        ui.label("Media directory:");
        let response = ui.add(
            egui::TextEdit::singleline(&mut app.settings.general.assets_dir).desired_width(200.0),
        );
        if response.lost_focus() {
            changed = true;
        }
    });
    ui.indent("media_hint", |ui| {
        ui.weak("Relative to ROM root. Leave empty for \"{root}-media\" sibling convention.");
    });

    if changed {
        let _ = crate::settings::save_settings(&app.settings);
    }
}

fn show_external_tools_section(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    ui.strong("External Tools");
    ui.add_space(4.0);

    let mut changed = false;
    let mut editing = false;

    ui.horizontal(|ui| {
        ui.label("chdman path:");
        let response = ui.add(
            egui::TextEdit::singleline(&mut app.settings.general.chdman_path)
                .desired_width(280.0)
                .hint_text("chdman (from PATH)"),
        );
        editing = response.has_focus();
        // D9: save on focus-loss, not per keystroke. Keep the Browse-button
        // save below as-is — a discrete event, not a keystroke stream.
        if response.lost_focus() {
            changed = true;
        }
        if ui.button("Browse...").clicked()
            && let Some(path) = rfd::FileDialog::new().pick_file()
        {
            app.settings.general.chdman_path = path.display().to_string();
            changed = true;
        }
    });
    ui.indent("chdman_hint", |ui| {
        ui.weak("Used to compress disc images to CHD. Leave empty to use chdman from PATH.");
    });

    // D1: probe chdman on a background thread whenever the configured path
    // changes — `Chdman::detect` spawns a subprocess with no timeout, and a
    // hung configured binary must not freeze the UI thread. Not per
    // keystroke while the field is focused, and not while a probe for this
    // exact path is already in flight.
    let path_key = app.settings.general.chdman_path.trim().to_string();
    let needs_probe = app
        .chdman_probe
        .as_ref()
        .is_none_or(|(probed_for, _)| probed_for != &path_key);
    if needs_probe && !editing && !app.chdman_probe_in_flight {
        app.chdman_probe_in_flight = true;
        let tx = app.message_tx.clone();
        let egui_ctx = ui.ctx().clone();
        let key = path_key.clone();
        std::thread::spawn(move || {
            let result = retro_junk_lib::chd_convert::Chdman::detect_from_setting(&key);
            let _ = tx.send(crate::state::AppMessage::ChdmanProbeResult { key, result });
            egui_ctx.request_repaint();
        });
    }

    if app.chdman_probe_in_flight {
        ui.indent("chdman_status", |ui| {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak("Checking for chdman…");
            });
        });
    } else if let Some((_, result)) = &app.chdman_probe {
        let (status_color, status_text, install_hint): (
            egui::Color32,
            String,
            Option<&'static str>,
        ) = match result {
            Ok(chdman) => {
                let version = chdman.version.as_deref().unwrap_or("(unknown version)");
                (
                    STATUS_OK,
                    format!("chdman {version} found: {}", chdman.path.display()),
                    None,
                )
            }
            Err(e) => (
                STATUS_ERR,
                e.to_string(),
                Some(retro_junk_lib::chd_convert::ChdmanUnavailable::install_hint()),
            ),
        };

        ui.indent("chdman_status", |ui| {
            ui.horizontal(|ui| {
                ui.colored_label(status_color, &status_text);
                // Forces a fresh probe next frame — fixes a stale cached
                // Err staying stuck if the user installs chdman without
                // restarting the app.
                if ui.small_button("Re-check").clicked() {
                    app.chdman_probe = None;
                }
            });
            if let Some(hint) = install_hint {
                ui.weak(hint);
            }
        });
    }

    if changed {
        let _ = crate::settings::save_settings(&app.settings);
    }
}

fn show_cache_section(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    ui.strong("Cache Management");
    ui.add_space(4.0);

    // Library cache (stored in SQLite)
    ui.horizontal(|ui| {
        ui.label("Library cache: stored in catalog DB");
        if ui.small_button("Clear All").clicked() {
            if let Some(ref conn) = app.catalog_db
                && let Err(e) = crate::cache::clear_all_caches(conn)
            {
                log::warn!("Failed to clear library caches: {}", e);
            }
        }
    });

    // DAT cache
    let dat_cache_size = retro_junk_dat::cache::total_cache_size().unwrap_or(0);
    ui.horizontal(|ui| {
        ui.label(format!(
            "DAT cache: {}",
            format_bytes_approx(dat_cache_size)
        ));
        if ui.small_button("Clear All").clicked()
            && let Err(e) = retro_junk_dat::cache::clear()
        {
            log::warn!("Failed to clear DAT cache: {}", e);
        }
    });
}

use retro_junk_lib::util::format_bytes_approx;

enum RecentAction {
    Open(std::path::PathBuf),
    ClearCache(std::path::PathBuf),
    Remove(usize),
}
