//! Detail panel for the focused library entry.
//!
//! # Sizing contract
//!
//! egui persists a side panel's width as *the laid-out width of its content*
//! every frame — a single label that refuses to wrap silently widens the
//! panel, overrides any width the user dragged, and clips everything else
//! once the panel hits its max size. Labels inside `ui.horizontal(..)`
//! default to `TextWrapMode::Extend`, so **every dynamic value rendered here
//! must go through a wrapping helper** ([`copyable_label`], [`detail_row`],
//! [`note`]) or otherwise constrain its width (the region `ComboBox`
//! truncates, images fit to the panel width). `detail_panel_tests.rs` guards
//! this invariant.

#[cfg(test)]
#[path = "detail_panel_tests.rs"]
mod detail_panel_tests;

use retro_junk_catalog::CatalogTag;
use retro_junk_lib::Region;

use crate::app::RetroJunkApp;
use crate::state::{DISPLAY_ASSET_TYPES, EntryStatus};
use crate::theme::{STATUS_ERR, STATUS_WARN, STATUS_WARN_STRONG};

/// Indent for fields nested under a per-disc or per-reference heading.
const NESTED_INDENT: f32 = 16.0;

/// Render the detail panel for the focused entry.
pub fn show(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    ui.heading("Details");
    ui.separator();

    let (Some(console_id), Some(entry_id)) =
        (app.ui_state.selected_console, app.ui_state.focused_entry)
    else {
        ui.label("Select an entry to view details.");
        return;
    };

    let Some(console_idx) = app.browser.find_by_id(console_id) else {
        ui.label("Console not found.");
        return;
    };
    let Some(entry_idx) = app.browser.consoles[console_idx].entry_index(entry_id) else {
        ui.label("Entry not found.");
        return;
    };

    // Lazy media discovery: kick off background load on first focus.
    // Sets an empty sentinel immediately to prevent re-triggering.
    if app.browser.consoles[console_idx].entries[entry_idx]
        .asset_paths
        .is_none()
        && !app.browser.asset_discovery_in_flight.contains(&entry_id)
    {
        // Sentinel: mark as "loading" so we don't spawn again next frame
        app.browser.consoles[console_idx].entries[entry_idx].asset_paths =
            Some(std::collections::HashMap::new());
        app.browser.asset_discovery_in_flight.insert(entry_id);

        if let Some(ref root_path) = app.root_path {
            let folder_name = app.browser.consoles[console_idx].folder_name.clone();
            let entry = &app.browser.consoles[console_idx].entries[entry_idx];
            let entry_name = entry.game_entry.display_name().to_string();
            let rom_stem = entry.game_entry.rom_stem().to_owned();

            crate::backend::assets::load_assets_for_entry(
                app.message_tx.clone(),
                ui.ctx().clone(),
                root_path.clone(),
                folder_name,
                entry_name,
                rom_stem,
                app.settings.general.assets_dir.clone(),
            );
        }
    }

    egui::ScrollArea::vertical().show(ui, |ui| {
        // Borrow console/entry for the read-only section before the region ComboBox.
        let console = &app.browser.consoles[console_idx];
        let entry = &console.entries[entry_idx];

        // Status
        ui.horizontal(|ui| {
            ui.label("Status:");
            let effective = entry.effective_status();
            let (text, color) = match effective {
                EntryStatus::Unknown => ("Unknown", effective.color()),
                EntryStatus::Unrecognized => ("Unrecognized", effective.color()),
                EntryStatus::Ambiguous => ("Ambiguous", effective.color()),
                EntryStatus::Matched => ("Matched", effective.color()),
                EntryStatus::Tagged(CatalogTag::Homebrew) => ("Homebrew", effective.color()),
                EntryStatus::Tagged(CatalogTag::Modded) => ("Modded", effective.color()),
            };
            let resp = ui.colored_label(color, text);
            resp.context_menu(|ui| {
                if ui.button("Copy").clicked() {
                    crate::util::copy_and_close(ui, text.to_string());
                }
            });
        });

        // Show ambiguous candidates if applicable
        if entry.status == EntryStatus::Ambiguous && !entry.ambiguous_candidates.is_empty() {
            ui.add_space(2.0);
            ui.label(egui::RichText::new("Could be one of:").weak());
            for candidate in &entry.ambiguous_candidates {
                ui.horizontal_top(|ui| {
                    ui.add_space(8.0);
                    copyable_label(ui, &format!("- {candidate}"));
                });
            }
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new("Calculate hashes to resolve.")
                    .weak()
                    .italics(),
            );
        } else if entry.status == EntryStatus::Ambiguous && entry.ambiguous_candidates.is_empty() {
            ui.add_space(2.0);
            if let Some(ref discs) = entry.disc_identifications {
                let unresolved: Vec<usize> = discs
                    .iter()
                    .enumerate()
                    .filter(|(_, d)| d.dat_match.is_none())
                    .map(|(i, _)| i + 1)
                    .collect();
                if !unresolved.is_empty() {
                    let disc_list = unresolved
                        .iter()
                        .map(std::string::ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ");
                    ui.label(
                        egui::RichText::new(format!("Disc {disc_list} not matched in database."))
                            .weak(),
                    );
                }
            }
            ui.label(
                egui::RichText::new("Calculate hashes to resolve.")
                    .weak()
                    .italics(),
            );
        }

        ui.add_space(4.0);

        // Platform
        field_row(ui, "Platform", console.platform_name);

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
                let names: Vec<&str> = detected_regions
                    .iter()
                    .map(retro_junk_core::Region::name)
                    .collect();
                format!("Auto-detect ({})", names.join(", "))
            }
        } else {
            effective
                .first()
                .map_or_else(|| "Unknown".to_string(), |r| r.name().to_string())
        };

        let mut new_override = current_override;

        ui.horizontal(|ui| {
            ui.label("Region:");
            let combo_id = egui::Id::new("region_override_combo")
                .with(console_idx)
                .with(entry_idx);
            egui::ComboBox::from_id_salt(combo_id)
                .selected_text(&combo_label)
                // A long "Auto-detect (…)" label must truncate, not widen the panel.
                .wrap_mode(egui::TextWrapMode::Truncate)
                .show_ui(ui, |ui| {
                    // Auto-detect option (clears override)
                    let auto_label = if detected_regions.is_empty() {
                        "Auto-detect".to_string()
                    } else {
                        let names: Vec<&str> = detected_regions
                            .iter()
                            .map(retro_junk_core::Region::name)
                            .collect();
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
            app.browser.consoles[console_idx].entries[entry_idx].region_override = new_override;
            app.save_entry_cache(console_idx, &[entry_idx], ui.ctx());
        }

        // Warning text
        if app.settings.general.warn_on_region_override
            && let Some(overridden) = new_override
        {
            let should_warn = match detected_regions.as_slice() {
                // No detection: nothing to contradict
                [] => false,
                // Specific detection: warn if override differs
                [only] => *only != overridden,
                // Ambiguous: warn if override not in detected set
                _ => !detected_regions.contains(&overridden),
            };

            if should_warn {
                warning_note(
                    ui,
                    8.0,
                    &format!(
                        "Overriding detected region ({})",
                        detected_regions
                            .iter()
                            .map(retro_junk_core::Region::name)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                );
            }
        }

        // Re-borrow after potential mutation
        let console = &app.browser.consoles[console_idx];
        let entry = &console.entries[entry_idx];

        // Folder
        field_row(ui, "Folder", &console.folder_name);

        // File info
        let file_name =
            if let retro_junk_lib::scanner::GameEntry::MultiDisc { name, .. } = &entry.game_entry {
                name.clone()
            } else {
                let path = entry.game_entry.analysis_path();
                path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?")
                    .to_string()
            };
        field_row(ui, "File", &file_name);

        if let Some(ref id) = entry.identification
            && id.file_size != 0
        {
            field_row(
                ui,
                "Size",
                &retro_junk_lib::util::format_bytes(id.file_size),
            );
        }

        ui.separator();

        // Identification fields
        if let Some(ref id) = entry.identification {
            ui.label(egui::RichText::new("Identification").strong());
            ui.add_space(2.0);

            if !id.serial_number.is_empty() {
                detail_row(ui, "Serial", &id.serial_number);
            }
            if !id.internal_name.is_empty() {
                detail_row(ui, "Internal Name", &id.internal_name);
            }
            if !id.maker_code.is_empty() {
                detail_row(ui, "Maker", &id.maker_code);
            }
            if !id.version.is_empty() {
                detail_row(ui, "Version", &id.version);
            }
            if !id.regions.is_empty() {
                let regions: Vec<&str> = id
                    .regions
                    .iter()
                    .map(retro_junk_core::Region::name)
                    .collect();
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

        // Disc details for multi-disc entries
        if let Some(ref discs) = entry.disc_identifications
            && !discs.is_empty()
        {
            ui.add_space(4.0);
            ui.separator();
            ui.label(egui::RichText::new("Disc Details").strong());
            ui.add_space(2.0);

            for (i, disc) in discs.iter().enumerate() {
                let disc_file = disc
                    .path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?");
                detail_row(ui, &format!("Disc {}", i + 1), disc_file);
                if !disc.identification.serial_number.is_empty() {
                    nested_detail_row(ui, "Serial", &disc.identification.serial_number);
                }
                if !disc.identification.internal_name.is_empty() {
                    nested_detail_row(ui, "Internal", &disc.identification.internal_name);
                }
                if let Some(ref hashes) = disc.hashes {
                    nested_detail_row(ui, "CRC32", &hashes.crc32);
                    if let Some(ref sha1) = hashes.sha1 {
                        nested_detail_row(ui, "SHA1", sha1);
                    }
                    for warning in &hashes.warnings {
                        warning_note(ui, NESTED_INDENT, warning);
                    }
                }
                if let Some(ref dm) = disc.dat_match {
                    nested_detail_row(ui, "DAT", &dm.rom_name);
                } else if entry.status == EntryStatus::Ambiguous {
                    ui.horizontal_top(|ui| {
                        ui.add_space(NESTED_INDENT);
                        ui.label(egui::RichText::new("DAT:").weak());
                        ui.label(
                            egui::RichText::new("unresolved").color(EntryStatus::Ambiguous.color()),
                        );
                    });
                }
            }
        }

        // Broken references warning
        if let Some(ref broken) = entry.broken_references
            && !broken.is_empty()
        {
            ui.add_space(4.0);
            ui.separator();
            ui.label(
                egui::RichText::new("\u{26a0} Broken References")
                    .strong()
                    .color(STATUS_WARN_STRONG),
            );
            ui.add_space(2.0);

            for br in broken {
                let ref_name = br
                    .ref_file
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?");
                note(
                    ui,
                    0.0,
                    egui::RichText::new(format!("{} ({})", ref_name, br.format)).weak(),
                );
                for target in &br.missing_targets {
                    note(
                        ui,
                        NESTED_INDENT,
                        egui::RichText::new(format!("Missing: {target}")).color(STATUS_ERR),
                    );
                }
            }
        }

        // CUE sheet compatibility issues
        if let Some(ref issues) = entry.cue_compat_issues
            && !issues.is_empty()
        {
            let has_unfixable = issues.iter().any(|i| !i.can_auto_fix);

            ui.add_space(4.0);
            ui.separator();
            let header = if has_unfixable {
                "\u{26a0} CUE Sheet Compatibility (requires re-dump)"
            } else {
                "\u{26a0} CUE Sheet Compatibility"
            };
            ui.label(
                egui::RichText::new(header)
                    .strong()
                    .color(STATUS_WARN_STRONG),
            );
            ui.add_space(2.0);

            for issue in issues {
                let (suffix, color) = if issue.can_auto_fix {
                    ("fixable", STATUS_WARN_STRONG)
                } else {
                    ("re-dump required", STATUS_ERR)
                };
                note(
                    ui,
                    0.0,
                    egui::RichText::new(format!(
                        "{}: {} ({})",
                        issue.file_name, issue.summary, suffix
                    ))
                    .color(color),
                );
            }
        }

        // Hashes (single-file entries only; multi-disc hashes shown per-disc above)
        if entry.disc_identifications.is_none()
            && let Some(ref hashes) = entry.hashes
        {
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
            for warning in &hashes.warnings {
                warning_note(ui, 0.0, warning);
            }
        }

        // DAT match
        if let Some(ref dm) = entry.dat_match {
            ui.add_space(4.0);
            ui.separator();
            ui.label(egui::RichText::new("DAT Match").strong());
            ui.add_space(2.0);
            detail_row(ui, "Game", &dm.game_name);
            detail_row(ui, "Method", &format!("{:?}", dm.method));
            if dm.cross_region {
                let dat_region: &str = if dm.region.is_empty() {
                    "unknown"
                } else {
                    &dm.region
                };
                let detected = entry.identification.as_ref().map_or_else(
                    || "unknown".to_string(),
                    |id| {
                        id.regions
                            .iter()
                            .map(retro_junk_core::Region::name)
                            .collect::<Vec<_>>()
                            .join(", ")
                    },
                );
                warning_note(
                    ui,
                    0.0,
                    &format!(
                        "Hash matches {dat_region} release \u{2014} detected region is {detected}"
                    ),
                );
            }
        }

        // Titles (from catalog DB enrichment)
        if !entry.cover_title.is_empty() || !entry.screen_title.is_empty() {
            ui.add_space(4.0);
            ui.separator();
            ui.label(egui::RichText::new("Titles").strong());
            ui.add_space(2.0);
            if !entry.cover_title.is_empty() {
                detail_row(ui, "Box Title", &entry.cover_title);
            }
            if !entry.screen_title.is_empty() {
                detail_row(ui, "Screen Title", &entry.screen_title);
            }
        }

        // Media
        if let Some(ref media) = entry.asset_paths
            && !media.is_empty()
        {
            ui.add_space(4.0);
            ui.separator();
            ui.label(egui::RichText::new("Media").strong());
            ui.add_space(2.0);

            let panel_width = ui.available_width();

            for &mt in DISPLAY_ASSET_TYPES {
                if let Some(path) = media.get(&mt) {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(mt.to_string()).weak());

                    let uri = format!("bytes://media/{}", path.display());
                    let image = egui::Image::new(uri)
                        .fit_to_exact_size(egui::vec2(panel_width, panel_width))
                        .maintain_aspect_ratio(true)
                        .corner_radius(4.0);

                    let response = ui.add(image);
                    if let Some(path_str) = path.to_str() {
                        response.on_hover_text(path_str);
                    }
                }
            }
        }
    });
}

/// A label that wraps within the available width and offers "Copy" on
/// right-click. The explicit wrap mode matters: inside `ui.horizontal(..)`
/// labels default to extending, which widens the whole panel (see module doc).
fn copyable_label(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let resp = ui.add(egui::Label::new(text).wrap());
    resp.context_menu(|ui| {
        if ui.button("Copy").clicked() {
            crate::util::copy_and_close(ui, text.to_string());
        }
    });
    resp
}

/// A top-level `Name: value` row (normal-weight name, wrapping copyable value).
fn field_row(ui: &mut egui::Ui, label: &str, value: &str) {
    row_impl(ui, 0.0, egui::RichText::new(format!("{label}:")), value);
}

/// A `Name: value` row with a de-emphasized name, as used in the
/// Identification/Hashes/DAT sections.
fn detail_row(ui: &mut egui::Ui, label: &str, value: &str) {
    row_impl(
        ui,
        0.0,
        egui::RichText::new(format!("{label}:")).weak(),
        value,
    );
}

/// A [`detail_row`] indented one level (per-disc fields).
fn nested_detail_row(ui: &mut egui::Ui, label: &str, value: &str) {
    row_impl(
        ui,
        NESTED_INDENT,
        egui::RichText::new(format!("{label}:")).weak(),
        value,
    );
}

fn row_impl(ui: &mut egui::Ui, indent: f32, name: egui::RichText, value: &str) {
    // Top-aligned so the name stays on the first line when the value wraps.
    ui.horizontal_top(|ui| {
        if indent > 0.0 {
            ui.add_space(indent);
        }
        ui.label(name);
        copyable_label(ui, value);
    });
}

/// A styled one-off line (warning/error text) that wraps within the panel.
fn note(ui: &mut egui::Ui, indent: f32, text: egui::RichText) {
    ui.horizontal_top(|ui| {
        if indent > 0.0 {
            ui.add_space(indent);
        }
        ui.add(egui::Label::new(text).wrap());
    });
}

/// A small yellow "⚠ …" [`note`].
fn warning_note(ui: &mut egui::Ui, indent: f32, text: &str) {
    note(
        ui,
        indent,
        egui::RichText::new(format!("\u{26a0} {text}"))
            .small()
            .color(STATUS_WARN),
    );
}
