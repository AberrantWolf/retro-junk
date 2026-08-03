//! `retro-junk suggestions list|show|apply|dismiss|reopen|ignore|ignores|unignore`.
//!
//! Suggestions are proposed-but-unapplied commands. Import suggestions were
//! pre-processed at arrival; `apply` re-validates the package fingerprint
//! and executes through the shared import path.
//!
//! Everything the GUI's Inbox can do is here too, and by the same calls: the
//! filter, the bulk dismiss, the undo, and the ignore rules all come from
//! `retro_junk_work`, so a decision taken at the terminal and one taken in the
//! Inbox are the same decision made the same way.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use retro_junk_work::suggestions::SuggestionFilter;
use retro_junk_work::{ExecContext, LockEtiquette, ReconcileMode, ToolPaths};

use crate::CliError;
use crate::cli_types::SuggestionsAction;

pub(crate) fn run_suggestions(action: SuggestionsAction) -> Result<(), CliError> {
    match action {
        SuggestionsAction::List { kind, pattern, db } => run_list(kind, pattern, db),
        SuggestionsAction::Show { id, db } => run_show(id, db),
        SuggestionsAction::Apply {
            id,
            choice,
            profile,
            db,
        } => run_apply(id, choice.as_deref(), profile.as_deref(), db),
        SuggestionsAction::Dismiss {
            ids,
            kind,
            pattern,
            dry_run,
            db,
        } => run_dismiss(ids, kind.as_deref(), pattern.as_deref(), dry_run, db),
        SuggestionsAction::Reopen { ids, db } => run_reopen(&ids, db),
        SuggestionsAction::Ignore {
            pattern,
            note,
            profile,
            db,
        } => run_ignore(&pattern, &note, profile.as_deref(), db),
        SuggestionsAction::Ignores { profile, db } => run_ignores(profile.as_deref(), db),
        SuggestionsAction::Unignore {
            pattern,
            profile,
            db,
        } => run_unignore(&pattern, profile.as_deref(), db),
    }
}

/// Open reviews, narrowed to the ones being asked about.
fn run_list(
    kind: Option<String>,
    pattern: Option<String>,
    db: Option<PathBuf>,
) -> Result<(), CliError> {
    let conn = open(db)?;
    let filter = SuggestionFilter::new(kind.as_deref(), pattern.as_deref().unwrap_or(""));
    let all = list_open(&conn)?;
    let total = all.len();
    let shown = filter.select(all);
    if shown.is_empty() {
        if total == 0 {
            log::info!("No open suggestions");
        } else {
            log::info!("None of the {total} open suggestion(s) match that filter");
        }
        return Ok(());
    }
    for suggestion in &shown {
        let actions = retro_junk_work::offered_actions(suggestion);
        let note = if actions.choices.is_empty() {
            String::new()
        } else {
            format!("  [{} candidates]", actions.choices.len())
        };
        log::info!(
            "#{:<4} {:<14} {:<40} confidence {:.2}  ({}){note}",
            suggestion.id,
            suggestion.kind,
            suggestion.target_id,
            suggestion.confidence,
            suggestion.created_at
        );
    }
    if shown.len() != total {
        log::info!("Showing {} of {total} open suggestion(s)", shown.len());
    }
    Ok(())
}

/// One review in full, including the candidates `--choice` accepts.
fn run_show(id: i64, db: Option<PathBuf>) -> Result<(), CliError> {
    let conn = open(db)?;
    let suggestion = retro_junk_db::work::get_suggestion(&conn, id)
        .map_err(|error| CliError::database(error.to_string()))?
        .ok_or_else(|| CliError::other(format!("suggestion {id} not found")))?;
    log::info!(
        "#{} {} on {} ({})",
        suggestion.id,
        suggestion.kind,
        suggestion.target_id,
        match &suggestion.resolved_at {
            Some(at) => format!("{} at {at}", suggestion.resolution),
            None => "open".to_owned(),
        }
    );
    // Payloads are JSON by convention; pretty-print when possible.
    match serde_json::from_str::<serde_json::Value>(&suggestion.payload_json) {
        Ok(value) => log::info!(
            "{}",
            serde_json::to_string_pretty(&value)
                .unwrap_or_else(|_| suggestion.payload_json.clone())
        ),
        Err(_) => log::info!("{}", suggestion.payload_json),
    }
    let actions = retro_junk_work::offered_actions(&suggestion);
    for candidate in &actions.choices {
        log::info!(
            "  candidate --choice {}  ({}) {}",
            candidate.id,
            candidate.kind.as_str(),
            candidate.label
        );
    }
    Ok(())
}

fn run_apply(
    id: i64,
    choice: Option<&str>,
    profile: Option<&str>,
    db: Option<PathBuf>,
) -> Result<(), CliError> {
    let ctx = exec_context(profile, db)?;
    let cancelled = Arc::new(AtomicBool::new(false));
    retro_junk_work::daemon::install_signal_handlers(&cancelled);
    let summary = retro_junk_work::apply_suggestion_choice(&ctx, id, choice, &cancelled)
        .map_err(|error| CliError::other(error.to_string()))?;
    log::info!("Applied suggestion #{id}: {summary}");
    Ok(())
}

fn run_reopen(ids: &[i64], db: Option<PathBuf>) -> Result<(), CliError> {
    if ids.is_empty() {
        return Err(CliError::other("name at least one suggestion to reopen"));
    }
    let mut conn = open(db)?;
    let outcome = retro_junk_db::work::reopen_suggestions(&mut conn, ids)
        .map_err(|error| CliError::database(error.to_string()))?;
    log::info!("Reopened {} suggestion(s)", outcome.reopened.len());
    if !outcome.superseded.is_empty() {
        log::warn!(
            "Left {} closed: a newer suggestion about the same target is already open",
            outcome.superseded.len()
        );
    }
    if !outcome.missing.is_empty() {
        log::warn!("No suggestion exists for id(s): {:?}", outcome.missing);
    }
    Ok(())
}

fn run_ignore(
    pattern: &str,
    note: &str,
    profile: Option<&str>,
    db: Option<PathBuf>,
) -> Result<(), CliError> {
    let ctx = exec_context(profile, db)?;
    let outcome = retro_junk_work::ignore_playables(&ctx, pattern, note)
        .map_err(|error| CliError::other(error.to_string()))?;
    log::info!(
        "Ignoring '{}' from now on; closed {} open review(s) it covers",
        outcome.rule.pattern,
        outcome.dismissed
    );
    log::info!(
        "Revoke it with `retro-junk suggestions unignore '{}'`",
        outcome.rule.pattern
    );
    Ok(())
}

fn run_ignores(profile: Option<&str>, db: Option<PathBuf>) -> Result<(), CliError> {
    let ctx = exec_context(profile, db)?;
    let rules =
        retro_junk_work::ignore_rules(&ctx).map_err(|error| CliError::other(error.to_string()))?;
    if rules.is_empty() {
        log::info!("No ignore rules; every unaccounted playable file is filed for review");
        return Ok(());
    }
    for rule in &rules {
        if rule.note.is_empty() {
            log::info!("{}", rule.pattern);
        } else {
            log::info!("{}  — {}", rule.pattern, rule.note);
        }
    }
    Ok(())
}

fn run_unignore(pattern: &str, profile: Option<&str>, db: Option<PathBuf>) -> Result<(), CliError> {
    let ctx = exec_context(profile, db)?;
    let removed = retro_junk_work::unignore_playables(&ctx, pattern)
        .map_err(|error| CliError::other(error.to_string()))?;
    if removed {
        log::info!("Revoked '{pattern}'; the next adoption sweep files those files again");
    } else {
        log::info!("No ignore rule for '{pattern}'");
    }
    Ok(())
}

/// Dismiss by id, or by describing a group.
///
/// The two ways are deliberately exclusive. Naming ids and describing a group
/// in one command would leave it ambiguous what "dismiss" just did, and this
/// is a command that closes hundreds of rows at once.
fn run_dismiss(
    ids: Vec<i64>,
    kind: Option<&str>,
    pattern: Option<&str>,
    dry_run: bool,
    db: Option<PathBuf>,
) -> Result<(), CliError> {
    let describes_a_group = kind.is_some() || pattern.is_some();
    if ids.is_empty() && !describes_a_group {
        return Err(CliError::other(
            "name suggestion ids, or describe a group with --kind / --match",
        ));
    }
    if !ids.is_empty() && describes_a_group {
        return Err(CliError::other(
            "dismiss ids or a --kind/--match group, not both",
        ));
    }

    let mut conn = open(db)?;
    let targets = if ids.is_empty() {
        let filter = SuggestionFilter::new(kind, pattern.unwrap_or(""));
        filter
            .select(list_open(&conn)?)
            .into_iter()
            .map(|suggestion| suggestion.id)
            .collect()
    } else {
        ids
    };
    if targets.is_empty() {
        log::info!("Nothing matches; dismissed nothing");
        return Ok(());
    }
    if dry_run {
        log::info!("Would dismiss {} suggestion(s):", targets.len());
        for suggestion in list_open(&conn)?
            .into_iter()
            .filter(|suggestion| targets.contains(&suggestion.id))
        {
            log::info!("  #{} {}", suggestion.id, suggestion.target_id);
        }
        return Ok(());
    }
    let dismissed = retro_junk_db::work::resolve_suggestions(&mut conn, &targets, "dismissed")
        .map_err(|error| CliError::database(error.to_string()))?;
    log::info!("Dismissed {} suggestion(s)", dismissed.len());
    if dismissed.len() < targets.len() {
        log::info!("{} were already resolved", targets.len() - dismissed.len());
    }
    if !dismissed.is_empty() {
        // Dismissal is reversible two ways, and saying so is the point: the
        // exact rows can be put back, and re-running the sweep would file them
        // again anyway. Neither touched a file.
        log::info!(
            "Undo with `retro-junk suggestions reopen {}`",
            dismissed
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
    Ok(())
}

fn list_open(
    conn: &retro_junk_db::Connection,
) -> Result<Vec<retro_junk_db::work::Suggestion>, CliError> {
    retro_junk_db::work::list_open_suggestions(conn, None)
        .map_err(|error| CliError::database(error.to_string()))
}

fn open(db: Option<PathBuf>) -> Result<retro_junk_db::Connection, CliError> {
    let path = match db {
        Some(path) => path,
        None => retro_junk_lib::settings::ensure_catalog_database_location()?,
    };
    retro_junk_db::open_database(&path).map_err(|error| CliError::database(error.to_string()))
}

fn exec_context(profile: Option<&str>, db: Option<PathBuf>) -> Result<ExecContext, CliError> {
    let profile = retro_junk_work::profiles::resolve_profile(profile).ok_or_else(|| {
        CliError::config("no collection profile found; pass --profile or configure one")
    })?;
    let db_path = match db {
        Some(path) => path,
        None => retro_junk_lib::settings::ensure_catalog_database_location()?,
    };
    let general = retro_junk_work::profiles::load_general();
    let roots = retro_junk_lib::archive_ops::FrontendRoots::from_settings(
        &profile.playable_root,
        &general.assets_dir,
        &general.metadata_dir,
    );
    Ok(ExecContext {
        profile,
        db_path,
        tools: ToolPaths {
            chdman: PathBuf::from(general.chdman_path.trim()),
            redumper: PathBuf::new(),
            dolphin_tool: PathBuf::new(),
        },
        scrape: retro_junk_work::AutomationPolicy::load().scrape_settings(),
        roots,
        analyzers: Arc::new(retro_junk_lib::create_default_context()),
        owner: ExecContext::owner_string("cli"),
        lock: LockEtiquette::InteractiveWait,
        reconcile: ReconcileMode::PerAction,
    })
}
