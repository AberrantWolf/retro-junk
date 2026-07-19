use crate::app::RetroJunkApp;
use crate::backend;
use crate::state::{FocusedPanel, ScanStatus};
use crate::util;
use crate::widgets::keyboard_nav;
use crate::widgets::status_badge;

/// Render the manufacturer-grouped console tree.
pub fn show(ui: &mut egui::Ui, app: &mut RetroJunkApp, ctx: &egui::Context) {
    if app.browser.consoles.is_empty() {
        if app.ui_state.loading_library {
            ui.label("Loading library...");
        } else {
            ui.label("No consoles found.");
        }
        return;
    }

    // Collect unique manufacturers in order
    let manufacturers: Vec<&str> = {
        let mut seen = Vec::new();
        for c in &app.browser.consoles {
            if !seen.contains(&c.manufacturer) {
                seen.push(c.manufacturer);
            }
        }
        seen
    };

    // Build ordered console indices (manufacturer-grouped, matching render order)
    let ordered_console_indices: Vec<usize> = {
        let consoles = &app.browser.consoles;
        manufacturers
            .iter()
            .flat_map(|&mfr| (0..consoles.len()).filter(move |&i| consoles[i].manufacturer == mfr))
            .collect()
    };

    // Keyboard navigation (only when console tree has focus)
    if app.ui_state.focused_panel == FocusedPanel::ConsoleTree {
        let current_pos = app.ui_state.selected_console.and_then(|selected| {
            ordered_console_indices
                .iter()
                .position(|&i| app.browser.consoles[i].id == Some(selected))
        });

        if let Some(action) = keyboard_nav::process_list_nav(
            ui,
            current_pos,
            ordered_console_indices.len(),
            10, // page size for console tree
        ) {
            let new_console_idx = ordered_console_indices[action.new_index];
            let new_console_id = app.browser.consoles[new_console_idx].id;
            if new_console_id.is_some() && app.ui_state.selected_console != new_console_id {
                app.ui_state.selected_console = new_console_id;
                app.ui_state.scroll_to_console = Some(new_console_idx);
                app.ui_state.focused_entry = None;
                app.ui_state.selected_entries.clear();
                app.ui_state.filter_text.clear();
                app.ui_state.page_offset = 0;

                if app.browser.consoles[new_console_idx].scan_status == ScanStatus::NotScanned {
                    backend::scan::quick_scan_console(app, new_console_idx, ctx);
                } else if let Some(id) = new_console_id {
                    app.request_console_page(id, ctx);
                    app.ensure_dat_index(new_console_idx, ctx);
                }
            }
        }
    }

    for mfr in manufacturers {
        egui::CollapsingHeader::new(egui::RichText::new(mfr).strong())
            .id_salt(format!("mfr_{mfr}"))
            .default_open(true)
            .show(ui, |ui| {
                for i in 0..app.browser.consoles.len() {
                    if app.browser.consoles[i].manufacturer != mfr {
                        continue;
                    }

                    let console = &app.browser.consoles[i];
                    let persisted_entry_count = app.browser.entry_count(console);
                    let console_id = console.id;
                    let is_selected = app.ui_state.selected_console == console_id;

                    let label = match console.scan_status {
                        ScanStatus::NotScanned => console.folder_name.clone(),
                        ScanStatus::Scanning => format!("{} (...)", console.folder_name),
                        ScanStatus::Scanned => {
                            if console.loose_disc_files.is_empty() {
                                format!("{} ({})", console.folder_name, persisted_entry_count)
                            } else {
                                format!("{}  ({}*)", console.folder_name, persisted_entry_count)
                            }
                        }
                    };

                    // Inactive entry pages are deliberately evicted; the summary
                    // projection retains the console's aggregate status.
                    let worst_status =
                        console_id.and_then(|id| app.browser.console_statuses.get(&id).copied());

                    let folder_path = console.folder_path.clone();
                    let entry_count = persisted_entry_count;
                    let is_scanned = console.scan_status == ScanStatus::Scanned;

                    // Pin the row id to the stable console index so the
                    // selectable_label's id does not shift when the optional
                    // status badge is (or isn't) allocated alongside it. Without
                    // this, a NotScanned -> Scanned transition toggles the badge
                    // allocation, shifting positional auto-ids and producing
                    // "changed id between passes" warnings + scroll resets.
                    let label_resp = ui
                        .push_id(i, |ui| {
                            ui.horizontal(|ui| {
                                if let Some(status) = worst_status {
                                    status_badge::show(ui, status);
                                }
                                ui.selectable_label(is_selected, &label)
                            })
                            .inner
                        })
                        .inner;

                    if label_resp.clicked() && !is_selected {
                        let Some(console_id) = app.browser.consoles[i].id else {
                            continue;
                        };
                        app.ui_state.selected_console = Some(console_id);
                        app.ui_state.focused_entry = None;
                        app.ui_state.selected_entries.clear();
                        app.ui_state.filter_text.clear();
                        app.ui_state.page_offset = 0;
                        app.ui_state.focused_panel = FocusedPanel::ConsoleTree;

                        // Trigger quick-scan if not already scanned
                        if app.browser.consoles[i].scan_status == ScanStatus::NotScanned {
                            backend::scan::quick_scan_console(app, i, ctx);
                        } else {
                            app.request_console_page(console_id, ctx);
                            app.ensure_dat_index(i, ctx);
                        }
                    }

                    // Scroll to this console only on the frame keyboard
                    // navigation targeted it — not every frame, which would pin
                    // the view and block manual scrolling. Mirrors the game
                    // table's `scroll_to_row` handling.
                    if app.ui_state.scroll_to_console == Some(i) {
                        label_resp.scroll_to_me(Some(egui::Align::Center));
                    }

                    // Context menu on the selectable label
                    label_resp.context_menu(|ui| {
                        show_console_context_menu(
                            ui,
                            app,
                            ctx,
                            i,
                            &folder_path,
                            entry_count,
                            is_scanned,
                        );
                    });
                }
            });
    }

    // Consume the one-shot scroll request; the targeted row (if visible) has
    // now scrolled itself into view.
    app.ui_state.scroll_to_console = None;
}

/// Render the context menu for a console tree entry.
fn show_console_context_menu(
    ui: &mut egui::Ui,
    app: &mut RetroJunkApp,
    ctx: &egui::Context,
    console_idx: usize,
    folder_path: &std::path::Path,
    entry_count: usize,
    is_scanned: bool,
) {
    if ui.button("Rescan").clicked() {
        // Reset scan status to allow re-scanning
        app.browser.consoles[console_idx].scan_status = ScanStatus::NotScanned;
        backend::scan::quick_scan_console(app, console_idx, ctx);
        ui.close();
    }

    if ui
        .add_enabled(
            is_scanned && entry_count > 0,
            egui::Button::new("Calculate All Hashes"),
        )
        .clicked()
    {
        app.calculate_all_hashes(console_idx, ctx);
        ui.close();
    }

    if ui
        .add_enabled(
            is_scanned && entry_count > 0,
            egui::Button::new("Re-scrape All Media"),
        )
        .clicked()
    {
        app.ui_state.selected_entries = app.browser.consoles[console_idx]
            .entries
            .iter()
            .filter_map(|entry| entry.id)
            .collect();
        backend::assets::rescrape_media_for_selection(app, console_idx, ctx);
        ui.close();
    }

    // Organize: only for disc-based consoles with loose disc files
    {
        let console = &app.browser.consoles[console_idx];
        let has_loose = !console.loose_disc_files.is_empty();
        let is_disc_based = app
            .context
            .get_by_platform(console.platform)
            .is_some_and(|r| r.analyzer.dat_source() == retro_junk_lib::DatSource::Redump);
        if is_disc_based {
            let label = if has_loose {
                format!("Organize ({} loose files)", console.loose_disc_files.len())
            } else {
                "Organize".to_string()
            };
            if ui
                .add_enabled(is_scanned && has_loose, egui::Button::new(label))
                .clicked()
            {
                backend::organize::organize_console(app, console_idx, ctx);
                ui.close();
            }
        }
    }

    // Compress to CHD: only for consoles whose analyzer supports it. Gated
    // (advisory — start_compression and the D1 planning op hold the actual
    // guarantee) while a compression is already running for this console.
    if backend::chd_compress::console_supports_chd(app, console_idx) {
        let busy = app.chd_compress_busy(&app.browser.consoles[console_idx].folder_name);
        let button = ui
            .add_enabled(
                is_scanned && entry_count > 0 && !busy,
                egui::Button::new("Compress All to CHD…"),
            )
            .on_disabled_hover_text("A CHD compression is already running for this console");
        if button.clicked() {
            let all_entries: Vec<usize> = (0..entry_count).collect();
            backend::chd_compress::open_compress_dialog(app, console_idx, &all_entries);
            ui.close();
        }
    }

    ui.separator();

    ui.menu_button("Export", |ui| {
        if ui
            .add_enabled(
                is_scanned && entry_count > 0,
                egui::Button::new("gamelist.xml (ES-DE)"),
            )
            .clicked()
        {
            backend::export::generate_gamelist(app, console_idx, ctx);
            ui.close();
        }
    });

    ui.separator();

    if ui.button(util::REVEAL_LABEL).clicked() {
        util::reveal_in_file_manager(folder_path);
        ui.close();
    }
}
