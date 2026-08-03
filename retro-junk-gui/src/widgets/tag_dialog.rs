use crate::app::RetroJunkApp;
use crate::state::TagDialog;

/// How many catalog works the manual-tag search offers at once — enough to
/// find the right one, short enough to scan by eye.
const MOD_SEARCH_RESULT_LIMIT: u32 = 20;

/// Whether the row a dialog is about has been hashed yet.
///
/// Content is the only identity a mod or a homebrew title has — nothing else
/// survives a rename or a copy to another machine — so an unhashed row can be
/// tagged but cannot yet be recorded beside the collection. Saying so beats
/// letting the decision quietly fail to travel.
fn entry_is_hashed(
    app: &RetroJunkApp,
    console_id: retro_junk_db::LibraryConsoleId,
    entry_id: retro_junk_db::LibraryEntryId,
) -> bool {
    app.browser
        .find_by_id(console_id)
        .and_then(|console_idx| app.browser.consoles[console_idx].entry_by_id(entry_id))
        .is_some_and(|entry| entry.hashes.is_some())
}

/// The caveat shown for a row with no digests yet.
fn show_unhashed_note(ui: &mut egui::Ui) {
    ui.weak(
        "This file has not been hashed yet, so the tag stays on this machine \
         until it is \u{2014} hash it to have the decision travel with the collection.",
    );
    ui.add_space(4.0);
}

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
    let content_is_known = match app.ui_state.tag_dialog {
        TagDialog::Homebrew {
            console_id,
            entry_id,
            ..
        } => entry_is_hashed(app, console_id, entry_id),
        _ => true,
    };

    let outcome = crate::widgets::modal::show(
        ctx,
        "tag_homebrew_dialog",
        "Mark as Homebrew",
        360.0,
        |ui| {
            ui.label("Enter the homebrew game name:");
            ui.add_space(4.0);
            if !content_is_known {
                show_unhashed_note(ui);
            }

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
            let collection_root = app.collection_root();
            let Some(console_idx) = app.browser.find_by_id(console_id) else {
                app.ui_state.tag_dialog = TagDialog::None;
                return;
            };
            let Some(entry_idx) = app.browser.consoles[console_idx].entry_index(entry_id) else {
                app.ui_state.tag_dialog = TagDialog::None;
                return;
            };

            let console = &app.browser.consoles[console_idx];
            let platform_id = app.context.get_by_platform(console.platform).map_or_else(
                || console.folder_name.clone(),
                |registered| registered.analyzer.short_name().to_owned(),
            );
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
                    collection_root,
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
    let content_is_known = match app.ui_state.tag_dialog {
        TagDialog::ModSearch {
            console_id,
            entry_id,
            ..
        } => entry_is_hashed(app, console_id, entry_id),
        _ => true,
    };

    let outcome = crate::widgets::modal::show(
        ctx,
        "tag_mod_search_dialog",
        "Mark as Modded Version of\u{2026}",
        400.0,
        |ui| {
            ui.label("Search for the original game:");
            ui.add_space(4.0);
            if !content_is_known {
                show_unhashed_note(ui);
            }

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
                            let result = retro_junk_backend::queries::open_catalog(&db_path)
                                .and_then(|connection| {
                                    retro_junk_backend::queries::catalog::works_for_platform(
                                        &connection,
                                        &requested_query,
                                        &requested_platform,
                                        MOD_SEARCH_RESULT_LIMIT,
                                    )
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
                    if !disc_number.trim().is_empty() && parse_disc_number(disc_number).is_none() {
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
            let collection_root = app.collection_root();
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
                        collection_root,
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
