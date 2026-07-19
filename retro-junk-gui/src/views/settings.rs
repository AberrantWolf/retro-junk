#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;

use crate::app::{ChdmanProbe, RetroJunkApp};
use crate::widgets::results_dialog::{STATUS_ERR, STATUS_OK, STATUS_WARN};

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
        show_scraper_section(ui, app);
        ui.add_space(16.0);
        show_cache_section(ui, app);
    });

    show_credential_info_popup(ui.ctx(), app);
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
                    app.delete_library_cache(&path, ui.ctx());
                }
                RecentAction::Remove(idx) => {
                    let path = app.settings.library.recent_roots[idx].path.clone();
                    app.delete_library_cache(&path, ui.ctx());
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
    let needs_probe = match &app.ui_state.chdman_probe {
        ChdmanProbe::Idle => true,
        ChdmanProbe::Probing => false,
        ChdmanProbe::Done { path, .. } => path != &path_key,
    };
    if needs_probe && !editing {
        app.ui_state.chdman_probe = ChdmanProbe::Probing;
        let tx = app.message_tx.clone();
        let egui_ctx = ui.ctx().clone();
        let key = path_key.clone();
        std::thread::spawn(move || {
            let result = retro_junk_lib::chd_convert::Chdman::detect_from_setting(&key);
            let _ = tx.send(crate::state::AppMessage::ChdmanProbeResult { key, result });
            egui_ctx.request_repaint();
        });
    }

    if matches!(app.ui_state.chdman_probe, ChdmanProbe::Probing) {
        ui.indent("chdman_status", |ui| {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak("Checking for chdman…");
            });
        });
    } else if let ChdmanProbe::Done { result, .. } = &app.ui_state.chdman_probe {
        let (status_color, status_text, install_hint): (
            egui::Color32,
            String,
            Option<&'static str>,
        ) = match result {
            Ok(chdman) => {
                let version: &str = if chdman.version.is_empty() {
                    "(unknown version)"
                } else {
                    &chdman.version
                };
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
                    app.ui_state.chdman_probe = ChdmanProbe::Idle;
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

/// How long a cached credential-provenance snapshot stays fresh. Short enough
/// that edits made in an external editor show up on the next repaint after
/// returning to the window, long enough to avoid re-reading the file and
/// environment on every frame.
const CREDENTIAL_STATUS_TTL: std::time::Duration = std::time::Duration::from_secs(2);

fn show_scraper_section(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    use retro_junk_scraper::{CREDENTIAL_FIELDS, CredentialSource};

    ui.strong("ScreenScraper");
    ui.add_space(4.0);

    // Config file row
    let config_path = retro_junk_scraper::config_path();
    ui.horizontal(|ui| {
        ui.label("Config file:");
        match &config_path {
            Some(path) => {
                ui.monospace(path.display().to_string());
                if !path.exists() {
                    ui.weak("(not created yet)");
                }
            }
            None => {
                ui.colored_label(STATUS_ERR, "could not determine config directory");
            }
        }
        if ui.button("Open Config File").clicked() {
            match retro_junk_scraper::ensure_config_file() {
                Ok((path, created)) => {
                    if created {
                        log::info!("Created credentials template at {}", path.display());
                    }
                    crate::util::open_in_default_app(&path);
                    // The user is about to edit the file — drop the cached
                    // provenance so the next repaint re-reads it.
                    app.ui_state.credential_status = None;
                }
                Err(e) => {
                    app.ui_state.error_list.push(crate::state::UserError {
                        category: "Config".to_string(),
                        message: format!("Failed to create credentials file: {e}"),
                    });
                }
            }
        }
    });
    ui.indent("scraper_config_hint", |ui| {
        ui.weak(
            "Credentials for the ScreenScraper API, used when scraping metadata and artwork. \
             Environment variables override values in the file.",
        );
    });

    ui.add_space(8.0);

    // Refresh the cached provenance snapshot when stale. Reading it means
    // touching the filesystem and environment, so don't do it every frame.
    let stale = app
        .ui_state
        .credential_status
        .as_ref()
        .is_none_or(|(at, _)| at.elapsed() > CREDENTIAL_STATUS_TTL);
    if stale {
        app.ui_state.credential_status = Some((
            std::time::Instant::now(),
            retro_junk_scraper::credential_sources(),
        ));
    }
    // Keep the statuses live while the view is visible, even without input.
    ui.ctx().request_repaint_after(CREDENTIAL_STATUS_TTL);

    let sources = &app
        .ui_state
        .credential_status
        .as_ref()
        .expect("just refreshed")
        .1;
    let mut open_info = None;

    for meta in &CREDENTIAL_FIELDS {
        let source = sources.by_key(meta.key).expect("known field key");

        let (color, source_text) = match source {
            CredentialSource::Missing if meta.required => {
                (STATUS_ERR, "not set (required)".to_string())
            }
            CredentialSource::Missing => (egui::Color32::GRAY, "not set (optional)".to_string()),
            CredentialSource::Default => (STATUS_WARN, source.to_string()),
            _ => (STATUS_OK, source.to_string()),
        };

        ui.horizontal(|ui| {
            ui.colored_label(color, "●");
            ui.label(meta.label);
            ui.weak(format!("({source_text})"));
            if ui
                .small_button("ℹ")
                .on_hover_text(format!("What is {}?", meta.label))
                .clicked()
            {
                open_info = Some(meta);
            }
        });
    }

    if open_info.is_some() {
        app.ui_state.credential_info_popup = open_info;
    }
}

/// Modal explaining one credential field: what it is for and where to get it.
fn show_credential_info_popup(ctx: &egui::Context, app: &mut RetroJunkApp) {
    let Some(meta) = app.ui_state.credential_info_popup else {
        return;
    };

    let mut dismiss = false;
    let mut open = true;

    egui::Window::new(meta.label)
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .default_width(420.0)
        .show(ctx, |ui| {
            ui.label(meta.description);
            ui.add_space(8.0);

            ui.strong("Where to get it");
            ui.label(meta.how_to_obtain);
            ui.add_space(8.0);

            ui.strong("How to set it");
            ui.horizontal(|ui| {
                ui.label("Environment variable:");
                ui.monospace(meta.env_var);
            });
            ui.horizontal(|ui| {
                ui.label("Config file key:");
                ui.monospace(format!("[screenscraper] {}", meta.key));
            });

            ui.separator();
            if ui.button("Close").clicked() {
                dismiss = true;
            }
        });

    if dismiss || !open {
        app.ui_state.credential_info_popup = None;
    }
}

fn show_cache_section(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    ui.strong("Cache Management");
    ui.add_space(4.0);

    // Library cache (stored in SQLite)
    ui.horizontal(|ui| {
        ui.label("Library cache: stored in catalog DB");
        if ui.small_button("Clear All").clicked() {
            app.clear_library_caches(ui.ctx());
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
            log::warn!("Failed to clear DAT cache: {e}");
        }
    });
}

use retro_junk_lib::util::format_bytes_approx;

enum RecentAction {
    Open(std::path::PathBuf),
    ClearCache(std::path::PathBuf),
    Remove(usize),
}
