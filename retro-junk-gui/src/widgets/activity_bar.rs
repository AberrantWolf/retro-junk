use std::sync::atomic::Ordering;

use retro_junk_lib::util::format_bytes_approx;

use crate::state::{BackgroundOperation, OperationProgress, OperationUnit};

/// Format a `BackgroundOperation`'s `progress_current/progress_total` pair per
/// its `ProgressDisplay` (D7).
fn format_progress(progress: &OperationProgress) -> String {
    match progress {
        OperationProgress::Indeterminate => String::new(),
        OperationProgress::Determinate {
            completed,
            total,
            unit: OperationUnit::Bytes,
        } => format!(
            "{} / {}",
            format_bytes_approx(*completed),
            format_bytes_approx(*total),
        ),
        OperationProgress::Determinate {
            completed,
            total,
            unit: OperationUnit::Items,
        } => {
            format!("{completed}/{total}")
        }
    }
}

/// Render the activity bar showing background operation progress.
pub fn show(ui: &mut egui::Ui, operations: &mut [BackgroundOperation]) {
    for op in operations.iter_mut() {
        ui.horizontal(|ui| {
            // Long-running operations can last for hours. An egui Spinner
            // requests animation frames continuously, forcing the entire
            // immediate-mode window to redraw at display cadence even when
            // progress is unchanged.
            ui.label(egui::RichText::new(&op.title).strong());
            if op.phase.label != op.title {
                ui.weak(&op.phase.label);
            }
            if let Some(step) = op.phase.optional_step {
                ui.weak(format!("Step {}/{}", step.current, step.total));
            }

            match &op.phase.progress {
                OperationProgress::Indeterminate => {
                    // A low-cadence pulse communicates live work without
                    // repainting the whole immediate-mode UI at display rate.
                    let t = ui.input(|input| input.time);
                    let fraction = ((t * 1.5).sin() * 0.35 + 0.5) as f32;
                    ui.add(egui::ProgressBar::new(fraction).desired_width(200.0));
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(250));
                }
                progress @ OperationProgress::Determinate {
                    completed, total, ..
                } => {
                    let fraction = if *total == 0 {
                        0.0
                    } else {
                        *completed as f32 / *total as f32
                    };
                    ui.add(
                        egui::ProgressBar::new(fraction)
                            .desired_width(200.0)
                            .text(format_progress(progress)),
                    );
                }
            }

            if op.cancellable {
                if op.cancel_requested {
                    ui.add_enabled(false, egui::Button::new("Cancelling…"));
                } else if ui.small_button("Cancel").clicked() {
                    op.cancel_requested = true;
                    op.cancel_token.store(true, Ordering::Relaxed);
                }
            }
        });
    }
}

#[cfg(test)]
#[path = "activity_bar_tests.rs"]
mod tests;
