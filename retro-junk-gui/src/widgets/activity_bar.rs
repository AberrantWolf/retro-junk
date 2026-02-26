use std::sync::atomic::Ordering;

use crate::state::BackgroundOperation;

/// Render the activity bar showing background operation progress.
pub fn show(ui: &mut egui::Ui, operations: &mut [BackgroundOperation]) {
    for op in operations.iter() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(&op.description);

            if op.progress_total > 0 {
                let fraction = op.progress_fraction();
                ui.add(
                    egui::ProgressBar::new(fraction)
                        .desired_width(200.0)
                        .text(format!("{}/{}", op.progress_current, op.progress_total)),
                );
            }

            if ui.small_button("Cancel").clicked() {
                op.cancel_token.store(true, Ordering::Relaxed);
            }
        });
    }
}
