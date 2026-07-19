use retro_junk_catalog::CatalogTag;

use crate::app::RetroJunkApp;
use crate::state::TagDialog;

/// Render any active tag dialog as a modal window.
pub fn show(ctx: &egui::Context, app: &mut RetroJunkApp) {
    match &app.ui_state.tag_dialog {
        TagDialog::None => {}
        TagDialog::Homebrew { .. } => show_homebrew_dialog(ctx, app),
        TagDialog::ModSearch { .. } => show_mod_search_dialog(ctx, app),
    }
}

fn show_homebrew_dialog(ctx: &egui::Context, app: &mut RetroJunkApp) {
    let mut confirmed = false;
    let mut cancelled = false;

    egui::Window::new("Mark as Homebrew")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label("Enter the homebrew game name:");
            ui.add_space(4.0);

            if let TagDialog::Homebrew { ref mut name, .. } = app.ui_state.tag_dialog {
                let resp = ui.text_edit_singleline(name);
                // Auto-focus the text field on first frame
                if !resp.has_focus() {
                    resp.request_focus();
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Confirm").clicked()
                        || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                    {
                        confirmed = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancelled = true;
                    }
                });
            }
        });

    if confirmed {
        if let TagDialog::Homebrew {
            ref name,
            console_idx,
            entry_idx,
        } = app.ui_state.tag_dialog
        {
            let name = name.clone();

            // Create in DB if available
            if let Some(ref conn) = app.catalog_db
                && let Some(console) = app.library.consoles.get(console_idx)
            {
                let platform_id = console.folder_name.as_str();
                let region = console
                    .entries
                    .get(entry_idx)
                    .and_then(|e: &crate::state::LibraryEntry| {
                        e.effective_regions().first().map(|r| r.code().to_string())
                    })
                    .unwrap_or_else(|| "unknown".to_string());

                if let Err(e) =
                    retro_junk_db::create_homebrew_work(conn, &name, platform_id, &region)
                {
                    log::error!("Failed to create homebrew work: {e}");
                    app.push_error(
                        "Database Error",
                        format!("Failed to create homebrew work: {e}"),
                    );
                }
            }

            // Set tag on the entry
            if let Some(entry) = app
                .library
                .consoles
                .get_mut(console_idx)
                .and_then(|c| c.entries.get_mut(entry_idx))
            {
                entry.tag = Some(CatalogTag::Homebrew);
            }
            app.save_entry_cache(console_idx, &[entry_idx]);
        }
        app.ui_state.tag_dialog = TagDialog::None;
    } else if cancelled {
        app.ui_state.tag_dialog = TagDialog::None;
    }
}

fn show_mod_search_dialog(ctx: &egui::Context, app: &mut RetroJunkApp) {
    let mut cancelled = false;
    let mut confirmed = false;

    egui::Window::new("Mark as Modded Version of\u{2026}")
        .collapsible(false)
        .resizable(true)
        .default_size([400.0, 350.0])
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label("Search for the original game:");
            ui.add_space(4.0);

            let mut query_changed = false;
            if let TagDialog::ModSearch {
                ref mut query,
                ref mut results,
                ref mut selected,
                ..
            } = app.ui_state.tag_dialog
            {
                let resp = ui.text_edit_singleline(query);
                if resp.changed() {
                    query_changed = true;
                    *selected = None;
                }

                // Run search when query changes
                if query_changed && query.len() >= 2 {
                    if let Some(ref conn) = app.catalog_db {
                        *results =
                            retro_junk_db::search_works(conn, query, 20, 0).unwrap_or_default();
                    }
                } else if query.len() < 2 {
                    results.clear();
                }

                ui.add_space(4.0);

                // Results list
                egui::ScrollArea::vertical()
                    .max_height(250.0)
                    .show(ui, |ui| {
                        for (i, work) in results.iter().enumerate() {
                            let is_selected = *selected == Some(i);
                            if ui
                                .selectable_label(is_selected, &work.canonical_name)
                                .clicked()
                            {
                                *selected = Some(i);
                            }
                        }
                        if results.is_empty() && query.len() >= 2 {
                            ui.weak("No results found.");
                        }
                    });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let can_confirm = selected.is_some();
                    if ui
                        .add_enabled(can_confirm, egui::Button::new("Confirm"))
                        .clicked()
                    {
                        confirmed = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancelled = true;
                    }
                });
            }
        });

    if confirmed {
        if let TagDialog::ModSearch {
            ref results,
            selected: Some(sel_idx),
            console_idx,
            entry_idx,
            ..
        } = app.ui_state.tag_dialog
            && let Some(work) = results.get(sel_idx)
        {
            let work_id = work.id.clone();

            // Create in DB if available
            if let Some(ref conn) = app.catalog_db
                && let Some(console) = app.library.consoles.get(console_idx)
            {
                let platform_id = console.folder_name.as_str();
                let entry_ref = console.entries.get(entry_idx);
                let region = entry_ref
                    .and_then(|e: &crate::state::LibraryEntry| {
                        e.effective_regions().first().map(|r| r.code().to_string())
                    })
                    .unwrap_or_else(|| "unknown".to_string());

                // Convert hashes if available
                let hashes =
                    entry_ref
                        .and_then(|e| e.hashes.as_ref())
                        .map(|h| retro_junk_db::MediaHashes {
                            crc32: h.crc32.clone(),
                            sha1: h.sha1.clone(),
                            md5: h.md5.clone(),
                            file_size: h.data_size as i64,
                        });

                if let Err(e) = retro_junk_db::create_modded_media(
                    conn,
                    &work_id,
                    platform_id,
                    &region,
                    hashes.as_ref(),
                ) {
                    log::error!("Failed to create modded media: {e}");
                    app.push_error(
                        "Database Error",
                        format!("Failed to create modded media: {e}"),
                    );
                }
            }

            // Set tag on the entry
            if let Some(entry) = app
                .library
                .consoles
                .get_mut(console_idx)
                .and_then(|c| c.entries.get_mut(entry_idx))
            {
                entry.tag = Some(CatalogTag::Modded);
            }
            app.save_entry_cache(console_idx, &[entry_idx]);
        }
        app.ui_state.tag_dialog = TagDialog::None;
    } else if cancelled {
        app.ui_state.tag_dialog = TagDialog::None;
    }
}
