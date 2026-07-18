use std::sync::atomic::Ordering;

use retro_junk_lib::util::format_bytes_approx;

use crate::state::{BackgroundOperation, ProgressDisplay};

/// Format a `BackgroundOperation`'s `progress_current/progress_total` pair per
/// its `ProgressDisplay` (D7).
fn format_progress(op: &BackgroundOperation) -> String {
    match op.display {
        ProgressDisplay::Bytes => format!(
            "{} / {}",
            format_bytes_approx(op.progress_current),
            format_bytes_approx(op.progress_total),
        ),
        ProgressDisplay::Percent => format!("{:.0}%", op.progress_fraction() * 100.0),
        ProgressDisplay::Count => format!("{}/{}", op.progress_current, op.progress_total),
    }
}

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
                        .text(format_progress(op)),
                );
            }

            if ui.small_button("Cancel").clicked() {
                op.cancel_token.store(true, Ordering::Relaxed);
            }
        });
    }
}

#[cfg(test)]
#[path = "activity_bar_tests.rs"]
mod tests;
