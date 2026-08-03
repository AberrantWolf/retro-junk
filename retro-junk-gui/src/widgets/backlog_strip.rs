//! Backlog summary strip: what convergence still owes the current scope.
//!
//! One chip per action kind with outstanding work, from the same
//! `summarize_convergence` aggregation behind `retro-junk status` and the
//! daemon's logging. Kinds with nothing outstanding are omitted rather than
//! shown as zeros, so a converged library reads as one short line instead of
//! six empty counters.

use retro_junk_db::convergence::{ActionKind, KindCounts};

use crate::app::RetroJunkApp;
use crate::backend::convergence::kind_label;
use crate::widgets::icons;

/// Draw the strip. Returns `true` if the user asked to run the backlog.
pub fn show(ui: &mut egui::Ui, app: &RetroJunkApp) -> bool {
    let backlog = &app.ui_state.backlog;
    let outstanding: Vec<(ActionKind, KindCounts)> = backlog
        .summary
        .per_kind
        .iter()
        .filter(|(_, counts)| {
            counts.pending > 0
                || counts.blocked > 0
                || counts.errored > 0
                || counts.running > 0
                // Tried and unresolved is not pending work, but it is not
                // "up to date" either — hiding it would quietly drop every
                // disc the catalog cannot name.
                || counts.unresolved > 0
        })
        .map(|(kind, counts)| (*kind, *counts))
        .collect();

    let mut run_requested = false;
    ui.horizontal_wrapped(|ui| {
        ui.strong("Backlog:");
        if outstanding.is_empty() {
            if app.ui_state.backlog_loading {
                ui.spinner();
            } else {
                ui.colored_label(crate::theme::STATUS_OK, "up to date")
                    .on_hover_text(
                        "Every archived release in this scope is verified, built, and projected.",
                    );
            }
            return;
        }
        for (kind, counts) in &outstanding {
            chip(ui, *kind, counts);
        }
        if ui
            .button(icons::labeled(icons::RESCAN, "Converge"))
            .on_hover_text(
                "Run the pending actions above through the same executor the \
                 daemon and `retro-junk sync` use",
            )
            .clicked()
        {
            run_requested = true;
        }
        if app.ui_state.backlog_loading {
            ui.spinner();
        }
    });
    run_requested
}

fn chip(ui: &mut egui::Ui, kind: ActionKind, counts: &KindCounts) {
    let label = kind_label(kind);
    // Running work is the most informative state, then failures, then
    // blocked, then plain pending — one chip shows the most urgent of them.
    let (text, color) = if counts.running > 0 {
        (
            format!("{label} {} running", counts.running),
            crate::theme::STATUS_OK,
        )
    } else if counts.errored > 0 {
        (
            format!("{label} {} failed", counts.errored),
            crate::theme::STATUS_ERR,
        )
    } else if counts.blocked > 0 {
        (
            format!("{label} {} blocked", counts.blocked),
            crate::theme::STATUS_WARN,
        )
    } else if counts.pending > 0 {
        (
            format!("{label} {}", counts.pending),
            ui.visuals().text_color(),
        )
    } else {
        (
            format!("{label} {} unresolved", counts.unresolved),
            crate::theme::STATUS_WARN,
        )
    };
    egui::Frame::NONE
        .inner_margin(egui::Margin::symmetric(6, 1))
        .corner_radius(4.0)
        .fill(ui.visuals().faint_bg_color)
        .show(ui, |ui| {
            ui.colored_label(color, text).on_hover_text(format!(
                "{} done, {} pending, {} blocked, {} failed, {} running\n\
                 {} tried and matched no single catalog medium; re-running is \
                 a deliberate act, since reproducing a disc is expensive",
                counts.done,
                counts.pending,
                counts.blocked,
                counts.errored,
                counts.running,
                counts.unresolved,
            ));
        });
}
