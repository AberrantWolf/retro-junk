//! Transient completion feedback, stacked above the status bar.
//!
//! Toasts are for work that finished successfully and has no other surface:
//! a build, an artwork restore, an organize run. Anything the user must act
//! on stays an error-modal entry (`RetroJunkApp::push_error`) — a message
//! that vanishes on its own is the wrong place for a problem.
//!
//! Written in-house rather than pulled from `egui-notify`, which is still
//! built against egui 0.34 and would drag a second copy of egui into the
//! binary.

use std::time::{Duration, Instant};

#[cfg(test)]
#[path = "toasts_tests.rs"]
mod tests;

/// How long a toast stays fully opaque before fading out.
const HOLD: Duration = Duration::from_secs(4);
/// Fade-out length; the toast is removed once it elapses.
const FADE: Duration = Duration::from_millis(400);

struct Toast {
    message: String,
    raised: Instant,
}

impl Toast {
    /// Opacity in `0.0..=1.0`, or `None` once the toast has expired.
    fn opacity(&self, now: Instant) -> Option<f32> {
        let age = now.saturating_duration_since(self.raised);
        if age <= HOLD {
            return Some(1.0);
        }
        let fading = age.saturating_sub(HOLD);
        (fading < FADE).then(|| 1.0 - fading.as_secs_f32() / FADE.as_secs_f32())
    }
}

/// A queue of live toasts. Held by the app and drawn once per frame.
#[derive(Default)]
pub struct Toasts {
    items: Vec<Toast>,
}

impl Toasts {
    /// Raise a success toast. Repeating the same message refreshes the
    /// existing one instead of stacking duplicates — batch operations that
    /// report per-item completion should not bury the screen.
    pub fn success(&mut self, message: impl Into<String>) {
        let message = message.into();
        let raised = Instant::now();
        if let Some(existing) = self.items.iter_mut().find(|item| item.message == message) {
            existing.raised = raised;
            return;
        }
        self.items.push(Toast { message, raised });
    }

    /// Drop expired toasts. Returns `true` while any remain, so the caller
    /// knows to keep repainting.
    fn retain_live(&mut self, now: Instant) -> bool {
        self.items.retain(|item| item.opacity(now).is_some());
        !self.items.is_empty()
    }

    /// Draw the stack in the bottom-right corner. Clicking a toast dismisses
    /// it immediately.
    pub fn show(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if !self.retain_live(now) {
            return;
        }
        // Sit clear of the status bar / activity bar along the bottom edge.
        let mut offset = egui::vec2(-12.0, -34.0);
        let mut dismissed = None;
        for (index, item) in self.items.iter().enumerate().rev() {
            let Some(opacity) = item.opacity(now) else {
                continue;
            };
            let response = egui::Area::new(egui::Id::new(("toast", index)))
                .anchor(egui::Align2::RIGHT_BOTTOM, offset)
                .order(egui::Order::Foreground)
                .interactable(true)
                .show(ctx, |ui| {
                    ui.set_opacity(opacity);
                    egui::Frame::popup(ui.style())
                        .fill(ui.visuals().panel_fill)
                        .stroke(egui::Stroke::new(1.0, crate::theme::STATUS_OK))
                        .show(ui, |ui| {
                            ui.set_max_width(340.0);
                            ui.horizontal(|ui| {
                                ui.colored_label(
                                    crate::theme::STATUS_OK,
                                    crate::widgets::icons::SUCCESS,
                                );
                                ui.label(&item.message);
                            });
                        });
                })
                .response;
            if response.interact(egui::Sense::click()).clicked() {
                dismissed = Some(index);
            }
            offset.y -= response.rect.height() + 6.0;
        }
        if let Some(index) = dismissed {
            self.items.remove(index);
        }
        // Drive the fade and the eventual removal without a busy loop.
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}
