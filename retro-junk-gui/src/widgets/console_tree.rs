use crate::app::RetroJunkApp;
use crate::backend;
use crate::state::ScanStatus;
use crate::util;
use crate::widgets::status_badge;

/// Render the manufacturer-grouped console tree.
pub fn show(ui: &mut egui::Ui, app: &mut RetroJunkApp, ctx: &egui::Context) {
    if app.library.consoles.is_empty() {
        if app.loading_library {
            ui.label("Loading library...");
        } else {
            ui.label("No consoles found.");
        }
        return;
    }

    // Collect unique manufacturers in order
    let manufacturers: Vec<&str> = {
        let mut seen = Vec::new();
        for c in &app.library.consoles {
            if !seen.contains(&c.manufacturer) {
                seen.push(c.manufacturer);
            }
        }
        seen
    };

    for mfr in manufacturers {
        egui::CollapsingHeader::new(egui::RichText::new(mfr).strong())
            .id_salt(format!("mfr_{}", mfr))
            .default_open(true)
            .show(ui, |ui| {
                for i in 0..app.library.consoles.len() {
                    if app.library.consoles[i].manufacturer != mfr {
                        continue;
                    }

                    let console = &app.library.consoles[i];
                    let is_selected = app.selected_console == Some(i);

                    let label = match console.scan_status {
                        ScanStatus::NotScanned => console.folder_name.clone(),
                        ScanStatus::Scanning => format!("{} (...)", console.folder_name),
                        ScanStatus::Scanned => {
                            format!("{} ({})", console.folder_name, console.entries.len())
                        }
                    };

                    // Compute worst status across all entries (only for scanned consoles)
                    let worst_status = if console.scan_status == ScanStatus::Scanned
                        && !console.entries.is_empty()
                    {
                        console
                            .entries
                            .iter()
                            .map(|e| e.status)
                            .max_by_key(|s| s.severity())
                    } else {
                        None
                    };

                    let folder_path = console.folder_path.clone();
                    let entry_count = console.entries.len();
                    let is_scanned = console.scan_status == ScanStatus::Scanned;

                    let resp = ui.horizontal(|ui| {
                        if let Some(status) = worst_status {
                            status_badge::show(ui, status);
                        }
                        ui.selectable_label(is_selected, &label)
                    });

                    let label_resp = resp.inner;

                    if label_resp.clicked() && !is_selected {
                        app.selected_console = Some(i);
                        app.focused_entry = None;
                        app.selected_entries.clear();
                        app.filter_text.clear();

                        // Trigger quick-scan if not already scanned
                        if app.library.consoles[i].scan_status == ScanStatus::NotScanned {
                            backend::scan::quick_scan_console(app, i, ctx);
                        }
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
        app.library.consoles[console_idx].scan_status = ScanStatus::NotScanned;
        backend::scan::quick_scan_console(app, console_idx, ctx);
        ui.close_menu();
    }

    if ui
        .add_enabled(
            is_scanned && entry_count > 0,
            egui::Button::new("Calculate All Hashes"),
        )
        .clicked()
    {
        // Select all entries, then compute hashes
        app.selected_entries = (0..entry_count).collect();
        backend::hash::compute_hashes_for_selection(app, console_idx);
        ui.close_menu();
    }

    if ui
        .add_enabled(
            is_scanned && entry_count > 0,
            egui::Button::new("Re-scrape All Media"),
        )
        .clicked()
    {
        app.selected_entries = (0..entry_count).collect();
        backend::media::rescrape_media_for_selection(app, console_idx, ctx);
        ui.close_menu();
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
            ui.close_menu();
        }
    });

    ui.separator();

    if ui.button(util::REVEAL_LABEL).clicked() {
        util::reveal_in_file_manager(folder_path);
        ui.close_menu();
    }
}
