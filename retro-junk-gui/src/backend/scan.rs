//! Thin dispatch to `retro_junk_backend::ops::scan`. Scheduling, progress
//! forwarding, and message delivery only — folder discovery, entry
//! snapshotting (including the database join), analysis, and catalog
//! resolution live in the backend.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use retro_junk_backend::ops::OpCtx;
use retro_junk_backend::ops::scan::{ConsoleScanError, ConsoleScanRequest};

use crate::app::RetroJunkApp;
use crate::backend::worker::{forward_phases, spawn_background_op};
use crate::state::{AppMessage, LibraryEntry, OperationKind, ProgressDisplay};

// Folder aliasing rules live with the discovery logic in the backend; keep
// the old `crate::backend::scan::` path working for GUI callers.
pub(crate) use retro_junk_backend::ops::scan::projection_alias_key;

/// Scan a root folder for console subfolders on a background thread.
///
/// Discovery — alias suppression and archive-only projection folders
/// included — happens in the backend; this only forwards each match as a
/// message with its display metadata attached.
pub fn scan_root_folder(app: &mut RetroJunkApp, root: PathBuf, ctx: &egui::Context) {
    let context = app.context.clone();
    let ctx = ctx.clone();
    let archive_root = app
        .settings
        .library
        .active_profile()
        .filter(|profile| profile.playable_root == root)
        .map(|profile| profile.archive_root.clone());

    spawn_background_op(
        app,
        "Scanning folders...".to_string(),
        OperationKind::UiFetch,
        String::new(),
        ProgressDisplay::Count,
        move |_op_id, cancel, tx| {
            match retro_junk_backend::ops::scan::discover_console_folders(
                &context,
                &root,
                archive_root.as_deref(),
            ) {
                Ok(matches) => {
                    for cf in matches {
                        if cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        if let Some(registered) = context.get_by_platform(cf.platform) {
                            let _ = tx.send(AppMessage::ConsoleFolderFound {
                                platform: cf.platform,
                                folder_name: cf.folder_name,
                                folder_path: cf.path,
                                manufacturer: registered.metadata.manufacturer,
                                platform_name: registered.metadata.platform_name,
                            });
                        }
                    }
                }
                Err(e) => {
                    log::warn!("Failed to scan root folder: {e}");
                }
            }
            let _ = tx.send(AppMessage::FolderScanComplete);
            ctx.request_repaint();
        },
    );
}

/// Quick-scan a single console folder: discover game entries, then analyze each.
///
/// The index is resolved only at this UI boundary; persisted work uses durable IDs.
pub fn quick_scan_console(app: &mut RetroJunkApp, console_idx: usize, ctx: &egui::Context) {
    if app.browser.consoles[console_idx].scan_status != crate::state::ScanStatus::NotScanned {
        return;
    }
    app.browser.consoles[console_idx].scan_status = crate::state::ScanStatus::Scanning;

    let Some(target) = ConsoleScanTarget::from_app(app, console_idx) else {
        app.browser.consoles[console_idx].scan_status = crate::state::ScanStatus::NotScanned;
        return;
    };
    start_console_scan(app, target, ctx);
}

/// Everything a durable scan needs, captured before the worker starts. The
/// database ID is the identity; names and paths are immutable job inputs and
/// are never resolved again through the active UI projection.
#[derive(Debug, Clone)]
pub struct ConsoleScanTarget {
    pub console_id: Option<retro_junk_db::LibraryConsoleId>,
    pub root_path: PathBuf,
    pub platform: retro_junk_core::Platform,
    pub folder_name: String,
    pub folder_path: PathBuf,
    pub platform_name: String,
}

impl ConsoleScanTarget {
    pub fn from_app(app: &RetroJunkApp, console_idx: usize) -> Option<Self> {
        let console = app.browser.consoles.get(console_idx)?;
        Some(Self {
            console_id: console.id,
            root_path: app.root_path.clone()?,
            platform: console.platform,
            folder_name: console.folder_name.clone(),
            folder_path: console.folder_path.clone(),
            platform_name: console.platform_name.to_owned(),
        })
    }

    pub fn durable(app: &RetroJunkApp, console_idx: usize) -> Option<Self> {
        let target = Self::from_app(app, console_idx)?;
        target.console_id?;
        Some(target)
    }
}

pub fn restart_console_scan(
    app: &mut RetroJunkApp,
    target: ConsoleScanTarget,
    ctx: &egui::Context,
) {
    if let Some(console_id) = target.console_id {
        app.submit_store(
            crate::backend::library_store::LibraryStoreRequest::MarkConsoleStale(console_id),
            ctx,
        );
        if let Some(console) = app
            .browser
            .consoles
            .iter_mut()
            .find(|console| console.id == Some(console_id))
        {
            console.scan_status = crate::state::ScanStatus::Scanning;
            console.fingerprint = None;
            console.loose_disc_files.clear();
        }
    }
    start_console_scan(app, target, ctx);
}

fn start_console_scan(app: &mut RetroJunkApp, target: ConsoleScanTarget, ctx: &egui::Context) {
    let ConsoleScanTarget {
        console_id,
        root_path,
        platform,
        folder_name,
        folder_path,
        platform_name,
    } = target;

    let context = app.context.clone();
    let db_path = app.db_path.clone();
    let ctx = ctx.clone();

    let description = format!("Scanning {platform_name} ({folder_name})");
    let scope = folder_name.clone();

    let request = ConsoleScanRequest {
        console_id,
        root_path,
        platform,
        folder_name: folder_name.clone(),
        folder_path,
    };

    spawn_background_op(
        app,
        description,
        OperationKind::Scan,
        scope,
        ProgressDisplay::Count,
        move |op_id, cancel, tx| {
            let progress = forward_phases(op_id, tx.clone());
            match retro_junk_backend::ops::scan::scan_console(
                &context,
                db_path.as_deref(),
                &request,
                &OpCtx::new(&cancel, &progress),
            ) {
                Ok(outcome) => {
                    let _ = tx.send(AppMessage::ScanSnapshotPrepared {
                        folder_name: folder_name.clone(),
                        console_id,
                        result: outcome.snapshot,
                    });
                    let _ = tx.send(AppMessage::ScanProjectionInfo {
                        folder_name: folder_name.clone(),
                        loose_disc_files: outcome.loose_disc_files,
                        fingerprint: outcome.fingerprint,
                    });
                }
                Err(ConsoleScanError::Failed(message)) => {
                    let _ = tx.send(AppMessage::ConsoleScanFailed {
                        folder_name: folder_name.clone(),
                        error: Some(message),
                    });
                }
                Err(ConsoleScanError::Cancelled) => {
                    let _ = tx.send(AppMessage::ConsoleScanFailed {
                        folder_name: folder_name.clone(),
                        error: None,
                    });
                }
            }
            let _ = tx.send(AppMessage::OperationComplete { op_id });
            ctx.request_repaint();
        },
    );
}

/// Re-analyze selected entries without rediscovering the folder.
///
/// The UI-thread pass only clones the selected entries; analysis and catalog
/// resolution run in the backend on the worker thread.
pub fn rescan_selected_entries(app: &mut RetroJunkApp, console_idx: usize, ctx: &egui::Context) {
    let console = &app.browser.consoles[console_idx];
    let selected: Vec<LibraryEntry> = app
        .ui_state
        .selected_entries
        .iter()
        .copied()
        .filter_map(|id| console.entry_by_id(id).cloned())
        .collect();

    if selected.is_empty() {
        return;
    }

    let context = app.context.clone();
    let folder_name = console.folder_name.clone();
    let platform = console.platform;
    let db_path = app.db_path.clone();
    let ctx = ctx.clone();

    let count = selected.len();
    let noun = if count == 1 { "entry" } else { "entries" };
    let description = format!("Rescanning {count} {noun}");
    let scope = folder_name.clone();

    spawn_background_op(
        app,
        description,
        OperationKind::Scan,
        scope,
        ProgressDisplay::Count,
        move |op_id, cancel, tx| {
            let progress = forward_phases(op_id, tx.clone());
            if let Some(entries) = retro_junk_backend::ops::scan::analyze_entries(
                &context,
                platform,
                db_path.as_deref(),
                selected,
                &OpCtx::new(&cancel, &progress),
            ) {
                let _ = tx.send(AppMessage::EntryAnalysisSnapshotsComplete {
                    folder_name,
                    entries,
                });
            }

            let _ = tx.send(AppMessage::OperationComplete { op_id });
            ctx.request_repaint();
        },
    );
}
