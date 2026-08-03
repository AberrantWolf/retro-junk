//! Thin dispatch to `retro_junk_backend::ops::chd_compress`. Scheduling,
//! busy-guarding, progress forwarding, and message delivery only — planning
//! and the compress → verify → delete pipeline live in the backend.

#[cfg(test)]
#[path = "chd_compress_tests.rs"]
mod tests;

use std::path::PathBuf;

use retro_junk_backend::ops::OpCtx;

use crate::app::RetroJunkApp;
use crate::backend::worker::spawn_background_op;
use crate::state::{AppMessage, ChdCompressPrompt, OperationKind, ProgressDisplay};

/// Whether a console's analyzer supports CHD compression for any source kind.
/// Gates the context-menu entries so cartridge consoles never see them.
pub fn console_supports_chd(app: &RetroJunkApp, console_idx: usize) -> bool {
    let console = &app.browser.consoles[console_idx];
    retro_junk_backend::ops::chd_compress::platform_supports_chd(&app.context, console.platform)
}

/// Open the "Compress to CHD" dialog for the given entries of a console.
///
/// D1: this is a thin UI-thread collector — only path clones, no I/O. The
/// actual chdman probe and `plan_batch` (cue/gdi parsing, per-track
/// `fs::metadata`) run on a background thread in the backend's `plan_prompt`.
/// The dialog only appears once `AppMessage::ChdCompressPromptReady` arrives.
pub fn open_compress_dialog(app: &mut RetroJunkApp, console_idx: usize, entry_indices: &[usize]) {
    let console = &app.browser.consoles[console_idx];
    let folder_name = console.folder_name.clone();
    let platform = console.platform;

    // D3: the menu items are gated by `chd_compress_busy` too, but this is
    // the guarantee — a stray double-click or command can't queue a second
    // planning pass (or a plan while a compression is running) for the same
    // console folder.
    if app.chd_compress_busy(&folder_name) {
        log::info!("Compress to CHD: a compression is already running for {folder_name}, ignoring");
        return;
    }

    // Collect candidate input paths. The backend's `plan_batch` owns skip
    // classification and duplicate-output detection — this loop does no I/O.
    let mut inputs = Vec::new();
    for &i in entry_indices {
        let Some(entry) = console.entries.get(i) else {
            continue;
        };
        let candidates: Vec<PathBuf> = match &entry.game_entry {
            retro_junk_lib::scanner::GameEntry::SingleFile(p) => vec![p.clone()],
            retro_junk_lib::scanner::GameEntry::MultiDisc { files, .. } => files.clone(),
        };
        inputs.extend(candidates);
    }

    let context = app.context.clone();
    let chdman_setting = app.settings.general.chdman_path.clone();
    let description = format!("Preparing CHD compression for {folder_name}");
    let scope = folder_name.clone();

    spawn_background_op(
        app,
        description,
        OperationKind::ChdCompress,
        scope,
        ProgressDisplay::Count,
        move |op_id, _cancel, tx| {
            let prompt = retro_junk_backend::ops::chd_compress::plan_prompt(
                &context,
                platform,
                folder_name,
                &inputs,
                &chdman_setting,
                crate::widgets::icons::ARROW_RIGHT,
            );
            let _ = tx.send(AppMessage::ChdCompressPromptReady { prompt });
            let _ = tx.send(AppMessage::OperationComplete { op_id });
        },
    );
}

/// Consume the confirmed prompt and run the compression on a worker thread.
pub fn start_compression(app: &mut RetroJunkApp, ctx: &egui::Context) {
    let Some(prompt) = app.ui_state.chd_compress_prompt.take() else {
        return;
    };
    let Ok(chdman) = prompt.chdman else {
        return;
    };
    if prompt.items.is_empty() {
        return;
    }
    let ChdCompressPrompt {
        folder_name,
        items,
        delete_sources,
        ..
    } = prompt;
    let rescan_target = app
        .browser
        .find_by_folder(&folder_name)
        .and_then(|index| crate::backend::scan::ConsoleScanTarget::durable(app, index));

    // D3: the guarantee half of the overlap guard (the menu items are the
    // advisory half). Belt-and-suspenders against a race between opening the
    // dialog and confirming it.
    if app.chd_compress_busy(&folder_name) {
        log::info!("Compress to CHD: a compression is already running for {folder_name}, ignoring");
        return;
    }

    let ctx = ctx.clone();
    let description = format!("Compressing {} disc(s) to CHD", items.len());
    let scope = folder_name.clone();

    spawn_background_op(
        app,
        description,
        OperationKind::ChdCompress,
        scope,
        ProgressDisplay::Percent,
        move |op_id, cancel, tx| {
            // The backend reports on an abstract unit scale rendered as a
            // percentage; forward its reports as raw progress so the
            // description set at spawn time stays put.
            let progress_tx = tx.clone();
            let progress_ctx = ctx.clone();
            let progress =
                move |_phase: &str, _unit: retro_junk_io::ProgressUnit, current, total| {
                    let _ = progress_tx.send(AppMessage::OperationProgress {
                        op_id,
                        current,
                        total,
                    });
                    progress_ctx.request_repaint();
                };
            let results = retro_junk_backend::ops::chd_compress::run_compression(
                &chdman,
                &items,
                delete_sources,
                &OpCtx::new(&cancel, &progress),
            );
            let _ = tx.send(AppMessage::ChdCompressComplete {
                folder_name,
                rescan_target,
                results,
            });
            let _ = tx.send(AppMessage::OperationComplete { op_id });
            ctx.request_repaint();
        },
    );
}
