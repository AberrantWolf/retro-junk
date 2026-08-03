//! Loading and resolving the review inbox.
//!
//! The inbox shows three things the user is expected to decide about:
//! open suggestions (proposed-but-unapplied commands), incoming packages
//! whose pre-processing ended in an honest error state, and the count of
//! unresolved catalog disagreements.
//!
//! Applying goes through `retro_junk_work::apply_suggestion_choice`,
//! dismissing through `retro_junk_db::work::resolve_suggestions`, and ignoring
//! through `retro_junk_work::ignore_playables` — the same calls
//! `retro-junk suggestions apply|dismiss|ignore` make, so a decision taken
//! here and one taken at the terminal are the same decision.
//!
//! Everything a row needs to *render* is computed once, here, off the render
//! thread: its display name, its group, what actions it offers, and whether
//! its file is still on disk. A card that asked the filesystem whether its
//! path exists while drawing would issue one `stat` per row per frame — sixty
//! thousand a second on a thousand-row backlog, against a library that is
//! often on a network share.

use retro_junk_db::work::{IncomingPackage, Suggestion};
use retro_junk_work::ScrapeSuggestionPayload;
use retro_junk_work::incoming::{IMPORT_SUGGESTION_KIND, ImportSuggestionPayload};
use retro_junk_work::suggestions::{OfferedActions, SuggestionFilter};

use crate::app::RetroJunkApp;
use crate::state::{AppMessage, OperationKind, ProgressDisplay};

/// One reviewable suggestion, rendered to everything a row shows.
///
/// Parsing JSON per frame for every visible card would be wasteful and the
/// payload never changes while the suggestion is open.
pub struct InboxItem {
    pub suggestion: Suggestion,
    /// What this row is called: a file name, or a game's title.
    pub headline: String,
    /// Where it lives, shown dimmed beside the name. Empty when the headline
    /// is already the whole story.
    pub location: String,
    /// A word for why it is here: `unmatched`, `ambiguous`, `ready`.
    pub status: String,
    pub details: Vec<String>,
    /// What can be done about it, decided by the one dispatch in
    /// `retro_junk_work::suggestions` — so a row never offers a button that
    /// does nothing.
    pub actions: OfferedActions,
    /// Whether the target file was on disk when this loaded.
    pub exists: bool,
    /// The pile this row belongs to.
    pub group: String,
}

#[derive(Default)]
pub struct InboxContents {
    pub items: Vec<InboxItem>,
    /// Packages whose pre-processing failed: bad dump, no match, ambiguous.
    pub failed_packages: Vec<IncomingPackage>,
    pub unresolved_disagreements: usize,
    /// How many open reviews there are of each kind, for the filter chips.
    pub counts_by_kind: Vec<(String, usize)>,
    /// Ignore rules in force, so the view can show and revoke them.
    pub ignore_rules: Vec<retro_junk_archive::IgnoreRule>,
    /// The playable root these reviews are relative to, so a row can be
    /// revealed in the file manager without re-resolving the profile.
    pub playable_root: Option<std::path::PathBuf>,
}

impl InboxContents {
    /// What the sidebar badge counts: everything awaiting a decision.
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.items.len() + self.failed_packages.len()
    }

    /// The rows a filter is currently talking about, in the requested order.
    ///
    /// Borrowed rather than cloned: this runs every frame over every open row,
    /// and the rows themselves never change while a filter is being typed.
    pub fn visible(&self, filter: &SuggestionFilter, sort: InboxSort) -> Vec<&InboxItem> {
        let mut rows: Vec<&InboxItem> = self
            .items
            .iter()
            .filter(|item| filter.matches(&item.suggestion))
            .collect();
        match sort {
            // Ids ascend with creation, and unlike the stored timestamp they
            // break ties, so two rows filed in the same second keep a stable
            // order instead of shuffling between frames.
            InboxSort::Newest => {
                rows.sort_by_key(|item| std::cmp::Reverse(item.suggestion.id));
            }
            InboxSort::Oldest => {
                rows.sort_by_key(|left| left.suggestion.id);
            }
            InboxSort::Confidence => rows.sort_by(|left, right| {
                right
                    .suggestion
                    .confidence
                    .total_cmp(&left.suggestion.confidence)
                    .then_with(|| left.suggestion.id.cmp(&right.suggestion.id))
            }),
            InboxSort::Path => rows.sort_by(|left, right| {
                left.suggestion
                    .target_id
                    .to_ascii_lowercase()
                    .cmp(&right.suggestion.target_id.to_ascii_lowercase())
                    .then_with(|| left.suggestion.id.cmp(&right.suggestion.id))
            }),
        }
        rows
    }
}

/// The order a person wants to work through the backlog in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InboxSort {
    /// Newest first, so a fresh arrival is not buried under old noise. This is
    /// the default because the old rows are the ones already decided against.
    #[default]
    Newest,
    Oldest,
    /// Strongest identification first — the ones most likely to be accepted.
    Confidence,
    /// By path, which groups a platform's files together within a pile.
    Path,
}

impl InboxSort {
    pub const ALL: [Self; 4] = [Self::Newest, Self::Oldest, Self::Confidence, Self::Path];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Newest => "Newest first",
            Self::Oldest => "Oldest first",
            Self::Confidence => "Strongest match first",
            Self::Path => "By path",
        }
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
    let playable_root = app
        .settings
        .library
        .active_profile()
        .map(|profile| profile.playable_root.clone());
    let collection_root = app.collection_root();
    std::thread::spawn(move || {
        let result = retro_junk_db::open_database(&db_path)
            .map_err(|error| error.to_string())
            .and_then(|connection| {
                let items = retro_junk_db::work::list_open_suggestions(&connection, None)
                    .map_err(|error| error.to_string())?
                    .into_iter()
                    .map(|suggestion| describe(suggestion, playable_root.as_deref()))
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
                let counts_by_kind = retro_junk_db::work::open_suggestion_counts(&connection)
                    .map_err(|error| error.to_string())?;
                // A collection with no rules is the normal case, and an
                // unreadable rule store must not cost the whole inbox.
                let ignore_rules = collection_root
                    .as_deref()
                    .map(retro_junk_archive::load_rules)
                    .transpose()
                    .unwrap_or_else(|error| {
                        log::warn!("could not read ignore rules: {error}");
                        None
                    })
                    .unwrap_or_default();
                Ok(InboxContents {
                    items,
                    failed_packages,
                    unresolved_disagreements,
                    counts_by_kind,
                    ignore_rules,
                    playable_root,
                })
            });
        let _ = sender.send(AppMessage::InboxReady { result });
        repaint.request_repaint();
    });
}

/// Turn a stored suggestion into what the row shows.
fn describe(suggestion: Suggestion, playable_root: Option<&std::path::Path>) -> InboxItem {
    let actions = retro_junk_work::offered_actions(&suggestion);
    let (headline, location, status, details, group) = match suggestion.kind.as_str() {
        IMPORT_SUGGESTION_KIND => describe_import(&suggestion),
        retro_junk_work::SCRAPE_SUGGESTION_KIND => describe_scrape(&suggestion),
        retro_junk_work::ADOPT_SUGGESTION_KIND => describe_adoption(&suggestion),
        _ => (
            suggestion.target_id.clone(),
            String::new(),
            String::new(),
            Vec::new(),
            suggestion.kind.clone(),
        ),
    };
    // One filesystem question per row per load, never per frame.
    let exists = target_path(&suggestion, playable_root).is_some_and(|path| path.exists());
    InboxItem {
        suggestion,
        headline,
        location,
        status,
        details,
        actions,
        exists,
        group,
    }
}

/// Where on disk a review's target lives, when it has one.
///
/// Adoption reviews record a path relative to the playable root; imports
/// record an absolute path to the incoming package.
#[must_use]
pub fn target_path(
    suggestion: &Suggestion,
    playable_root: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    if suggestion.target_kind != "path" {
        return None;
    }
    if suggestion.kind == retro_junk_work::ADOPT_SUGGESTION_KIND {
        return playable_root.map(|root| root.join(&suggestion.target_id));
    }
    Some(std::path::PathBuf::from(&suggestion.target_id))
}

type Described = (String, String, String, Vec<String>, String);

fn describe_import(suggestion: &Suggestion) -> Described {
    let group = "Incoming imports".to_owned();
    let Ok(payloads) =
        serde_json::from_str::<Vec<ImportSuggestionPayload>>(&suggestion.payload_json)
    else {
        return (
            suggestion.target_id.clone(),
            String::new(),
            String::new(),
            Vec::new(),
            group,
        );
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
    let status = payloads
        .first()
        .map(|payload| payload.disposition.clone())
        .unwrap_or_default();
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
    (
        headline,
        suggestion.target_id.clone(),
        status,
        details,
        group,
    )
}

fn describe_scrape(suggestion: &Suggestion) -> Described {
    let Ok(payload) = serde_json::from_str::<ScrapeSuggestionPayload>(&suggestion.payload_json)
    else {
        return (
            suggestion.target_id.clone(),
            String::new(),
            String::new(),
            Vec::new(),
            "Artwork".to_owned(),
        );
    };
    let mut details = vec![payload.reason.clone()];
    if !payload.missing.is_empty() {
        details.push(format!("would fetch: {}", payload.missing.join(", ")));
    }
    details.push(format!(
        "{} — {}",
        payload.platform_id, suggestion.target_id
    ));
    let group = format!("Artwork · {}", payload.platform_id);
    (
        payload.label,
        payload.platform_id.clone(),
        "weak match".to_owned(),
        details,
        group,
    )
}

fn describe_adoption(suggestion: &Suggestion) -> Described {
    let payload = retro_junk_work::adoption::read_payload(suggestion);
    let path = std::path::Path::new(&payload.relative_path);
    let headline = path.file_name().map_or_else(
        || payload.relative_path.clone(),
        |name| name.to_string_lossy().into_owned(),
    );
    let location = path
        .parent()
        .map(|parent| parent.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut details = vec![payload.relative_path.clone()];
    if !payload.detail.is_empty() {
        details.push(payload.detail.clone());
    }
    for candidate in &payload.candidates {
        details.push(format!(
            "    candidate ({}): {}",
            candidate.kind.as_str(),
            candidate.label
        ));
    }
    // Grouped by platform directory: that is how the noise actually clusters,
    // and it is the unit someone decides about ("GameCube, not yet").
    let platform = retro_junk_work::adoption::platform_of(&payload.relative_path);
    let group = if platform.is_empty() {
        "Unaccounted playable files".to_owned()
    } else {
        format!("Unaccounted playable files · {platform}")
    };
    (headline, location, payload.status.clone(), details, group)
}

/// Execute a suggestion through the shared dispatch, answering its question if
/// it asked one.
pub fn apply(
    app: &mut RetroJunkApp,
    id: i64,
    choice: Option<String>,
    label: &str,
    ctx: &egui::Context,
) {
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
            let result =
                retro_junk_work::apply_suggestion_choice(&exec, id, choice.as_deref(), &cancel)
                    .map_err(|error| error.to_string());
            let _ = sender.send(AppMessage::ArchiveOperationComplete { op_id, result });
            let _ = sender.send(AppMessage::InboxChanged);
        },
    );
    ctx.request_repaint_after(std::time::Duration::from_millis(20));
}

/// Apply many suggestions, one after another.
///
/// Sequential on purpose: each one takes the archive lock and imports files,
/// and running them at once would only make them fight. The run reports what
/// happened to each rather than stopping at the first refusal — a batch of
/// twenty-five imports where one package has gone missing should still import
/// the other twenty-four.
pub fn apply_many(app: &mut RetroJunkApp, ids: Vec<i64>, ctx: &egui::Context) {
    let exec = match crate::backend::convergence::exec_context(app) {
        Ok(exec) => exec,
        Err(error) => {
            app.push_error("Apply suggestions", error);
            return;
        }
    };
    let total = ids.len();
    crate::backend::worker::spawn_background_op(
        app,
        format!("Applying {total} suggestion(s)"),
        OperationKind::ArchiveImport,
        "archive".to_owned(),
        ProgressDisplay::Count,
        move |op_id, cancel, sender| {
            let mut applied = 0_usize;
            let mut failures = Vec::new();
            for id in ids {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                match retro_junk_work::apply_suggestion_choice(&exec, id, None, &cancel) {
                    Ok(_) => applied += 1,
                    Err(error) => failures.push(format!("#{id}: {error}")),
                }
                let _ = sender.send(AppMessage::InboxChanged);
            }
            let result = if failures.is_empty() {
                Ok(format!("Applied {applied} of {total}"))
            } else {
                Err(format!(
                    "Applied {applied} of {total}; {} could not be applied:\n{}",
                    failures.len(),
                    failures.join("\n")
                ))
            };
            let _ = sender.send(AppMessage::ArchiveOperationComplete { op_id, result });
            let _ = sender.send(AppMessage::InboxChanged);
        },
    );
    ctx.request_repaint_after(std::time::Duration::from_millis(20));
}

/// Resolve suggestions without executing them.
///
/// The ids that were actually closed come back on [`AppMessage::InboxDismissed`]
/// so the view can offer to put exactly those back.
pub fn dismiss(app: &mut RetroJunkApp, ids: Vec<i64>, ctx: &egui::Context) {
    let Some(db_path) = app.db_path.clone() else {
        return;
    };
    let sender = app.message_tx.clone();
    let repaint = ctx.clone();
    std::thread::spawn(move || {
        let outcome = retro_junk_db::open_database(&db_path)
            .map_err(|error| error.to_string())
            .and_then(|mut connection| {
                retro_junk_db::work::resolve_suggestions(&mut connection, &ids, "dismissed")
                    .map_err(|error| error.to_string())
            });
        match outcome {
            Ok(dismissed) => {
                let _ = sender.send(AppMessage::InboxDismissed { ids: dismissed });
            }
            Err(error) => log::warn!("could not dismiss suggestions: {error}"),
        }
        let _ = sender.send(AppMessage::InboxChanged);
        repaint.request_repaint();
    });
}

/// Put dismissed suggestions back in front of the user.
pub fn reopen(app: &mut RetroJunkApp, ids: Vec<i64>, ctx: &egui::Context) {
    let Some(db_path) = app.db_path.clone() else {
        return;
    };
    let sender = app.message_tx.clone();
    let repaint = ctx.clone();
    std::thread::spawn(move || {
        match retro_junk_db::open_database(&db_path)
            .map_err(|error| error.to_string())
            .and_then(|mut connection| {
                retro_junk_db::work::reopen_suggestions(&mut connection, &ids)
                    .map_err(|error| error.to_string())
            }) {
            Ok(outcome) => {
                if !outcome.is_complete() {
                    log::info!(
                        "reopened {}; {} superseded by newer reviews, {} no longer exist",
                        outcome.reopened.len(),
                        outcome.superseded.len(),
                        outcome.missing.len()
                    );
                }
            }
            Err(error) => log::warn!("could not reopen suggestions: {error}"),
        }
        let _ = sender.send(AppMessage::InboxChanged);
        repaint.request_repaint();
    });
}

/// Record a durable ignore rule and close the reviews it covers.
pub fn ignore(app: &mut RetroJunkApp, pattern: String, note: String, ctx: &egui::Context) {
    let exec = match crate::backend::convergence::exec_context(app) {
        Ok(exec) => exec,
        Err(error) => {
            app.push_error("Ignore files", error);
            return;
        }
    };
    let sender = app.message_tx.clone();
    let repaint = ctx.clone();
    std::thread::spawn(move || {
        match retro_junk_work::ignore_playables(&exec, &pattern, &note) {
            Ok(outcome) => log::info!(
                "ignoring '{}' from now on; closed {} open review(s)",
                outcome.rule.pattern,
                outcome.dismissed
            ),
            Err(error) => log::warn!("could not ignore '{pattern}': {error}"),
        }
        let _ = sender.send(AppMessage::InboxChanged);
        repaint.request_repaint();
    });
}

/// Revoke an ignore rule; the next sweep files those files again.
pub fn unignore(app: &mut RetroJunkApp, pattern: String, ctx: &egui::Context) {
    let exec = match crate::backend::convergence::exec_context(app) {
        Ok(exec) => exec,
        Err(error) => {
            app.push_error("Revoke ignore rule", error);
            return;
        }
    };
    let sender = app.message_tx.clone();
    let repaint = ctx.clone();
    std::thread::spawn(move || {
        if let Err(error) = retro_junk_work::unignore_playables(&exec, &pattern) {
            log::warn!("could not revoke '{pattern}': {error}");
        }
        let _ = sender.send(AppMessage::InboxChanged);
        repaint.request_repaint();
    });
}

/// Stop tracking a failed incoming package.
///
/// The file itself is untouched, so a watcher that still sees it in the drop
/// folder will observe it again — this clears the row, it does not decide
/// anything about the package.
pub fn forget_package(app: &mut RetroJunkApp, path: String, ctx: &egui::Context) {
    let Some(db_path) = app.db_path.clone() else {
        return;
    };
    let sender = app.message_tx.clone();
    let repaint = ctx.clone();
    std::thread::spawn(move || {
        if let Err(error) = retro_junk_db::open_database(&db_path)
            .map_err(|error| error.to_string())
            .and_then(|mut connection| {
                retro_junk_db::work::remove_incoming_package(&mut connection, &path)
                    .map_err(|error| error.to_string())
            })
        {
            log::warn!("could not forget package {path}: {error}");
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
