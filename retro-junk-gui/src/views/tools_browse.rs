use egui_extras::{Column, TableBuilder};

use crate::app::RetroJunkApp;
use crate::state::{BrowseTable, TableViewState};
use crate::views::tools::{format_number, truncate_str};

/// Render the database browser tab.
pub fn show(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    // Load platforms if not yet loaded (shared with Dashboard)
    if app.tools_state.platforms.is_empty() {
        let conn = app.catalog_db.as_ref().unwrap();
        app.tools_state.platforms = retro_junk_db::list_platforms(conn).unwrap_or_default();
    }

    // Toolbar: table selector + search + platform filter
    let query_changed = show_toolbar(ui, app);
    if query_changed {
        app.tools_state.browse.table_state.reset_query();
    }

    ui.add_space(4.0);

    // Run query if needed
    if app.tools_state.browse.table_state.needs_query {
        run_query(app);
    }

    // Results count + table
    let total = app.tools_state.browse.table_state.total_count;
    ui.label(format!("{} result(s)", format_number(total)));
    ui.add_space(4.0);

    match app.tools_state.browse.active_table {
        BrowseTable::Releases => show_releases_table(ui, app),
        BrowseTable::Media => show_media_table(ui, app),
        BrowseTable::Works => show_works_table(ui, app),
        BrowseTable::Companies => show_companies_table(ui, app),
        BrowseTable::Collection => show_collection_table(ui, app),
        BrowseTable::ImportLog => show_import_log_table(ui, app),
    }

    // Pagination footer
    ui.add_space(4.0);
    let page_changed = show_pagination(ui, &mut app.tools_state.browse.table_state);
    if page_changed {
        app.tools_state.browse.table_state.needs_query = true;
    }
}

// ── Toolbar ─────────────────────────────────────────────────────────────────

/// Returns true if the query parameters changed (table, search, or platform filter).
fn show_toolbar(ui: &mut egui::Ui, app: &mut RetroJunkApp) -> bool {
    let mut changed = false;
    let browse = &mut app.tools_state.browse;
    let active = browse.active_table;

    ui.horizontal(|ui| {
        // Table selector dropdown
        ui.label("View:");
        egui::ComboBox::from_id_salt("browse_table_select")
            .selected_text(active.label())
            .width(120.0)
            .show_ui(ui, |ui| {
                for &table in BrowseTable::ALL {
                    if ui
                        .selectable_value(&mut browse.active_table, table, table.label())
                        .changed()
                    {
                        changed = true;
                        // Clear platform filter if new table doesn't support it
                        if !table.has_platform_filter() {
                            browse.table_state.platform_filter = None;
                        }
                    }
                }
            });

        // Search box
        if active.has_search() {
            ui.add_space(12.0);
            ui.label("Search:");
            let resp = ui.add(
                egui::TextEdit::singleline(&mut browse.table_state.search_text)
                    .desired_width(200.0)
                    .hint_text("Type to search..."),
            );
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                changed = true;
            }
            if !browse.table_state.search_text.is_empty() {
                if ui.small_button("Go").clicked() {
                    changed = true;
                }
                if ui.small_button("Clear").clicked() {
                    browse.table_state.search_text.clear();
                    changed = true;
                }
            }
        }

        // Platform filter
        if active.has_platform_filter() {
            ui.add_space(12.0);
            ui.label("Platform:");
            let current_label = match &browse.table_state.platform_filter {
                Some(pid) => app
                    .tools_state
                    .platforms
                    .iter()
                    .find(|p| p.id == *pid)
                    .map(|p| p.short_name.as_str())
                    .unwrap_or("???"),
                None => "All",
            };
            egui::ComboBox::from_id_salt("browse_platform_filter")
                .selected_text(current_label)
                .width(150.0)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_value(&mut browse.table_state.platform_filter, None, "All")
                        .changed()
                    {
                        changed = true;
                    }
                    for p in &app.tools_state.platforms {
                        if ui
                            .selectable_value(
                                &mut browse.table_state.platform_filter,
                                Some(p.id.clone()),
                                &p.short_name,
                            )
                            .changed()
                        {
                            changed = true;
                        }
                    }
                });
        }
    });

    changed
}

// ── Pagination ──────────────────────────────────────────────────────────────

/// Render Prev / [page input] / N Next pagination. Returns true if page changed.
fn show_pagination(ui: &mut egui::Ui, state: &mut TableViewState) -> bool {
    let total_pages = state.total_pages().max(1);
    let mut changed = false;

    ui.horizontal(|ui| {
        // Prev button
        if ui
            .add_enabled(state.page > 0, egui::Button::new("Prev"))
            .clicked()
        {
            state.page = state.page.saturating_sub(1);
            state.page_input = (state.page + 1).to_string();
            changed = true;
        }

        // Editable page number (1-indexed for display)
        let resp = ui.add(
            egui::TextEdit::singleline(&mut state.page_input)
                .desired_width(40.0)
                .horizontal_align(egui::Align::Center),
        );
        if resp.lost_focus() {
            if let Ok(n) = state.page_input.trim().parse::<u32>() {
                let clamped = n.clamp(1, total_pages);
                if clamped - 1 != state.page {
                    state.page = clamped - 1;
                    changed = true;
                }
                state.page_input = clamped.to_string();
            } else {
                // Reset to current
                state.page_input = (state.page + 1).to_string();
            }
        }

        ui.label(format!("/ {}", total_pages));

        // Next button
        if ui
            .add_enabled(state.page + 1 < total_pages, egui::Button::new("Next"))
            .clicked()
        {
            state.page += 1;
            state.page_input = (state.page + 1).to_string();
            changed = true;
        }
    });

    changed
}

// ── Query Execution ─────────────────────────────────────────────────────────

fn run_query(app: &mut RetroJunkApp) {
    let conn = app.catalog_db.as_ref().unwrap();
    let browse = &mut app.tools_state.browse;
    let ts = &mut browse.table_state;
    ts.needs_query = false;

    let query = if ts.search_text.is_empty() {
        "%"
    } else {
        &ts.search_text
    };
    let pid = ts.platform_filter.as_deref();
    let limit = ts.page_size;
    let offset = ts.offset();

    match browse.active_table {
        BrowseTable::Releases => {
            ts.total_count = retro_junk_db::count_releases_search(conn, query, pid).unwrap_or(0);
            browse.releases = retro_junk_db::search_releases_paged(conn, query, pid, limit, offset)
                .unwrap_or_default();
        }
        BrowseTable::Media => {
            ts.total_count = retro_junk_db::count_media_search(conn, query, pid).unwrap_or(0);
            browse.media_rows =
                retro_junk_db::search_media(conn, query, pid, limit, offset).unwrap_or_default();
        }
        BrowseTable::Works => {
            ts.total_count = retro_junk_db::count_works_search(conn, query).unwrap_or(0);
            // search_works doesn't return release_count; use WorkRow and wrap
            let work_rows =
                retro_junk_db::search_works(conn, query, limit, offset).unwrap_or_default();
            browse.works = work_rows
                .into_iter()
                .map(|w| retro_junk_db::WorkWithCount {
                    id: w.id,
                    canonical_name: w.canonical_name,
                    release_count: 0,
                })
                .collect();
        }
        BrowseTable::Companies => {
            ts.total_count = retro_junk_db::count_companies_search(conn, query).unwrap_or(0);
            browse.companies =
                retro_junk_db::search_companies(conn, query, limit, offset).unwrap_or_default();
        }
        BrowseTable::Collection => {
            ts.total_count = retro_junk_db::count_collection(conn, pid).unwrap_or(0);
            browse.collection =
                retro_junk_db::list_collection_paged(conn, pid, limit, offset).unwrap_or_default();
        }
        BrowseTable::ImportLog => {
            // Import log doesn't paginate heavily; just load all
            browse.import_logs =
                retro_junk_db::list_import_logs(conn, Some(500)).unwrap_or_default();
            ts.total_count = browse.import_logs.len() as i64;
        }
    }
}

// ── Table Renderers ─────────────────────────────────────────────────────────

const ROW_HEIGHT: f32 = 20.0;

fn show_releases_table(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    let available = ui.available_width();
    let platform_col = 80.0;
    let region_col = 60.0;
    let serial_col = 110.0;
    let date_col = 90.0;
    let title_col =
        (available - platform_col - region_col - serial_col - date_col - 24.0).max(150.0);

    let max_height = ui.available_height() - 40.0;

    TableBuilder::new(ui)
        .striped(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .max_scroll_height(max_height)
        .column(Column::exact(title_col))
        .column(Column::exact(platform_col))
        .column(Column::exact(region_col))
        .column(Column::exact(serial_col))
        .column(Column::exact(date_col))
        .header(ROW_HEIGHT, |mut header| {
            header.col(|ui| {
                ui.strong("Title");
            });
            header.col(|ui| {
                ui.strong("Platform");
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
                ROW_HEIGHT,
                app.tools_state.browse.releases.len(),
                |mut row| {
                    let r = &app.tools_state.browse.releases[row.index()];
                    row.col(|ui| {
                        ui.label(truncate_str(&r.title, 60));
                    });
                    row.col(|ui| {
                        ui.label(&r.platform_id);
                    });
                    row.col(|ui| {
                        ui.label(&r.region);
                    });
                    row.col(|ui| {
                        ui.label(r.game_serial.as_deref().unwrap_or(""));
                    });
                    row.col(|ui| {
                        ui.label(r.release_date.as_deref().unwrap_or(""));
                    });
                },
            );
        });
}

fn show_media_table(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    let available = ui.available_width();
    let source_col = 80.0;
    let size_col = 80.0;
    let sha1_col = 120.0;
    let name_col = (available - source_col - size_col - sha1_col - 20.0).max(150.0);

    let max_height = ui.available_height() - 40.0;

    TableBuilder::new(ui)
        .striped(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .max_scroll_height(max_height)
        .column(Column::exact(name_col))
        .column(Column::exact(source_col))
        .column(Column::exact(size_col))
        .column(Column::exact(sha1_col))
        .header(ROW_HEIGHT, |mut header| {
            header.col(|ui| {
                ui.strong("DAT Name");
            });
            header.col(|ui| {
                ui.strong("Source");
            });
            header.col(|ui| {
                ui.strong("Size");
            });
            header.col(|ui| {
                ui.strong("SHA1");
            });
        })
        .body(|body| {
            body.rows(
                ROW_HEIGHT,
                app.tools_state.browse.media_rows.len(),
                |mut row| {
                    let m = &app.tools_state.browse.media_rows[row.index()];
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
                        let s = m.file_size.map(format_file_size).unwrap_or_default();
                        ui.label(s);
                    });
                    row.col(|ui| {
                        ui.label(truncate_str(m.sha1.as_deref().unwrap_or(""), 16));
                    });
                },
            );
        });
}

fn show_works_table(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    let available = ui.available_width();
    let name_col = (available - 16.0).max(200.0);

    let max_height = ui.available_height() - 40.0;

    TableBuilder::new(ui)
        .striped(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .max_scroll_height(max_height)
        .column(Column::exact(name_col))
        .header(ROW_HEIGHT, |mut header| {
            header.col(|ui| {
                ui.strong("Canonical Name");
            });
        })
        .body(|body| {
            body.rows(ROW_HEIGHT, app.tools_state.browse.works.len(), |mut row| {
                let w = &app.tools_state.browse.works[row.index()];
                row.col(|ui| {
                    ui.label(truncate_str(&w.canonical_name, 100));
                });
            });
        });
}

fn show_companies_table(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    let available = ui.available_width();
    let country_col = 80.0;
    let aliases_col = 70.0;
    let name_col = (available - country_col - aliases_col - 16.0).max(150.0);

    let max_height = ui.available_height() - 40.0;

    TableBuilder::new(ui)
        .striped(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .max_scroll_height(max_height)
        .column(Column::exact(name_col))
        .column(Column::exact(country_col))
        .column(Column::exact(aliases_col))
        .header(ROW_HEIGHT, |mut header| {
            header.col(|ui| {
                ui.strong("Name");
            });
            header.col(|ui| {
                ui.strong("Country");
            });
            header.col(|ui| {
                ui.strong("Aliases");
            });
        })
        .body(|body| {
            body.rows(
                ROW_HEIGHT,
                app.tools_state.browse.companies.len(),
                |mut row| {
                    let c = &app.tools_state.browse.companies[row.index()];
                    row.col(|ui| {
                        ui.label(truncate_str(&c.name, 60));
                    });
                    row.col(|ui| {
                        ui.label(c.country.as_deref().unwrap_or(""));
                    });
                    row.col(|ui| {
                        ui.label(format_number(c.alias_count));
                    });
                },
            );
        });
}

fn show_collection_table(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    let available = ui.available_width();
    let platform_col = 80.0;
    let region_col = 60.0;
    let verified_col = 80.0;
    let title_col = (available - platform_col - region_col - verified_col - 20.0).max(150.0);

    let max_height = ui.available_height() - 40.0;

    TableBuilder::new(ui)
        .striped(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .max_scroll_height(max_height)
        .column(Column::exact(title_col))
        .column(Column::exact(platform_col))
        .column(Column::exact(region_col))
        .column(Column::exact(verified_col))
        .header(ROW_HEIGHT, |mut header| {
            header.col(|ui| {
                ui.strong("Title");
            });
            header.col(|ui| {
                ui.strong("Platform");
            });
            header.col(|ui| {
                ui.strong("Region");
            });
            header.col(|ui| {
                ui.strong("Verified");
            });
        })
        .body(|body| {
            body.rows(
                ROW_HEIGHT,
                app.tools_state.browse.collection.len(),
                |mut row| {
                    let c = &app.tools_state.browse.collection[row.index()];
                    row.col(|ui| {
                        ui.label(truncate_str(&c.title, 60));
                    });
                    row.col(|ui| {
                        ui.label(&c.platform_id);
                    });
                    row.col(|ui| {
                        ui.label(&c.region);
                    });
                    row.col(|ui| {
                        let label = if c.verified_at.is_some() { "Yes" } else { "" };
                        ui.label(label);
                    });
                },
            );
        });
}

fn show_import_log_table(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    let available = ui.available_width();
    let type_col = 80.0;
    let date_col = 140.0;
    let created_col = 70.0;
    let updated_col = 70.0;
    let name_col = (available - type_col - date_col - created_col - updated_col - 24.0).max(120.0);

    let max_height = ui.available_height() - 40.0;

    TableBuilder::new(ui)
        .striped(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .max_scroll_height(max_height)
        .column(Column::exact(name_col))
        .column(Column::exact(type_col))
        .column(Column::exact(date_col))
        .column(Column::exact(created_col))
        .column(Column::exact(updated_col))
        .header(ROW_HEIGHT, |mut header| {
            header.col(|ui| {
                ui.strong("Source");
            });
            header.col(|ui| {
                ui.strong("Type");
            });
            header.col(|ui| {
                ui.strong("Imported At");
            });
            header.col(|ui| {
                ui.strong("Created");
            });
            header.col(|ui| {
                ui.strong("Updated");
            });
        })
        .body(|body| {
            body.rows(
                ROW_HEIGHT,
                app.tools_state.browse.import_logs.len(),
                |mut row| {
                    let log = &app.tools_state.browse.import_logs[row.index()];
                    row.col(|ui| {
                        ui.label(truncate_str(&log.source_name, 40));
                    });
                    row.col(|ui| {
                        ui.label(&log.source_type);
                    });
                    row.col(|ui| {
                        ui.label(&log.imported_at);
                    });
                    row.col(|ui| {
                        ui.label(format_number(log.records_created));
                    });
                    row.col(|ui| {
                        ui.label(format_number(log.records_updated));
                    });
                },
            );
        });
}

// ── Helpers ─────────────────────────────────────────────────────────────────

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
