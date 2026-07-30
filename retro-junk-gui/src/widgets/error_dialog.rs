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

    let outcome = crate::widgets::modal::show(ctx, "error_dialog", "Errors", 500.0, |ui| {
        let count = errors.len();
        ui.label(format!(
            "{} error{} occurred",
            count,
            if count == 1 { "" } else { "s" }
        ));
        ui.add_space(4.0);

        egui::ScrollArea::vertical()
            .max_height(300.0)
            .show(ui, |ui| {
                for error in errors.iter() {
                    ui.horizontal(|ui| {
                        ui.colored_label(crate::theme::STATUS_ERR, &error.category);
                        ui.label(&error.message);
                    });
                }
            });

        crate::widgets::modal::footer(ui, |ui| ui.button("Dismiss").clicked())
    });

    if outcome.inner || outcome.dismissed {
        errors.clear();
    }
}
