use std::collections::HashMap;

use egui_extras::{Column, TableBuilder};

use crate::app::RetroJunkApp;
use crate::views::tools::{format_number, truncate_str};

/// Render the database browser tab.
pub fn show(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    let conn = app.catalog_db.as_ref().unwrap();

    // Load platforms if not yet loaded (shared with Dashboard)
    if app.tools_state.platforms.is_empty() {
        app.tools_state.platforms = retro_junk_db::list_platforms(conn).unwrap_or_default();
    }

    // Load release counts per platform on first open
    if !app.tools_state.browse.counts_loaded {
        app.tools_state.browse.counts_loaded = true;
        if let Ok(counts) = retro_junk_db::platform_release_counts(conn) {
            app.tools_state.browse.platform_release_counts =
                counts.into_iter().collect::<HashMap<_, _>>();
        }
    }

    // Two-pane layout: platform tree (left) + content (right)
    egui::SidePanel::left("browse_platform_tree")
        .resizable(true)
        .default_width(200.0)
        .width_range(150.0..=300.0)
        .show_inside(ui, |ui| {
            ui.add_space(4.0);
            ui.strong("Platforms");
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                show_platform_tree(ui, app);
            });
        });

    // Right content area
    egui::CentralPanel::default().show_inside(ui, |ui| {
        if app.tools_state.browse.selected_platform.is_none() {
            ui.add_space(32.0);
            ui.centered_and_justified(|ui| {
                ui.weak("Select a platform to browse its catalog.");
            });
        } else if app.tools_state.browse.selected_work_idx.is_some() {
            show_work_releases(ui, app);
        } else {
            show_works_list(ui, app);
        }
    });
}

/// Render the manufacturer-grouped platform tree with release counts.
fn show_platform_tree(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    // Collect platform data needed for rendering (avoids borrow conflicts)
    let platform_info: Vec<(String, String, String)> = app
        .tools_state
        .platforms
        .iter()
        .map(|p| (p.id.clone(), p.short_name.clone(), p.manufacturer.clone()))
        .collect();

    // Collect unique manufacturers in order
    let manufacturers: Vec<&str> = {
        let mut seen = Vec::new();
        for (_, _, mfr) in &platform_info {
            if !seen.contains(&mfr.as_str()) {
                seen.push(mfr.as_str());
            }
        }
        seen
    };

    let mut clicked_platform: Option<String> = None;

    for mfr in &manufacturers {
        egui::CollapsingHeader::new(egui::RichText::new(*mfr).strong())
            .id_salt(format!("browse_mfr_{}", mfr))
            .default_open(true)
            .show(ui, |ui| {
                for (id, short_name, p_mfr) in &platform_info {
                    if p_mfr.as_str() != *mfr {
                        continue;
                    }

                    let count = app
                        .tools_state
                        .browse
                        .platform_release_counts
                        .get(id)
                        .copied()
                        .unwrap_or(0);
                    let label = if count > 0 {
                        format!("{} ({})", short_name, format_number(count))
                    } else {
                        short_name.clone()
                    };

                    let is_selected =
                        app.tools_state.browse.selected_platform.as_deref() == Some(id.as_str());

                    if ui.selectable_label(is_selected, &label).clicked() && !is_selected {
                        clicked_platform = Some(id.clone());
                    }
                }
            });
    }

    if let Some(pid) = clicked_platform {
        select_platform(app, &pid);
    }
}

/// Handle platform selection: load works for the platform.
fn select_platform(app: &mut RetroJunkApp, platform_id: &str) {
    let browse = &mut app.tools_state.browse;
    browse.selected_platform = Some(platform_id.to_string());
    browse.selected_work_idx = None;
    browse.releases.clear();
    browse.selected_release_idx = None;
    browse.release_media.clear();
    browse.filter_text.clear();

    let conn = app.catalog_db.as_ref().unwrap();
    browse.works = retro_junk_db::works_for_platform(conn, platform_id).unwrap_or_default();
}

/// Render the filterable works list for the selected platform.
fn show_works_list(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    let platform_name = app
        .tools_state
        .browse
        .selected_platform
        .as_deref()
        .and_then(|pid| {
            app.tools_state
                .platforms
                .iter()
                .find(|p| p.id == pid)
                .map(|p| p.display_name.as_str())
        })
        .unwrap_or("Unknown");

    ui.strong(format!("Works on {}", platform_name));
    ui.add_space(4.0);

    // Filter box
    ui.horizontal(|ui| {
        ui.label("Filter:");
        ui.add(
            egui::TextEdit::singleline(&mut app.tools_state.browse.filter_text)
                .desired_width(250.0)
                .hint_text("Type to filter..."),
        );
        if !app.tools_state.browse.filter_text.is_empty() && ui.small_button("Clear").clicked() {
            app.tools_state.browse.filter_text.clear();
        }
    });

    // Build filtered indices
    let filter_lower = app.tools_state.browse.filter_text.to_lowercase();
    let filtered_indices: Vec<usize> = app
        .tools_state
        .browse
        .works
        .iter()
        .enumerate()
        .filter(|(_, w)| {
            filter_lower.is_empty() || w.canonical_name.to_lowercase().contains(&filter_lower)
        })
        .map(|(i, _)| i)
        .collect();

    ui.add_space(4.0);
    ui.label(format!(
        "{} work(s){}",
        filtered_indices.len(),
        if !filter_lower.is_empty() {
            format!(" (of {} total)", app.tools_state.browse.works.len())
        } else {
            String::new()
        }
    ));
    ui.add_space(4.0);

    if filtered_indices.is_empty() {
        ui.weak("No works match the filter.");
        return;
    }

    // Works table
    let row_height = 20.0;
    let available = ui.available_width();
    let count_col = 80.0;
    let title_col = (available - count_col - 16.0).max(200.0);

    let mut clicked_work_idx: Option<usize> = None;
    let max_table_height = ui.available_height() - 20.0;

    TableBuilder::new(ui)
        .striped(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .max_scroll_height(max_table_height)
        .column(Column::exact(title_col))
        .column(Column::exact(count_col))
        .header(row_height, |mut header| {
            header.col(|ui| {
                ui.strong("Title");
            });
            header.col(|ui| {
                ui.strong("Releases");
            });
        })
        .body(|body| {
            body.rows(row_height, filtered_indices.len(), |mut row| {
                let fi = row.index();
                let work_idx = filtered_indices[fi];
                let work = &app.tools_state.browse.works[work_idx];

                row.col(|ui| {
                    if ui
                        .selectable_label(false, truncate_str(&work.canonical_name, 80))
                        .clicked()
                    {
                        clicked_work_idx = Some(work_idx);
                    }
                });
                row.col(|ui| {
                    ui.label(format_number(work.release_count));
                });
            });
        });

    // Handle click after table (borrow resolved)
    if let Some(work_idx) = clicked_work_idx {
        select_work(app, work_idx);
    }
}

/// Handle work selection: load releases for this work on the selected platform.
fn select_work(app: &mut RetroJunkApp, work_idx: usize) {
    let browse = &mut app.tools_state.browse;
    browse.selected_work_idx = Some(work_idx);
    browse.selected_release_idx = None;
    browse.release_media.clear();

    let work_id = &browse.works[work_idx].id;
    let platform_id = browse.selected_platform.as_deref().unwrap_or("");
    let conn = app.catalog_db.as_ref().unwrap();

    // Get all releases for this work, filter to current platform
    let all_releases = retro_junk_db::releases_for_work(conn, work_id).unwrap_or_default();
    browse.releases = all_releases
        .into_iter()
        .filter(|r| r.platform_id == platform_id)
        .collect();
}

/// Render the releases view for a selected work (breadcrumb + table + detail).
fn show_work_releases(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    let work_idx = match app.tools_state.browse.selected_work_idx {
        Some(idx) => idx,
        None => return,
    };

    let work_name = app
        .tools_state
        .browse
        .works
        .get(work_idx)
        .map(|w| w.canonical_name.as_str())
        .unwrap_or("Unknown");

    let platform_name = app
        .tools_state
        .browse
        .selected_platform
        .as_deref()
        .and_then(|pid| {
            app.tools_state
                .platforms
                .iter()
                .find(|p| p.id == pid)
                .map(|p| p.short_name.as_str())
        })
        .unwrap_or("???");

    // Breadcrumb navigation
    ui.horizontal(|ui| {
        if ui.small_button(platform_name).clicked() {
            app.tools_state.browse.selected_work_idx = None;
            app.tools_state.browse.releases.clear();
            app.tools_state.browse.selected_release_idx = None;
            app.tools_state.browse.release_media.clear();
            return;
        }
        ui.label(">");
        ui.strong(work_name);
    });

    ui.separator();
    ui.add_space(4.0);

    ui.label(format!(
        "{} release(s)",
        app.tools_state.browse.releases.len()
    ));
    ui.add_space(4.0);

    if app.tools_state.browse.releases.is_empty() {
        ui.weak("No releases found for this work on this platform.");
        return;
    }

    // Releases table
    let row_height = 20.0;
    let available = ui.available_width();
    let region_col = 60.0;
    let serial_col = 120.0;
    let date_col = 100.0;
    let title_col = (available - region_col - serial_col - date_col - 24.0).max(150.0);

    let mut clicked_release_idx: Option<usize> = None;

    let selected_release = app.tools_state.browse.selected_release_idx;

    TableBuilder::new(ui)
        .striped(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .max_scroll_height(200.0)
        .column(Column::exact(title_col))
        .column(Column::exact(region_col))
        .column(Column::exact(serial_col))
        .column(Column::exact(date_col))
        .header(row_height, |mut header| {
            header.col(|ui| {
                ui.strong("Title");
            });
            header.col(|ui| {
                ui.strong("Region");
            });
            header.col(|ui| {
                ui.strong("Serial");
            });
            header.col(|ui| {
                ui.strong("Date");
            });
        })
        .body(|body| {
            body.rows(
                row_height,
                app.tools_state.browse.releases.len(),
                |mut row| {
                    let idx = row.index();
                    let release = &app.tools_state.browse.releases[idx];
                    let selected = selected_release == Some(idx);

                    row.set_selected(selected);

                    row.col(|ui| {
                        if ui
                            .selectable_label(selected, truncate_str(&release.title, 60))
                            .clicked()
                        {
                            clicked_release_idx = Some(idx);
                        }
                    });
                    row.col(|ui| {
                        ui.label(&release.region);
                    });
                    row.col(|ui| {
                        ui.label(release.game_serial.as_deref().unwrap_or(""));
                    });
                    row.col(|ui| {
                        ui.label(release.release_date.as_deref().unwrap_or(""));
                    });
                },
            );
        });

    // Handle click after table
    if let Some(idx) = clicked_release_idx {
        select_release(app, idx);
    }

    // Show detail for selected release
    if app.tools_state.browse.selected_release_idx.is_some() {
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        show_release_detail(ui, app);
    }
}

/// Handle release selection: load media for this release.
fn select_release(app: &mut RetroJunkApp, idx: usize) {
    let browse = &mut app.tools_state.browse;
    browse.selected_release_idx = Some(idx);

    let release_id = &browse.releases[idx].id;
    let conn = app.catalog_db.as_ref().unwrap();
    browse.release_media = retro_junk_db::media_for_release(conn, release_id).unwrap_or_default();
}

/// Render the detail panel for a selected release.
fn show_release_detail(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    let idx = match app.tools_state.browse.selected_release_idx {
        Some(i) => i,
        None => return,
    };

    let release = match app.tools_state.browse.releases.get(idx) {
        Some(r) => r,
        None => return,
    };

    ui.strong("Release Detail");
    ui.add_space(4.0);

    // Resolve company names lazily
    let conn = app.catalog_db.as_ref().unwrap();
    let publisher_name = resolve_company_name(
        &mut app.tools_state.browse.company_name_cache,
        conn,
        release.publisher_id.as_deref(),
    );
    let developer_name = resolve_company_name(
        &mut app.tools_state.browse.company_name_cache,
        conn,
        release.developer_id.as_deref(),
    );

    egui::ScrollArea::vertical().show(ui, |ui| {
        // Metadata grid
        egui::Grid::new("release_detail_grid")
            .num_columns(2)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                detail_row(ui, "Title", &release.title);
                if let Some(ref alt) = release.alt_title {
                    detail_row(ui, "Alt Title", alt);
                }
                detail_row(ui, "Region", &release.region);
                if !release.revision.is_empty() {
                    detail_row(ui, "Revision", &release.revision);
                }
                if !release.variant.is_empty() {
                    detail_row(ui, "Variant", &release.variant);
                }
                if let Some(ref serial) = release.game_serial {
                    detail_row(ui, "Serial", serial);
                }
                if let Some(ref date) = release.release_date {
                    detail_row(ui, "Released", date);
                }
                if let Some(ref name) = publisher_name {
                    detail_row(ui, "Publisher", name);
                }
                if let Some(ref name) = developer_name {
                    detail_row(ui, "Developer", name);
                }
                if let Some(ref genre) = release.genre {
                    detail_row(ui, "Genre", genre);
                }
                if let Some(ref players) = release.players {
                    detail_row(ui, "Players", players);
                }
                if let Some(rating) = release.rating {
                    detail_row(ui, "Rating", &format!("{:.1}", rating));
                }
                if let Some(ref ss_id) = release.screenscraper_id {
                    detail_row(ui, "ScreenScraper ID", ss_id);
                }
            });

        // Description
        if let Some(ref desc) = release.description {
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Description").strong());
            ui.add_space(2.0);
            ui.label(desc);
        }

        // Media table
        if !app.tools_state.browse.release_media.is_empty() {
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Media Entries").strong());
            ui.add_space(4.0);

            let media_row_height = 20.0;
            let avail = ui.available_width();
            let name_col = (avail * 0.5).max(150.0);
            let source_col = 80.0;
            let size_col = 80.0;

            TableBuilder::new(ui)
                .striped(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .max_scroll_height(200.0)
                .column(Column::exact(name_col))
                .column(Column::exact(source_col))
                .column(Column::exact(size_col))
                .header(media_row_height, |mut header| {
                    header.col(|ui| {
                        ui.strong("DAT Name");
                    });
                    header.col(|ui| {
                        ui.strong("Source");
                    });
                    header.col(|ui| {
                        ui.strong("Size");
                    });
                })
                .body(|body| {
                    body.rows(
                        media_row_height,
                        app.tools_state.browse.release_media.len(),
                        |mut row| {
                            let m = &app.tools_state.browse.release_media[row.index()];
                            row.col(|ui| {
                                ui.label(truncate_str(
                                    m.dat_name.as_deref().unwrap_or("(unnamed)"),
                                    60,
                                ));
                            });
                            row.col(|ui| {
                                ui.label(m.dat_source.as_deref().unwrap_or(""));
                            });
                            row.col(|ui| {
                                let size_str =
                                    m.file_size.map(|s| format_file_size(s)).unwrap_or_default();
                                ui.label(size_str);
                            });
                        },
                    );
                });
        }
    });
}

fn detail_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(egui::RichText::new(format!("{}:", label)).weak());
    ui.label(value);
    ui.end_row();
}

/// Resolve a company ID to its display name, caching results.
fn resolve_company_name(
    cache: &mut HashMap<String, String>,
    conn: &retro_junk_db::Connection,
    company_id: Option<&str>,
) -> Option<String> {
    let id = company_id?;
    if let Some(name) = cache.get(id) {
        return Some(name.clone());
    }
    let name = retro_junk_db::get_company_name(conn, id).ok().flatten()?;
    cache.insert(id.to_string(), name.clone());
    Some(name)
}

fn format_file_size(bytes: i64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}
