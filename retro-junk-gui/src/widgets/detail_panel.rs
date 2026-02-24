use crate::app::RetroJunkApp;
use crate::state::EntryStatus;

/// Render the detail panel for the focused entry.
pub fn show(ui: &mut egui::Ui, app: &RetroJunkApp) {
    ui.heading("Details");
    ui.separator();

    let (console_idx, entry_idx) = match (app.selected_console, app.focused_entry) {
        (Some(ci), Some(ei)) => (ci, ei),
        _ => {
            ui.label("Select an entry to view details.");
            return;
        }
    };

    let console = &app.library.consoles[console_idx];
    let entry = match console.entries.get(entry_idx) {
        Some(e) => e,
        None => {
            ui.label("Entry not found.");
            return;
        }
    };

    egui::ScrollArea::vertical().show(ui, |ui| {
        // Status
        ui.horizontal(|ui| {
            ui.label("Status:");
            let (text, color) = match entry.status {
                EntryStatus::Unknown => ("Unknown", entry.status.color()),
                EntryStatus::Unrecognized => ("Unrecognized", entry.status.color()),
                EntryStatus::Ambiguous => ("Ambiguous", entry.status.color()),
                EntryStatus::Matched => ("Matched", entry.status.color()),
            };
            ui.colored_label(color, text);
        });

        // Show ambiguous candidates if applicable
        if entry.status == EntryStatus::Ambiguous && !entry.ambiguous_candidates.is_empty() {
            ui.add_space(2.0);
            ui.label(egui::RichText::new("Could be one of:").weak());
            for candidate in &entry.ambiguous_candidates {
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.label(format!("- {}", candidate));
                });
            }
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new("Calculate hashes to resolve.")
                    .weak()
                    .italics(),
            );
        }

        ui.add_space(4.0);

        // Platform
        ui.horizontal(|ui| {
            ui.label("Platform:");
            ui.label(console.platform_name);
        });

        // Folder
        ui.horizontal(|ui| {
            ui.label("Folder:");
            ui.label(&console.folder_name);
        });

        // File info
        ui.horizontal(|ui| {
            ui.label("File:");
            let path = entry.game_entry.analysis_path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
            ui.label(name);
        });

        if let Some(ref id) = entry.identification {
            if let Some(size) = id.file_size {
                ui.horizontal(|ui| {
                    ui.label("Size:");
                    ui.label(retro_junk_lib::util::format_bytes(size));
                });
            }
        }

        ui.separator();

        // Identification fields
        if let Some(ref id) = entry.identification {
            ui.label(egui::RichText::new("Identification").strong());
            ui.add_space(2.0);

            if let Some(ref serial) = id.serial_number {
                detail_row(ui, "Serial", serial);
            }
            if let Some(ref name) = id.internal_name {
                detail_row(ui, "Internal Name", name);
            }
            if let Some(ref maker) = id.maker_code {
                detail_row(ui, "Maker", maker);
            }
            if let Some(ref version) = id.version {
                detail_row(ui, "Version", version);
            }
            if !id.regions.is_empty() {
                let regions: Vec<&str> = id.regions.iter().map(|r| r.name()).collect();
                detail_row(ui, "Region", &regions.join(", "));
            }

            // Extra fields
            if !id.extra.is_empty() {
                ui.add_space(4.0);
                ui.label(egui::RichText::new("Extra").strong());
                ui.add_space(2.0);
                let mut keys: Vec<&String> = id.extra.keys().collect();
                keys.sort();
                for key in keys {
                    detail_row(ui, key, &id.extra[key]);
                }
            }
        }

        // Hashes
        if let Some(ref hashes) = entry.hashes {
            ui.add_space(4.0);
            ui.separator();
            ui.label(egui::RichText::new("Hashes").strong());
            ui.add_space(2.0);
            detail_row(ui, "CRC32", &hashes.crc32);
            if let Some(ref sha1) = hashes.sha1 {
                detail_row(ui, "SHA1", sha1);
            }
            if let Some(ref md5) = hashes.md5 {
                detail_row(ui, "MD5", md5);
            }
            detail_row(ui, "Data Size", &retro_junk_lib::util::format_bytes(hashes.data_size));
        }

        // DAT match
        if let Some(ref dm) = entry.dat_match {
            ui.add_space(4.0);
            ui.separator();
            ui.label(egui::RichText::new("DAT Match").strong());
            ui.add_space(2.0);
            detail_row(ui, "Game", &dm.game_name);
            detail_row(ui, "Method", &format!("{:?}", dm.method));
        }
    });
}

fn detail_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(format!("{}:", label)).weak());
        ui.label(value);
    });
}
