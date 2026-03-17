//! Modal error dialog that shows accumulated user-visible errors.

#[cfg(test)]
#[path = "error_dialog_tests.rs"]
mod tests;

use crate::state::UserError;

/// Render the error dialog. Shows when `errors` is non-empty; dismissed by
/// clearing the list.
pub fn show(ctx: &egui::Context, errors: &mut Vec<UserError>) {
    if errors.is_empty() {
        return;
    }

    let mut dismiss = false;
    let mut open = true;

    egui::Window::new("Errors")
        .collapsible(false)
        .resizable(true)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .default_width(500.0)
        .show(ctx, |ui| {
            let count = errors.len();
            ui.label(format!(
                "{} error{} occurred",
                count,
                if count == 1 { "" } else { "s" }
            ));
            ui.separator();

            egui::ScrollArea::vertical()
                .max_height(300.0)
                .show(ui, |ui| {
                    for error in errors.iter() {
                        ui.horizontal(|ui| {
                            ui.colored_label(egui::Color32::from_rgb(220, 50, 50), &error.category);
                            ui.label(&error.message);
                        });
                    }
                });

            ui.separator();
            if ui.button("Dismiss").clicked() {
                dismiss = true;
            }
        });

    if dismiss || !open {
        errors.clear();
    }
}
