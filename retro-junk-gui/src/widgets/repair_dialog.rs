//! Confirmation dialog for byte-level ROM repair.
//!
//! A repair rewrites files in place, so the user sees what would change and
//! answers before anything is touched. Backups are on by default: the whole
//! point of a repair is that the file is nearly right already, and an
//! unrecoverable mistake would cost the dump.

use crate::app::RetroJunkApp;

pub fn show(ctx: &egui::Context, app: &mut RetroJunkApp) {
    if app.ui_state.repair_prompt.is_none() {
        return;
    }
    let mut start = false;
    let mut close = false;
    let dismissed = {
        let (folder_name, prompt) = app.ui_state.repair_prompt.as_ref().unwrap();
        let folder_name = folder_name.clone();
        let prompt = prompt.clone();
        crate::widgets::modal::show(ctx, "repair_dialog", "Repair files", 560.0, |ui| {
            ui.label(format!(
                "{} file(s) in {folder_name} differ from the catalog only in padding — \
                 trailing or leading bytes some tool added or trimmed. The game data is \
                 intact, but the file will never match a catalog entry until those bytes \
                 are restored.",
                prompt.repairable
            ));
            ui.add_space(6.0);
            if !prompt.sample.is_empty() {
                ui.label(egui::RichText::new("For example:").strong());
                for line in &prompt.sample {
                    ui.label(format!("  {line}"));
                }
                if prompt.repairable > prompt.sample.len() {
                    ui.weak(format!(
                        "  …and {} more",
                        prompt.repairable - prompt.sample.len()
                    ));
                }
                ui.add_space(6.0);
            }
            ui.weak(format!(
                "{} file(s) already match; {} match no catalog entry and are left alone.",
                prompt.already_correct, prompt.no_match
            ));
            for skipped in &prompt.skipped {
                ui.weak(skipped);
            }
            ui.add_space(6.0);
            ui.checkbox(
                &mut app.ui_state.repair_create_backup,
                "Keep a .bak copy of every file before rewriting it",
            );
            crate::widgets::modal::footer(ui, |ui| {
                if ui
                    .button(crate::widgets::icons::labeled(
                        crate::widgets::icons::RESCAN,
                        "Repair",
                    ))
                    .clicked()
                {
                    start = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        })
        .dismissed
    };

    if start {
        let (folder_name, _) = app.ui_state.repair_prompt.take().unwrap();
        let create_backup = app.ui_state.repair_create_backup;
        crate::backend::repair::execute_console_repairs(app, folder_name, create_backup);
    } else if close || dismissed {
        app.ui_state.repair_prompt = None;
    }
}
