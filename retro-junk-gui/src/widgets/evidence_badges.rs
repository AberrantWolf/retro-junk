//! Per-row convergence badges: one dot per evidence class.
//!
//! Present · integrity · catalog · playable · artwork, read straight off the
//! `ArchiveReleaseSummary` the Library page already loads — the same counts
//! the Collection roll-up shows, so a row can never disagree with the
//! summary above it. Clicking a dot opens a popover naming what the class
//! means, the last error recorded against it, and a re-run that goes through
//! the shared executor.

use retro_junk_backend::completion::{Completion, Fraction, FractionLevel};
use retro_junk_db::convergence::{ActionKind, BlockedReason};

use crate::app::RetroJunkApp;

const DOT_SPACING: f32 = 11.0;
const DOT_RADIUS: f32 = 3.5;

/// Colors for the fold's levels. The only thing this widget decides is how
/// a level looks — never what a level is.
fn level_color(level: FractionLevel, ui: &egui::Ui) -> egui::Color32 {
    match level {
        FractionLevel::Complete => crate::theme::STATUS_OK,
        FractionLevel::Partial => crate::theme::STATUS_WARN_STRONG,
        FractionLevel::Empty => ui.visuals().text_color().linear_multiply(0.35),
        // Nothing expected, or nothing measurable: dim, with the popover
        // saying which and why.
        FractionLevel::NotApplicable | FractionLevel::Unknown(_) => {
            ui.visuals().text_color().linear_multiply(0.12)
        }
    }
}

/// One evidence class on one release.
#[derive(Clone, Copy)]
struct Class {
    label: &'static str,
    /// What the class asserts, shown in the popover.
    meaning: &'static str,
    fraction: Fraction,
    /// The action that produces this evidence, if the tool can produce it.
    action: Option<ActionKind>,
}

/// The five classes, read straight off the one completion fold.
///
/// Every dot, the detail panel, the Collection roll-up, and the CLI render
/// the same `Completion`, so a row cannot disagree with the summary above it
/// or with what the daemon will actually do.
fn classes(completion: &Completion, missing_playables: u64) -> [Class; 5] {
    [
        Class {
            label: "present",
            meaning: "Preservation masters stored and present on this device",
            fraction: completion.presence,
            // Presence is a fact about the filesystem, not work to run.
            action: None,
        },
        Class {
            label: "integrity",
            meaning: "Stored bytes re-hashed against the recorded dump evidence",
            fraction: completion.integrity,
            action: Some(ActionKind::VerifyIntegrity),
        },
        Class {
            label: "catalog",
            meaning: "Every expected disc matched against Redump/No-Intro",
            fraction: completion.catalog,
            action: Some(ActionKind::VerifyCatalog),
        },
        Class {
            label: "playable",
            meaning: if missing_playables > 0 {
                "A built playable is not where its evidence says — find it before building another"
            } else {
                "Preferred playable representation built and present"
            },
            fraction: completion.playable,
            // A playable that moved is not a missing playable. Rebuilding one
            // writes a second copy beside the file the library already holds,
            // so re-adopt first and let the next pass decide if a build is
            // still owed.
            action: Some(if missing_playables > 0 {
                ActionKind::AdoptPlayable
            } else {
                ActionKind::BuildPlayable
            }),
        },
        Class {
            label: "artwork",
            meaning: "Every expected artwork type archived and projected",
            fraction: completion.artwork,
            // Fetching what is missing is the action; projecting what is
            // already archived cannot make an incomplete set complete.
            action: Some(if completion.artwork.is_complete() {
                ActionKind::ProjectAssets
            } else {
                ActionKind::Scrape
            }),
        },
    ]
}

/// What the user asked for by clicking through a badge popover.
pub struct RerunRequest {
    pub archive_release_id: String,
    pub kind: ActionKind,
    pub label: String,
}

/// Draw the badge strip for one archive release. Returns a re-run request
/// when the user clicked one in a popover.
pub fn show(
    ui: &mut egui::Ui,
    app: &RetroJunkApp,
    release: &retro_junk_db::ArchivedLibraryListItem,
) -> Option<RerunRequest> {
    let completion = Completion::for_release(&release.facts, &app.ui_state.expected_assets);
    let classes = classes(&completion, release.facts.missing_playables);
    let errors = app
        .ui_state
        .backlog
        .release_errors(&release.summary.archive_release_id);
    let blocked = app
        .ui_state
        .backlog
        .release_blocked(&release.summary.archive_release_id);

    let mut request = None;
    for (index, class) in classes.iter().enumerate() {
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(DOT_SPACING, DOT_SPACING),
            egui::Sense::click_and_drag(),
        );
        let error = class
            .action
            .and_then(|kind| errors.iter().find(|(errored, _)| *errored == kind));
        let blocked = class
            .action
            .and_then(|kind| blocked.iter().find(|(blocked, _)| *blocked == kind));
        if ui.is_rect_visible(rect) {
            let color = if error.is_some() {
                crate::theme::STATUS_ERR
            } else if blocked.is_some() {
                crate::theme::STATUS_WARN
            } else {
                level_color(class.fraction.level(), ui)
            };
            ui.painter().circle_filled(rect.center(), DOT_RADIUS, color);
        }
        let response =
            response.on_hover_text(summary_line(class, error.is_some(), blocked.is_some()));

        // A distinct id per release keeps two rows' popovers independent
        // even as virtualization recycles row slots.
        let popup_id = egui::Id::new((
            "evidence",
            release.summary.archive_release_id.as_str(),
            index,
        ));
        if let Some(inner) = egui::Popup::menu(&response)
            .id(popup_id)
            .show(|ui| popup_body(ui, release, class, error, blocked))
            && inner.inner
            && let Some(kind) = class.action
        {
            request = Some(RerunRequest {
                archive_release_id: release.summary.archive_release_id.clone(),
                kind,
                label: release.summary.title.clone(),
            });
        }
    }
    request
}

fn summary_line(class: &Class, errored: bool, blocked: bool) -> String {
    let mut line = format!("{}: {}", class.label, class.fraction.describe());
    if errored {
        line.push_str("\nLast run failed — click for details");
    } else if blocked {
        line.push_str("\nBlocked — click for why");
    }
    line
}

/// Returns `true` when the user clicked "Run again".
fn popup_body(
    ui: &mut egui::Ui,
    release: &retro_junk_db::ArchivedLibraryListItem,
    class: &Class,
    error: Option<&(ActionKind, retro_junk_db::work::WorkError)>,
    blocked: Option<&(ActionKind, BlockedReason)>,
) -> bool {
    ui.set_max_width(340.0);
    ui.strong(format!("{} — {}", release.summary.title, class.label));
    ui.label(class.meaning);
    ui.label(summary_line(class, false, false));
    if let Some((_, error)) = error {
        ui.add_space(4.0);
        ui.colored_label(
            crate::theme::STATUS_ERR,
            crate::widgets::icons::labeled(
                crate::widgets::icons::WARNING,
                &format!("Failed {}", error.occurred_at),
            ),
        );
        ui.label(&error.message);
    }
    if let Some((_, reason)) = blocked {
        ui.add_space(4.0);
        ui.colored_label(
            crate::theme::STATUS_WARN,
            crate::widgets::icons::labeled(crate::widgets::icons::WARNING, "Blocked"),
        );
        ui.label(reason.to_string());
        // The worker skips a blocked action before it ever reaches the
        // executor, so a "run again" here would produce nothing but a
        // second silent no-op — the reason above is the only thing to show.
        return false;
    }
    let Some(kind) = class.action else {
        return false;
    };
    ui.add_space(4.0);
    let label = if error.is_some() {
        "Try again"
    } else {
        "Run again"
    };
    ui.button(crate::widgets::icons::labeled(
        crate::widgets::icons::RESCAN,
        label,
    ))
    .on_hover_text(format!(
        "Runs {} for this release through the same executor the daemon uses",
        kind.as_str()
    ))
    .clicked()
}
