use retro_junk_lib::Region;

use crate::app::RetroJunkApp;
use crate::state::{DISPLAY_MEDIA_TYPES, EntryStatus};

/// Render the detail panel for the focused entry.
pub fn show(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    ui.heading("Details");
    ui.separator();

    let (console_idx, entry_idx) = match (app.selected_console, app.focused_entry) {
        (Some(ci), Some(ei)) => (ci, ei),
        _ => {
            ui.label("Select an entry to view details.");
            return;
        }
    };

    if app
        .library
        .consoles
        .get(console_idx)
        .and_then(|c| c.entries.get(entry_idx))
        .is_none()
    {
        ui.label("Entry not found.");
        return;
    }

    // Lazy media discovery: kick off background load on first focus.
    // Sets an empty sentinel immediately to prevent re-triggering.
    if app.library.consoles[console_idx].entries[entry_idx]
        .media_paths
        .is_none()
    {
        // Sentinel: mark as "loading" so we don't spawn again next frame
        app.library.consoles[console_idx].entries[entry_idx].media_paths =
            Some(std::collections::HashMap::new());

        if let Some(ref root_path) = app.root_path {
            let folder_name = app.library.consoles[console_idx].folder_name.clone();
            let rom_stem = app.library.consoles[console_idx].entries[entry_idx]
                .game_entry
                .rom_stem()
                .to_owned();

            crate::backend::media::load_media_for_entry(
                app.message_tx.clone(),
                ui.ctx().clone(),
                root_path.clone(),
                folder_name,
                entry_idx,
                rom_stem,
                app.settings.general.media_dir.clone(),
            );
        }
    }

    egui::ScrollArea::vertical().show(ui, |ui| {
        // Borrow console/entry for the read-only section before the region ComboBox.
        let console = &app.library.consoles[console_idx];
        let entry = &console.entries[entry_idx];

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

        // Region (ComboBox with override support)
        // Extract needed data before dropping borrows for potential mutation.
        let detected_regions: Vec<Region> = entry
            .identification
            .as_ref()
            .map(|id| id.regions.clone())
            .unwrap_or_default();
        let effective = entry.effective_regions();
        let current_override = entry.region_override;

        // Build display text for the current selection
        let combo_label = if current_override.is_none() {
            if detected_regions.is_empty() {
                "Unknown".to_string()
            } else {
                let names: Vec<&str> = detected_regions.iter().map(|r| r.name()).collect();
                format!("Auto-detect ({})", names.join(", "))
            }
        } else {
            effective
                .first()
                .map(|r| r.name().to_string())
                .unwrap_or_else(|| "Unknown".to_string())
        };

        let mut new_override = current_override;

        ui.horizontal(|ui| {
            ui.label("Region:");
            let combo_id = egui::Id::new("region_override_combo")
                .with(console_idx)
                .with(entry_idx);
            egui::ComboBox::from_id_salt(combo_id)
                .selected_text(&combo_label)
                .show_ui(ui, |ui| {
                    // Auto-detect option (clears override)
                    let auto_label = if detected_regions.is_empty() {
                        "Auto-detect".to_string()
                    } else {
                        let names: Vec<&str> = detected_regions.iter().map(|r| r.name()).collect();
                        format!("Auto-detect ({})", names.join(", "))
                    };
                    if ui
                        .selectable_label(current_override.is_none(), &auto_label)
                        .clicked()
                    {
                        new_override = None;
                    }

                    ui.separator();

                    // If ambiguous (>1 detected), group detected first
                    if detected_regions.len() > 1 {
                        ui.label(egui::RichText::new("Detected:").weak().small());
                        for &r in &detected_regions {
                            if ui
                                .selectable_label(current_override == Some(r), r.name())
                                .clicked()
                            {
                                new_override = Some(r);
                            }
                        }
                        ui.separator();
                        ui.label(egui::RichText::new("Other:").weak().small());
                        for &r in Region::ALL {
                            if !detected_regions.contains(&r)
                                && ui
                                    .selectable_label(current_override == Some(r), r.name())
                                    .clicked()
                            {
                                new_override = Some(r);
                            }
                        }
                    } else {
                        // Specific (1) or none: show all regions flat
                        for &r in Region::ALL {
                            if ui
                                .selectable_label(current_override == Some(r), r.name())
                                .clicked()
                            {
                                new_override = Some(r);
                            }
                        }
                    }
                });
        });

        // Apply override change
        if new_override != current_override {
            app.library.consoles[console_idx].entries[entry_idx].region_override = new_override;
            app.save_library_cache();
        }

        // Warning text
        if app.settings.general.warn_on_region_override
            && let Some(overridden) = new_override
        {
            let should_warn = if detected_regions.len() == 1 {
                // Specific detection: warn if override differs
                detected_regions[0] != overridden
            } else if detected_regions.len() > 1 {
                // Ambiguous: warn if override not in detected set
                !detected_regions.contains(&overridden)
            } else {
                false
            };

            if should_warn {
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(format!(
                            "\u{26a0} Overriding detected region ({})",
                            detected_regions
                                .iter()
                                .map(|r| r.name())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ))
                        .small()
                        .color(egui::Color32::from_rgb(220, 180, 30)),
                    );
                });
            }
        }

        // Re-borrow after potential mutation
        let console = &app.library.consoles[console_idx];
        let entry = &console.entries[entry_idx];

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

        if let Some(ref id) = entry.identification
            && let Some(size) = id.file_size
        {
            ui.horizontal(|ui| {
                ui.label("Size:");
                ui.label(retro_junk_lib::util::format_bytes(size));
            });
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
            detail_row(
                ui,
                "Data Size",
                &retro_junk_lib::util::format_bytes(hashes.data_size),
            );
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

        // Titles (from catalog DB enrichment)
        if entry.cover_title.is_some() || entry.screen_title.is_some() {
            ui.add_space(4.0);
            ui.separator();
            ui.label(egui::RichText::new("Titles").strong());
            ui.add_space(2.0);
            if let Some(ref ct) = entry.cover_title {
                detail_row(ui, "Box Title", ct);
            }
            if let Some(ref st) = entry.screen_title {
                detail_row(ui, "Screen Title", st);
            }
        }

        // Media
        if let Some(ref media) = entry.media_paths
            && !media.is_empty()
        {
            ui.add_space(4.0);
            ui.separator();
            ui.label(egui::RichText::new("Media").strong());
            ui.add_space(2.0);

            let panel_width = ui.available_width();

            for &mt in DISPLAY_MEDIA_TYPES {
                if let Some(path) = media.get(&mt) {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(mt.to_string()).weak());

                    let uri = format!("bytes://media/{}", path.display());
                    let image = egui::Image::new(uri)
                        .fit_to_exact_size(egui::vec2(panel_width, panel_width))
                        .maintain_aspect_ratio(true)
                        .rounding(4.0);

                    let response = ui.add(image);
                    if let Some(path_str) = path.to_str() {
                        response.on_hover_text(path_str);
                    }
                }
            }
        }
    });
}

fn detail_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(format!("{}:", label)).weak());
        ui.label(value);
    });
}
