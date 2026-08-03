//! CHD compression: plan jobs for a set of selected inputs, then run the
//! compress → round-trip-verify → (optionally) delete pipeline via the
//! shared chdman wrapper in `retro_junk_lib::chd_convert`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use retro_junk_core::{ChdExtensionRole, Platform};
use retro_junk_io::ProgressUnit;
use retro_junk_lib::AnalysisContext;
use retro_junk_lib::chd_convert::{self, Chdman, CompressionJob, VerificationOutcome};
use retro_junk_lib::util::format_bytes_approx;

use super::OpCtx;

/// Progress-bar resolution per job (fractions of one disc's pipeline).
const JOB_PROGRESS_UNITS: u64 = 1000;

/// One disc queued in the CHD-compression confirmation prompt.
pub struct ChdCompressItem {
    pub job: CompressionJob,
    /// Precomputed at plan time: "{input} ({size}) -> {output}". Avoids
    /// re-deriving file-name strings and byte formatting every frame a
    /// confirmation dialog is open.
    pub display_line: String,
}

/// A selected disc that cannot be compressed, with the reason shown to the user.
pub struct ChdCompressSkip {
    pub entry_name: String,
    pub reason: String,
}

/// Everything a "Compress to CHD" confirmation needs.
///
/// Also carries the chdman probe result: when chdman is missing the prompt
/// becomes the explanation of why compression is unavailable and how to
/// install it, instead of silently hiding the feature.
pub struct ChdCompressPrompt {
    pub folder_name: String,
    pub items: Vec<ChdCompressItem>,
    pub skipped: Vec<ChdCompressSkip>,
    pub chdman: Result<Chdman, chd_convert::ChdmanUnavailable>,
    /// Delete original files after a verified compression.
    pub delete_sources: bool,
}

pub struct ChdCompressResult {
    /// The chdman input this result is for (one entry can hold several discs).
    pub input_name: String,
    /// The planned job this result came from — carries `input`/`output`/
    /// `source_files` so a completion handler can look up the affected
    /// library entry by path (rename-proof) instead of by display name, and
    /// know exactly which files were deleted.
    pub job: CompressionJob,
    pub outcome: ChdCompressOutcome,
}

pub enum ChdCompressOutcome {
    /// Compressed and round-trip verified. The output path is available via
    /// the sibling `ChdCompressResult::job.output` — not duplicated here.
    Compressed {
        input_bytes: u64,
        output_bytes: u64,
        tracks: usize,
        sources_deleted: bool,
        delete_failures: Vec<String>,
    },
    /// Round-trip verification failed; the .chd was removed and the original
    /// files were kept.
    VerifyFailed {
        detail: String,
    },
    Error {
        message: String,
    },
    Cancelled,
}

/// Whether a platform's analyzer supports CHD compression for any source
/// kind. Gates frontend entry points so cartridge consoles never offer it.
#[must_use]
pub fn platform_supports_chd(context: &AnalysisContext, platform: Platform) -> bool {
    context.get_by_platform(platform).is_some_and(|registered| {
        registered
            .analyzer
            .chd_extensions()
            .iter()
            .any(|(_, role)| matches!(role, ChdExtensionRole::Source(_)))
    })
}

/// Probe chdman and plan the compression batch for `inputs`.
///
/// This is the blocking half of opening the dialog: the chdman probe is a
/// subprocess spawn, and `plan_batch` parses cue/gdi sheets and stats every
/// track file, so it must not run on a render thread. `plan_batch` owns skip
/// classification and duplicate-output detection (including the "bin"/"img"
/// companion-data noise inside .m3u folders).
///
/// `arrow` is the separator drawn between input and output in each item's
/// precomputed display line — frontends pass their own glyph.
#[must_use]
pub fn plan_prompt(
    context: &AnalysisContext,
    platform: Platform,
    folder_name: String,
    inputs: &[PathBuf],
    chdman_setting: &str,
    arrow: &str,
) -> ChdCompressPrompt {
    let chdman = Chdman::detect_from_setting(chdman_setting);

    let (items, skipped) = match context.get_by_platform(platform) {
        Some(registered) => {
            let analyzer = registered.analyzer.as_ref();
            let batch = chd_convert::plan_batch(inputs, analyzer);
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
                    // Precompute the display line at plan time so a
                    // confirmation dialog doesn't re-derive filename strings
                    // and byte formatting every frame.
                    let display_line = format!(
                        "{input_name} ({}) {arrow} {output_name}",
                        format_bytes_approx(job.input_bytes),
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

    ChdCompressPrompt {
        folder_name,
        items,
        skipped,
        chdman,
        delete_sources: false,
    }
}

/// Run the confirmed batch: compress each job, round-trip verify it, and on
/// a verified result finalize it (update .m3u references, optionally delete
/// the sources). Cancellation marks the remaining jobs `Cancelled` rather
/// than dropping them, so every planned job gets a reported outcome.
///
/// Progress is reported through `ctx.progress` on an abstract unit scale:
/// [`JOB_PROGRESS_UNITS`] units per job, throttled to changes in the
/// displayed unit count.
pub fn run_compression(
    chdman: &Chdman,
    items: &[ChdCompressItem],
    delete_sources: bool,
    ctx: &OpCtx,
) -> Vec<ChdCompressResult> {
    let total_units = items.len() as u64 * JOB_PROGRESS_UNITS;
    let mut results: Vec<ChdCompressResult> = Vec::new();

    for (i, item) in items.iter().enumerate() {
        let input_name = item
            .job
            .input
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        if ctx.cancelled() {
            results.push(ChdCompressResult {
                input_name,
                job: item.job.clone(),
                outcome: ChdCompressOutcome::Cancelled,
            });
            continue;
        }

        let base_units = i as u64 * JOB_PROGRESS_UNITS;
        (ctx.progress)(
            "Compressing to CHD",
            ProgressUnit::Items,
            base_units,
            total_units,
        );

        // Throttle progress reports to changes in displayed units.
        let last_units = AtomicU64::new(u64::MAX);
        let progress = |phase, frac| {
            let units =
                (chd_convert::job_fraction(phase, frac) * JOB_PROGRESS_UNITS as f64).round() as u64;
            if last_units.swap(units, Ordering::Relaxed) != units {
                (ctx.progress)(
                    "Compressing to CHD",
                    ProgressUnit::Items,
                    base_units + units,
                    total_units,
                );
            }
        };

        let outcome = match chd_convert::compress_to_chd(chdman, &item.job, &progress, ctx.cancel) {
            Ok(outcome) => match outcome.verification {
                VerificationOutcome::Verified { tracks } => {
                    let report = chd_convert::finalize_verified(&item.job, delete_sources);
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
            Err(chd_convert::ChdConvertError::Cancelled) => ChdCompressOutcome::Cancelled,
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

    results
}
