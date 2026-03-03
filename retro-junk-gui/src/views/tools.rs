use egui_extras::{Column, TableBuilder};

use crate::app::RetroJunkApp;
use crate::state::{DISAGREEMENT_FIELDS, DisagreementContext, ToolsState};

/// Render the Tools (catalog management) view.
pub fn show(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    ui.heading("Catalog Tools");
    ui.separator();
    ui.add_space(8.0);

    let has_db = app.catalog_db.is_some();
    if !has_db {
        show_no_database(ui);
        return;
    }

    // Refresh data from DB when flagged
    if app.tools_state.needs_refresh {
        let conn = app.catalog_db.as_ref().unwrap();
        refresh_data(&mut app.tools_state, conn);
    }

    egui::ScrollArea::vertical().show(ui, |ui| {
        show_stats_section(ui, &app.tools_state);
        ui.add_space(16.0);
        show_disagreements_section(ui, app);
    });
}

fn show_no_database(ui: &mut egui::Ui) {
    ui.add_space(16.0);
    ui.label("No catalog database found.");
    ui.add_space(4.0);
    ui.weak("Run 'retro-junk catalog import all' to create one.");
}

/// Reload stats, platforms, and disagreements from the database.
fn refresh_data(state: &mut ToolsState, conn: &retro_junk_db::Connection) {
    state.needs_refresh = false;

    state.stats = retro_junk_db::catalog_stats(conn).ok();
    state.platforms = retro_junk_db::list_platforms(conn).unwrap_or_default();

    let filter = retro_junk_db::DisagreementFilter {
        platform_id: state.filter_platform.as_deref(),
        field: state.filter_field.as_deref(),
        limit: Some(500),
        ..Default::default()
    };
    state.disagreements =
        retro_junk_db::list_unresolved_disagreements(conn, &filter).unwrap_or_default();

    // Clamp or clear selection
    if let Some(idx) = state.selected_idx {
        if idx >= state.disagreements.len() {
            if state.disagreements.is_empty() {
                state.selected_idx = None;
                state.selected_context = None;
            } else {
                state.selected_idx = Some(state.disagreements.len() - 1);
                state.selected_context = None;
            }
        }
    }

    // Reload context for current selection
    if let Some(idx) = state.selected_idx {
        load_disagreement_context(state, conn, idx);
    }
}

fn show_stats_section(ui: &mut egui::Ui, state: &ToolsState) {
    ui.strong("Catalog Statistics");
    ui.add_space(4.0);

    let Some(ref stats) = state.stats else {
        ui.weak("No statistics available.");
        return;
    };

    egui::Grid::new("catalog_stats_grid")
        .num_columns(2)
        .spacing([40.0, 4.0])
        .show(ui, |ui| {
            stat_row(ui, "Platforms", stats.platforms);
            stat_row(ui, "Works", stats.works);
            stat_row(ui, "Releases", stats.releases);
            stat_row(ui, "Media entries", stats.media);
            stat_row(ui, "Assets", stats.assets);
            stat_row(ui, "Collection (owned)", stats.collection_owned);
            stat_row(
                ui,
                "Unresolved disagreements",
                stats.unresolved_disagreements,
            );
        });
}

fn stat_row(ui: &mut egui::Ui, label: &str, value: i64) {
    ui.label(label);
    ui.label(format_number(value));
    ui.end_row();
}

fn format_number(n: i64) -> String {
    if n < 1_000 {
        return n.to_string();
    }
    // Simple thousands separator
    let s = n.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

fn show_disagreements_section(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    ui.strong("Disagreements");
    ui.add_space(4.0);

    let filter_changed = show_filter_toolbar(ui, &mut app.tools_state);
    if filter_changed {
        app.tools_state.needs_refresh = true;
        let conn = app.catalog_db.as_ref().unwrap();
        refresh_data(&mut app.tools_state, conn);
    }

    ui.add_space(4.0);
    ui.label(format!(
        "{} disagreement(s)",
        app.tools_state.disagreements.len()
    ));
    ui.add_space(4.0);

    if app.tools_state.disagreements.is_empty() {
        ui.weak("No unresolved disagreements.");
        return;
    }

    let new_selection = show_disagreement_table(ui, &app.tools_state);
    if let Some(idx) = new_selection
        && Some(idx) != app.tools_state.selected_idx
    {
        app.tools_state.selected_idx = Some(idx);
        app.tools_state.selected_context = None;
        let conn = app.catalog_db.as_ref().unwrap();
        load_disagreement_context(&mut app.tools_state, conn, idx);
    }

    if app.tools_state.selected_idx.is_some() {
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        show_resolver(ui, app);
    }
}

/// Returns `true` if the filter changed and data needs refreshing.
fn show_filter_toolbar(ui: &mut egui::Ui, state: &mut ToolsState) -> bool {
    let mut changed = false;

    ui.horizontal(|ui| {
        // Platform filter
        ui.label("Platform:");
        let current_platform_label = match &state.filter_platform {
            Some(pid) => state
                .platforms
                .iter()
                .find(|p| p.id == *pid)
                .map(|p| p.short_name.as_str())
                .unwrap_or("???"),
            None => "All",
        };
        egui::ComboBox::from_id_salt("tools_platform_filter")
            .selected_text(current_platform_label)
            .width(150.0)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_value(&mut state.filter_platform, None, "All")
                    .changed()
                {
                    changed = true;
                }
                for p in &state.platforms {
                    if ui
                        .selectable_value(
                            &mut state.filter_platform,
                            Some(p.id.clone()),
                            &p.short_name,
                        )
                        .changed()
                    {
                        changed = true;
                    }
                }
            });

        ui.add_space(16.0);

        // Field filter
        ui.label("Field:");
        let current_field_label = state.filter_field.as_deref().unwrap_or("All");
        egui::ComboBox::from_id_salt("tools_field_filter")
            .selected_text(current_field_label)
            .width(120.0)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_value(&mut state.filter_field, None, "All")
                    .changed()
                {
                    changed = true;
                }
                for &field in DISAGREEMENT_FIELDS {
                    if ui
                        .selectable_value(&mut state.filter_field, Some(field.to_string()), field)
                        .changed()
                    {
                        changed = true;
                    }
                }
            });
    });

    if changed {
        state.selected_idx = None;
        state.selected_context = None;
    }

    changed
}

/// Renders the disagreement table. Returns `Some(idx)` if a row was clicked.
fn show_disagreement_table(ui: &mut egui::Ui, state: &ToolsState) -> Option<usize> {
    let mut clicked_idx = None;
    let row_height = 20.0;
    let max_height = 300.0;

    let available = ui.available_width();
    // Columns: #(50) | Type(60) | Field(90) | Source A (flexible) | Source B (flexible)
    let fixed_width = 50.0 + 60.0 + 90.0 + 16.0; // plus some spacing
    let remaining = (available - fixed_width).max(200.0);
    let val_col_width = remaining / 2.0;

    TableBuilder::new(ui)
        .striped(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .max_scroll_height(max_height)
        .column(Column::exact(50.0)) // #
        .column(Column::exact(60.0)) // Type
        .column(Column::exact(90.0)) // Field
        .column(Column::exact(val_col_width)) // Source A
        .column(Column::exact(val_col_width)) // Source B
        .header(row_height, |mut header| {
            header.col(|ui| {
                ui.strong("#");
            });
            header.col(|ui| {
                ui.strong("Type");
            });
            header.col(|ui| {
                ui.strong("Field");
            });
            header.col(|ui| {
                ui.strong("Source A");
            });
            header.col(|ui| {
                ui.strong("Source B");
            });
        })
        .body(|body| {
            body.rows(row_height, state.disagreements.len(), |mut row| {
                let idx = row.index();
                let d = &state.disagreements[idx];
                let selected = state.selected_idx == Some(idx);

                row.set_selected(selected);

                row.col(|ui| {
                    if ui.selectable_label(selected, format!("{}", d.id)).clicked() {
                        clicked_idx = Some(idx);
                    }
                });
                row.col(|ui| {
                    ui.label(&d.entity_type);
                });
                row.col(|ui| {
                    ui.label(&d.field);
                });
                row.col(|ui| {
                    let label = format!(
                        "{}: {}",
                        &d.source_a,
                        d.value_a.as_deref().unwrap_or("(empty)")
                    );
                    ui.label(truncate_str(&label, 50));
                });
                row.col(|ui| {
                    let label = format!(
                        "{}: {}",
                        &d.source_b,
                        d.value_b.as_deref().unwrap_or("(empty)")
                    );
                    ui.label(truncate_str(&label, 50));
                });
            });
        });

    clicked_idx
}

/// Load entity context (title + platform) for the selected disagreement.
fn load_disagreement_context(state: &mut ToolsState, conn: &retro_junk_db::Connection, idx: usize) {
    let d = &state.disagreements[idx];

    let (entity_title, platform_id) = match d.entity_type.as_str() {
        "release" => {
            if let Ok(Some(rel)) = retro_junk_db::get_release_by_id(conn, &d.entity_id) {
                (rel.title, rel.platform_id)
            } else {
                (d.entity_id.clone(), String::new())
            }
        }
        "media" => {
            if let Ok(Some(media)) = retro_junk_db::get_media_by_id(conn, &d.entity_id) {
                if let Ok(Some(rel)) = retro_junk_db::get_release_by_id(conn, &media.release_id) {
                    (rel.title, rel.platform_id)
                } else {
                    (
                        media.dat_name.unwrap_or_else(|| d.entity_id.clone()),
                        String::new(),
                    )
                }
            } else {
                (d.entity_id.clone(), String::new())
            }
        }
        _ => (d.entity_id.clone(), String::new()),
    };

    let platform_name = if !platform_id.is_empty() {
        retro_junk_db::get_platform_display_name(conn, &platform_id)
            .ok()
            .flatten()
            .unwrap_or(platform_id)
    } else {
        String::new()
    };

    state.selected_context = Some(DisagreementContext {
        entity_title,
        platform_name,
    });
}

fn show_resolver(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    let Some(idx) = app.tools_state.selected_idx else {
        return;
    };
    if idx >= app.tools_state.disagreements.len() {
        return;
    }

    ui.strong("Resolve Disagreement");
    ui.add_space(4.0);

    // Show entity context
    if let Some(ref ctx) = app.tools_state.selected_context {
        ui.horizontal(|ui| {
            ui.label("Entity:");
            let label = if ctx.platform_name.is_empty() {
                ctx.entity_title.clone()
            } else {
                format!("{} ({})", ctx.entity_title, ctx.platform_name)
            };
            ui.strong(label);
        });
    }

    let d = &app.tools_state.disagreements[idx];

    ui.horizontal(|ui| {
        ui.label("Field:");
        ui.strong(&d.field);
    });

    ui.add_space(4.0);

    // Show side-by-side values
    egui::Grid::new("resolver_values")
        .num_columns(2)
        .spacing([20.0, 4.0])
        .show(ui, |ui| {
            ui.colored_label(
                egui::Color32::from_rgb(100, 160, 255),
                format!("{} (A):", &d.source_a),
            );
            ui.label(d.value_a.as_deref().unwrap_or("(empty)"));
            ui.end_row();

            ui.colored_label(
                egui::Color32::from_rgb(255, 200, 60),
                format!("{} (B):", &d.source_b),
            );
            ui.label(d.value_b.as_deref().unwrap_or("(empty)"));
            ui.end_row();
        });

    ui.add_space(8.0);

    // Clone fields we need for the mutable DB call
    let disagreement_id = d.id;
    let entity_type = d.entity_type.clone();
    let entity_id = d.entity_id.clone();
    let field = d.field.clone();
    let value_a = d.value_a.clone();
    let value_b = d.value_b.clone();
    let source_a_label = d.source_a.clone();
    let source_b_label = d.source_b.clone();

    // Resolution buttons
    let mut resolution: Option<(&str, Option<&str>)> = None;

    ui.horizontal(|ui| {
        if ui.button(format!("Accept {}", source_a_label)).clicked() {
            resolution = Some(("source_a", value_a.as_deref()));
        }
        if ui.button(format!("Accept {}", source_b_label)).clicked() {
            resolution = Some(("source_b", value_b.as_deref()));
        }
        if ui.button("Skip").clicked() {
            // Advance to next without resolving
            let next = idx + 1;
            if next < app.tools_state.disagreements.len() {
                app.tools_state.selected_idx = Some(next);
                app.tools_state.selected_context = None;
                let conn = app.catalog_db.as_ref().unwrap();
                load_disagreement_context(&mut app.tools_state, conn, next);
            } else {
                app.tools_state.selected_idx = None;
                app.tools_state.selected_context = None;
            }
        }
    });

    // Apply resolution
    if let Some((resolution_str, chosen_value)) = resolution {
        let conn = app.catalog_db.as_ref().unwrap();

        // Step 1: Apply the value to the entity
        if let Some(value) = chosen_value {
            if let Err(e) = retro_junk_db::apply_disagreement_resolution(
                conn,
                &entity_type,
                &entity_id,
                &field,
                value,
            ) {
                log::warn!("Failed to apply resolution: {}", e);
            }
        }

        // Step 2: Mark disagreement as resolved
        if let Err(e) = retro_junk_db::resolve_disagreement(conn, disagreement_id, resolution_str) {
            log::warn!("Failed to resolve disagreement: {}", e);
        }

        // Refresh to pick up changes — keeps selected_idx at same position,
        // which now points to the next item since the resolved one is gone.
        app.tools_state.needs_refresh = true;
    }
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max - 3])
    }
}
