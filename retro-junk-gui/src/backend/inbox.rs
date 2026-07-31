//! Loading and resolving the review inbox.
//!
//! The inbox shows three things the user is expected to decide about:
//! open suggestions (proposed-but-unapplied commands), incoming packages
//! whose pre-processing ended in an honest error state, and the count of
//! unresolved catalog disagreements.
//!
//! Applying goes through `retro_junk_work::apply_suggestion` and dismissing
//! through `retro_junk_db::work::resolve_suggestion` — the same calls
//! `retro-junk suggestions apply|dismiss` makes, so a decision taken here and
//! one taken at the terminal are the same decision.

use retro_junk_db::work::{IncomingPackage, Suggestion};
use retro_junk_work::ScrapeSuggestionPayload;
use retro_junk_work::incoming::{IMPORT_SUGGESTION_KIND, ImportSuggestionPayload};

use crate::app::RetroJunkApp;
use crate::state::{AppMessage, OperationKind, ProgressDisplay};

/// One reviewable suggestion with its payload already rendered to lines —
/// parsing JSON per frame for every visible card would be wasteful and the
/// payload never changes while the suggestion is open.
pub struct InboxItem {
    pub suggestion: Suggestion,
    pub headline: String,
    pub details: Vec<String>,
    /// Whether this kind can be executed from here, per the one dispatch in
    /// `retro_junk_work::suggestions` — so a card never offers a button that
    /// does nothing.
    pub applicable: bool,
}

#[derive(Default)]
pub struct InboxContents {
    pub items: Vec<InboxItem>,
    /// Packages whose pre-processing failed: bad dump, no match, ambiguous.
    pub failed_packages: Vec<IncomingPackage>,
    pub unresolved_disagreements: usize,
}

impl InboxContents {
    /// What the sidebar badge counts: everything awaiting a decision.
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.items.len() + self.failed_packages.len()
    }
}

/// Load the inbox off the render thread.
pub fn load(app: &mut RetroJunkApp, ctx: &egui::Context) {
    let Some(db_path) = app.db_path.clone() else {
        return;
    };
    if app.ui_state.inbox_loading {
        return;
    }
    app.ui_state.inbox_loading = true;
    let sender = app.message_tx.clone();
    let repaint = ctx.clone();
    std::thread::spawn(move || {
        let result = retro_junk_db::open_database(&db_path)
            .map_err(|error| error.to_string())
            .and_then(|connection| {
                let items = retro_junk_db::work::list_open_suggestions(&connection, None)
                    .map_err(|error| error.to_string())?
                    .into_iter()
                    .map(describe)
                    .collect();
                let failed_packages =
                    retro_junk_db::work::list_incoming_packages(&connection, Some("error"))
                        .map_err(|error| error.to_string())?;
                let unresolved_disagreements = retro_junk_db::list_unresolved_disagreements(
                    &connection,
                    &retro_junk_db::DisagreementFilter::default(),
                )
                .map_err(|error| error.to_string())?
                .len();
                Ok(InboxContents {
                    items,
                    failed_packages,
                    unresolved_disagreements,
                })
            });
        let _ = sender.send(AppMessage::InboxReady { result });
        repaint.request_repaint();
    });
}

/// Turn a stored suggestion into what the card shows.
fn describe(suggestion: Suggestion) -> InboxItem {
    let applicable = retro_junk_work::is_applicable(&suggestion.kind);
    let (headline, details) = match suggestion.kind.as_str() {
        IMPORT_SUGGESTION_KIND => describe_import(&suggestion),
        retro_junk_work::SCRAPE_SUGGESTION_KIND => describe_scrape(&suggestion),
        "adopt_playable" => describe_adoption(&suggestion),
        _ => (suggestion.target_id.clone(), Vec::new()),
    };
    InboxItem {
        suggestion,
        headline,
        details,
        applicable,
    }
}

fn describe_import(suggestion: &Suggestion) -> (String, Vec<String>) {
    let Ok(payloads) =
        serde_json::from_str::<Vec<ImportSuggestionPayload>>(&suggestion.payload_json)
    else {
        return (suggestion.target_id.clone(), Vec::new());
    };
    let headline = payloads.first().map_or_else(
        || suggestion.target_id.clone(),
        |payload| {
            if payload.title.trim().is_empty() {
                suggestion.target_id.clone()
            } else if payload.region.trim().is_empty() {
                payload.title.clone()
            } else {
                format!("{} ({})", payload.title, payload.region)
            }
        },
    );
    let mut details = vec![suggestion.target_id.clone()];
    for payload in &payloads {
        details.push(format!(
            "{} — {} · {}",
            payload.platform_id, payload.identification, payload.disposition
        ));
        for candidate in &payload.candidates {
            details.push(format!("    candidate: {candidate}"));
        }
        for warning in &payload.warnings {
            details.push(format!("    warning: {warning}"));
        }
    }
    (headline, details)
}

fn describe_scrape(suggestion: &Suggestion) -> (String, Vec<String>) {
    let Ok(payload) = serde_json::from_str::<ScrapeSuggestionPayload>(&suggestion.payload_json)
    else {
        return (suggestion.target_id.clone(), Vec::new());
    };
    let mut details = vec![payload.reason.clone()];
    if !payload.missing.is_empty() {
        details.push(format!("would fetch: {}", payload.missing.join(", ")));
    }
    details.push(format!(
        "{} — {}",
        payload.platform_id, suggestion.target_id
    ));
    (payload.label, details)
}

fn describe_adoption(suggestion: &Suggestion) -> (String, Vec<String>) {
    let value: serde_json::Value =
        serde_json::from_str(&suggestion.payload_json).unwrap_or(serde_json::Value::Null);
    let field = |name: &str| {
        value
            .get(name)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let status = field("status");
    let detail = field("detail");
    let mut details = vec![suggestion.target_id.clone()];
    if !status.is_empty() {
        details.push(status);
    }
    if !detail.is_empty() {
        details.push(detail);
    }
    (
        std::path::Path::new(&suggestion.target_id)
            .file_name()
            .map_or_else(
                || suggestion.target_id.clone(),
                |name| name.to_string_lossy().into_owned(),
            ),
        details,
    )
}

/// Execute a suggestion through the shared dispatch.
pub fn apply(app: &mut RetroJunkApp, id: i64, label: String, ctx: &egui::Context) {
    let exec = match crate::backend::convergence::exec_context(app) {
        Ok(exec) => exec,
        Err(error) => {
            app.push_error("Apply suggestion", error);
            return;
        }
    };
    crate::backend::worker::spawn_background_op(
        app,
        format!("Applying {label}"),
        OperationKind::ArchiveImport,
        "archive".to_owned(),
        ProgressDisplay::Count,
        move |op_id, cancel, sender| {
            let result = retro_junk_work::apply_suggestion(&exec, id, &cancel)
                .map_err(|error| error.to_string());
            let _ = sender.send(AppMessage::ArchiveOperationComplete { op_id, result });
            let _ = sender.send(AppMessage::InboxChanged);
        },
    );
    ctx.request_repaint_after(std::time::Duration::from_millis(20));
}

/// Resolve a suggestion without executing it.
pub fn dismiss(app: &mut RetroJunkApp, id: i64, ctx: &egui::Context) {
    let Some(db_path) = app.db_path.clone() else {
        return;
    };
    let sender = app.message_tx.clone();
    let repaint = ctx.clone();
    std::thread::spawn(move || {
        let outcome = retro_junk_db::open_database(&db_path)
            .map_err(|error| error.to_string())
            .and_then(|mut connection| {
                retro_junk_db::work::resolve_suggestion(&mut connection, id, "dismissed")
                    .map_err(|error| error.to_string())
            });
        if let Err(error) = outcome {
            log::warn!("could not dismiss suggestion {id}: {error}");
        }
        let _ = sender.send(AppMessage::InboxChanged);
        repaint.request_repaint();
    });
}

/// Send a failed package back through pre-processing and run that pass now.
///
/// The daemon would pick a re-queued package up on its next tick, but the
/// GUI should not require a daemon to answer a button press: it runs the
/// same `process_pending_incoming` the daemon runs, under the same policy.
pub fn retry_package(app: &mut RetroJunkApp, package: &IncomingPackage, ctx: &egui::Context) {
    let exec = match crate::backend::convergence::exec_context(app) {
        Ok(exec) => exec,
        Err(error) => {
            app.push_error("Retry package", error);
            return;
        }
    };
    let policy = retro_junk_work::AutomationPolicy::load();
    let path = package.path.clone();
    let label = std::path::Path::new(&path)
        .file_name()
        .map_or_else(|| path.clone(), |name| name.to_string_lossy().into_owned());
    crate::backend::worker::spawn_background_op(
        app,
        format!("Re-processing {label}"),
        OperationKind::ArchiveImport,
        "archive".to_owned(),
        ProgressDisplay::Count,
        move |op_id, cancel, sender| {
            let result = retro_junk_db::open_database(&exec.db_path)
                .map_err(|error| error.to_string())
                .and_then(|mut connection| {
                    retro_junk_db::work::requeue_incoming_package(&mut connection, &path)
                        .map_err(|error| error.to_string())
                })
                .and_then(|()| {
                    retro_junk_work::incoming::process_pending_incoming(&exec, &policy, &cancel)
                        .map_err(|error| error.to_string())
                })
                .map(|stats| {
                    format!(
                        "Re-processed: {} imported, {} awaiting review, {} still failing",
                        stats.imported, stats.suggested, stats.errored
                    )
                });
            let _ = sender.send(AppMessage::ArchiveOperationComplete { op_id, result });
            let _ = sender.send(AppMessage::InboxChanged);
        },
    );
    ctx.request_repaint_after(std::time::Duration::from_millis(20));
}
