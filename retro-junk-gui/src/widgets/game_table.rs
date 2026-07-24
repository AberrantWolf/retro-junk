use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::PathBuf;

use egui_extras::{Column, TableBuilder};
use retro_junk_lib::Region;

use crate::app::RetroJunkApp;
use crate::backend;
use crate::state::{AssetStatus, EntryStatus, FocusedPanel, TagDialog};
use crate::util;
use crate::widgets::keyboard_nav;
use crate::widgets::status_badge;

/// Render the sortable, filterable game table for the selected console.
pub fn show(ui: &mut egui::Ui, app: &mut RetroJunkApp, ctx: &egui::Context) {
    let Some(console_idx) = app.selected_console_index() else {
        return;
    };

    let console = &app.browser.consoles[console_idx];
    let Some(console_id) = console.id else { return };
    let list_columns = app.context.get_by_platform(console.platform).map_or_else(
        EntryListColumns::default,
        |registered| EntryListColumns {
            serial: registered.metadata.identification.serial,
            internal_name: registered.metadata.identification.internal_name,
            region: registered.metadata.identification.region,
            dat_match: registered.analyzer.has_dat_support(),
        },
    );
    let Some(page) = app
        .browser
        .active_page
        .as_ref()
        .filter(|page| page.console_id == console_id)
    else {
        ui.label("Loading library entries…");
        return;
    };
    let visible_ids: Vec<_> = page
        .rows
        .iter()
        .filter(|row| row.archive_release_id.is_none())
        .map(|row| row.id)
        .collect();

    // Cmd+A / Ctrl+A: select all visible entries
    if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::A)) {
        app.ui_state.selected_entries = visible_ids.iter().copied().collect();
    }

    // Keyboard navigation (only when game table has focus)
    if app.ui_state.focused_panel == FocusedPanel::GameTable {
        let current_filtered_pos = visible_ids
            .iter()
            .position(|id| Some(*id) == app.ui_state.focused_entry);

        let text_height_estimate = egui::TextStyle::Body
            .resolve(ui.style())
            .size
            .max(ui.spacing().interact_size.y);
        let page_size = (ui.available_height() / text_height_estimate).max(1.0) as usize;

        if let Some(action) =
            keyboard_nav::process_list_nav(ui, current_filtered_pos, visible_ids.len(), page_size)
        {
            let entry_id = visible_ids[action.new_index];

            if action.extend_selection {
                // Shift: extend selection from focused to new position
                if let Some(focused_pos) = current_filtered_pos {
                    let start = focused_pos.min(action.new_index);
                    let end = focused_pos.max(action.new_index);
                    for &id in &visible_ids[start..=end] {
                        app.ui_state.selected_entries.insert(id);
                    }
                } else {
                    app.ui_state.selected_entries.insert(entry_id);
                }
            } else {
                app.ui_state.selected_entries.clear();
                app.ui_state.selected_entries.insert(entry_id);
            }

            app.ui_state.focused_entry = Some(entry_id);
            app.ui_state.scroll_to_row = Some(action.new_index);
        }
    }

    // Status summary
    let total = page.logical_count;
    let matched = page.counts.matched;
    let likely = page.counts.likely;
    let ambiguous = page.counts.ambiguous;
    let unrecognized = page.counts.unrecognized;
    let tagged = page.counts.tagged;
    let archive_release_count = page.archived_releases.len();

    // Rich rows are sparse (focus/selection only). Index them once so a large
    // selected page does not turn projection building into O(page × details).
    let entry_lookup: HashMap<_, _> = console
        .entries
        .iter()
        .filter_map(|entry| entry.id.map(|id| (id, entry)))
        .collect();

    // Pre-extract row data to avoid borrowing issues
    let mut row_data: Vec<RowData> = page
        .archived_releases
        .iter()
        .map(|release| {
            let summary = &release.summary;
            RowData {
                entry_id: None,
                archive_release_id: Some(summary.archive_release_id.clone()),
                status: if summary.archive_complete {
                    EntryStatus::Matched
                } else {
                    EntryStatus::Unknown
                },
                has_broken_refs: false,
                has_hash_warnings: false,
                has_cue_compat_issues: false,
                asset_status: AssetStatus::Unknown,
                name: summary.title.clone(),
                file_path: None,
                serial: String::new(),
                internal_name: String::new(),
                regions: region_code(&summary.region),
                crc32: String::new(),
                dat_match: summary.catalog_release_id.clone().unwrap_or_default(),
                availability: AvailabilityState::from_archive(release),
            }
        })
        .collect();
    row_data.extend(
        page.rows
            .iter()
            // A catalog-bound playable entry is one representation of the logical
            // archive release row above, not a second game in the user's Library.
            .filter(|projection| projection.archive_release_id.is_none())
            .map(|projection| {
                let entry = entry_lookup.get(&projection.id).copied();
                RowData {
                    entry_id: Some(projection.id),
                    archive_release_id: None,
                    status: projection_status(&projection.status, &projection.tag),
                    has_broken_refs: projection.has_broken_references,
                    has_hash_warnings: projection.has_hash_warnings,
                    has_cue_compat_issues: projection.has_cue_compat_issues,
                    asset_status: app
                        .browser
                        .asset_statuses
                        .get(&projection.id)
                        .copied()
                        .unwrap_or(AssetStatus::Unknown),
                    name: projection.display_name.clone(),
                    file_path: entry.map(|entry| entry.game_entry.analysis_path().to_path_buf()),
                    serial: projection.serial.clone(),
                    internal_name: projection.internal_name.clone(),
                    regions: projected_regions(projection),
                    crc32: projection.crc32.clone(),
                    dat_match: projection.dat_game_name.clone(),
                    availability: AvailabilityState::from_projection(projection),
                }
            }),
    );
    row_data.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
            .then_with(|| left.regions.cmp(&right.regions))
    });

    ui.horizontal(|ui| {
        let mut summary = format!(
            "{total} games | {archive_release_count} archived releases | {matched} playable files verified | {likely} likely | {ambiguous} ambiguous | {unrecognized} unrecognized"
        );
        if tagged > 0 {
            let _ = write!(summary, " | {tagged} tagged");
        }
        let _ = write!(summary, " | showing {} logical rows", row_data.len());
        ui.label(summary);
    });

    ui.add_space(2.0);

    // Table wrapped in horizontal scroll area
    let text_height = egui::TextStyle::Body
        .resolve(ui.style())
        .size
        .max(ui.spacing().interact_size.y);
    // Keep header height in one place so the body height calculation stays in sync.
    let header_height = 20.0_f32;

    egui::ScrollArea::horizontal()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Measure available height *inside* the scroll area: egui has already
            // reduced it by the horizontal scrollbar thickness (when visible), so
            // we only need to subtract the table header to get the body height.
            let body_height = (ui.available_height() - header_height).max(0.0);

            let mut table = TableBuilder::new(ui)
                .striped(true)
                .resizable(true)
                .sense(egui::Sense::click())
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::exact(30.0)) // Status badge + media indicator
                .column(Column::initial(280.0).at_least(100.0)) // Name
                .column(Column::initial(145.0).at_least(90.0)); // Availability
            if list_columns.serial {
                table = table.column(Column::initial(120.0).at_least(60.0));
            }
            if list_columns.internal_name {
                table = table.column(Column::initial(140.0).at_least(60.0));
            }
            if list_columns.region {
                table = table.column(Column::initial(80.0).at_least(40.0));
            }
            table = table.column(Column::initial(80.0).at_least(60.0)); // CRC32
            if list_columns.dat_match {
                table = table.column(Column::initial(200.0).at_least(80.0));
            }
            table = table
                .min_scrolled_height(0.0)
                .max_scroll_height(body_height);

            table
                .header(header_height, |mut header| {
                    header.col(|ui| {
                        ui.strong("");
                    });
                    header.col(|ui| {
                        ui.strong("Name");
                    });
                    header.col(|ui| {
                        ui.strong("Availability");
                    });
                    if list_columns.serial {
                        header.col(|ui| {
                            ui.strong("Serial");
                        });
                    }
                    if list_columns.internal_name {
                        header.col(|ui| {
                            ui.strong("Internal Name");
                        });
                    }
                    if list_columns.region {
                        header.col(|ui| {
                            ui.strong("Region");
                        });
                    }
                    header.col(|ui| {
                        ui.strong("CRC32");
                    });
                    if list_columns.dat_match {
                        header.col(|ui| {
                            ui.strong("DAT Match");
                        });
                    }
                })
                .body(|body| {
                    body.rows(text_height, row_data.len(), |mut row| {
                        let row_idx = row.index();
                        let data = &row_data[row_idx];
                        let is_selected = data
                            .entry_id
                            .is_some_and(|id| app.ui_state.selected_entries.contains(&id));
                        let is_focused = data
                            .entry_id
                            .is_some_and(|id| app.ui_state.focused_entry == Some(id))
                            || data.archive_release_id.as_ref()
                                == app.ui_state.focused_archive_release.as_ref();

                        // Highlight the entire row
                        row.set_selected(is_selected || is_focused);

                        // Status badge with tooltip (includes warning triangle and media indicator)
                        let r1 = row.col(|ui| {
                            let resp = status_badge::show_with_warning(
                                ui,
                                data.status,
                                data.has_broken_refs,
                                data.has_hash_warnings,
                                data.has_cue_compat_issues,
                                data.asset_status,
                            );
                            let mut tip = data.status.tooltip().to_string();
                            if data.has_broken_refs {
                                tip.push_str("\n\u{26a0} Broken file references");
                            }
                            if data.has_hash_warnings {
                                tip.push_str("\n\u{26a0} Hash warnings (see detail panel)");
                            }
                            if data.has_cue_compat_issues {
                                tip.push_str("\n\u{26a0} Non-standard CUE sheet format");
                            }
                            match data.asset_status {
                                AssetStatus::Unknown => {}
                                AssetStatus::None => tip.push_str("\nNo scraped media"),
                                AssetStatus::Partial { found, total } => {
                                    let _ = write!(tip, "\nPartial media ({found}/{total})");
                                }
                                AssetStatus::Complete => tip.push_str("\nMedia complete"),
                            }
                            resp.on_hover_text(tip);
                        });

                        // Paint text directly in cells so no WidgetRect is created,
                        // allowing the cell response to handle all click interaction.
                        let name_response = row.col(|ui| paint_cell_text(ui, &data.name));
                        let availability_response = row.col(|ui| {
                            paint_cell_text(ui, data.availability.label());
                        });
                        let availability_response = (
                            availability_response.0,
                            availability_response
                                .1
                                .on_hover_text(data.availability.tooltip()),
                        );

                        // Scroll-to-row: when keyboard navigation targets this row
                        if app.ui_state.scroll_to_row == Some(row_idx) {
                            r1.1.scroll_to_me(Some(egui::Align::Center));
                        }

                        // Union all column responses for click and context menu
                        let mut row_resp = r1.1 | name_response.1 | availability_response.1;
                        if list_columns.serial {
                            row_resp |= row.col(|ui| paint_cell_text(ui, &data.serial)).1;
                        }
                        if list_columns.internal_name {
                            row_resp |= row.col(|ui| paint_cell_text(ui, &data.internal_name)).1;
                        }
                        if list_columns.region {
                            row_resp |= row.col(|ui| paint_cell_text(ui, &data.regions)).1;
                        }
                        row_resp |= row.col(|ui| paint_cell_text(ui, &data.crc32)).1;
                        if list_columns.dat_match {
                            row_resp |= row.col(|ui| paint_cell_text(ui, &data.dat_match)).1;
                        }

                        // Right-click selection: if right-clicked row isn't selected, select just it
                        if row_resp.secondary_clicked() {
                            if let Some(entry_id) = data.entry_id {
                                if !app.ui_state.selected_entries.contains(&entry_id) {
                                    app.ui_state.selected_entries.clear();
                                    app.ui_state.selected_entries.insert(entry_id);
                                }
                                app.ui_state.focused_entry = Some(entry_id);
                                app.ui_state.focused_archive_release = None;
                                app.request_entry_detail(entry_id, ctx);
                            }
                        }

                        // Left-click
                        if row_resp.clicked() {
                            if let Some(entry_id) = data.entry_id {
                                let modifiers = ctx.input(|i| i.modifiers);
                                handle_row_click(app, entry_id, modifiers, &visible_ids);
                                app.ui_state.focused_archive_release = None;
                                app.request_entry_detail(entry_id, ctx);
                            } else if let Some(release_id) = data.archive_release_id.as_ref() {
                                app.ui_state.selected_entries.clear();
                                app.ui_state.focused_entry = None;
                                app.ui_state.focused_archive_release = Some(release_id.clone());
                            }
                            app.ui_state.focused_panel = FocusedPanel::GameTable;
                        }

                        // Context menu on unioned response
                        if data.entry_id.is_some() {
                            row_resp.context_menu(|ui| {
                                show_row_context_menu(ui, app, ctx, console_idx, data);
                            });
                        }
                    });
                });
        });

    // Clear scroll-to-row after rendering
    app.ui_state.scroll_to_row = None;
    app.request_selected_entry_details(ctx);
}

/// Render the context menu for a game table row.
fn show_row_context_menu(
    ui: &mut egui::Ui,
    app: &mut RetroJunkApp,
    ctx: &egui::Context,
    console_idx: usize,
    data: &RowData,
) {
    if !app.selected_entry_details_loaded() {
        ui.spinner();
        ui.label("Loading the complete selection…");
        return;
    }
    let Some(_entry_id) = data.entry_id else {
        return;
    };
    let Some(file_path) = data.file_path.as_ref() else {
        ui.label("Loading entry details…");
        return;
    };
    if ui.button("Rescan").clicked() {
        backend::scan::rescan_selected_entries(app, console_idx, ctx);
        ui.close();
    }

    if ui.button("Calculate Missing Hashes").clicked() {
        backend::hash::compute_hashes_for_selection(app, console_idx);
        ui.close();
    }
    if ui.button("Recalculate Hashes").clicked() {
        backend::hash::recompute_hashes_for_selection(app, console_idx);
        ui.close();
    }

    // Adaptive scrape menu items based on aggregate media state
    {
        let console = &app.browser.consoles[console_idx];
        let mut any_has_media = false;
        let mut all_have_all_media = true;
        let mut any_has_miximage = false;
        for &i in &app.ui_state.selected_entries {
            if console.entry_by_id(i).is_some() {
                let ms = app
                    .browser
                    .asset_statuses
                    .get(&i)
                    .copied()
                    .unwrap_or(AssetStatus::Unknown);
                match ms {
                    AssetStatus::Complete => {
                        any_has_media = true;
                    }
                    AssetStatus::Partial { .. } => {
                        any_has_media = true;
                        all_have_all_media = false;
                    }
                    _ => {
                        all_have_all_media = false;
                    }
                }
                if app.browser.entries_with_miximages.contains(&i) {
                    any_has_miximage = true;
                }
            }
        }

        if any_has_media && all_have_all_media {
            // All entries have complete media
            if ui.button("Re-scrape Media").clicked() {
                backend::assets::rescrape_media_for_selection(app, console_idx, ctx);
                ui.close();
            }
        } else if any_has_media {
            // Some entries have partial media
            if ui.button("Scrape All Media").clicked() {
                backend::assets::rescrape_media_for_selection(app, console_idx, ctx);
                ui.close();
            }
            if ui.button("Scrape Missing Media").clicked() {
                backend::assets::scrape_missing_media_for_selection(app, console_idx, ctx);
                ui.close();
            }
        } else {
            // No entry has any media
            if ui.button("Scrape Media").clicked() {
                backend::assets::rescrape_media_for_selection(app, console_idx, ctx);
                ui.close();
            }
        }

        // Miximage menu items (hidden when no media at all — scraping will auto-generate)
        if any_has_media {
            let label = if any_has_miximage {
                "Re-generate Miximages"
            } else {
                "Generate Miximages"
            };
            if ui.button(label).clicked() {
                backend::assets::regenerate_miximages_for_selection(app, console_idx, ctx);
                ui.close();
            }
        }
    }

    if ui.button("Auto Rename").clicked() {
        backend::rename::rename_selected_entries(app, console_idx, ctx);
        ui.close();
    }

    // Fix CUE Sheet — only show when at least one selected entry has CUE compat issues
    {
        let console = &app.browser.consoles[console_idx];
        let has_cue_issues = app.ui_state.selected_entries.iter().any(|&i| {
            console
                .entry_by_id(i)
                .is_some_and(super::super::state::LibraryEntry::has_cue_compat_issues)
        });
        if has_cue_issues && ui.button("Fix CUE Sheet").clicked() {
            backend::fix_cue::fix_cue_for_selection(app, console_idx, ctx);
            ui.close();
        }
    }

    // Compress to CHD — only for consoles whose analyzer supports it. The
    // dialog itself explains per-disc skips and a missing chdman. Gated
    // (advisory) while a compression is already running for this console.
    if backend::chd_compress::console_supports_chd(app, console_idx) {
        let busy = app.chd_compress_busy(&app.browser.consoles[console_idx].folder_name);
        let button = ui
            .add_enabled(!busy, egui::Button::new("Compress to CHD…"))
            .on_disabled_hover_text("A CHD compression is already running for this console");
        if button.clicked() {
            let selection = app.selected_entry_indices();
            backend::chd_compress::open_compress_dialog(app, console_idx, &selection);
            ui.close();
        }
    }

    // Set Region submenu
    show_set_region_submenu(ui, app, console_idx);

    // Tag menu items (only for single selection)
    if app.ui_state.selected_entries.len() == 1 {
        let entry_id = *app.ui_state.selected_entries.iter().next().unwrap();
        show_tag_menu_items(ui, app, console_idx, entry_id);
    }

    ui.separator();

    if ui.button(util::REVEAL_LABEL).clicked() {
        util::reveal_in_file_manager(file_path);
        ui.close();
    }

    ui.separator();

    if ui.button("Copy File Path").clicked() {
        let paths = collect_selected_field(app, console_idx, |entry| {
            Some(entry.game_entry.analysis_path().display().to_string())
        });
        crate::util::copy_and_close(ui, paths);
    }

    let has_serial = !data.serial.is_empty()
        || app.ui_state.selected_entries.iter().any(|&i| {
            app.browser.consoles[console_idx]
                .entry_by_id(i)
                .and_then(|e| e.identification.as_ref())
                .is_some_and(|id| !id.serial_number.is_empty())
        });
    if ui
        .add_enabled(has_serial, egui::Button::new("Copy Serial"))
        .clicked()
    {
        let serials = collect_selected_field(app, console_idx, |entry| {
            entry
                .identification
                .as_ref()
                .filter(|id| !id.serial_number.is_empty())
                .map(|id| id.serial_number.clone())
        });
        crate::util::copy_and_close(ui, serials);
    }

    let has_crc32 = !data.crc32.is_empty()
        || app.ui_state.selected_entries.iter().any(|&i| {
            app.browser.consoles[console_idx]
                .entry_by_id(i)
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
        crate::util::copy_and_close(ui, crcs);
    }

    let has_dat = !data.dat_match.is_empty()
        || app.ui_state.selected_entries.iter().any(|&i| {
            app.browser.consoles[console_idx]
                .entry_by_id(i)
                .and_then(|e| e.dat_match.as_ref())
                .is_some()
        });
    if ui
        .add_enabled(has_dat, egui::Button::new("Copy DAT Name"))
        .clicked()
    {
        let dat_names = collect_selected_field(app, console_idx, |entry| {
            entry.dat_match.as_ref().map(|dm| dm.game_name.clone())
        });
        crate::util::copy_and_close(ui, dat_names);
    }
}

/// Render the "Set Region" submenu with recommended and other regions.
fn show_set_region_submenu(ui: &mut egui::Ui, app: &mut RetroJunkApp, console_idx: usize) {
    // Pre-compute the intersection of detected regions across selected entries.
    // Entries with no detected regions are excluded from the intersection so they
    // don't narrow the recommendations to empty.
    let console = &app.browser.consoles[console_idx];
    let mut recommended: Option<HashSet<Region>> = None;
    for &i in &app.ui_state.selected_entries {
        if let Some(entry) = console.entry_by_id(i)
            && let Some(ref id) = entry.identification
            && !id.regions.is_empty()
        {
            let set: HashSet<Region> = id.regions.iter().copied().collect();
            recommended = Some(match recommended {
                Some(acc) => acc.intersection(&set).copied().collect(),
                None => set,
            });
        }
    }
    let recommended = recommended.unwrap_or_default();

    ui.menu_button("Set Region", |ui| {
        if ui.button("Auto-detect").clicked() {
            let ids: Vec<_> = app.ui_state.selected_entries.iter().copied().collect();
            app.set_entry_regions(ids, None, ui.ctx());
            ui.close();
        }

        if !recommended.is_empty() {
            ui.separator();
            ui.label("Recommended");
            for &region in Region::ALL {
                if recommended.contains(&region) && ui.button(region.name()).clicked() {
                    let ids: Vec<_> = app.ui_state.selected_entries.iter().copied().collect();
                    app.set_entry_regions(ids, Some(region), ui.ctx());
                    ui.close();
                }
            }
        }

        ui.separator();
        ui.label("Other Regions");
        for &region in Region::ALL {
            if !recommended.contains(&region) && ui.button(region.name()).clicked() {
                let ids: Vec<_> = app.ui_state.selected_entries.iter().copied().collect();
                app.set_entry_regions(ids, Some(region), ui.ctx());
                ui.close();
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
    let console = &app.browser.consoles[console_idx];
    let mut values: Vec<String> = app
        .ui_state
        .selected_entries
        .iter()
        .copied()
        .filter_map(|i| {
            let entry = console.entry_by_id(i)?;
            extractor(entry)
        })
        .collect();
    values.sort();
    values.join("\n")
}

/// Serial for a table row: the entry's own, falling back to the first
#[derive(Clone, Copy, Default)]
struct EntryListColumns {
    serial: bool,
    internal_name: bool,
    region: bool,
    dat_match: bool,
}

fn projected_regions(projection: &retro_junk_db::LibraryEntryListItem) -> String {
    if !projection.region_override.is_empty() {
        return format!("{}*", region_code(&projection.region_override));
    }
    projection
        .detected_regions
        .iter()
        .map(|region| region_code(region))
        .collect::<Vec<_>>()
        .join(", ")
}

fn region_code(region: &str) -> String {
    match region
        .to_ascii_lowercase()
        .replace([' ', '-'], "_")
        .as_str()
    {
        "japan" => "JPN".to_owned(),
        "usa" | "united_states" => "USA".to_owned(),
        "europe" => "EUR".to_owned(),
        "australia" => "AUS".to_owned(),
        "korea" => "KOR".to_owned(),
        "china" => "CHN".to_owned(),
        "taiwan" => "TWN".to_owned(),
        "asia" => "ASI".to_owned(),
        "brazil" => "BRA".to_owned(),
        "latinamerica" | "latin_america" => "LAT".to_owned(),
        "world" => "WLD".to_owned(),
        "unknown" => "UNK".to_owned(),
        _ => region.to_owned(),
    }
}

struct RowData {
    entry_id: Option<retro_junk_db::LibraryEntryId>,
    archive_release_id: Option<String>,
    status: EntryStatus,
    has_broken_refs: bool,
    has_hash_warnings: bool,
    has_cue_compat_issues: bool,
    asset_status: AssetStatus,
    name: String,
    file_path: Option<PathBuf>,
    serial: String,
    internal_name: String,
    regions: String,
    crc32: String,
    dat_match: String,
    availability: AvailabilityState,
}

#[derive(Clone)]
enum AvailabilityState {
    ArchivedOnly,
    IncompleteArchive,
    PlayableOnly { format: String },
    ArchivedAndPlayable { format: String },
    IncompleteArchiveAndPlayable { format: String },
    PreferredFormatMismatch { actual: String, preferred: String },
}

impl AvailabilityState {
    fn from_archive(release: &retro_junk_db::ArchivedLibraryListItem) -> Self {
        let summary = &release.summary;
        let playable = summary.playable_present_count > 0 || release.has_playable_library_entry;
        if summary.archive_complete {
            if release
                .action
                .as_ref()
                .is_some_and(|action| action.needs_playable)
                && playable
            {
                return Self::PreferredFormatMismatch {
                    actual: "another format".to_owned(),
                    preferred: "the configured format".to_owned(),
                };
            }
            if playable {
                Self::ArchivedAndPlayable {
                    format: "indexed representation".to_owned(),
                }
            } else {
                Self::ArchivedOnly
            }
        } else if playable {
            Self::IncompleteArchiveAndPlayable {
                format: "indexed representation".to_owned(),
            }
        } else {
            Self::IncompleteArchive
        }
    }

    fn from_projection(projection: &retro_junk_db::LibraryEntryListItem) -> Self {
        let format = display_format(&projection.playable_format);
        if !projection.archived {
            return Self::PlayableOnly { format };
        }
        if !projection.archive_complete {
            return Self::IncompleteArchiveAndPlayable { format };
        }
        if let Some(preferred) = projection.preferred_format.as_deref()
            && !formats_satisfy_policy(&projection.playable_format, preferred)
        {
            return Self::PreferredFormatMismatch {
                actual: format,
                preferred: display_format(preferred),
            };
        }
        Self::ArchivedAndPlayable { format }
    }

    fn label(&self) -> &str {
        match self {
            Self::ArchivedOnly => "Archived only",
            Self::IncompleteArchive => "Archive incomplete",
            Self::PlayableOnly { .. } => "Playable only",
            Self::ArchivedAndPlayable { .. } => "Archived + playable",
            Self::IncompleteArchiveAndPlayable { .. } => "Archive incomplete + playable",
            Self::PreferredFormatMismatch { .. } => "Wrong playable format",
        }
    }

    fn tooltip(&self) -> String {
        match self {
            Self::ArchivedOnly => {
                "The complete, verified release is archived but no playable copy is present"
                    .to_owned()
            }
            Self::IncompleteArchive => {
                "The archive does not yet contain every expected, catalog-verified disc".to_owned()
            }
            Self::PlayableOnly { format } => {
                format!("Playable as {format}, but no matching archival carrier is indexed")
            }
            Self::ArchivedAndPlayable { format } => {
                format!("Archived and playable as {format}")
            }
            Self::IncompleteArchiveAndPlayable { format } => {
                format!(
                    "Playable as {format}; the archive does not yet have every expected disc catalog-verified"
                )
            }
            Self::PreferredFormatMismatch { actual, preferred } => {
                format!("Playable as {actual}; the archive policy prefers {preferred}")
            }
        }
    }
}

fn formats_satisfy_policy(actual: &str, preferred: &str) -> bool {
    let actual = actual.to_ascii_lowercase().replace('-', "_");
    let preferred = preferred.to_ascii_lowercase().replace('-', "_");
    if actual == preferred {
        return true;
    }
    match preferred.as_str() {
        "cue_bin" => matches!(actual.as_str(), "cue" | "bin"),
        "rom" => !matches!(
            actual.as_str(),
            "chd" | "rvz" | "iso" | "cue" | "bin" | "gdi" | "cso" | "dax"
        ),
        _ => false,
    }
}

fn display_format(format: &str) -> String {
    if format.is_empty() {
        "unknown format".to_owned()
    } else {
        format.to_ascii_uppercase().replace('_', "-")
    }
}

fn projection_status(status: &str, tag: &str) -> EntryStatus {
    match tag {
        "homebrew" => EntryStatus::Tagged(retro_junk_catalog::CatalogTag::Homebrew),
        "modded" => EntryStatus::Tagged(retro_junk_catalog::CatalogTag::Modded),
        _ => match status {
            "matched" => EntryStatus::Matched,
            "likely" => EntryStatus::LikelyMatched,
            "ambiguous" => EntryStatus::Ambiguous,
            "unrecognized" => EntryStatus::Unrecognized,
            _ => EntryStatus::Unknown,
        },
    }
}

fn handle_row_click(
    app: &mut RetroJunkApp,
    entry_id: retro_junk_db::LibraryEntryId,
    modifiers: egui::Modifiers,
    visible_ids: &[retro_junk_db::LibraryEntryId],
) {
    if modifiers.ctrl || modifiers.command {
        // Toggle selection
        if app.ui_state.selected_entries.contains(&entry_id) {
            // Deselect: don't move focused_entry to this row — that would keep
            // it visually highlighted through the is_focused path.
            app.ui_state.selected_entries.remove(&entry_id);
            return;
        }
        app.ui_state.selected_entries.insert(entry_id);
    } else if modifiers.shift {
        // Range select
        if let Some(focused) = app.ui_state.focused_entry
            && let (Some(start), Some(end)) = (
                visible_ids.iter().position(|id| *id == focused),
                visible_ids.iter().position(|id| *id == entry_id),
            )
        {
            for &id in &visible_ids[start.min(end)..=start.max(end)] {
                app.ui_state.selected_entries.insert(id);
            }
        } else {
            app.ui_state.selected_entries.clear();
            app.ui_state.selected_entries.insert(entry_id);
        }
    } else {
        // Single select
        app.ui_state.selected_entries.clear();
        app.ui_state.selected_entries.insert(entry_id);
    }

    app.ui_state.focused_entry = Some(entry_id);
}

/// Paint text in a table cell without creating a widget.
///
/// This avoids registering a `WidgetRect` that would intercept pointer events,
/// allowing the cell's own response (set via `TableBuilder::sense`) to handle
/// all click and context-menu interaction.
///
/// Text is clipped to the cell's available rect so it doesn't bleed into
/// adjacent columns.
/// Render tag-related context menu items for a single entry.
fn show_tag_menu_items(
    ui: &mut egui::Ui,
    app: &mut RetroJunkApp,
    console_idx: usize,
    entry_id: retro_junk_db::LibraryEntryId,
) {
    let Some(entry_idx) = app.browser.consoles[console_idx].entry_index(entry_id) else {
        return;
    };
    let Some(entry) = app
        .browser
        .consoles
        .get(console_idx)
        .and_then(|c| c.entries.get(entry_idx))
    else {
        return;
    };
    let effective = entry.effective_status();

    match effective {
        EntryStatus::Unrecognized => {
            ui.separator();
            if ui.button("Mark as Homebrew\u{2026}").clicked() {
                // Pre-fill with cleaned filename
                let name = entry.game_entry.display_name().to_string();
                app.ui_state.tag_dialog = TagDialog::Homebrew {
                    name,
                    console_id: app.browser.consoles[console_idx].id.unwrap(),
                    entry_id,
                };
                ui.close();
            }
            if ui.button("Mark as Modded Version of\u{2026}").clicked() {
                app.ui_state.tag_dialog = TagDialog::ModSearch {
                    query: String::new(),
                    results: Vec::new(),
                    selected: None,
                    console_id: app.browser.consoles[console_idx].id.unwrap(),
                    entry_id,
                };
                ui.close();
            }
        }
        EntryStatus::Tagged(_) => {
            ui.separator();
            if ui.button("Remove Tag").clicked() {
                app.set_entry_tags([entry_id], None, ui.ctx());
                ui.close();
            }
        }
        _ => {}
    }
}

fn paint_cell_text(ui: &mut egui::Ui, text: &str) {
    if text.is_empty() {
        return;
    }
    let rect = ui.max_rect();
    let font_id = egui::TextStyle::Body.resolve(ui.style());
    let color = ui.visuals().text_color();
    ui.painter().with_clip_rect(rect).text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        text,
        font_id,
        color,
    );
}

#[cfg(test)]
mod tests {
    use super::projection_status;
    use crate::state::EntryStatus;
    use retro_junk_catalog::CatalogTag;

    #[test]
    fn projected_tags_override_stored_analysis_status() {
        assert_eq!(
            projection_status("unrecognized", "homebrew"),
            EntryStatus::Tagged(CatalogTag::Homebrew)
        );
        assert_eq!(
            projection_status("matched", "modded"),
            EntryStatus::Tagged(CatalogTag::Modded)
        );
        assert_eq!(projection_status("ambiguous", ""), EntryStatus::Ambiguous);
        assert_eq!(projection_status("likely", ""), EntryStatus::LikelyMatched);
    }

    #[test]
    fn native_rom_extensions_satisfy_rom_policy() {
        assert!(super::formats_satisfy_policy("nes", "rom"));
        assert!(super::formats_satisfy_policy("sfc", "rom"));
        assert!(!super::formats_satisfy_policy("chd", "rom"));
        assert!(super::formats_satisfy_policy("cue", "cue_bin"));
    }
}
