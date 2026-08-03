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

use retro_junk_db::convergence::{ActionKind, BlockedReason};

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

/// Derive the five classes from the projected release summary.
///
/// `artwork` comes from the archived asset list rather than the summary
/// counts, because artwork lives in the archive as files rather than as
/// evidence records — but it is counted against the same expected set that
/// derivation uses, so "complete" means the same thing in the badge, the
/// backlog strip, and what the daemon will actually do.
fn classes(
    release: &retro_junk_db::ArchivedLibraryListItem,
    expected_assets: &retro_junk_frontend::AssetSelection,
) -> [Class; 5] {
    let summary = &release.summary;
    let held: std::collections::HashSet<retro_junk_frontend::AssetType> = release
        .archived_assets
        .iter()
        .filter_map(|asset| retro_junk_frontend::AssetType::from_archive_name(&asset.asset_type))
        .collect();
    let expected = expected_assets.types.len() as u64;
    let have = expected_assets
        .types
        .iter()
        .filter(|asset_type| held.contains(asset_type))
        .count() as u64;
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
            meaning: if summary.playable_missing_count > 0 {
                "A built playable is not where its evidence says — find it before building another"
            } else {
                "Preferred playable representation built and present"
            },
            level: EvidenceLevel::of(
                summary.satisfied_playable_count,
                summary.desired_playable_count,
            ),
            have: summary.satisfied_playable_count,
            expected: summary.desired_playable_count,
            // A playable that moved is not a missing playable. Rebuilding one
            // writes a second copy beside the file the library already holds,
            // so re-adopt first and let the next pass decide if a build is
            // still owed.
            action: Some(if summary.playable_missing_count > 0 {
                ActionKind::AdoptPlayable
            } else {
                ActionKind::BuildPlayable
            }),
        },
        Class {
            label: "artwork",
            meaning: "Every expected artwork type archived and projected",
            level: EvidenceLevel::of(have, expected),
            have,
            expected,
            // Fetching what is missing is the action; projecting what is
            // already archived cannot make an incomplete set complete.
            action: Some(if have >= expected {
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
    let classes = classes(release, &app.ui_state.expected_assets);
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
                class.level.color(ui)
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
    let mut line = format!("{}: {}", class.label, class.level.describe());
    if class.level != EvidenceLevel::NotApplicable {
        use std::fmt::Write as _;
        let _ = write!(line, " ({} of {})", class.have, class.expected);
    }
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
