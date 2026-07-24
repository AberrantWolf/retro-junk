use crate::app::RetroJunkApp;
use crate::backend;
use crate::widgets;

fn same_platform(left: &str, right: &str) -> bool {
    if left.eq_ignore_ascii_case(right) {
        return true;
    }
    match (
        left.parse::<retro_junk_core::Platform>(),
        right.parse::<retro_junk_core::Platform>(),
    ) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

/// Render the three-pane library view.
pub fn show(ui: &mut egui::Ui, app: &mut RetroJunkApp, ctx: &egui::Context) {
    if app.root_path.is_none() {
        // No folder open yet — show welcome screen
        show_welcome(ui, app, ctx);
        return;
    }

    // Three-pane layout: Console Tree | Game Table | Detail Panel

    // Console tree (left)
    egui::Panel::left("console_tree")
        .resizable(true)
        .default_size(200.0)
        .size_range(150.0..=300.0)
        .show(ui, |ui| {
            ui.add_space(4.0);

            // Open folder button at top of tree
            if ui.button("Open Folder...").clicked() {
                open_folder(app, ctx);
            }
            ui.separator();

            egui::ScrollArea::vertical()
                .id_salt("console_tree_scroll")
                .show(ui, |ui| {
                    widgets::console_tree::show(ui, app, ctx);
                });
        });

    // Detail panel (right, collapsible)
    if app.ui_state.detail_panel_open {
        egui::Panel::right("detail_panel")
            .resizable(true)
            .default_size(280.0)
            .size_range(200.0..=500.0)
            .show(ui, |ui| {
                widgets::detail_panel::show(ui, app);
            });
    }

    // Game table (center, fills remaining space). Stable id so toggling the
    // conditional detail panel above doesn't re-id the toolbar/table widgets.
    let policy_context = app.selected_console_index().and_then(|index| {
        let console = &app.browser.consoles[index];
        let platform_id = console.folder_name.clone();
        let platform = console.platform;
        let profile = app.settings.library.active_profile()?;
        if app.root_path.as_ref() != Some(&profile.playable_root) {
            return None;
        }
        let current = profile
            .platform_defaults
            .iter()
            .find(|default| default.platform_id.eq_ignore_ascii_case(&platform_id))
            .or_else(|| {
                profile
                    .platform_defaults
                    .iter()
                    .find(|default| same_platform(&default.platform_id, &platform_id))
            })
            .map(|default| default.policy.format.clone());
        Some((platform_id, platform, current))
    });
    let mut policy_change = None;
    crate::util::stable_central_panel(ui, "library_center", |ui| {
        // Toolbar
        ui.horizontal(|ui| {
            let filter_changed = ui
                .add(
                    egui::TextEdit::singleline(&mut app.ui_state.filter_text)
                        .hint_text("Filter...")
                        .desired_width(200.0),
                )
                .changed();
            if filter_changed && let Some(console_id) = app.ui_state.selected_console {
                app.ui_state.page_offset = 0;
                app.request_console_page(console_id, ctx);
            }

            if let Some((platform_id, platform, current)) = policy_context.as_ref() {
                ui.separator();
                ui.label("Preferred playable:");
                let mut selected = current.clone();
                let supported = super::playable_formats::supported(*platform);
                let selected_label = selected.as_ref().map_or_else(
                    || "No preference".to_owned(),
                    |format| {
                        let label = super::playable_formats::label(Some(format));
                        if supported.contains(format) {
                            label.to_owned()
                        } else {
                            format!("{label} (unsupported)")
                        }
                    },
                );
                egui::ComboBox::from_id_salt(("preferred_playable", platform_id))
                    .selected_text(selected_label)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut selected, None, "No preference");
                        if supported.is_empty() {
                            ui.weak("No modeled emulator-ready format");
                        }
                        for format in supported {
                            let label = super::playable_formats::label(Some(&format));
                            ui.selectable_value(&mut selected, Some(format), label);
                        }
                    })
                    .response
                    .on_hover_text(format!(
                        "Formats accepted by mainstream {} emulators",
                        platform.display_name()
                    ));
                if &selected != current {
                    policy_change = Some((platform_id.clone(), selected));
                }
            }

            if let Some(page) = app.browser.active_page.as_ref() {
                let offset = page.offset;
                let total = page.total_count;
                let page_size = retro_junk_db::LibraryEntryListQuery::DEFAULT_PAGE_SIZE;
                if ui
                    .add_enabled(offset > 0, egui::Button::new("Previous"))
                    .clicked()
                    && let Some(console_id) = app.ui_state.selected_console
                {
                    app.ui_state.page_offset = offset.saturating_sub(page_size);
                    app.request_console_page(console_id, ctx);
                }
                if ui
                    .add_enabled(offset + page_size < total, egui::Button::new("Next"))
                    .clicked()
                    && let Some(console_id) = app.ui_state.selected_console
                {
                    app.ui_state.page_offset = offset + page_size;
                    app.request_console_page(console_id, ctx);
                }
            }

            ui.separator();

            let has_selection = !app.ui_state.selected_entries.is_empty();
            let selection_ready = has_selection && app.selected_entry_details_loaded();
            let hash_button = ui
                .add_enabled(
                    selection_ready,
                    egui::Button::new("Calculate Missing Hashes"),
                )
                .on_disabled_hover_text(if has_selection {
                    "Loading the selected entry details…"
                } else {
                    "Select one or more entries"
                })
                .on_hover_text("Keeps stored results for unchanged files");
            if hash_button.clicked()
                && let Some(ci) = app.selected_console_index()
            {
                backend::hash::compute_hashes_for_selection(app, ci);
            }

            ui.separator();

            // Toggle detail panel
            let label = if app.ui_state.detail_panel_open {
                "Hide Detail"
            } else {
                "Show Detail"
            };
            if ui.button(label).clicked() {
                app.ui_state.detail_panel_open = !app.ui_state.detail_panel_open;
            }
        });

        ui.separator();

        if let Some(page) = app.browser.active_page.as_ref() {
            let counts = &page.availability_counts;
            ui.horizontal_wrapped(|ui| {
                ui.strong("Availability:");
                availability_chip(ui, counts.archived_and_playable, "Archived + playable");
                availability_chip(
                    ui,
                    counts.incomplete_archive_and_playable,
                    "Incomplete archive + playable",
                );
                availability_chip(ui, counts.archived_not_playable, "Archived, needs playable");
                availability_chip(ui, counts.playable_only, "Playable only");
                availability_chip(ui, counts.preferred_format_mismatch, "Wrong format");
            });
        }

        // Game table
        if app.ui_state.selected_console.is_some() {
            widgets::game_table::show(ui, app, ctx);
        } else {
            ui.centered_and_justified(|ui| {
                ui.label("Select a console from the tree to view its games.");
            });
        }
    });
    if let Some((platform_id, format)) = policy_change {
        set_preferred_playable_format(app, platform_id, format, ctx);
    }
}

fn availability_chip(ui: &mut egui::Ui, count: u64, label: &str) {
    ui.label(format!("{count} {label}"));
}

fn set_preferred_playable_format(
    app: &mut RetroJunkApp,
    platform_id: String,
    format: Option<retro_junk_archive::RepresentationFormat>,
    ctx: &egui::Context,
) {
    let Some(profile) = app.settings.library.active_profile().cloned() else {
        app.push_error("Playable policy", "No active collection profile".to_owned());
        return;
    };
    let Some(db_path) = app.db_path.clone() else {
        app.push_error(
            "Playable policy",
            "Catalog database is unavailable".to_owned(),
        );
        return;
    };
    let op_id = crate::state::next_operation_id();
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    app.operations.push(crate::state::BackgroundOperation::new(
        op_id,
        format!("Updating {platform_id} playable policy"),
        cancel,
        crate::state::OperationKind::Other,
        platform_id.clone(),
        crate::state::ProgressDisplay::Count,
    ));
    let sender = app.message_tx.clone();
    let handle = std::thread::spawn(move || {
        let result = (|| {
            let policy = format.map(|format| {
                let mut policy = profile
                    .platform_defaults
                    .iter()
                    .find(|default| default.platform_id.eq_ignore_ascii_case(&platform_id))
                    .or_else(|| {
                        profile
                            .platform_defaults
                            .iter()
                            .find(|default| same_platform(&default.platform_id, &platform_id))
                    })
                    .map(|default| default.policy.clone())
                    .unwrap_or(retro_junk_archive::DesiredPlayablePolicy {
                        format: format.clone(),
                        retain_canonical_intermediate: false,
                        allow_unverified: false,
                        options: std::collections::BTreeMap::new(),
                    });
                policy.format = format;
                policy
            });
            let manifest = retro_junk_archive::set_platform_playable_default(
                &profile.archive_root,
                &platform_id,
                policy,
            )
            .map_err(|error| error.to_string())?;
            let projected_policy = manifest
                .platform_defaults
                .iter()
                .find(|default| default.platform_id.eq_ignore_ascii_case(&platform_id))
                .or_else(|| {
                    manifest
                        .platform_defaults
                        .iter()
                        .find(|default| same_platform(&default.platform_id, &platform_id))
                })
                .map(|default| &default.policy);
            let (_, manifest_sha256) = retro_junk_archive::sha256_file(
                &profile.archive_root.join("retro-junk-archive.toml"),
                &std::sync::atomic::AtomicBool::new(false),
            )
            .map_err(|error| error.to_string())?;
            let mut connection =
                retro_junk_db::open_database(&db_path).map_err(|error| error.to_string())?;
            retro_junk_db::update_projected_platform_policy(
                &mut connection,
                &profile.profile_id.to_string(),
                &platform_id,
                projected_policy,
                &manifest_sha256,
            )
            .map_err(|error| error.to_string())?;
            Ok(manifest)
        })();
        let _ = sender.send(crate::state::AppMessage::PlayablePolicyUpdated { op_id, result });
    });
    app.op_threads.insert(op_id, handle);
    ctx.request_repaint_after(std::time::Duration::from_millis(20));
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
///
/// If the path lives on a fragile userspace network mount (GVFS/KIO-FUSE),
/// the switch is deferred to a confirmation modal instead; on confirmation the
/// dialog resumes via [`switch_to_root_unchecked`].
pub fn switch_to_root(app: &mut RetroJunkApp, new_root: std::path::PathBuf, ctx: &egui::Context) {
    if let Some(kind) = crate::util::fragile_mount_kind(&new_root) {
        app.ui_state.fragile_mount_prompt = Some(crate::state::FragileMountPrompt {
            root: new_root,
            kind,
        });
        return;
    }
    switch_to_root_unchecked(app, new_root, ctx);
}

/// Switch roots without the fragile-mount check. Called by the confirmation
/// dialog after the user accepts; everything else goes through
/// [`switch_to_root`].
pub fn switch_to_root_unchecked(
    app: &mut RetroJunkApp,
    new_root: std::path::PathBuf,
    ctx: &egui::Context,
) {
    app.release_detail_assets(ctx);
    // Reset UI state
    app.ui_state.selected_console = None;
    app.ui_state.focused_entry = None;
    app.ui_state.focused_archive_release = None;
    app.ui_state.selected_entries.clear();
    app.library_controller.switch_root();
    app.message_tx = app
        .message_tx
        .for_generation(app.library_controller.session_generation);
    app.cancel_root_scoped_operations();
    app.ui_state.pending_auto_scans.clear();
    app.ui_state.auto_scan_in_flight = None;
    if let Some(op_id) = app.ui_state.auto_scan_op_id.take() {
        app.operations.retain(|o| o.id != op_id);
    }
    app.reset_root_session_requests();

    // Set new root
    app.root_path = Some(new_root.clone());

    app.browser = crate::state::LibraryBrowserState::default();
    app.ui_state.loading_library = true;
    if let Some(conn) = app.catalog_db.as_mut() {
        crate::cache::migrate_json_cache(conn, &new_root, &app.context);
    }
    app.open_browser_root(&new_root, ctx);

    // Always scan disk to discover new/removed console folders.
    // ConsoleFolderFound handler deduplicates, so cached consoles keep their data.
    backend::scan::scan_root_folder(app, new_root.clone(), ctx);

    // Update settings: add/move to front of recent roots
    update_recent_roots(app, &new_root);

    // Save settings immediately
    app.settings.library.ensure_profile_for_root(&new_root);
    if let Err(e) = crate::settings::save_settings(&app.settings) {
        log::warn!("Failed to save settings: {e}");
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
            console_count: app.browser.consoles.len(),
        },
    );

    // Cap at 10 entries
    recent.truncate(10);
}
