//! Applying a reviewed suggestion.
//!
//! A suggestion is a proposed command, so applying one has to be exactly the
//! user having taken the action themselves. This is the single dispatch from
//! suggestion kind to the implementation that executes it, so
//! `retro-junk suggestions apply` and the GUI Inbox's Apply button cannot
//! drift — and so a card can never appear with a button that does nothing.

use std::sync::atomic::AtomicBool;

use serde::{Deserialize, Serialize};

use crate::adoption::{ADOPT_SUGGESTION_KIND, AdoptionCandidate};
use crate::executor::{ExecContext, WorkError};
use crate::incoming::IMPORT_SUGGESTION_KIND;

/// A release whose artwork match was too weak to publish unattended.
pub const SCRAPE_SUGGESTION_KIND: &str = "scrape";

/// What a scrape suggestion carries for review.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrapeSuggestionPayload {
    pub label: String,
    pub platform_id: String,
    /// Frontend system directory the artwork would land in.
    pub playable_platform_id: String,
    /// Artwork types the release is missing.
    pub missing: Vec<String>,
    /// Why it was not taken automatically.
    pub reason: String,
}

/// Which open reviews a person is currently talking about.
///
/// A backlog of a thousand rows is not reviewed one row at a time; it is
/// reviewed by describing a group — "everything ending `.txt`", "everything
/// under `rvz`" — and acting on the group. This is that description, and it
/// lives here so the filter that decides what a list *shows* is the same one
/// that decides what a bulk button *acts on*. Two implementations would mean a
/// button labelled "dismiss 412" could dismiss a different 412.
///
/// The pattern is matched against the review's target — the incoming package
/// path for an import, the playable-relative path for an adoption review — so
/// both directory-shaped and filename-shaped descriptions work against the
/// same field.
#[derive(Debug, Clone, Default)]
pub struct SuggestionFilter {
    /// Restrict to one kind of review, or all of them.
    pub kind: Option<String>,
    /// Restrict to targets this pattern describes. An empty pattern matches
    /// everything, so an empty filter box means "show all".
    pub pattern: retro_junk_io::glob::Pattern,
}

impl SuggestionFilter {
    #[must_use]
    pub fn new(kind: Option<&str>, pattern: &str) -> Self {
        Self {
            kind: kind
                .map(str::trim)
                .filter(|kind| !kind.is_empty())
                .map(ToOwned::to_owned),
            pattern: retro_junk_io::glob::Pattern::new(pattern.trim()),
        }
    }

    /// Whether this review is one of the ones being talked about.
    #[must_use]
    pub fn matches(&self, suggestion: &retro_junk_db::work::Suggestion) -> bool {
        self.kind
            .as_ref()
            .is_none_or(|kind| suggestion.kind == *kind)
            && self.pattern.matches(&suggestion.target_id)
    }

    /// Narrow a list of reviews to the ones being talked about.
    #[must_use]
    pub fn select(
        &self,
        suggestions: Vec<retro_junk_db::work::Suggestion>,
    ) -> Vec<retro_junk_db::work::Suggestion> {
        suggestions
            .into_iter()
            .filter(|suggestion| self.matches(suggestion))
            .collect()
    }
}

/// What a review surface can offer for one suggestion.
///
/// Derived here rather than at each surface, so a card in the GUI and a row in
/// the terminal agree about what is possible — and so a button never appears
/// that does nothing.
#[derive(Debug, Clone, Default)]
pub struct OfferedActions {
    /// Executable as it stands, with no further input.
    pub applicable: bool,
    /// Answers to choose between before it can be applied. Non-empty means the
    /// surface must ask before it can offer Apply.
    pub choices: Vec<AdoptionCandidate>,
    /// Whether "never file these again" is meaningful — true for reviews about
    /// a path, which is the only thing an ignore rule can describe.
    pub ignorable: bool,
}

/// What can be done about one open suggestion.
#[must_use]
pub fn offered_actions(suggestion: &retro_junk_db::work::Suggestion) -> OfferedActions {
    match suggestion.kind.as_str() {
        IMPORT_SUGGESTION_KIND | SCRAPE_SUGGESTION_KIND => OfferedActions {
            applicable: true,
            ..OfferedActions::default()
        },
        ADOPT_SUGGESTION_KIND => {
            let payload = crate::adoption::read_payload(suggestion);
            OfferedActions {
                // One candidate is not a choice, so it can be applied outright.
                applicable: !payload.candidates.is_empty(),
                choices: if payload.candidates.len() > 1 {
                    payload.candidates
                } else {
                    Vec::new()
                },
                ignorable: true,
            }
        }
        _ => OfferedActions::default(),
    }
}

/// Whether a suggestion of this kind can ever be executed from a review
/// surface.
///
/// A coarse test on the kind alone. Adoption reviews answer "it depends" —
/// only the ones the sweep found candidates for can be applied — so a surface
/// holding an actual suggestion should ask [`offered_actions`] instead.
#[must_use]
pub fn is_applicable(kind: &str) -> bool {
    matches!(
        kind,
        IMPORT_SUGGESTION_KIND | SCRAPE_SUGGESTION_KIND | ADOPT_SUGGESTION_KIND
    )
}

/// Execute an open suggestion and resolve it.
///
/// Returns a human-readable summary of what happened.
pub fn apply_suggestion(
    ctx: &ExecContext,
    id: i64,
    cancelled: &AtomicBool,
) -> Result<String, WorkError> {
    apply_suggestion_choice(ctx, id, None, cancelled)
}

/// Execute an open suggestion, answering any question it asked.
///
/// `choice` names one of the suggestion's candidates; it is only meaningful
/// for reviews that carry several, and is ignored by kinds that propose a
/// single command.
pub fn apply_suggestion_choice(
    ctx: &ExecContext,
    id: i64,
    choice: Option<&str>,
    cancelled: &AtomicBool,
) -> Result<String, WorkError> {
    let conn = retro_junk_db::open_database(&ctx.db_path).map_err(WorkError::msg)?;
    let suggestion = retro_junk_db::work::get_suggestion(&conn, id)?
        .ok_or_else(|| WorkError::Message(format!("no suggestion #{id}")))?;
    if suggestion.resolved_at.is_some() {
        return Err(WorkError::Message(format!(
            "suggestion #{id} was already {}",
            suggestion.resolution
        )));
    }
    drop(conn);

    match suggestion.kind.as_str() {
        IMPORT_SUGGESTION_KIND => {
            crate::incoming::apply_import_suggestion(ctx, id, cancelled)?;
            Ok(format!("Imported {}", suggestion.target_id))
        }
        SCRAPE_SUGGESTION_KIND => apply_scrape(ctx, &suggestion, cancelled),
        ADOPT_SUGGESTION_KIND => {
            crate::adoption::apply_adoption(ctx, &suggestion, choice, cancelled)
        }
        other => Err(WorkError::Message(format!(
            "suggestions of kind '{other}' record a decision rather than an action"
        ))),
    }
}

/// Run the scrape the suggestion proposed.
///
/// The user reviewing the card *is* the confirmation the weak match needed,
/// so this run drops `only_when_unambiguous` — otherwise applying would
/// re-file the same suggestion and the button would appear to do nothing.
fn apply_scrape(
    ctx: &ExecContext,
    suggestion: &retro_junk_db::work::Suggestion,
    cancelled: &AtomicBool,
) -> Result<String, WorkError> {
    let payload: ScrapeSuggestionPayload = serde_json::from_str(&suggestion.payload_json)
        .map_err(|error| WorkError::Message(format!("unreadable scrape suggestion: {error}")))?;

    let mut confirmed = ctx.clone();
    confirmed.scrape.only_when_unambiguous = false;
    // Its own scan: this context outlives one action, and the caller's scan is
    // not ours to share or to invalidate.
    confirmed.archive = crate::executor::ArchiveScan::default();
    // The archive lock is taken by `execute_action`, exactly as for a derived
    // scrape; applying is the same action with the review already done.
    confirmed.scrape.expected_assets =
        retro_junk_frontend::AssetSelection::from_names(&payload.missing);

    let action = retro_junk_db::convergence::ProposedAction {
        kind: retro_junk_db::convergence::ActionKind::Scrape,
        target: retro_junk_db::convergence::WorkTarget::Release(suggestion.target_id.clone()),
        profile_id: confirmed.profile.profile_id.to_string(),
        platform_id: payload.platform_id.clone(),
        playable_platform_id: payload.playable_platform_id.clone(),
        label: payload.label.clone(),
        blocked: None,
        build: None,
    };
    let outcome =
        crate::executor::execute_action(&confirmed, &action, &|_, _, _, _| {}, cancelled)?;
    match outcome {
        crate::executor::ActionOutcome::Completed { .. } => {
            let mut conn = retro_junk_db::open_database(&ctx.db_path).map_err(WorkError::msg)?;
            retro_junk_db::work::resolve_suggestion(&mut conn, suggestion.id, "applied")?;
            Ok(format!("Scraped {}", payload.label))
        }
        crate::executor::ActionOutcome::Blocked(reason)
        | crate::executor::ActionOutcome::Failed(reason) => Err(WorkError::Message(reason)),
        crate::executor::ActionOutcome::Cancelled => Err(WorkError::Cancelled),
        crate::executor::ActionOutcome::ClaimHeld(held) => Err(WorkError::Message(format!(
            "another run holds this release ({})",
            held.owner
        ))),
        crate::executor::ActionOutcome::ArchiveBusy => {
            Err(WorkError::Message("the archive is busy".to_owned()))
        }
    }
}

/// File a weak artwork match for review.
pub fn open_scrape_suggestion(
    conn: &mut retro_junk_db::Connection,
    action: &retro_junk_db::convergence::ProposedAction,
    weak: &crate::scrape::WeakMatch,
) -> Result<(), WorkError> {
    let payload = ScrapeSuggestionPayload {
        label: weak.label.clone(),
        platform_id: action.platform_id.clone(),
        playable_platform_id: action.playable_platform_id.clone(),
        missing: weak.missing.clone(),
        reason: "only a filename match was available".to_owned(),
    };
    let payload_json =
        serde_json::to_string(&payload).map_err(|error| WorkError::Message(error.to_string()))?;
    retro_junk_db::work::open_suggestion(
        conn,
        &retro_junk_db::work::NewSuggestion {
            kind: SCRAPE_SUGGESTION_KIND,
            target_kind: "release",
            target_id: &weak.archive_release_id,
            payload_json: &payload_json,
            // A filename match is the weakest tier the lookup has.
            confidence: 0.4,
            provenance: "convergence",
        },
    )?;
    Ok(())
}
