//! Thin dispatch to `retro_junk_backend::ops::rename`. Scheduling, progress
//! forwarding, and message delivery only — classification, catalog lookups,
//! disc-set verification, and the renames themselves live in the backend.
//!
//! The UI-thread pass here only copies what the selection already holds:
//! entry identities, file paths, and cached match data. No database is
//! opened and no directory is read on the render thread.

use retro_junk_backend::ops::OpCtx;
use retro_junk_backend::ops::rename::{RenameEntry, RenameEntryKind, RenameRequest};
use retro_junk_lib::rename::DiscMatchData;
use retro_junk_lib::scanner::GameEntry;

use crate::app::RetroJunkApp;
use crate::backend::worker::{forward_phases, spawn_background_op};
use crate::state::{AppMessage, OperationKind, ProgressDisplay};

/// Rename selected entries to their DAT-matched filenames.
///
/// Copies the selection's cached match data on the UI thread, then hands the
/// whole request to the backend, which resolves targets against the catalog
/// database (opened from the path, on the worker thread), verifies disc
/// sets, and executes every rename through the shared transactional engine
/// in `retro_junk_lib::rename` — companion media files and gamelist.xml
/// entries move inside each game's transaction.
pub fn rename_selected_entries(app: &mut RetroJunkApp, console_idx: usize, ctx: &egui::Context) {
    let console = &app.browser.consoles[console_idx];
    let folder_name = console.folder_name.clone();
    let rescan_target = crate::backend::scan::ConsoleScanTarget::durable(app, console_idx);

    let mut entries: Vec<RenameEntry> = Vec::new();
    for &i in &app.ui_state.selected_entries {
        let Some(entry) = console.entry_by_id(i) else {
            continue;
        };
        let entry_name = entry.game_entry.display_name().to_string();

        let kind = match &entry.game_entry {
            GameEntry::SingleFile(path) => RenameEntryKind::SingleFile {
                path: path.clone(),
                // Cached rom_name from dat_match, skipping cross-region
                // matches — anything else resolves in the backend.
                cached_names: entry
                    .dat_match
                    .as_ref()
                    .filter(|dm| !dm.rom_name.is_empty() && !dm.cross_region)
                    .map(|dm| (dm.game_name.clone(), dm.rom_name.clone())),
                hashes: entry.hashes.clone(),
                identification: entry.identification.clone(),
            },
            GameEntry::MultiDisc { files, .. } => RenameEntryKind::MultiDisc {
                files: files.clone(),
                // Per-disc cached matches; `target_filename` carries the raw
                // DAT rom_name, corrected in the backend before execution.
                resolved_discs: entry
                    .disc_identifications
                    .iter()
                    .flatten()
                    .filter_map(|disc| {
                        disc.dat_match.as_ref().map(|dm| DiscMatchData {
                            file_path: disc.path.clone(),
                            game_name: dm.game_name.clone(),
                            target_filename: dm.rom_name.clone(),
                        })
                    })
                    .collect(),
                game_name_override: entry
                    .dat_match
                    .as_ref()
                    .map(|dm| dm.game_name.clone())
                    .unwrap_or_default(),
            },
        };
        entries.push(RenameEntry {
            entry_id: i,
            entry_name,
            kind,
        });
    }

    if entries.is_empty() {
        return;
    }

    let ctx = ctx.clone();
    let context = app.context.clone();
    let catalog_db_path = app.db_path.clone();
    let platform = console.platform;
    let description = format!("Renaming {} entries", entries.len());

    // Candidate companion locations for this console: media directory and
    // gamelist.xml. The backend uses them only when they exist on disk, and
    // both move inside each game's rename transaction.
    let media_dir = app.root_path.as_ref().and_then(|rp| {
        retro_junk_lib::util::asset_dir_for_console(
            rp,
            &folder_name,
            &app.settings.general.assets_dir,
        )
    });
    let gamelist_path = app.root_path.as_ref().map(|rp| {
        retro_junk_lib::util::metadata_dir_for_console(
            rp,
            &folder_name,
            &app.settings.general.metadata_dir,
        )
        .join("gamelist.xml")
    });

    let request = RenameRequest {
        platform,
        entries,
        media_dir,
        gamelist_path,
    };
    let scope = folder_name.clone();

    spawn_background_op(
        app,
        description,
        OperationKind::Rename,
        scope,
        ProgressDisplay::Count,
        move |op_id, cancel, tx| {
            let progress = forward_phases(op_id, tx.clone());
            let results = retro_junk_backend::ops::rename::rename_entries(
                &context,
                catalog_db_path.as_deref(),
                &request,
                &OpCtx::new(&cancel, &progress),
            );
            let _ = tx.send(AppMessage::RenameComplete {
                folder_name,
                rescan_target,
                results,
            });
            ctx.request_repaint();
            Ok(())
        },
    );
}
