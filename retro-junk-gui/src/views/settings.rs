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
        show_automation_section(ui, app);
        ui.add_space(16.0);
        show_daemon_section(ui, app);
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

    let mut profile_action = None;
    if let Some(profile) = app.settings.library.active_profile() {
        ui.label(format!("Collection profile: {}", profile.display_name));
        ui.horizontal(|ui| {
            ui.label("Archive:");
            ui.monospace(profile.archive_root.display().to_string());
            if ui.small_button("Change…").clicked()
                && let Some(path) = rfd::FileDialog::new().pick_folder()
            {
                profile_action = Some(ProfileAction::Archive(path));
            }
        });
        ui.horizontal(|ui| {
            ui.label("Playable:");
            ui.monospace(profile.playable_root.display().to_string());
            if ui.small_button("Change…").clicked()
                && let Some(path) = rfd::FileDialog::new().pick_folder()
            {
                profile_action = Some(ProfileAction::Playable(path));
            }
        });
        ui.horizontal(|ui| {
            ui.label("Workspace:");
            ui.monospace(profile.workspace_root.display().to_string());
            if ui.small_button("Change…").clicked()
                && let Some(path) = rfd::FileDialog::new().pick_folder()
            {
                profile_action = Some(ProfileAction::Workspace(path));
            }
        });
        let mut network_mode = profile.network_mode;
        if ui
            .checkbox(
                &mut network_mode,
                "Network mode (stage large files locally before processing)",
            )
            .on_hover_text(
                "Useful for seek-heavy work on SMB/NFS. Turn this off to keep processing \
                 and required scratch on the source filesystem instead of staging inputs \
                 in the device-local workspace.",
            )
            .changed()
        {
            profile_action = Some(ProfileAction::NetworkMode(network_mode));
        }
        if network_mode {
            ui.weak(
                "Large inputs are copied to the local workspace first. This is usually best for CHD verification over a network mount.",
            );
        } else {
            ui.weak(
                "Large inputs stay on their source filesystem. Tools that require isolated scratch keep it on the archive filesystem.",
            );
        }
        let initialized = profile
            .archive_root
            .join("retro-junk-archive.toml")
            .is_file();
        if initialized {
            ui.colored_label(STATUS_OK, "Portable archive initialized");
        } else if ui.button("Initialize archive").clicked() {
            profile_action = Some(ProfileAction::Initialize);
        }
        ui.add_space(8.0);
    }
    if let Some(action) = profile_action {
        apply_profile_action(app, action);
    }

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
                    if let Err(error) = crate::settings::save_settings(&app.settings) {
                        app.push_error("Save settings", error.to_string());
                    }
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

enum ProfileAction {
    Archive(std::path::PathBuf),
    Playable(std::path::PathBuf),
    Workspace(std::path::PathBuf),
    NetworkMode(bool),
    Initialize,
}

fn apply_profile_action(app: &mut RetroJunkApp, action: ProfileAction) {
    let Some(id) = app.settings.library.current_profile else {
        return;
    };
    let Some(profile) = app
        .settings
        .library
        .profiles
        .iter_mut()
        .find(|profile| profile.profile_id == id)
    else {
        return;
    };
    match action {
        ProfileAction::Archive(path) => {
            profile.archive_root = path;
            profile.workspace_root = profile.archive_root.join(".retro-junk").join("work");
            let root_manifest = profile.archive_root.join("retro-junk-archive.toml");
            if root_manifest.is_file() {
                match retro_junk_archive::read_toml::<retro_junk_archive::ArchiveRootManifest>(
                    &root_manifest,
                ) {
                    Ok(manifest) => {
                        profile.profile_id = manifest.profile_id;
                        profile.display_name = manifest.display_name;
                        profile.platform_defaults = manifest.platform_defaults;
                        app.settings.library.current_profile = Some(profile.profile_id);
                    }
                    Err(error) => {
                        app.push_error("Open archive", error.to_string());
                        return;
                    }
                }
            }
        }
        ProfileAction::Playable(path) => {
            profile.playable_root.clone_from(&path);
            app.settings.library.current_root = Some(path);
        }
        ProfileAction::Workspace(path) => profile.workspace_root = path,
        ProfileAction::NetworkMode(enabled) => profile.network_mode = enabled,
        ProfileAction::Initialize => {
            let mut manifest = retro_junk_archive::ArchiveRootManifest::new(&profile.display_name);
            manifest.profile_id = profile.profile_id;
            manifest
                .platform_defaults
                .clone_from(&profile.platform_defaults);
            if let Err(error) =
                retro_junk_archive::initialize_archive(&profile.archive_root, &manifest)
            {
                app.push_error("Archive initialization", error.to_string());
                return;
            }
        }
    }
    if let Err(error) = crate::settings::save_settings(&app.settings) {
        app.push_error("Save settings", error.to_string());
    }
    if let Some(profile) = app.settings.library.active_profile().cloned()
        && app.catalog_db.is_some()
        && profile
            .archive_root
            .join("retro-junk-archive.toml")
            .is_file()
    {
        let _ = app
            .message_tx
            .send(crate::state::AppMessage::StartArchiveRefresh { profile });
    }
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

    if changed && let Err(error) = crate::settings::save_settings(&app.settings) {
        app.push_error("Save settings", error.to_string());
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

    if changed && let Err(error) = crate::settings::save_settings(&app.settings) {
        app.push_error("Save settings", error.to_string());
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

    show_scraper_account(ui, app);
    ui.add_space(8.0);

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

/// Account entry and login test.
///
/// Everything below this in the section explains where credentials *come
/// from*; this is the part a new user needs: type an account, press a
/// button, find out whether scraping will work and how much of today's
/// quota is left. Before it existed, a wrong password only surfaced as an
/// error toast partway through a scrape run.
fn show_scraper_account(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    let account = app
        .ui_state
        .scraper_account
        .get_or_insert_with(crate::state::ScraperAccount::load);

    ui.label("ScreenScraper account (optional, but raises your daily quota):");
    egui::Grid::new("screenscraper_account")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label("Username");
            ui.add(
                egui::TextEdit::singleline(&mut account.user_id)
                    .desired_width(220.0)
                    .hint_text("ScreenScraper username"),
            );
            ui.end_row();
            ui.label("Password");
            ui.add(
                egui::TextEdit::singleline(&mut account.user_password)
                    .desired_width(220.0)
                    .password(true),
            );
            ui.end_row();
        });

    let mut save_requested = false;
    let mut test_requested = false;
    ui.horizontal(|ui| {
        save_requested = ui
            .button(crate::widgets::icons::labeled(
                crate::widgets::icons::VERIFY,
                "Save",
            ))
            .on_hover_text("Writes the account into the ScreenScraper config file")
            .clicked();
        let testing = matches!(account.test, crate::state::LoginTest::Running);
        test_requested = ui
            .add_enabled(!testing, egui::Button::new("Test login"))
            .on_hover_text("Calls the ScreenScraper API and reports your quota")
            .clicked();
        if testing {
            ui.spinner();
        }
        ui.hyperlink_to("Create a free account", "https://www.screenscraper.fr");
    });

    match &account.test {
        // A test in flight is already shown by the spinner above.
        crate::state::LoginTest::Idle | crate::state::LoginTest::Running => {}
        crate::state::LoginTest::Ok(summary) => {
            ui.colored_label(STATUS_OK, summary);
        }
        crate::state::LoginTest::Failed(error) => {
            ui.colored_label(STATUS_ERR, error);
        }
    }

    if save_requested {
        save_scraper_account(app);
    }
    if test_requested {
        test_scraper_login(app, ui.ctx());
    }
}

fn save_scraper_account(app: &mut RetroJunkApp) {
    let Some(account) = app.ui_state.scraper_account.as_ref() else {
        return;
    };
    let (user_id, user_password) = (account.user_id.clone(), account.user_password.clone());
    // `save_to_file` writes the whole `[screenscraper]` table, so start from
    // what would be loaded today and change only the account fields.
    let result = retro_junk_scraper::Credentials::load().and_then(|credentials| {
        retro_junk_scraper::save_to_file(&retro_junk_scraper::Credentials {
            user_id,
            user_password,
            ..credentials
        })
    });
    match result {
        Ok(path) => {
            log::info!("Saved ScreenScraper account to {}", path.display());
            app.notify("Saved ScreenScraper account");
            // Provenance changed underneath the cached snapshot.
            app.ui_state.credential_status = None;
        }
        Err(error) => app.push_error("ScreenScraper account", error.to_string()),
    }
}

fn test_scraper_login(app: &mut RetroJunkApp, ctx: &egui::Context) {
    let Some(account) = app.ui_state.scraper_account.as_mut() else {
        return;
    };
    let (user_id, user_password) = (account.user_id.clone(), account.user_password.clone());
    account.test = crate::state::LoginTest::Running;
    let sender = app.message_tx.clone();
    let repaint = ctx.clone();
    std::thread::spawn(move || {
        // The unsaved field values are what the user is testing, so they
        // override whatever the config file currently holds.
        let result = retro_junk_scraper::Credentials::load()
            .map(|credentials| retro_junk_scraper::Credentials {
                user_id,
                user_password,
                ..credentials
            })
            .and_then(|credentials| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| retro_junk_scraper::ScrapeError::Config(error.to_string()))?
                    .block_on(retro_junk_scraper::ScreenScraperClient::new(credentials))
            })
            .map(|(_, info)| {
                format!(
                    "Signed in: {} of {} requests used today, {} threads, {} requests/minute",
                    info.requests_today(),
                    info.max_requests_per_day(),
                    info.max_threads(),
                    info.max_requests_per_minute(),
                )
            })
            .map_err(|error| error.to_string());
        let _ = sender.send(crate::state::AppMessage::ScraperLoginTested { result });
        repaint.request_repaint();
    });
}

/// Modal explaining one credential field: what it is for and where to get it.
fn show_credential_info_popup(ctx: &egui::Context, app: &mut RetroJunkApp) {
    let Some(meta) = app.ui_state.credential_info_popup else {
        return;
    };

    let outcome =
        crate::widgets::modal::show(ctx, "credential_info_dialog", meta.label, 420.0, |ui| {
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

            crate::widgets::modal::footer(ui, |ui| ui.button("Close").clicked())
        });

    if outcome.inner || outcome.dismissed {
        app.ui_state.credential_info_popup = None;
    }
}

/// Daemon status, start/stop, backlog, and the tail of its output.
///
/// The daemon stays a CLI subcommand — one install, one config — so this
/// section launches and signals that binary rather than embedding a second
/// daemon in the app process. Status comes from the same PID file and
/// heartbeat `retro-junk daemon status` reads, and the backlog is the same
/// strip the Library view shows, so no surface here can disagree with
/// another.
fn show_daemon_section(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    use crate::backend::daemon::{self, DaemonStatus};

    ui.strong("Convergence daemon");
    ui.add_space(4.0);
    ui.weak(
        "Watches incoming folders and keeps the archive converging in the \
         background, within the automation policy above. It keeps running \
         after this window closes.",
    );
    ui.add_space(4.0);

    let status = daemon::status(app.catalog_db.as_ref());
    let mut start_requested = false;
    let mut stop_requested = false;
    ui.horizontal(|ui| {
        match &status {
            DaemonStatus::NotRunning => {
                ui.colored_label(egui::Color32::GRAY, "Not running");
            }
            DaemonStatus::Stale(pid) => {
                ui.colored_label(
                    STATUS_WARN,
                    format!("Not running (pid {pid} exited without cleaning up)"),
                );
            }
            DaemonStatus::Running { pid, heartbeat } => {
                ui.colored_label(STATUS_OK, format!("Running (pid {pid})"));
                match heartbeat {
                    Some(at) => ui.weak(format!("last heartbeat {at}")),
                    None => ui.weak("starting up\u{2026}"),
                };
            }
        }
        let running = matches!(status, DaemonStatus::Running { .. });
        if running {
            stop_requested = ui.button("Stop").clicked();
        } else {
            start_requested = ui
                .button(crate::widgets::icons::labeled(
                    crate::widgets::icons::VERIFY,
                    "Start",
                ))
                .on_hover_text(
                    "Runs `retro-junk daemon start --foreground` as a background process",
                )
                .clicked();
        }
    });

    ui.add_space(6.0);
    // Same widget, same aggregation as the Library view's strip.
    crate::backend::convergence::ensure_backlog_loaded(app, ui.ctx());
    if crate::widgets::backlog_strip::show(ui, app)
        && let Some(scope) = app.ui_state.backlog_scope.clone()
    {
        crate::backend::convergence::run_scope(app, scope, ui.ctx());
    }

    ui.add_space(6.0);
    let log = daemon::log_tail(12);
    if log.is_empty() {
        ui.weak("No daemon output captured yet.");
    } else {
        egui::CollapsingHeader::new("Recent daemon output")
            .default_open(true)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(160.0)
                    .id_salt("daemon_log_tail")
                    .show(ui, |ui| {
                        for line in &log {
                            ui.monospace(line);
                        }
                    });
            });
    }

    if start_requested {
        match daemon::start() {
            Ok(()) => app.notify("Daemon starting"),
            Err(error) => app.push_error("Daemon", error),
        }
    }
    if stop_requested {
        match daemon::stop() {
            Ok(pid) => app.notify(format!("Stopped daemon pid {pid}")),
            Err(error) => app.push_error("Daemon", error),
        }
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

fn show_automation_section(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    use retro_junk_work::{AutoImportMode, BindConfidence};

    ui.strong("Automation");
    ui.add_space(4.0);
    ui.weak(
        "What the daemon and background runs do unattended. Safe, idempotent          work (verify, build, project) defaults to automatic; imports touch          files you placed, so they default to review.",
    );
    ui.add_space(4.0);
    let policy = app
        .ui_state
        .automation_policy
        .get_or_insert_with(retro_junk_work::AutomationPolicy::load);
    let mut changed = false;
    changed |= ui
        .checkbox(
            &mut policy.auto_verify,
            "Verify archive dumps automatically (append-only evidence)",
        )
        .changed();
    changed |= ui
        .checkbox(
            &mut policy.auto_build,
            "Build playable copies, project artwork, and update gamelists automatically",
        )
        .changed();
    ui.horizontal(|ui| {
        ui.label("Incoming dumps:");
        changed |= ui
            .selectable_value(&mut policy.auto_import, AutoImportMode::On, "Import")
            .changed();
        changed |= ui
            .selectable_value(&mut policy.auto_import, AutoImportMode::Suggest, "Suggest")
            .changed();
        changed |= ui
            .selectable_value(&mut policy.auto_import, AutoImportMode::Off, "Track only")
            .changed();
    });
    if policy.auto_import == AutoImportMode::On {
        ui.horizontal(|ui| {
            ui.label("Auto-import needs at least:");
            changed |= ui
                .selectable_value(
                    &mut policy.auto_bind_min_confidence,
                    BindConfidence::ExactHash,
                    "Exact hash",
                )
                .changed();
            changed |= ui
                .selectable_value(
                    &mut policy.auto_bind_min_confidence,
                    BindConfidence::ExactSerial,
                    "Header serial",
                )
                .changed();
            changed |= ui
                .selectable_value(
                    &mut policy.auto_bind_min_confidence,
                    BindConfidence::FolderSerial,
                    "Folder serial",
                )
                .changed();
        });
    }
    changed |= ui
        .checkbox(
            &mut policy.auto_scrape,
            "Fetch missing artwork from ScreenScraper automatically",
        )
        .changed();
    if policy.auto_scrape {
        changed |= ui
            .checkbox(
                &mut policy.scrape_only_when_unambiguous,
                "Only publish confident matches; send filename-only guesses to the Inbox",
            )
            .changed();
        ui.horizontal(|ui| {
            ui.label("Keep");
            changed |= ui
                .add(
                    egui::DragValue::new(&mut policy.scrape_daily_request_reserve)
                        .range(0..=20_000),
                )
                .changed();
            ui.label("daily requests in reserve for manual scraping");
        });
    }
    changed |= show_expected_artwork(ui, policy);
    changed |= ui
        .checkbox(
            &mut policy.verify_published_bytes,
            "Re-read published bytes at import (extra full read; background              verification covers this otherwise)",
        )
        .changed();
    ui.horizontal(|ui| {
        ui.label("Deep rescan every");
        changed |= ui
            .add(egui::DragValue::new(&mut policy.deep_rescan_hours).range(0..=720))
            .changed();
        ui.label("hours (0 = off)");
    });
    if changed {
        // The badge counts against this set on every paint, so refresh the
        // resolved copy rather than re-parsing the names per row.
        app.ui_state.expected_assets = policy.scrape_selection();
        if let Err(error) = policy.save() {
            app.push_error("Automation settings", error.to_string());
        }
    }
}

/// Pick the artwork types a release is expected to hold.
///
/// This drives three things at once — what a scrape fetches, what the
/// artwork badge counts, and what derivation calls missing — so it is worth
/// saying so on screen.
fn show_expected_artwork(
    ui: &mut egui::Ui,
    policy: &mut retro_junk_work::AutomationPolicy,
) -> bool {
    use retro_junk_frontend::AssetSelection;

    let mut changed = false;
    ui.add_space(4.0);
    ui.label("Artwork a game is expected to have:");
    ui.weak("Also what the artwork badge counts and what a scrape fetches.");
    let mut selected = policy.scrape_selection();
    ui.horizontal_wrapped(|ui| {
        for asset_type in AssetSelection::all().types {
            let mut on = selected.contains(asset_type);
            if ui
                .checkbox(&mut on, asset_type.to_string())
                .on_hover_text(format!("stored under {}/", asset_type.subdirectory()))
                .changed()
            {
                if on {
                    selected.types.push(asset_type);
                } else {
                    selected.types.retain(|held| *held != asset_type);
                }
                changed = true;
            }
        }
    });
    if changed {
        // Keep a stable order so the settings file does not churn as boxes
        // are toggled.
        selected.types.sort_unstable();
        policy.scrape_asset_types = selected.names();
    }
    changed
}
