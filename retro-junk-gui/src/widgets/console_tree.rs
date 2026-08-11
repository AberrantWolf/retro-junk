use std::cmp::Ordering;

use crate::app::RetroJunkApp;
use crate::backend;
use crate::state::{ConsoleState, FocusedPanel, ScanStatus};
use crate::util;
use crate::widgets::keyboard_nav;
use crate::widgets::status_badge;

fn compare_list_text(left: &str, right: &str) -> Ordering {
    left.to_ascii_lowercase()
        .cmp(&right.to_ascii_lowercase())
        .then_with(|| left.cmp(right))
}

fn console_list_name(console: &ConsoleState) -> &str {
    if console.platform == retro_junk_core::Platform::Ps1 {
        console.platform.short_name()
    } else {
        &console.folder_name
    }
}

fn ordered_console_indices(consoles: &[ConsoleState]) -> Vec<usize> {
    let mut indices = (0..consoles.len()).collect::<Vec<_>>();
    indices.sort_by(|&left, &right| {
        let left = &consoles[left];
        let right = &consoles[right];
        compare_list_text(left.manufacturer, right.manufacturer)
            .then_with(|| compare_list_text(console_list_name(left), console_list_name(right)))
            .then_with(|| compare_list_text(left.platform_name, right.platform_name))
            .then_with(|| compare_list_text(&left.folder_name, &right.folder_name))
            .then_with(|| left.folder_path.cmp(&right.folder_path))
    });
    indices
}

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

    // Sort the render projection rather than relying on async folder/database
    // discovery order. This keeps both pointer and keyboard navigation stable.
    let ordered_console_indices = ordered_console_indices(&app.browser.consoles);
    let manufacturers: Vec<String> = {
        let mut seen = Vec::new();
        for &index in &ordered_console_indices {
            let manufacturer = app.browser.consoles[index].manufacturer.to_owned();
            if !seen.contains(&manufacturer) {
                seen.push(manufacturer);
            }
        }
        seen
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
                app.ui_state.focused_archive_release = None;
                app.ui_state.selected_entries.clear();
                app.ui_state.selected_archive_releases.clear();
                app.ui_state.selected_library_rows.clear();
                app.ui_state.focused_library_row = None;
                app.ui_state.library_selection_anchor = None;
                app.ui_state.filter_text.clear();
                app.ui_state.page_offset = 0;

                if app.browser.consoles[new_console_idx].scan_status == ScanStatus::NotScanned {
                    backend::scan::quick_scan_console(app, new_console_idx, ctx);
                } else if let Some(id) = new_console_id {
                    app.request_console_page(id, ctx);
                }
            }
        }
    }

    for mfr in manufacturers {
        egui::CollapsingHeader::new(egui::RichText::new(&mfr).strong())
            .id_salt(format!("mfr_{mfr}"))
            .default_open(true)
            .show(ui, |ui| {
                for &i in &ordered_console_indices {
                    if app.browser.consoles[i].manufacturer != mfr {
                        continue;
                    }

                    let console = &app.browser.consoles[i];
                    let list_name = console_list_name(console);
                    let persisted_entry_count = app.browser.entry_count(console);
                    let console_id = console.id;
                    let is_selected = app.ui_state.selected_console == console_id;

                    let label = match console.scan_status {
                        ScanStatus::NotScanned => list_name.to_owned(),
                        ScanStatus::Scanning => format!("{list_name} (...)"),
                        ScanStatus::Scanned => {
                            if console.loose_disc_files.is_empty() {
                                format!("{list_name} ({persisted_entry_count})")
                            } else {
                                format!("{list_name}  ({persisted_entry_count}*)")
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
                                    status_badge::show_severity(ui, status);
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
                        app.ui_state.focused_archive_release = None;
                        app.ui_state.selected_entries.clear();
                        app.ui_state.selected_archive_releases.clear();
                        app.ui_state.selected_library_rows.clear();
                        app.ui_state.focused_library_row = None;
                        app.ui_state.library_selection_anchor = None;
                        app.ui_state.filter_text.clear();
                        app.ui_state.page_offset = 0;
                        app.ui_state.focused_panel = FocusedPanel::ConsoleTree;

                        // Trigger quick-scan if not already scanned
                        if app.browser.consoles[i].scan_status == ScanStatus::NotScanned {
                            backend::scan::quick_scan_console(app, i, ctx);
                        } else {
                            app.request_console_page(console_id, ctx);
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
            egui::Button::new("Re-scrape All Artwork"),
        )
        .clicked()
    {
        app.scrape_all_media(console_idx, ctx);
        ui.close();
    }

    if ui
        .add_enabled(
            is_scanned && entry_count > 0,
            egui::Button::new("Scrape Only Missing Artwork"),
        )
        .clicked()
    {
        app.scrape_missing_artwork(console_idx, ctx);
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
            app.compress_all_to_chd(console_idx, ctx);
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

#[cfg(test)]
mod tests {
    use super::{console_list_name, ordered_console_indices};
    use crate::test_support::test_console;
    use retro_junk_core::Platform;

    #[test]
    fn psx_folder_uses_canonical_ps1_list_name() {
        let console = test_console("psx", Vec::new());
        assert_eq!(console_list_name(&console), "ps1");
    }

    #[test]
    fn console_projection_is_sorted_by_displayed_name() {
        let mut psp = test_console("psp", Vec::new());
        psp.platform = Platform::Psp;
        psp.platform_name = "PlayStation Portable";

        let ps1 = test_console("psx", Vec::new());

        let mut ps2 = test_console("ps2", Vec::new());
        ps2.platform = Platform::Ps2;
        ps2.platform_name = "PlayStation 2";

        let consoles = vec![psp, ps1, ps2];
        let names = ordered_console_indices(&consoles)
            .into_iter()
            .map(|index| console_list_name(&consoles[index]))
            .collect::<Vec<_>>();

        assert_eq!(names, ["ps1", "ps2", "psp"]);
    }
}
