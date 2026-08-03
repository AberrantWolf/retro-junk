//! Thin dispatch to `retro_junk_backend::ops::hash`. Scheduling, progress
//! forwarding, and message delivery only — work flattening, hashing, CHD
//! staging, catalog matching, and disc-verification judgment live in the
//! backend.
//!
//! The UI-thread pass here only reads the selection (cloning the selected
//! entries as snapshots); no file is opened and no database is touched on
//! the render thread.

use retro_junk_backend::ops::OpCtx;
use retro_junk_backend::ops::hash::{HashRequest, HashWork, collect_hash_work};

use crate::app::RetroJunkApp;
use crate::backend::worker::{forward_phases, spawn_background_op};
use crate::state::{AppMessage, LibraryEntry, OperationKind, ProgressDisplay};

/// Compute hashes for selected entries in the active console.
pub fn compute_hashes_for_selection(app: &mut RetroJunkApp, console_idx: usize) {
    compute_hashes_for_selection_inner(app, console_idx, false);
}

/// Recompute hashes even when the current source fingerprint already has a
/// durable result. This is deliberately separate from the normal action so a
/// verification click does not reread unchanged network-hosted media.
pub fn recompute_hashes_for_selection(app: &mut RetroJunkApp, console_idx: usize) {
    compute_hashes_for_selection_inner(app, console_idx, true);
}

fn compute_hashes_for_selection_inner(
    app: &mut RetroJunkApp,
    console_idx: usize,
    include_cached: bool,
) {
    let console = &app.browser.consoles[console_idx];

    log::debug!(
        "compute_hashes_for_selection: console_idx={}, selected_entries={:?}, total_entries={}",
        console_idx,
        app.ui_state.selected_entries,
        console.entries.len()
    );

    let (work, snapshots) = collect_hash_work(
        app.ui_state
            .selected_entries
            .iter()
            .copied()
            .filter_map(|id| console.entry_by_id(id)),
        include_cached,
    );

    spawn_hash_work(app, console_idx, work, snapshots);
}

fn spawn_hash_work(
    app: &mut RetroJunkApp,
    console_idx: usize,
    work: Vec<HashWork>,
    snapshots: Vec<LibraryEntry>,
) {
    let console = &app.browser.consoles[console_idx];
    let platform = console.platform;
    let folder_name = console.folder_name.clone();

    if work.is_empty() {
        return;
    }

    let context = app.context.clone();
    let db_path = app.db_path.clone();
    let workspace_root = app
        .settings
        .library
        .active_profile()
        .map_or_else(retro_junk_io::default_transient_workspace, |profile| {
            profile.workspace_root.clone()
        });
    let stage_large_files = app
        .settings
        .library
        .active_profile()
        .is_none_or(|profile| profile.network_mode);
    let description = format!("Computing hashes ({} files)", work.len());
    let scope = folder_name.clone();

    let request = HashRequest {
        platform,
        work,
        workspace_root,
        stage_large_files,
    };

    spawn_background_op(
        app,
        description,
        OperationKind::Hash,
        scope,
        ProgressDisplay::Bytes,
        move |op_id, cancel, tx| {
            let progress = forward_phases(op_id, tx.clone());
            let report = retro_junk_backend::ops::hash::compute_entry_hashes(
                &context,
                db_path.as_deref(),
                request,
                &OpCtx::new(&cancel, &progress),
            );

            for failure in report.failures {
                let _ = tx.send(AppMessage::HashFailed {
                    folder_name: folder_name.clone(),
                    entry_id: failure.entry_id,
                    entry_name: failure.entry_name,
                    error: failure.error,
                });
            }

            // A cancelled run never delivers partial batches; only failures
            // observed before the cancel are reported.
            if !report.cancelled {
                let mut results_by_entry = report.results_by_entry;
                for entry in snapshots {
                    let Some(entry_id) = entry.id else {
                        continue;
                    };
                    let Some(results) = results_by_entry.remove(&entry_id) else {
                        continue;
                    };
                    let _ = tx.send(AppMessage::EntryHashBatchComplete {
                        folder_name: folder_name.clone(),
                        entry: Box::new(entry),
                        results,
                    });
                }
            }

            let _ = tx.send(AppMessage::OperationComplete { op_id });
        },
    );
}
