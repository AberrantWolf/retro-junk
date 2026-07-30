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

    let outcome = crate::widgets::modal::show(
        ctx,
        "tag_homebrew_dialog",
        "Mark as Homebrew",
        360.0,
        |ui| {
            ui.label("Enter the homebrew game name:");
            ui.add_space(4.0);

            if let TagDialog::Homebrew { ref mut name, .. } = app.ui_state.tag_dialog {
                let resp = ui.text_edit_singleline(name);
                // Auto-focus the text field on first frame
                if !resp.has_focus() {
                    resp.request_focus();
                }

                crate::widgets::modal::footer(ui, |ui| {
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
        },
    );
    cancelled |= outcome.dismissed;

    if confirmed {
        if let TagDialog::Homebrew {
            ref name,
            console_id,
            entry_id,
        } = app.ui_state.tag_dialog
        {
            let name = name.clone();
            let Some(console_idx) = app.browser.find_by_id(console_id) else {
                app.ui_state.tag_dialog = TagDialog::None;
                return;
            };
            let Some(entry_idx) = app.browser.consoles[console_idx].entry_index(entry_id) else {
                app.ui_state.tag_dialog = TagDialog::None;
                return;
            };

            let console = &app.browser.consoles[console_idx];
            let platform_id = app
                .context
                .get_by_platform(console.platform)
                .map(|registered| registered.analyzer.short_name().to_owned())
                .unwrap_or_else(|| console.folder_name.clone());
            let region = console.entries[entry_idx]
                .effective_regions()
                .first()
                .map_or_else(|| "unknown".to_owned(), |region| region.code().to_owned());
            app.submit_store(
                crate::backend::library_store::LibraryStoreRequest::CreateHomebrewAndTag {
                    entry_id,
                    name,
                    platform_id,
                    region,
                },
                ctx,
            );
        }
        app.ui_state.tag_dialog = TagDialog::None;
    } else if cancelled {
        app.ui_state.tag_dialog = TagDialog::None;
    }
}

fn show_mod_search_dialog(ctx: &egui::Context, app: &mut RetroJunkApp) {
    let mut cancelled = false;
    let mut confirmed = false;

    let outcome = crate::widgets::modal::show(
        ctx,
        "tag_mod_search_dialog",
        "Mark as Modded Version of\u{2026}",
        400.0,
        |ui| {
            ui.label("Search for the original game:");
            ui.add_space(4.0);

            let mut query_changed = false;
            if let TagDialog::ModSearch {
                ref mut query,
                ref mut results,
                ref mut selected,
                ref platform_id,
                disc_number_required,
                ref mut disc_number,
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
                    results.clear();
                    if let Some(db_path) = app.db_path.clone() {
                        let requested_query = query.clone();
                        let requested_platform = platform_id.clone();
                        let tx = app.message_tx.clone();
                        let repaint = ctx.clone();
                        std::thread::spawn(move || {
                            let result = retro_junk_db::open_database(&db_path)
                                .map_err(|error| error.to_string())
                                .and_then(|connection| {
                                    retro_junk_db::search_works_for_platform(
                                        &connection,
                                        &requested_query,
                                        &requested_platform,
                                        20,
                                    )
                                    .map_err(|error| error.to_string())
                                });
                            let _ = tx.send(crate::state::AppMessage::ModSearchResults {
                                query: requested_query,
                                result,
                            });
                            repaint.request_repaint();
                        });
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
                if disc_number_required {
                    ui.horizontal(|ui| {
                        ui.label("Disc number:");
                        ui.add(
                            egui::TextEdit::singleline(disc_number)
                                .desired_width(50.0)
                                .hint_text("1"),
                        );
                    });
                    ui.weak("Required because the selected playable entry is one disc image.");
                    if !disc_number.trim().is_empty()
                        && parse_disc_number(disc_number).is_none()
                    {
                        ui.colored_label(
                            ui.visuals().error_fg_color,
                            "Enter a positive whole number.",
                        );
                    }
                    ui.add_space(8.0);
                } else {
                    ui.weak(
                        "The selected playable entry is a game folder, so this tags the game as a whole.",
                    );
                    ui.add_space(8.0);
                }
                crate::widgets::modal::footer(ui, |ui| {
                    let can_confirm = selected.is_some()
                        && (!disc_number_required || parse_disc_number(disc_number).is_some());
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
        },
    );
    cancelled |= outcome.dismissed;

    if confirmed {
        if let TagDialog::ModSearch {
            ref results,
            selected: Some(sel_idx),
            ref platform_id,
            disc_number_required,
            ref disc_number,
            console_id,
            entry_id,
            ..
        } = app.ui_state.tag_dialog
            && let Some(work) = results.get(sel_idx)
        {
            let work_id = work.id.clone();
            let Some(console_idx) = app.browser.find_by_id(console_id) else {
                app.ui_state.tag_dialog = TagDialog::None;
                return;
            };
            let Some(entry_idx) = app.browser.consoles[console_idx].entry_index(entry_id) else {
                app.ui_state.tag_dialog = TagDialog::None;
                return;
            };

            if let Some(console) = app.browser.consoles.get(console_idx) {
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

                app.submit_store(
                    crate::backend::library_store::LibraryStoreRequest::CreateModdedAndTag {
                        entry_id,
                        work_id,
                        platform_id: platform_id.clone(),
                        region,
                        disc_number: if disc_number_required {
                            parse_disc_number(disc_number)
                        } else {
                            None
                        },
                        hashes,
                    },
                    ctx,
                );
            }
        }
        app.ui_state.tag_dialog = TagDialog::None;
    } else if cancelled {
        app.ui_state.tag_dialog = TagDialog::None;
    }
}

fn parse_disc_number(value: &str) -> Option<u32> {
    value
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|number| *number > 0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn disc_number_must_be_a_positive_integer() {
        assert_eq!(super::parse_disc_number(" 2 "), Some(2));
        assert_eq!(super::parse_disc_number(""), None);
        assert_eq!(super::parse_disc_number("0"), None);
        assert_eq!(super::parse_disc_number("-1"), None);
        assert_eq!(super::parse_disc_number("disc 2"), None);
    }
}
