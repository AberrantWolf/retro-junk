use egui_extras::{Column, TableBuilder};

use crate::app::RetroJunkApp;
use crate::state::EntryStatus;
use crate::widgets::status_badge;

/// Render the sortable, filterable game table for the selected console.
pub fn show(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
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
                if let Some(ref serial) = id.serial_number {
                    if serial.to_lowercase().contains(&filter) {
                        return true;
                    }
                }
                if let Some(ref iname) = id.internal_name {
                    if iname.to_lowercase().contains(&filter) {
                        return true;
                    }
                }
            }
            if let Some(ref dm) = entry.dat_match {
                if dm.game_name.to_lowercase().contains(&filter) {
                    return true;
                }
            }
            false
        })
        .map(|(i, _)| i)
        .collect();

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

                    let mut clicked = false;
                    let mut modifiers = egui::Modifiers::NONE;

                    // Status badge with tooltip
                    row.col(|ui| {
                        let response = status_badge::show(ui, data.status)
                            .on_hover_text(data.status.tooltip());
                        if response.clicked() {
                            clicked = true;
                        }
                        modifiers = ui.input(|i| i.modifiers);
                    });

                    // Name
                    row.col(|ui| {
                        let response =
                            ui.add(egui::Label::new(&data.name).sense(egui::Sense::click()));
                        if response.clicked() {
                            clicked = true;
                        }
                    });

                    // Serial
                    row.col(|ui| {
                        let text = data.serial.as_deref().unwrap_or("");
                        let response = ui.add(egui::Label::new(text).sense(egui::Sense::click()));
                        if response.clicked() {
                            clicked = true;
                        }
                    });

                    // Internal Name
                    row.col(|ui| {
                        let text = data.internal_name.as_deref().unwrap_or("");
                        let response = ui.add(egui::Label::new(text).sense(egui::Sense::click()));
                        if response.clicked() {
                            clicked = true;
                        }
                    });

                    // Region
                    row.col(|ui| {
                        let text = if data.regions.is_empty() {
                            ""
                        } else {
                            &data.regions
                        };
                        let response = ui.add(egui::Label::new(text).sense(egui::Sense::click()));
                        if response.clicked() {
                            clicked = true;
                        }
                    });

                    // CRC32
                    row.col(|ui| {
                        let text = data.crc32.as_deref().unwrap_or("");
                        let response = ui.add(egui::Label::new(text).sense(egui::Sense::click()));
                        if response.clicked() {
                            clicked = true;
                        }
                    });

                    // DAT Match
                    row.col(|ui| {
                        let text = data.dat_match.as_deref().unwrap_or("");
                        let response = ui.add(egui::Label::new(text).sense(egui::Sense::click()));
                        if response.clicked() {
                            clicked = true;
                        }
                    });

                    if clicked {
                        handle_row_click(app, data.entry_idx, modifiers);
                    }
                });
            });
    });
}

struct RowData {
    entry_idx: usize,
    status: EntryStatus,
    name: String,
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
