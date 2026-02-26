use std::collections::HashSet;
use std::path::PathBuf;

use egui_extras::{Column, TableBuilder};
use retro_junk_lib::Region;

use crate::app::RetroJunkApp;
use crate::backend;
use crate::state::EntryStatus;
use crate::util;
use crate::widgets::status_badge;

/// Render the sortable, filterable game table for the selected console.
pub fn show(ui: &mut egui::Ui, app: &mut RetroJunkApp, ctx: &egui::Context) {
    let console_idx = match app.selected_console {
        Some(i) => i,
        None => return,
    };

    let console = &app.library.consoles[console_idx];
    let filter = app.filter_text.to_lowercase();

    // Build filtered index list
    let filtered_indices: Vec<usize> = console
        .entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            if filter.is_empty() {
                return true;
            }
            let name = entry.game_entry.display_name().to_lowercase();
            if name.contains(&filter) {
                return true;
            }
            if let Some(ref id) = entry.identification {
                if let Some(ref serial) = id.serial_number
                    && serial.to_lowercase().contains(&filter)
                {
                    return true;
                }
                if let Some(ref iname) = id.internal_name
                    && iname.to_lowercase().contains(&filter)
                {
                    return true;
                }
            }
            if let Some(ref dm) = entry.dat_match
                && dm.game_name.to_lowercase().contains(&filter)
            {
                return true;
            }
            false
        })
        .map(|(i, _)| i)
        .collect();

    // Cmd+A / Ctrl+A: select all visible entries
    if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::A)) {
        app.selected_entries = filtered_indices.iter().copied().collect();
    }

    // Status summary
    let total = console.entries.len();
    let matched = console
        .entries
        .iter()
        .filter(|e| e.status == EntryStatus::Matched)
        .count();
    let ambiguous = console
        .entries
        .iter()
        .filter(|e| e.status == EntryStatus::Ambiguous)
        .count();
    let unrecognized = console
        .entries
        .iter()
        .filter(|e| e.status == EntryStatus::Unrecognized)
        .count();
    let showing = filtered_indices.len();

    // Pre-extract row data to avoid borrowing issues
    let row_data: Vec<RowData> = filtered_indices
        .iter()
        .map(|&i| {
            let entry = &console.entries[i];
            RowData {
                entry_idx: i,
                status: entry.status,
                name: entry.game_entry.display_name().to_string(),
                file_path: entry.game_entry.analysis_path().to_path_buf(),
                serial: entry
                    .identification
                    .as_ref()
                    .and_then(|id| id.serial_number.clone()),
                internal_name: entry
                    .identification
                    .as_ref()
                    .and_then(|id| id.internal_name.clone()),
                regions: {
                    let codes: Vec<&str> =
                        entry.effective_regions().iter().map(|r| r.code()).collect();
                    let text = codes.join(", ");
                    if entry.region_override.is_some() && !text.is_empty() {
                        format!("{}*", text)
                    } else {
                        text
                    }
                },
                crc32: entry.hashes.as_ref().map(|h| h.crc32.clone()),
                dat_match: entry.dat_match.as_ref().map(|dm| dm.game_name.clone()),
            }
        })
        .collect();

    ui.horizontal(|ui| {
        ui.label(format!(
            "{} entries | {} matched | {} ambiguous | {} unrecognized | showing {}",
            total, matched, ambiguous, unrecognized, showing
        ));
    });

    ui.add_space(2.0);

    // Table wrapped in horizontal scroll area
    let available_height = ui.available_height();
    let text_height = egui::TextStyle::Body
        .resolve(ui.style())
        .size
        .max(ui.spacing().interact_size.y);

    egui::ScrollArea::horizontal().show(ui, |ui| {
        let table = TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .sense(egui::Sense::click())
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::exact(20.0)) // Status badge
            .column(Column::initial(280.0).at_least(100.0)) // Name
            .column(Column::initial(120.0).at_least(60.0)) // Serial
            .column(Column::initial(140.0).at_least(60.0)) // Internal Name
            .column(Column::initial(80.0).at_least(40.0)) // Region
            .column(Column::initial(80.0).at_least(60.0)) // CRC32
            .column(Column::initial(200.0).at_least(80.0)) // DAT Match
            .min_scrolled_height(0.0)
            .max_scroll_height(available_height);

        table
            .header(20.0, |mut header| {
                header.col(|ui| {
                    ui.strong("");
                });
                header.col(|ui| {
                    ui.strong("Name");
                });
                header.col(|ui| {
                    ui.strong("Serial");
                });
                header.col(|ui| {
                    ui.strong("Internal Name");
                });
                header.col(|ui| {
                    ui.strong("Region");
                });
                header.col(|ui| {
                    ui.strong("CRC32");
                });
                header.col(|ui| {
                    ui.strong("DAT Match");
                });
            })
            .body(|body| {
                body.rows(text_height, row_data.len(), |mut row| {
                    let row_idx = row.index();
                    let data = &row_data[row_idx];
                    let is_selected = app.selected_entries.contains(&data.entry_idx);
                    let is_focused = app.focused_entry == Some(data.entry_idx);

                    // Highlight the entire row
                    row.set_selected(is_selected || is_focused);

                    // Status badge with tooltip
                    let r1 = row.col(|ui| {
                        status_badge::show(ui, data.status).on_hover_text(data.status.tooltip());
                    });

                    // Paint text directly in cells so no WidgetRect is created,
                    // allowing the cell response to handle all click interaction.
                    let r2 = row.col(|ui| paint_cell_text(ui, &data.name));
                    let r3 = row.col(|ui| {
                        paint_cell_text(ui, data.serial.as_deref().unwrap_or(""));
                    });
                    let r4 = row.col(|ui| {
                        paint_cell_text(ui, data.internal_name.as_deref().unwrap_or(""));
                    });
                    let r5 = row.col(|ui| {
                        paint_cell_text(
                            ui,
                            if data.regions.is_empty() {
                                ""
                            } else {
                                &data.regions
                            },
                        );
                    });
                    let r6 = row.col(|ui| {
                        paint_cell_text(ui, data.crc32.as_deref().unwrap_or(""));
                    });
                    let r7 = row.col(|ui| {
                        paint_cell_text(ui, data.dat_match.as_deref().unwrap_or(""));
                    });

                    // Union all column responses for click and context menu
                    let row_resp = r1.1 | r2.1 | r3.1 | r4.1 | r5.1 | r6.1 | r7.1;

                    // Right-click selection: if right-clicked row isn't selected, select just it
                    if row_resp.secondary_clicked() {
                        if !app.selected_entries.contains(&data.entry_idx) {
                            app.selected_entries.clear();
                            app.selected_entries.insert(data.entry_idx);
                        }
                        app.focused_entry = Some(data.entry_idx);
                    }

                    // Left-click
                    if row_resp.clicked() {
                        let modifiers = ctx.input(|i| i.modifiers);
                        handle_row_click(app, data.entry_idx, modifiers);
                    }

                    // Context menu on unioned response
                    row_resp.context_menu(|ui| {
                        show_row_context_menu(ui, app, ctx, console_idx, data);
                    });
                });
            });
    });
}

/// Render the context menu for a game table row.
fn show_row_context_menu(
    ui: &mut egui::Ui,
    app: &mut RetroJunkApp,
    ctx: &egui::Context,
    console_idx: usize,
    data: &RowData,
) {
    if ui.button("Calculate Hashes").clicked() {
        backend::hash::compute_hashes_for_selection(app, console_idx);
        ui.close_menu();
    }

    if ui.button("Re-scrape Media").clicked() {
        backend::media::rescrape_media_for_selection(app, console_idx, ctx);
        ui.close_menu();
    }

    if ui.button("Re-generate Miximages").clicked() {
        backend::media::regenerate_miximages_for_selection(app, console_idx, ctx);
        ui.close_menu();
    }

    if ui.button("Auto Rename").clicked() {
        backend::rename::rename_selected_entries(app, console_idx, ctx);
        ui.close_menu();
    }

    // Set Region submenu
    show_set_region_submenu(ui, app, console_idx);

    ui.separator();

    if ui.button(util::REVEAL_LABEL).clicked() {
        util::reveal_in_file_manager(&data.file_path);
        ui.close_menu();
    }

    ui.separator();

    if ui.button("Copy File Path").clicked() {
        let paths = collect_selected_field(app, console_idx, |entry| {
            Some(entry.game_entry.analysis_path().display().to_string())
        });
        ui.output_mut(|o| o.copied_text = paths);
        ui.close_menu();
    }

    let has_serial = data.serial.is_some()
        || app.selected_entries.iter().any(|&i| {
            app.library.consoles[console_idx]
                .entries
                .get(i)
                .and_then(|e| e.identification.as_ref())
                .and_then(|id| id.serial_number.as_ref())
                .is_some()
        });
    if ui
        .add_enabled(has_serial, egui::Button::new("Copy Serial"))
        .clicked()
    {
        let serials = collect_selected_field(app, console_idx, |entry| {
            entry
                .identification
                .as_ref()
                .and_then(|id| id.serial_number.clone())
        });
        ui.output_mut(|o| o.copied_text = serials);
        ui.close_menu();
    }

    let has_crc32 = data.crc32.is_some()
        || app.selected_entries.iter().any(|&i| {
            app.library.consoles[console_idx]
                .entries
                .get(i)
                .and_then(|e| e.hashes.as_ref())
                .is_some()
        });
    if ui
        .add_enabled(has_crc32, egui::Button::new("Copy CRC32"))
        .clicked()
    {
        let crcs = collect_selected_field(app, console_idx, |entry| {
            entry.hashes.as_ref().map(|h| h.crc32.clone())
        });
        ui.output_mut(|o| o.copied_text = crcs);
        ui.close_menu();
    }

    let has_dat = data.dat_match.is_some()
        || app.selected_entries.iter().any(|&i| {
            app.library.consoles[console_idx]
                .entries
                .get(i)
                .and_then(|e| e.dat_match.as_ref())
                .is_some()
        });
    if ui
        .add_enabled(has_dat, egui::Button::new("Copy DAT Name"))
        .clicked()
    {
        let dats = collect_selected_field(app, console_idx, |entry| {
            entry.dat_match.as_ref().map(|dm| dm.game_name.clone())
        });
        ui.output_mut(|o| o.copied_text = dats);
        ui.close_menu();
    }
}

/// Render the "Set Region" submenu with recommended and other regions.
fn show_set_region_submenu(ui: &mut egui::Ui, app: &mut RetroJunkApp, console_idx: usize) {
    // Pre-compute the intersection of detected regions across selected entries.
    // Entries with no detected regions are excluded from the intersection so they
    // don't narrow the recommendations to empty.
    let console = &app.library.consoles[console_idx];
    let mut recommended: Option<HashSet<Region>> = None;
    for &i in &app.selected_entries {
        if let Some(entry) = console.entries.get(i) {
            if let Some(ref id) = entry.identification {
                if !id.regions.is_empty() {
                    let set: HashSet<Region> = id.regions.iter().copied().collect();
                    recommended = Some(match recommended {
                        Some(acc) => acc.intersection(&set).copied().collect(),
                        None => set,
                    });
                }
            }
        }
    }
    let recommended = recommended.unwrap_or_default();

    ui.menu_button("Set Region", |ui| {
        if ui.button("Auto-detect").clicked() {
            for &i in &app.selected_entries.clone() {
                if let Some(entry) = app.library.consoles[console_idx].entries.get_mut(i) {
                    entry.region_override = None;
                }
            }
            app.save_library_cache();
            ui.close_menu();
        }

        if !recommended.is_empty() {
            ui.separator();
            ui.label("Recommended");
            for &region in Region::ALL {
                if recommended.contains(&region) && ui.button(region.name()).clicked() {
                    for &i in &app.selected_entries.clone() {
                        if let Some(entry) = app.library.consoles[console_idx].entries.get_mut(i) {
                            entry.region_override = Some(region);
                        }
                    }
                    app.save_library_cache();
                    ui.close_menu();
                }
            }
        }

        ui.separator();
        ui.label("Other Regions");
        for &region in Region::ALL {
            if !recommended.contains(&region) && ui.button(region.name()).clicked() {
                for &i in &app.selected_entries.clone() {
                    if let Some(entry) = app.library.consoles[console_idx].entries.get_mut(i) {
                        entry.region_override = Some(region);
                    }
                }
                app.save_library_cache();
                ui.close_menu();
            }
        }
    });
}

/// Collect a field from all selected entries, joining with newlines.
/// Entries where the extractor returns `None` are skipped.
fn collect_selected_field(
    app: &RetroJunkApp,
    console_idx: usize,
    extractor: impl Fn(&crate::state::LibraryEntry) -> Option<String>,
) -> String {
    let console = &app.library.consoles[console_idx];
    let mut values: Vec<String> = app
        .selected_entries
        .iter()
        .copied()
        .filter_map(|i| {
            let entry = console.entries.get(i)?;
            extractor(entry)
        })
        .collect();
    values.sort();
    values.join("\n")
}

struct RowData {
    entry_idx: usize,
    status: EntryStatus,
    name: String,
    file_path: PathBuf,
    serial: Option<String>,
    internal_name: Option<String>,
    regions: String,
    crc32: Option<String>,
    dat_match: Option<String>,
}

fn handle_row_click(app: &mut RetroJunkApp, entry_idx: usize, modifiers: egui::Modifiers) {
    if modifiers.ctrl || modifiers.command {
        // Toggle selection
        if app.selected_entries.contains(&entry_idx) {
            app.selected_entries.remove(&entry_idx);
        } else {
            app.selected_entries.insert(entry_idx);
        }
    } else if modifiers.shift {
        // Range select
        if let Some(focused) = app.focused_entry {
            let start = focused.min(entry_idx);
            let end = focused.max(entry_idx);
            for i in start..=end {
                app.selected_entries.insert(i);
            }
        } else {
            app.selected_entries.clear();
            app.selected_entries.insert(entry_idx);
        }
    } else {
        // Single select
        app.selected_entries.clear();
        app.selected_entries.insert(entry_idx);
    }

    app.focused_entry = Some(entry_idx);
}

/// Paint text in a table cell without creating a widget.
///
/// This avoids registering a `WidgetRect` that would intercept pointer events,
/// allowing the cell's own response (set via `TableBuilder::sense`) to handle
/// all click and context-menu interaction.
fn paint_cell_text(ui: &mut egui::Ui, text: &str) {
    if text.is_empty() {
        return;
    }
    let font_id = egui::TextStyle::Body.resolve(ui.style());
    let color = ui.visuals().text_color();
    ui.painter().text(
        ui.max_rect().left_center(),
        egui::Align2::LEFT_CENTER,
        text,
        font_id,
        color,
    );
}
