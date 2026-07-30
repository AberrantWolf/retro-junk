//! One modal scaffold for every dialog in the app.
//!
//! Each dialog used to hand-roll `egui::Window` with its own `open` flag,
//! anchor, collapsible/resizable settings, and (inconsistent) Escape
//! handling — seven near-identical preambles that drifted apart. This module
//! owns modal chrome exactly once: backdrop dim, input blocking, centre
//! anchoring, and dismissal via Escape or a backdrop click. A dialog body is
//! then only its content and its own buttons.
//!
//! `egui::Modal` (egui 0.30+) provides the backdrop and focus trap;
//! [`ModalResponse::should_close`] folds Escape and backdrop clicks into one
//! signal, which [`Dismissal`] re-exposes so callers keep a single
//! "the user closed this" branch instead of separate `open`/`dismiss` flags.

#[cfg(test)]
#[path = "modal_tests.rs"]
mod tests;

/// What a dialog produced this frame.
pub struct DialogOutcome<R> {
    /// Whatever the content closure returned.
    pub inner: R,
    /// The user dismissed the dialog (Escape, or a click on the backdrop).
    pub dismissed: bool,
}

/// Render a titled modal dialog.
///
/// `id` must be stable across frames — it identifies the modal's area and
/// therefore its stacking order among nested modals. `width` fixes the
/// content width so long messages wrap instead of stretching the dialog to
/// the window edge; bodies that can grow without bound are expected to wrap
/// themselves in a height-capped [`egui::ScrollArea`].
pub fn show<R>(
    ctx: &egui::Context,
    id: &str,
    title: &str,
    width: f32,
    contents: impl FnOnce(&mut egui::Ui) -> R,
) -> DialogOutcome<R> {
    let response = egui::Modal::new(egui::Id::new(id)).show(ctx, |ui| {
        ui.set_width(width);
        ui.heading(title);
        ui.separator();
        contents(ui)
    });
    DialogOutcome {
        dismissed: response.should_close(),
        inner: response.inner,
    }
}

/// A right-aligned footer button row, separated from the body above it.
///
/// Every dialog ends this way; keeping the separator and spacing here stops
/// each one from choosing its own.
pub fn footer<R>(ui: &mut egui::Ui, buttons: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.separator();
    ui.horizontal(|ui| buttons(ui)).inner
}
