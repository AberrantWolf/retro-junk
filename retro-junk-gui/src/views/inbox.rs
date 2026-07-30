//! Review inbox: everything waiting on a decision, as one-click cards.
//!
//! Automation is allowed to act on what it is confident about; everything
//! else lands here rather than happening silently or being lost in a log.
//! Three sources, one list: open suggestions (proposed-but-unapplied
//! commands), incoming packages whose pre-processing ended in an honest
//! error state, and a pointer to unresolved catalog disagreements.

use crate::app::RetroJunkApp;
use crate::backend::inbox::InboxItem;
use crate::widgets::icons;

/// What the user clicked, applied after the list finishes rendering so the
/// card loop keeps its borrow of the loaded contents.
enum Action {
    Apply { id: i64, label: String },
    Dismiss(i64),
    Retry(retro_junk_db::work::IncomingPackage),
}

pub fn show(ui: &mut egui::Ui, app: &mut RetroJunkApp, ctx: &egui::Context) {
    if std::mem::take(&mut app.ui_state.inbox_dirty) {
        crate::backend::inbox::load(app, ctx);
    }

    ui.horizontal(|ui| {
        ui.heading("Inbox");
        if app.ui_state.inbox_loading {
            ui.spinner();
        }
        if ui
            .button(icons::labeled(icons::RESCAN, "Refresh"))
            .clicked()
        {
            crate::backend::inbox::load(app, ctx);
        }
    });
    ui.separator();

    let mut action = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let inbox = &app.ui_state.inbox;
            if inbox.pending_count() == 0 {
                ui.add_space(24.0);
                ui.vertical_centered(|ui| {
                    ui.colored_label(crate::theme::STATUS_OK, "Nothing needs review.");
                    ui.weak(
                        "Confident, unambiguous work happens automatically; anything \
                         the tool should not decide for you shows up here.",
                    );
                });
            }

            for item in &inbox.items {
                if let Some(chosen) = suggestion_card(ui, item) {
                    action = Some(chosen);
                }
            }

            if !inbox.failed_packages.is_empty() {
                ui.add_space(8.0);
                ui.strong("Incoming packages that could not be processed");
                for package in &inbox.failed_packages {
                    if let Some(chosen) = package_card(ui, package) {
                        action = Some(chosen);
                    }
                }
            }

            if inbox.unresolved_disagreements > 0 {
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(
                        crate::theme::STATUS_WARN,
                        format!(
                            "{} unresolved catalog disagreement(s)",
                            inbox.unresolved_disagreements
                        ),
                    );
                    ui.weak("— review them under Tools \u{2192} Browse \u{2192} Disagreements.");
                });
            }
        });

    match action {
        Some(Action::Apply { id, label }) => crate::backend::inbox::apply(app, id, label, ctx),
        Some(Action::Dismiss(id)) => crate::backend::inbox::dismiss(app, id, ctx),
        Some(Action::Retry(package)) => crate::backend::inbox::retry_package(app, &package, ctx),
        None => {}
    }
}

fn suggestion_card(ui: &mut egui::Ui, item: &InboxItem) -> Option<Action> {
    let mut action = None;
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.strong(&item.headline);
            ui.weak(format!("({})", item.suggestion.kind));
            ui.weak(confidence_label(item.suggestion.confidence));
        });
        for detail in &item.details {
            ui.weak(detail);
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let apply = ui
                .add_enabled(
                    item.applicable,
                    egui::Button::new(icons::labeled(icons::VERIFY, "Apply")),
                )
                .on_hover_text(
                    "Re-validates the package, then imports it through the same path \
                     `retro-junk suggestions apply` uses",
                )
                .on_disabled_hover_text(
                    "This suggestion records a decision the tool cannot make for you; \
                     resolve it where the files are, then dismiss it",
                );
            if apply.clicked() {
                action = Some(Action::Apply {
                    id: item.suggestion.id,
                    label: item.headline.clone(),
                });
            }
            if ui.button("Dismiss").clicked() {
                action = Some(Action::Dismiss(item.suggestion.id));
            }
            if item.suggestion.target_kind == "path" {
                let path = std::path::Path::new(&item.suggestion.target_id);
                if ui
                    .add_enabled(
                        path.exists(),
                        egui::Button::new(icons::labeled(icons::REVEAL, crate::util::REVEAL_LABEL)),
                    )
                    .clicked()
                {
                    crate::util::reveal_in_file_manager(path);
                }
            }
            ui.weak(format!("seen {}", item.suggestion.created_at));
        });
    });
    action
}

fn package_card(
    ui: &mut egui::Ui,
    package: &retro_junk_db::work::IncomingPackage,
) -> Option<Action> {
    let mut action = None;
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.colored_label(crate::theme::STATUS_ERR, icons::WARNING);
            ui.strong(&package.path);
        });
        if !package.detail.is_empty() {
            ui.weak(&package.detail);
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui
                .button(icons::labeled(icons::RESCAN, "Try again"))
                .on_hover_text("Re-runs identification for this package now")
                .clicked()
            {
                action = Some(Action::Retry(package.clone()));
            }
            let path = std::path::Path::new(&package.path);
            if ui
                .add_enabled(
                    path.exists(),
                    egui::Button::new(icons::labeled(icons::REVEAL, crate::util::REVEAL_LABEL)),
                )
                .clicked()
            {
                crate::util::reveal_in_file_manager(path);
            }
            ui.weak(format!("first seen {}", package.first_seen));
        });
    });
    action
}

fn card(ui: &mut egui::Ui, contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style())
        .fill(ui.visuals().faint_bg_color)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            contents(ui);
        });
    ui.add_space(4.0);
}

/// Confidence is stored as the weakest identification in the package; the
/// thresholds mirror `BindConfidence`'s ordering so the wording matches what
/// the automation policy would have done with it.
fn confidence_label(confidence: f64) -> &'static str {
    if confidence >= 1.0 {
        "exact hash"
    } else if confidence >= 0.7 {
        "header serial"
    } else if confidence >= 0.4 {
        "folder serial"
    } else {
        "unidentified"
    }
}

#[cfg(test)]
#[path = "inbox_tests.rs"]
mod tests;
