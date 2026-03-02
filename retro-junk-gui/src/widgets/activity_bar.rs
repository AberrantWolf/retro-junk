use std::sync::atomic::Ordering;

use retro_junk_lib::util::format_bytes_approx;

use crate::state::BackgroundOperation;

/// Render the activity bar showing background operation progress.
pub fn show(ui: &mut egui::Ui, operations: &mut [BackgroundOperation]) {
    for op in operations.iter() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(&op.description);

            if op.progress_total > 0 {
                let fraction = op.progress_fraction();
                let text = if op.progress_is_bytes {
                    format!(
                        "{} / {}",
                        format_bytes_approx(op.progress_current),
                        format_bytes_approx(op.progress_total),
                    )
                } else {
                    format!("{}/{}", op.progress_current, op.progress_total)
                };
                ui.add(
                    egui::ProgressBar::new(fraction)
                        .desired_width(200.0)
                        .text(text),
                );
            }

            if ui.small_button("Cancel").clicked() {
                op.cancel_token.store(true, Ordering::Relaxed);
            }
        });
    }
}
