//! Per-row convergence badges: one dot per evidence class.
//!
//! Present · integrity · catalog · playable · artwork, read straight off the
//! `ArchiveReleaseSummary` the Library page already loads — the same counts
//! the Collection roll-up shows, so a row can never disagree with the
//! summary above it. Clicking a dot opens a popover naming what the class
//! means, the last error recorded against it, and a re-run that goes through
//! the shared executor.

#[cfg(test)]
#[path = "evidence_badges_tests.rs"]
mod tests;

use retro_junk_db::convergence::ActionKind;

use crate::app::RetroJunkApp;

const DOT_SPACING: f32 = 11.0;
const DOT_RADIUS: f32 = 3.5;

/// How complete one evidence class is for a release.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EvidenceLevel {
    /// Nothing of this class exists yet.
    Absent,
    /// Some of the expected evidence exists.
    Partial,
    /// Everything expected for this class is present.
    Complete,
    /// The class does not apply to this release (nothing is expected).
    NotApplicable,
}

impl EvidenceLevel {
    /// `have` of `expected`, where zero expected means "not applicable".
    #[must_use]
    pub fn of(have: u64, expected: u64) -> Self {
        match (have, expected) {
            (_, 0) => Self::NotApplicable,
            (0, _) => Self::Absent,
            (have, expected) if have >= expected => Self::Complete,
            _ => Self::Partial,
        }
    }

    fn color(self, ui: &egui::Ui) -> egui::Color32 {
        match self {
            Self::Complete => crate::theme::STATUS_OK,
            Self::Partial => crate::theme::STATUS_WARN_STRONG,
            Self::Absent => ui.visuals().text_color().linear_multiply(0.35),
            Self::NotApplicable => ui.visuals().text_color().linear_multiply(0.12),
        }
    }

    fn describe(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::Absent => "missing",
            Self::NotApplicable => "not expected",
        }
    }
}

/// One evidence class on one release.
#[derive(Clone, Copy)]
struct Class {
    label: &'static str,
    /// What the class asserts, shown in the popover.
    meaning: &'static str,
    level: EvidenceLevel,
    have: u64,
    expected: u64,
    /// The action that produces this evidence, if the tool can produce it.
    action: Option<ActionKind>,
}

/// Derive the five classes from the projected release summary. `artwork`
/// comes from the archived asset list rather than the summary counts,
/// because artwork lives in the archive as files, not as evidence records.
fn classes(release: &retro_junk_db::ArchivedLibraryListItem) -> [Class; 5] {
    let summary = &release.summary;
    [
        Class {
            label: "present",
            meaning: "Preservation masters stored and present on this device",
            level: EvidenceLevel::of(
                summary.preservation_present_count,
                summary.preservation_count,
            ),
            have: summary.preservation_present_count,
            expected: summary.preservation_count,
            // Presence is a fact about the filesystem, not work to run.
            action: None,
        },
        Class {
            label: "integrity",
            meaning: "Stored bytes re-hashed against the recorded dump evidence",
            level: EvidenceLevel::of(summary.integrity_verified_count, summary.carrier_count),
            have: summary.integrity_verified_count,
            expected: summary.carrier_count,
            action: Some(ActionKind::VerifyIntegrity),
        },
        Class {
            label: "catalog",
            meaning: "Every expected disc matched against Redump/No-Intro",
            level: EvidenceLevel::of(summary.verified_disc_count, summary.expected_disc_count),
            have: summary.verified_disc_count,
            expected: summary.expected_disc_count,
            action: Some(ActionKind::VerifyCatalog),
        },
        Class {
            label: "playable",
            meaning: "Preferred playable representation built and present",
            level: EvidenceLevel::of(
                summary.satisfied_playable_count,
                summary.desired_playable_count,
            ),
            have: summary.satisfied_playable_count,
            expected: summary.desired_playable_count,
            action: Some(ActionKind::BuildPlayable),
        },
        Class {
            label: "artwork",
            meaning: "Scraped artwork archived and projected to the frontend",
            level: if release.archived_assets.is_empty() {
                EvidenceLevel::Absent
            } else {
                EvidenceLevel::Complete
            },
            have: release.archived_assets.len() as u64,
            expected: 1,
            action: Some(ActionKind::ProjectAssets),
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
    let classes = classes(release);
    let errors = app
        .ui_state
        .backlog
        .release_errors(&release.summary.archive_release_id);

    let mut request = None;
    for (index, class) in classes.iter().enumerate() {
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(DOT_SPACING, DOT_SPACING),
            egui::Sense::click_and_drag(),
        );
        let error = class
            .action
            .and_then(|kind| errors.iter().find(|(errored, _)| *errored == kind));
        if ui.is_rect_visible(rect) {
            let color = if error.is_some() {
                crate::theme::STATUS_ERR
            } else {
                class.level.color(ui)
            };
            ui.painter().circle_filled(rect.center(), DOT_RADIUS, color);
        }
        let response = response.on_hover_text(summary_line(class, error.is_some()));

        // A distinct id per release keeps two rows' popovers independent
        // even as virtualization recycles row slots.
        let popup_id = egui::Id::new((
            "evidence",
            release.summary.archive_release_id.as_str(),
            index,
        ));
        if let Some(inner) = egui::Popup::menu(&response)
            .id(popup_id)
            .show(|ui| popup_body(ui, release, class, error))
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

fn summary_line(class: &Class, errored: bool) -> String {
    let mut line = format!("{}: {}", class.label, class.level.describe());
    if class.level != EvidenceLevel::NotApplicable {
        use std::fmt::Write as _;
        let _ = write!(line, " ({} of {})", class.have, class.expected);
    }
    if errored {
        line.push_str("\nLast run failed — click for details");
    }
    line
}

/// Returns `true` when the user clicked "Run again".
fn popup_body(
    ui: &mut egui::Ui,
    release: &retro_junk_db::ArchivedLibraryListItem,
    class: &Class,
    error: Option<&(ActionKind, retro_junk_db::work::WorkError)>,
) -> bool {
    ui.set_max_width(340.0);
    ui.strong(format!("{} — {}", release.summary.title, class.label));
    ui.label(class.meaning);
    ui.label(summary_line(class, false));
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
