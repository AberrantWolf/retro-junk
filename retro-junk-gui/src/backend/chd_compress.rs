//! CHD compression backend: plans jobs from the selection, then runs the
//! compress → round-trip-verify → (optionally) delete pipeline on a worker
//! thread via the shared chdman wrapper in `retro_junk_lib::chd_convert`.

#[cfg(test)]
#[path = "chd_compress_tests.rs"]
mod tests;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use retro_junk_core::ChdExtensionRole;
use retro_junk_lib::chd_convert::{self, Chdman, VerificationOutcome};
use retro_junk_lib::util::format_bytes_approx;

use crate::app::RetroJunkApp;
use crate::backend::worker::spawn_background_op;
use crate::state::{
    AppMessage, ChdCompressItem, ChdCompressOutcome, ChdCompressPrompt, ChdCompressResult,
    ChdCompressSkip, OperationKind, ProgressDisplay,
};

/// Progress-bar resolution per job (fractions of one disc's pipeline).
const JOB_PROGRESS_UNITS: u64 = 1000;

/// Whether a console's analyzer supports CHD compression for any source kind.
/// Gates the context-menu entries so cartridge consoles never see them.
pub fn console_supports_chd(app: &RetroJunkApp, console_idx: usize) -> bool {
    let console = &app.library.consoles[console_idx];
    app.context
        .get_by_platform(console.platform)
        .is_some_and(|rc| {
            rc.analyzer
                .chd_extensions()
                .iter()
                .any(|(_, role)| matches!(role, ChdExtensionRole::Source(_)))
        })
}

/// Open the "Compress to CHD" dialog for the given entries of a console.
///
/// D1: this is a thin UI-thread collector — only path clones, no I/O. The
/// actual chdman probe and `plan_batch` (cue/gdi parsing, per-track
/// `fs::metadata`) run on a background thread, following the analyzer-into-
/// worker pattern used by `backend::hash`: the context is Arc-cloned and the
/// analyzer looked up again inside the closure. The dialog only appears once
/// `AppMessage::ChdCompressPromptReady` arrives.
pub fn open_compress_dialog(app: &mut RetroJunkApp, console_idx: usize, entry_indices: &[usize]) {
    let console = &app.library.consoles[console_idx];
    let folder_name = console.folder_name.clone();
    let platform = console.platform;

    // D3: the menu items are gated by `chd_compress_busy` too, but this is
    // the guarantee — a stray double-click or command can't queue a second
    // planning pass (or a plan while a compression is running) for the same
    // console folder.
    if app.chd_compress_busy(&folder_name) {
        log::info!(
            "Compress to CHD: a compression is already running for {}, ignoring",
            folder_name
        );
        return;
    }

    // Collect candidate input paths. `plan_batch` owns skip classification
    // and duplicate-output detection (including the "bin"/"img"
    // companion-data noise inside .m3u folders) — this loop does no I/O.
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
    let description = format!("Preparing CHD compression for {}", folder_name);
    let scope = Some(folder_name.clone());

    spawn_background_op(
        app,
        description,
        OperationKind::ChdCompress,
        scope,
        ProgressDisplay::Count,
        move |op_id, _cancel, tx| {
            // The chdman probe (a subprocess spawn) and plan_batch (cue/gdi
            // parsing + per-track fs::metadata) both run here, off the UI
            // thread — this is exactly the blocking work D1 moves off of it.
            let chdman = Chdman::detect_from_setting(&chdman_setting);

            let (items, skipped) = match context.get_by_platform(platform) {
                Some(registered) => {
                    let analyzer = registered.analyzer.as_ref();
                    let batch = chd_convert::plan_batch(&inputs, analyzer);
                    let items = batch
                        .jobs
                        .into_iter()
                        .map(|job| {
                            let input_name = job
                                .input
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string();
                            let output_name = job
                                .output
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string();
                            // D8: precompute the display line at plan time so
                            // the confirmation dialog doesn't re-derive
                            // filename strings and byte formatting every frame.
                            let display_line = format!(
                                "{input_name} ({}) \u{2192} {output_name}",
                                format_bytes_approx(job.input_bytes)
                            );
                            ChdCompressItem { job, display_line }
                        })
                        .collect();
                    let skipped = batch
                        .skips
                        .into_iter()
                        .map(|skip| ChdCompressSkip {
                            entry_name: skip
                                .input
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string(),
                            reason: skip.error.to_string(),
                        })
                        .collect();
                    (items, skipped)
                }
                None => (Vec::new(), Vec::new()),
            };

            let prompt = ChdCompressPrompt {
                folder_name: folder_name.clone(),
                items,
                skipped,
                chdman,
                delete_sources: false,
            };

            let _ = tx.send(AppMessage::ChdCompressPromptReady { prompt });
            let _ = tx.send(AppMessage::OperationComplete { op_id });
        },
    );
}

/// Consume the confirmed prompt and run the compression on a worker thread.
pub fn start_compression(app: &mut RetroJunkApp, ctx: &egui::Context) {
    let Some(prompt) = app.chd_compress_prompt.take() else {
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

    // D3: the guarantee half of the overlap guard (the menu items are the
    // advisory half). Belt-and-suspenders against a race between opening the
    // dialog and confirming it.
    if app.chd_compress_busy(&folder_name) {
        log::info!(
            "Compress to CHD: a compression is already running for {}, ignoring",
            folder_name
        );
        return;
    }

    let ctx = ctx.clone();
    let total_units = items.len() as u64 * JOB_PROGRESS_UNITS;
    let description = format!("Compressing {} disc(s) to CHD", items.len());
    let scope = Some(folder_name.clone());

    spawn_background_op(
        app,
        description,
        OperationKind::ChdCompress,
        scope,
        ProgressDisplay::Percent,
        move |op_id, cancel, tx| {
            let mut results: Vec<ChdCompressResult> = Vec::new();

            for (i, item) in items.iter().enumerate() {
                let input_name = item
                    .job
                    .input
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();

                if cancel.load(Ordering::Relaxed) {
                    results.push(ChdCompressResult {
                        input_name,
                        job: item.job.clone(),
                        outcome: ChdCompressOutcome::Cancelled,
                    });
                    continue;
                }

                let base_units = i as u64 * JOB_PROGRESS_UNITS;
                let _ = tx.send(AppMessage::OperationProgress {
                    op_id,
                    current: base_units,
                    total: total_units,
                });

                // Throttle progress messages to changes in displayed units.
                let last_units = AtomicU64::new(u64::MAX);
                let progress = |phase, frac| {
                    let units = (chd_convert::job_fraction(phase, frac) * JOB_PROGRESS_UNITS as f64)
                        .round() as u64;
                    if last_units.swap(units, Ordering::Relaxed) != units {
                        let _ = tx.send(AppMessage::OperationProgress {
                            op_id,
                            current: base_units + units,
                            total: total_units,
                        });
                        ctx.request_repaint();
                    }
                };

                let outcome =
                    match chd_convert::compress_to_chd(&chdman, &item.job, &progress, &cancel) {
                        Ok(outcome) => match outcome.verification {
                            VerificationOutcome::Verified { tracks } => {
                                let report =
                                    chd_convert::finalize_verified(&item.job, delete_sources);
                                for e in &report.m3u_errors {
                                    log::warn!("Failed to update .m3u references: {e}");
                                }
                                let delete_failures = report
                                    .delete_failures
                                    .into_iter()
                                    .map(|(path, why)| {
                                        format!(
                                            "{}: {why}",
                                            path.file_name().unwrap_or_default().to_string_lossy()
                                        )
                                    })
                                    .collect();
                                ChdCompressOutcome::Compressed {
                                    input_bytes: outcome.input_bytes,
                                    output_bytes: outcome.output_bytes,
                                    tracks,
                                    sources_deleted: report.sources_deleted,
                                    delete_failures,
                                }
                            }
                            VerificationOutcome::Mismatch { detail } => {
                                ChdCompressOutcome::VerifyFailed { detail }
                            }
                        },
                        Err(chd_convert::ChdConvertError::Cancelled) => {
                            ChdCompressOutcome::Cancelled
                        }
                        Err(e) => ChdCompressOutcome::Error {
                            message: e.to_string(),
                        },
                    };

                results.push(ChdCompressResult {
                    input_name,
                    job: item.job.clone(),
                    outcome,
                });
            }

            let _ = tx.send(AppMessage::ChdCompressComplete {
                folder_name,
                results,
            });
            let _ = tx.send(AppMessage::OperationComplete { op_id });
            ctx.request_repaint();
        },
    );
}
