//! `retro-junk suggestions list|show|apply|dismiss`.
//!
//! Suggestions are proposed-but-unapplied commands. Import suggestions were
//! pre-processed at arrival; `apply` re-validates the package fingerprint
//! and executes through the shared import path.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use retro_junk_work::{ExecContext, LockEtiquette, ReconcileMode, ToolPaths};

use crate::CliError;
use crate::cli_types::SuggestionsAction;

pub(crate) fn run_suggestions(action: SuggestionsAction) -> Result<(), CliError> {
    match action {
        SuggestionsAction::List { db } => {
            let conn = open(db)?;
            let open_rows = retro_junk_db::work::list_open_suggestions(&conn, None)
                .map_err(|error| CliError::database(error.to_string()))?;
            if open_rows.is_empty() {
                log::info!("No open suggestions");
                return Ok(());
            }
            for suggestion in open_rows {
                log::info!(
                    "#{:<4} {:<10} {:<40} confidence {:.2}  ({})",
                    suggestion.id,
                    suggestion.kind,
                    suggestion.target_id,
                    suggestion.confidence,
                    suggestion.created_at
                );
            }
            Ok(())
        }
        SuggestionsAction::Show { id, db } => {
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
                    serde_json::to_string_pretty(&value).unwrap_or(suggestion.payload_json)
                ),
                Err(_) => log::info!("{}", suggestion.payload_json),
            }
            Ok(())
        }
        SuggestionsAction::Apply { id, profile, db } => {
            let ctx = exec_context(profile.as_deref(), db)?;
            let cancelled = std::sync::Arc::new(AtomicBool::new(false));
            retro_junk_work::daemon::install_signal_handlers(&cancelled);
            let summary = retro_junk_work::apply_suggestion(&ctx, id, &cancelled)
                .map_err(|error| CliError::other(error.to_string()))?;
            log::info!("Applied suggestion #{id}: {summary}");
            Ok(())
        }
        SuggestionsAction::Dismiss { id, db } => {
            let mut conn = open(db)?;
            let resolved = retro_junk_db::work::resolve_suggestion(&mut conn, id, "dismissed")
                .map_err(|error| CliError::database(error.to_string()))?;
            if resolved {
                log::info!("Dismissed suggestion #{id}");
                Ok(())
            } else {
                Err(CliError::other(format!(
                    "suggestion {id} was already resolved"
                )))
            }
        }
    }
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
