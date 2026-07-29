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
use crate::state::{DISPLAY_ASSET_TYPES, DiscVerification, EntryStatus};
use crate::theme::{STATUS_ERR, STATUS_WARN, STATUS_WARN_STRONG};

/// Indent for fields nested under a per-disc or per-reference heading.
const NESTED_INDENT: f32 = 16.0;

/// Render the detail panel for the focused entry.
pub fn show(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    ui.heading("Details");
    ui.separator();

    if app.selected_library_row_count() > 1 {
        show_multi_selection(ui, app);
        return;
    }

    if app.ui_state.focused_archive_release.is_some() {
        show_archive_release(ui, app);
        return;
    }

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
    app.request_entry_detail(entry_id, ui.ctx());
    let Some(entry_idx) = app.browser.consoles[console_idx].entry_index(entry_id) else {
        ui.spinner();
        ui.label("Loading entry details…");
        return;
    };

    ensure_entry_assets(app, console_idx, entry_id, ui.ctx());
    show_file_actions(ui, app, console_idx, true);
    ui.separator();

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
                EntryStatus::LikelyMatched => ("Likely match", effective.color()),
                EntryStatus::Matched => ("Verified", effective.color()),
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

        if entry.status == EntryStatus::LikelyMatched {
            ui.add_space(2.0);
            let explanation = if entry.hashes.is_some()
                || entry
                    .disc_identifications
                    .as_ref()
                    .is_some_and(|discs| discs.iter().any(|disc| disc.hashes.is_some()))
            {
                "The ROM header identifies one DAT release, but the calculated hash does not verify its bytes."
            } else {
                "The ROM header identifies one DAT release. Calculate hashes to verify the ROM bytes."
            };
            ui.label(egui::RichText::new(explanation).weak().italics());
        }

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
            if let Some(entry_id) = app.browser.consoles[console_idx].entries[entry_idx].id {
                app.set_entry_regions([entry_id], new_override, ui.ctx());
            }
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
                if disc.disc_verification != DiscVerification::NotApplicable {
                    nested_detail_row(
                        ui,
                        "Disc Integrity",
                        disc_verification_label(disc.disc_verification),
                    );
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
            if entry.disc_verification != DiscVerification::NotApplicable {
                detail_row(
                    ui,
                    "Disc Integrity",
                    disc_verification_label(entry.disc_verification),
                );
            }
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
        if let Some(ref media) = entry.asset_paths {
            show_media(ui, media);
        }
    });
}

fn show_multi_selection(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    let count = app.selected_library_row_count();
    ui.strong(format!("{count} library items selected"));
    ui.weak("Only values and actions shared by the selection are shown.");
    ui.add_space(8.0);

    let Some(console_idx) = app.selected_console_index() else {
        return;
    };
    let console = &app.browser.consoles[console_idx];
    let folder_name = console.folder_name.clone();
    detail_row(ui, "Console", console.platform_name);

    let selected_releases = app
        .browser
        .active_page
        .as_ref()
        .map(|page| {
            page.archived_releases
                .iter()
                .filter(|release| {
                    app.ui_state
                        .selected_archive_releases
                        .contains(&release.summary.archive_release_id)
                })
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let grouped_ids = selected_releases
        .iter()
        .flat_map(|release| {
            release
                .playable_library_entries
                .iter()
                .map(|entry| entry.id)
        })
        .collect::<std::collections::HashSet<_>>();
    let selected_entries = app.ui_state.selected_entries.iter().filter_map(|id| {
        (!grouped_ids.contains(id))
            .then(|| console.entry_by_id(*id))
            .flatten()
    });

    let regions = selected_releases
        .iter()
        .map(|release| release.summary.region.clone())
        .chain(selected_entries.clone().map(|entry| {
            entry
                .effective_regions()
                .iter()
                .map(retro_junk_lib::Region::name)
                .collect::<Vec<_>>()
                .join(", ")
        }))
        .collect::<Vec<_>>();
    if regions.len() == count
        && let Some(region) = common_value(&regions).filter(|region| !region.is_empty())
    {
        detail_row(ui, "Region", region);
    }

    let archive_states = selected_releases
        .iter()
        .map(|release| {
            if release.summary.archive_complete {
                "Complete"
            } else {
                "Incomplete"
            }
        })
        .collect::<Vec<_>>();
    if archive_states.len() == count
        && let Some(state) = common_value(&archive_states)
    {
        detail_row(ui, "Archive", state);
    }

    ui.add_space(10.0);
    let details_ready = app.selected_entry_details_loaded();
    let archive_busy = app
        .operations
        .iter()
        .any(|operation| operation.scope == "archive");
    let all_releases_scrapeable = selected_releases
        .iter()
        .all(|release| release.scrape_identity.is_some());
    let can_scrape = details_ready
        && (!app.ui_state.selected_entries.is_empty() || !selected_releases.is_empty())
        && all_releases_scrapeable;
    let scrape = ui
        .add_enabled(
            can_scrape && !archive_busy,
            egui::Button::new("Scrape only missing artwork"),
        )
        .on_disabled_hover_text(if archive_busy {
            "Archive work is queued or running"
        } else if details_ready {
            "One or more archived releases has no reliable scraper identity"
        } else {
            "Loading the complete selection…"
        });
    if scrape.clicked() {
        crate::backend::assets::scrape_missing_artwork_for_selection(app, console_idx, ui.ctx());
    }

    ui.horizontal_wrapped(|ui| {
        let has_playable_entries = !app.ui_state.selected_entries.is_empty();
        if ui
            .add_enabled(
                details_ready && has_playable_entries && !archive_busy,
                egui::Button::new("Calculate missing hashes"),
            )
            .clicked()
        {
            crate::backend::hash::compute_hashes_for_selection(app, console_idx);
        }
        if ui
            .add_enabled(
                details_ready && has_playable_entries && !archive_busy,
                egui::Button::new("Rescan"),
            )
            .clicked()
        {
            crate::backend::scan::rescan_selected_entries(app, console_idx, ui.ctx());
        }
    });

    let archive_actions = selected_releases
        .iter()
        .filter_map(|release| release.action.clone())
        .collect::<Vec<_>>();
    if !selected_releases.is_empty() {
        let all_have_actions = archive_actions.len() == selected_releases.len();
        let all_ready = all_have_actions
            && archive_actions.iter().all(|action| {
                action.buildable && (!action.needs_playable || action.preferred_format.is_some())
            });
        let any_needs_playable = archive_actions.iter().any(|action| action.needs_playable);
        let any_needs_verification = archive_actions.iter().any(|action| {
            action
                .carriers
                .iter()
                .any(|carrier| !carrier.catalog_verified)
        });
        let label = if !any_needs_playable {
            "Verify selected archives"
        } else if any_needs_verification {
            "Verify & make selected playable"
        } else {
            "Make selected playable"
        };
        let button = ui
            .add_enabled(all_ready && !archive_busy, egui::Button::new(label))
            .on_disabled_hover_text(if archive_busy {
                "Archive work is queued or running"
            } else {
                "Every selected archive must be complete and have a preferred playable format"
            });
        if button.clicked() {
            for action in archive_actions {
                let format = action
                    .preferred_format
                    .as_deref()
                    .and_then(parse_playable_format)
                    .or_else(|| {
                        (!action.needs_playable)
                            .then_some(retro_junk_archive::RepresentationFormat::Rom)
                    });
                if let Some(format) = format {
                    crate::backend::playable_build::start(
                        app,
                        action,
                        &format,
                        folder_name.clone(),
                        ui.ctx(),
                    );
                }
            }
        }
    }
}

fn common_value<T: PartialEq>(values: &[T]) -> Option<&T> {
    let first = values.first()?;
    values.iter().all(|value| value == first).then_some(first)
}

fn show_archive_release(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    let Some(release_id) = app.ui_state.focused_archive_release.as_deref() else {
        return;
    };
    let release = app
        .browser
        .active_page
        .as_ref()
        .and_then(|page| {
            page.archived_releases
                .iter()
                .find(|row| row.summary.archive_release_id == release_id)
        })
        .cloned();
    let Some(release) = release else {
        ui.label("This archival release is no longer in the active Library view.");
        return;
    };
    let grouped_entry_id = release
        .playable_library_entries
        .first()
        .map(|entry| entry.id);
    if let (Some(console_idx), Some(entry_id)) = (app.selected_console_index(), grouped_entry_id) {
        ensure_entry_assets(app, console_idx, entry_id, ui.ctx());
    }
    let grouped_media = grouped_entry_id.and_then(|entry_id| {
        let console_idx = app.selected_console_index()?;
        app.browser.consoles[console_idx]
            .entry_by_id(entry_id)?
            .asset_paths
            .clone()
    });
    let summary = &release.summary;
    let mut requested_build = None;
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.label(egui::RichText::new(&summary.title).strong().size(18.0));
        ui.add_space(4.0);
        detail_row(ui, "Region", nonempty_or(&summary.region, "Unknown"));
        if !summary.revision.is_empty() {
            detail_row(ui, "Revision", &summary.revision);
        }
        detail_row(ui, "Platform", &summary.platform_id);
        if let Some(catalog_id) = summary.catalog_release_id.as_deref() {
            detail_row(ui, "Catalog release", catalog_id);
        }

        ui.add_space(8.0);
        ui.separator();
        ui.label(egui::RichText::new("Archive").strong());
        let archive_state = if summary.archive_complete {
            "Complete and catalog-verified"
        } else if summary.expected_disc_count == 0 {
            "Not catalog-bound; completeness is unknown"
        } else {
            "Incomplete"
        };
        detail_row(ui, "State", archive_state);
        detail_row(
            ui,
            "Verified discs",
            &format!(
                "{} / {}",
                summary.verified_disc_count, summary.expected_disc_count
            ),
        );
        detail_row(
            ui,
            "Physical copies",
            &summary.physical_copy_count.to_string(),
        );

        ui.add_space(8.0);
        ui.separator();
        ui.label(egui::RichText::new("Playable").strong());
        detail_row(
            ui,
            "Present",
            &format!(
                "{} / {} indexed representation(s)",
                summary.playable_present_count, summary.playable_count
            ),
        );
        if !release.playable_library_entries.is_empty() {
            let noun = if release.playable_library_entries.len() == 1 {
                "Library file"
            } else {
                "Library files"
            };
            detail_row(
                ui,
                noun,
                &format!(
                    "{} grouped with this release",
                    release.playable_library_entries.len()
                ),
            );
            for entry in &release.playable_library_entries {
                detail_row(
                    ui,
                    "Playable",
                    &format!(
                        "{} ({})",
                        entry.display_name,
                        entry.playable_format.to_ascii_uppercase().replace('_', "-")
                    ),
                );
            }
        }
        for (index, representation) in release.playable_representations.iter().enumerate() {
            detail_row(
                ui,
                &format!("Derived file {}", index + 1),
                &format!(
                    "{} ({})",
                    representation.relative_path,
                    representation.format.to_ascii_uppercase().replace('_', "-")
                ),
            );
        }
        if summary.desired_playable_count > 0 {
            detail_row(
                ui,
                "Preferred policy",
                &format!(
                    "{} / {} disc(s) satisfied",
                    summary.satisfied_playable_count, summary.desired_playable_count
                ),
            );
        } else {
            detail_row(ui, "Preferred policy", "Not configured");
        }

        ui.add_space(10.0);
        if !release.playable_library_entries.is_empty() {
            ui.weak(format!(
                "File actions apply to {} grouped playable file(s).",
                release.playable_library_entries.len()
            ));
            if let Some(console_idx) = app.selected_console_index() {
                show_file_actions(ui, app, console_idx, false);
            }
            ui.add_space(6.0);
        } else if let Some(console_idx) = app.selected_console_index() {
            let scrape = ui
                .add_enabled(
                    release.scrape_identity.is_some(),
                    egui::Button::new("Scrape Media"),
                )
                .on_disabled_hover_text(
                    "This archive release is not catalog-bound, so it has no reliable scraper identity",
                );
            if scrape.clicked() {
                crate::backend::assets::scrape_missing_media_for_selection(
                    app,
                    console_idx,
                    ui.ctx(),
                );
            }
            ui.weak("Scraped originals will be stored in the archive.");
            ui.add_space(6.0);
        }
        if let Some(media) = grouped_media.as_ref() {
            show_media(ui, media);
        }
        show_archived_media(ui, &release.archived_assets);
        if !release.archived_assets.is_empty() {
            let has_frontend_target = !release.playable_library_entries.is_empty()
                || !release.playable_representations.is_empty();
            let restore = ui
                .add_enabled(
                    has_frontend_target,
                    egui::Button::new("Restore archived media files"),
                )
                .on_hover_text(
                    "Rebuild frontend artwork and video files without contacting ScreenScraper",
                )
                .on_disabled_hover_text("Make or adopt a playable representation first");
            if restore.clicked() {
                let frontend_stems = release
                    .playable_library_entries
                    .iter()
                    .filter_map(|entry| {
                        let name = entry.display_name.as_str();
                        if name.to_ascii_lowercase().ends_with(".m3u") {
                            Some(name.to_owned())
                        } else {
                            std::path::Path::new(name)
                                .file_stem()
                                .and_then(|value| value.to_str())
                                .map(str::to_owned)
                        }
                    })
                    .collect();
            let folder_name = app.selected_console_index().map_or_else(
                || summary.platform_id.clone(),
                |index| app.browser.consoles[index].folder_name.clone(),
            );
            crate::backend::assets::restore_archived_media_for_release(
                app,
                summary.archive_release_id.clone(),
                folder_name,
                    frontend_stems,
            );
            }
        }
        if let Some(action) = release.action.as_ref() {
            let needs_verification = action
                .carriers
                .iter()
                .any(|carrier| !carrier.catalog_verified);
            let label = archive_action_label(action, needs_verification);
            let ready =
                action.buildable && (!action.needs_playable || action.preferred_format.is_some());
            let archive_busy = app
                .operations
                .iter()
                .any(|operation| operation.scope == "archive");
            let button = ui.add_enabled(ready && !archive_busy, egui::Button::new(label));
            let button = if action.needs_playable && action.preferred_format.is_none() {
                button.on_disabled_hover_text(
                    "Choose a preferred playable format for this console first",
                )
            } else if action.archived_disc_count < action.expected_disc_count {
                button.on_disabled_hover_text(format!(
                    "Archive is incomplete: {}/{} expected discs are present",
                    action.archived_disc_count, action.expected_disc_count
                ))
            } else if archive_busy {
                button.on_disabled_hover_text("Archive work is queued or running")
            } else if !action.buildable {
                button.on_disabled_hover_text(
                    "One or more archived discs has no supported in-app conversion path",
                )
            } else {
                button
            };
            if button.clicked() {
                let format = action
                    .preferred_format
                    .as_deref()
                    .and_then(parse_playable_format)
                    .or_else(|| {
                        (!action.needs_playable)
                            .then_some(retro_junk_archive::RepresentationFormat::Rom)
                    });
                if let Some(format) = format {
                    requested_build = Some((action.clone(), format));
                }
            }
        } else {
            ui.weak("No archive or playable action is currently needed.");
        }
    });
    if let Some((action, format)) = requested_build {
        let playable_platform_id = app
            .selected_console_index()
            .map(|index| app.browser.consoles[index].folder_name.clone())
            .unwrap_or_else(|| summary.platform_id.clone());
        crate::backend::playable_build::start(app, action, &format, playable_platform_id, ui.ctx());
    }
}

fn show_file_actions(
    ui: &mut egui::Ui,
    app: &mut RetroJunkApp,
    console_idx: usize,
    allow_rename: bool,
) {
    let has_selection = !app.ui_state.selected_entries.is_empty();
    let details_ready = has_selection && app.selected_entry_details_loaded();
    let disabled_reason = if has_selection {
        "Loading the playable file details…"
    } else {
        "No playable file is grouped with this row"
    };
    ui.horizontal_wrapped(|ui| {
        let scrape = ui
            .add_enabled(details_ready, egui::Button::new("Scrape Media"))
            .on_disabled_hover_text(disabled_reason);
        if scrape.clicked() {
            crate::backend::assets::scrape_missing_media_for_selection(app, console_idx, ui.ctx());
        }
        let hashes = ui
            .add_enabled(details_ready, egui::Button::new("Calculate Hashes"))
            .on_disabled_hover_text(disabled_reason);
        if hashes.clicked() {
            crate::backend::hash::compute_hashes_for_selection(app, console_idx);
        }
        let rescan = ui
            .add_enabled(details_ready, egui::Button::new("Rescan"))
            .on_disabled_hover_text(disabled_reason);
        if rescan.clicked() {
            crate::backend::scan::rescan_selected_entries(app, console_idx, ui.ctx());
        }
        if allow_rename {
            let rename = ui
                .add_enabled(details_ready, egui::Button::new("Auto Rename"))
                .on_disabled_hover_text(disabled_reason);
            if rename.clicked() {
                crate::backend::rename::rename_selected_entries(app, console_idx, ui.ctx());
            }
        }
    });
}

fn ensure_entry_assets(
    app: &mut RetroJunkApp,
    console_idx: usize,
    entry_id: retro_junk_db::LibraryEntryId,
    ctx: &egui::Context,
) {
    let Some(entry_idx) = app.browser.consoles[console_idx].entry_index(entry_id) else {
        return;
    };
    if app.browser.consoles[console_idx].entries[entry_idx]
        .asset_paths
        .is_some()
        || app.browser.asset_discovery_in_flight.contains(&entry_id)
    {
        return;
    }

    // Empty is the in-flight sentinel; image bytes remain lazy in egui's file
    // loader and are never retained by this projection.
    app.browser.consoles[console_idx].entries[entry_idx].asset_paths =
        Some(std::collections::HashMap::new());
    app.browser.asset_discovery_in_flight.insert(entry_id);

    let Some(root_path) = app.root_path.clone() else {
        return;
    };
    let folder_name = app.browser.consoles[console_idx].folder_name.clone();
    let entry = &app.browser.consoles[console_idx].entries[entry_idx];
    crate::backend::assets::load_assets_for_entry(
        app.message_tx.clone(),
        ctx.clone(),
        root_path,
        folder_name,
        entry_id,
        entry.game_entry.display_name().to_string(),
        entry.game_entry.rom_stem().to_owned(),
        app.settings.general.assets_dir.clone(),
    );
}

fn show_media(
    ui: &mut egui::Ui,
    media: &std::collections::HashMap<retro_junk_frontend::AssetType, std::path::PathBuf>,
) {
    if media.is_empty() {
        return;
    }
    ui.add_space(4.0);
    ui.separator();
    ui.label(egui::RichText::new("Media").strong());
    ui.add_space(2.0);

    let panel_width = ui.available_width();
    for &asset_type in DISPLAY_ASSET_TYPES {
        if let Some(path) = media.get(&asset_type) {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(asset_type.to_string()).weak());
            let uri = crate::state::asset_image_uri(path);
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

fn show_archived_media(ui: &mut egui::Ui, assets: &[retro_junk_db::ArchivedReleaseAsset]) {
    if assets.is_empty() {
        return;
    }
    ui.add_space(4.0);
    ui.separator();
    ui.label(egui::RichText::new("Archived Media").strong());
    ui.weak("Release-level source files stored in the authoritative archive.");
    ui.add_space(2.0);

    let panel_width = ui.available_width();
    for asset in assets {
        ui.add_space(4.0);
        let label = if asset.source.is_empty() {
            asset.asset_type.clone()
        } else {
            format!("{} · {}", asset.asset_type, asset.source)
        };
        ui.label(egui::RichText::new(label).weak());
        let path = std::path::Path::new(&asset.absolute_path);
        let uri = crate::state::asset_image_uri(path);
        let image = egui::Image::new(uri)
            .fit_to_exact_size(egui::vec2(panel_width, panel_width))
            .maintain_aspect_ratio(true)
            .corner_radius(4.0);
        ui.add(image).on_hover_text(&asset.absolute_path);
    }
}

fn nonempty_or<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.is_empty() { fallback } else { value }
}

fn archive_action_label(
    action: &retro_junk_db::ArchivedPlayableGap,
    needs_verification: bool,
) -> &'static str {
    if action.needs_playlist && !action.needs_playable && needs_verification {
        "Verify & create playlist"
    } else if action.needs_playlist && !action.needs_playable {
        "Create multi-disc playlist"
    } else if !action.needs_playable {
        "Verify archive"
    } else if needs_verification && !action.allow_unverified {
        "Verify & make playable"
    } else if needs_verification {
        "Make playable (unverified)"
    } else {
        "Make playable"
    }
}

fn parse_playable_format(value: &str) -> Option<retro_junk_archive::RepresentationFormat> {
    match value {
        "rom" => Some(retro_junk_archive::RepresentationFormat::Rom),
        "chd" => Some(retro_junk_archive::RepresentationFormat::Chd),
        "rvz" => Some(retro_junk_archive::RepresentationFormat::Rvz),
        "iso" => Some(retro_junk_archive::RepresentationFormat::Iso),
        "cue_bin" | "cue-bin" => Some(retro_junk_archive::RepresentationFormat::CueBin),
        _ => None,
    }
}

fn disc_verification_label(verification: DiscVerification) -> &'static str {
    match verification {
        DiscVerification::NotApplicable => "Not applicable",
        DiscVerification::Complete => "Complete track set verified",
        DiscVerification::Incomplete => "Incomplete or mismatched track set",
        DiscVerification::InvalidLayout => "Invalid CUE layout",
    }
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
