//! Applying a reviewed suggestion.
//!
//! A suggestion is a proposed command, so applying one has to be exactly the
//! user having taken the action themselves. This is the single dispatch from
//! suggestion kind to the implementation that executes it, so
//! `retro-junk suggestions apply` and the GUI Inbox's Apply button cannot
//! drift — and so a card can never appear with a button that does nothing.

use std::sync::atomic::AtomicBool;

use serde::{Deserialize, Serialize};

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

/// Whether a suggestion of this kind can be executed from a review surface.
///
/// Kinds that record a decision the user still has to make elsewhere are
/// reviewable but not appliable; offering Apply for them would promise an
/// action that does not exist.
#[must_use]
pub fn is_applicable(kind: &str) -> bool {
    matches!(kind, IMPORT_SUGGESTION_KIND | SCRAPE_SUGGESTION_KIND)
}

/// Execute an open suggestion and resolve it.
///
/// Returns a human-readable summary of what happened.
pub fn apply_suggestion(
    ctx: &ExecContext,
    id: i64,
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

    let mut confirmed = ExecContext {
        profile: ctx.profile.clone(),
        db_path: ctx.db_path.clone(),
        tools: ctx.tools.clone(),
        scrape: crate::executor::ScrapeSettings {
            only_when_unambiguous: false,
            ..ctx.scrape.clone()
        },
        roots: ctx.roots.clone(),
        analyzers: std::sync::Arc::clone(&ctx.analyzers),
        owner: ctx.owner.clone(),
        lock: ctx.lock,
        reconcile: ctx.reconcile,
    };
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
    let outcome = crate::executor::execute_action(&confirmed, &action, &|_, _, _| {}, cancelled)?;
    match outcome {
        crate::executor::ActionOutcome::Completed { .. } => {
            let mut conn = retro_junk_db::open_database(&ctx.db_path).map_err(WorkError::msg)?;
            retro_junk_db::work::resolve_suggestion(&mut conn, suggestion.id, "applied")?;
            Ok(format!("Scraped {}", payload.label))
        }
        crate::executor::ActionOutcome::Blocked(reason) => Err(WorkError::Message(reason)),
        crate::executor::ActionOutcome::Cancelled => {
            Err(WorkError::Message("operation cancelled".to_owned()))
        }
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
