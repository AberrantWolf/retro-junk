//! Compress disc images to CHD via chdman, with round-trip verification.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use indicatif::{ProgressBar, ProgressStyle};
use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;

use retro_junk_lib::chd_convert::{
    ChdPhase, Chdman, ChdmanUnavailable, VerificationOutcome, compress_to_chd, finalize_verified,
    job_fraction, plan_batch,
};
use retro_junk_lib::scanner::{extension_set, scan_game_entries};
use retro_junk_lib::util::format_bytes_approx;
use retro_junk_lib::{AnalysisContext, Platform};

use crate::CliError;

#[cfg(test)]
#[path = "compress_tests.rs"]
mod tests;

/// Run the compress command.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_compress(
    ctx: &AnalysisContext,
    dry_run: bool,
    delete_sources: bool,
    assume_yes: bool,
    chdman_override: Option<PathBuf>,
    consoles: Option<Vec<Platform>>,
    limit: Option<usize>,
    library_path: PathBuf,
    quiet: bool,
) -> Result<(), CliError> {
    let chdman = match Chdman::detect(chdman_override.as_deref().unwrap_or(Path::new(""))) {
        Ok(c) => c,
        Err(e) => {
            log::error!("{}", e.to_string().if_supports_color(Stdout, |t| t.red()));
            log::error!("{}", ChdmanUnavailable::install_hint());
            log::error!("If chdman is installed somewhere unusual, pass --chdman <path>.");
            return Err(CliError::external_tool(
                "chdman is required for CHD compression",
            ));
        }
    };

    log::info!(
        "Using chdman {} ({})",
        if chdman.version.is_empty() {
            "(unknown version)"
        } else {
            &chdman.version
        },
        chdman
            .path
            .display()
            .if_supports_color(Stdout, |t| t.dimmed()),
    );
    if dry_run {
        log::info!(
            "{}",
            "Dry run: no files will be created or deleted"
                .if_supports_color(Stdout, |t| t.dimmed()),
        );
    }
    crate::log_blank();

    let scan = match crate::scan_folders(ctx, &library_path, &consoles) {
        Some(s) => s,
        None => return Ok(()),
    };

    let mut total_compressed = 0usize;
    let mut total_failed = 0usize;
    let mut total_in_bytes = 0u64;
    let mut total_out_bytes = 0u64;

    for cf in &scan.matches {
        let console = ctx.get_by_platform(cf.platform).ok_or_else(|| {
            CliError::unknown_system(format!("No analyzer for platform {:?}", cf.platform))
        })?;
        let analyzer = console.analyzer.as_ref();

        // Plan: every compressible disc image in this folder. plan_batch is
        // the single place skip classification and duplicate-output
        // detection live; it also drops companion track files (bin/img)
        // silently, the same policy the GUI applies.
        let extensions = extension_set(analyzer.file_extensions());
        let entries = scan_game_entries(&cf.path, &extensions)?;
        let inputs: Vec<PathBuf> = entries
            .iter()
            .flat_map(|e| e.all_files().iter().cloned())
            .collect();
        let batch = plan_batch(&inputs, analyzer);
        let mut jobs = batch.jobs;
        let already_chd = batch.already_chd;
        let skips = batch.skips;
        if let Some(limit) = limit {
            jobs.truncate(limit);
        }

        if jobs.is_empty() && skips.is_empty() && already_chd == 0 {
            continue;
        }

        log::info!(
            "{} {}",
            console
                .metadata
                .platform_name
                .if_supports_color(Stdout, |t| t.bold()),
            format!("({})", cf.folder_name).if_supports_color(Stdout, |t| t.dimmed()),
        );
        if already_chd > 0 {
            log::info!(
                "  {} {} already CHD",
                "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                already_chd,
            );
        }
        for skip in &skips {
            let name = skip
                .input
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            log::warn!(
                "  {} {}: {}",
                "?".if_supports_color(Stdout, |t| t.yellow()),
                name.if_supports_color(Stdout, |t| t.dimmed()),
                skip.error,
            );
        }
        let folder_in_bytes: u64 = jobs.iter().map(|j| j.input_bytes).sum();
        for job in &jobs {
            log::info!(
                "  {} {} ({})",
                "\u{1F5DC}".if_supports_color(Stdout, |t| t.cyan()),
                job.input
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .if_supports_color(Stdout, |t| t.bold()),
                format_bytes_approx(job.input_bytes).if_supports_color(Stdout, |t| t.dimmed()),
            );
        }

        if jobs.is_empty() || dry_run {
            crate::log_blank();
            continue;
        }

        if !assume_yes {
            print!(
                "\n  Compress {} disc(s) ({})? [y/N] ",
                jobs.len(),
                format_bytes_approx(folder_in_bytes),
            );
            std::io::stdout().flush()?;
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            if !input.trim().eq_ignore_ascii_case("y") {
                log::info!("  {}", "Skipped".if_supports_color(Stdout, |t| t.dimmed()));
                crate::log_blank();
                continue;
            }
        }

        let cancel = AtomicBool::new(false);
        for job in &jobs {
            let name = job
                .input
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            let pb = if quiet {
                ProgressBar::hidden()
            } else {
                let pb = ProgressBar::new(100);
                pb.set_style(
                    ProgressStyle::with_template("  {bar:30.cyan} {percent:>3}% {msg}")
                        .expect("static pattern"),
                );
                pb
            };
            let last_phase = std::cell::Cell::new(None::<ChdPhase>);
            let progress = |phase: ChdPhase, frac: f64| {
                pb.set_position((job_fraction(phase, frac) * 100.0) as u64);
                if last_phase.replace(Some(phase)) != Some(phase) {
                    pb.set_message(match phase {
                        ChdPhase::Compressing => format!("compressing {name}"),
                        ChdPhase::Extracting => format!("verifying {name} (extract)"),
                        ChdPhase::Verifying => format!("verifying {name} (compare)"),
                    });
                }
            };

            match compress_to_chd(&chdman, job, &progress, &cancel) {
                Ok(outcome) => {
                    pb.finish_and_clear();
                    match outcome.verification {
                        VerificationOutcome::Verified { tracks } => {
                            total_compressed += 1;
                            total_in_bytes += outcome.input_bytes;
                            total_out_bytes += outcome.output_bytes;
                            let mut note = String::new();
                            if delete_sources {
                                let report = finalize_verified(job, true);
                                for e in &report.m3u_errors {
                                    log::warn!("  Failed to update .m3u references: {e}");
                                }
                                note = if report.delete_failures.is_empty() {
                                    ", originals deleted".to_string()
                                } else {
                                    format!(
                                        ", {} original(s) could not be deleted",
                                        report.delete_failures.len()
                                    )
                                };
                            }
                            log::info!(
                                "  {} {} \u{2192} {} ({:.0}%, {} track(s) verified{})",
                                "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                                name.if_supports_color(Stdout, |t| t.bold()),
                                format_bytes_approx(outcome.output_bytes),
                                outcome.output_bytes as f64 / outcome.input_bytes.max(1) as f64
                                    * 100.0,
                                tracks,
                                note,
                            );
                        }
                        VerificationOutcome::Mismatch { detail } => {
                            total_failed += 1;
                            log::warn!(
                                "  {} {}: verification failed ({detail}) — .chd discarded, originals kept",
                                "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                                name,
                            );
                        }
                    }
                }
                Err(e) => {
                    pb.finish_and_clear();
                    total_failed += 1;
                    log::warn!(
                        "  {} {}: {}",
                        "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                        name,
                        e,
                    );
                }
            }
        }
        crate::log_blank();
    }

    log::info!("{}", "Summary:".if_supports_color(Stdout, |t| t.bold()));
    if total_compressed > 0 {
        log::info!(
            "  {} {} disc(s) compressed and verified: {} \u{2192} {} (saved {})",
            "\u{2714}".if_supports_color(Stdout, |t| t.green()),
            total_compressed,
            format_bytes_approx(total_in_bytes),
            format_bytes_approx(total_out_bytes),
            format_bytes_approx(total_in_bytes.saturating_sub(total_out_bytes)),
        );
    } else {
        log::info!("  Nothing compressed.");
    }
    if total_failed > 0 {
        log::warn!(
            "  {} {} failed",
            "\u{2718}".if_supports_color(Stdout, |t| t.red()),
            total_failed,
        );
    }

    Ok(())
}
