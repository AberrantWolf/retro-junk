//! Generic results-dialog scaffold shared by every batch operation that
//! reports per-item outcomes (rename, CUE fix, CHD compression).
//!
//! Before this module existed, each caller hand-rolled its own
//! Window/summary/ScrollArea/colored-rows/OK dialog — three near-identical
//! ~100-line copies. `show_results_dialog` extracts the common scaffold;
//! callers only supply the summary line and one row per item.

// Status colors, re-exported for the row-renderer callbacks (the shared
// definitions live in `crate::theme`).
pub use crate::theme::{STATUS_ERR, STATUS_OK, STATUS_WARN};

/// Generic modal listing per-item outcomes of a batch operation.
///
/// `items` holds the batch results (typically from the app's `results_dialog`
/// enum). Returns `true` when the user dismisses the dialog (closes the
/// window or clicks OK) — the caller then clears its results state.
/// `summary` renders the header line from the full result slice;
/// `row` renders one line per item inside a scrolling area.
pub fn show_results_dialog<T>(
    ctx: &egui::Context,
    title: &str,
    items: &[T],
    summary: impl Fn(&[T]) -> String,
    row: impl Fn(&mut egui::Ui, &T),
) -> bool {
    let outcome = crate::widgets::modal::show(ctx, "results_dialog", title, 500.0, |ui| {
        ui.label(summary(items));
        ui.add_space(4.0);

        egui::ScrollArea::vertical()
            .max_height(400.0)
            .show(ui, |ui| {
                for item in items {
                    ui.horizontal(|ui| row(ui, item));
                }
            });

        crate::widgets::modal::footer(ui, |ui| ui.button("OK").clicked())
    });

    outcome.inner || outcome.dismissed
}

#[cfg(test)]
#[path = "results_dialog_tests.rs"]
mod tests;
