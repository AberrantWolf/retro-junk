use std::path::Path;

use indicatif::{ProgressBar, ProgressStyle};
use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;

use retro_junk_lib::organize::{
    OrganizeOptions, OrganizeProgress, execute_organize_job, plan_organize,
};
use retro_junk_lib::{AnalysisContext, DatSource};

use crate::CliError;
use crate::cli_types::{ConsoleFilterArgs, OrganizeArgs};

// Linear per-console organize orchestration (plan, report, execute, summarize).
#[allow(clippy::too_many_lines)]
pub(crate) fn run_organize(
    ctx: &AnalysisContext,
    args: OrganizeArgs,
    library_path: &Path,
    quiet: bool,
) -> Result<(), CliError> {
    let OrganizeArgs {
        dry_run,
        roms: ConsoleFilterArgs { consoles, limit },
        include_single_disc,
    } = args;
    let options = OrganizeOptions {
        limit,
        include_single_disc,
    };
    let db_path = retro_junk_lib::settings::ensure_catalog_database_location()
        .map_err(|error| CliError::database(error.to_string()))?;
    let catalog = retro_junk_db::open_database(&db_path).map_err(|error| {
        CliError::database(format!(
            "Failed to open catalog database at {}: {error}",
            db_path.display()
        ))
    })?;

    log::info!(
        "Organizing disc images in: {}",
        library_path
            .display()
            .if_supports_color(Stdout, |t| t.cyan()),
    );
    if dry_run {
        log::info!(
            "{}",
            "Dry run: no files will be moved".if_supports_color(Stdout, |t| t.dimmed()),
        );
    }
    if include_single_disc {
        log::info!(
            "{}",
            "Including single-disc games".if_supports_color(Stdout, |t| t.dimmed()),
        );
    }
    crate::log_blank();

    let Some(scan) = crate::scan_folders(ctx, library_path, consoles.as_deref()) else {
        return Ok(());
    };

    let mut total_jobs = 0usize;
    let mut total_files_moved = 0usize;
    let mut total_unmatched = 0usize;
    let mut total_skipped = 0usize;
    let mut total_errors: Vec<String> = Vec::new();
    let mut found_any = false;

    for cf in &scan.matches {
        let console = ctx.get_by_platform(cf.platform).ok_or_else(|| {
            CliError::unknown_system(format!("No analyzer for platform {:?}", cf.platform))
        })?;

        // Only process disc-based consoles (Redump DATs)
        if console.analyzer.dat_source() != DatSource::Redump {
            continue;
        }

        if !console.analyzer.has_dat_support() {
            log::warn!(
                "  {} Skipping \"{}\" — no DAT support yet",
                "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                cf.folder_name,
            );
            continue;
        }

        found_any = true;

        // Progress bar
        let pb = if quiet {
            ProgressBar::hidden()
        } else {
            let pb = ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::with_template("  {spinner:.cyan} {msg}")
                    .expect("static pattern")
                    .tick_chars("/-\\|"),
            );
            pb
        };

        let progress_callback = |progress: OrganizeProgress| match progress {
            OrganizeProgress::Scanning { file_count } => {
                pb.set_message(format!("Found {file_count} loose disc files"));
                pb.tick();
            }
            OrganizeProgress::ReadingDisc {
                ref file_name,
                index,
                total,
            } => {
                pb.set_message(format!("[{}/{}] Reading {}", index + 1, total, file_name));
                pb.tick();
            }
            OrganizeProgress::Hashing {
                ref file_name,
                bytes_done,
                bytes_total,
            } => {
                if let Some(pct) = (bytes_done * 100).checked_div(bytes_total) {
                    pb.set_message(format!("Hashing {file_name} ({pct}%)"));
                }
                pb.tick();
            }
            OrganizeProgress::Done => {
                pb.finish_and_clear();
            }
        };

        match plan_organize(
            &catalog,
            &cf.path,
            console.analyzer.as_ref(),
            &options,
            &progress_callback,
        ) {
            Ok(plan) => {
                pb.finish_and_clear();

                log::info!(
                    "{} {}",
                    console
                        .metadata
                        .platform_name
                        .if_supports_color(Stdout, |t| t.bold()),
                    format!("({})", cf.folder_name).if_supports_color(Stdout, |t| t.dimmed()),
                );

                if plan.jobs.is_empty()
                    && plan.unmatched.is_empty()
                    && plan.skipped_single_disc == 0
                {
                    log::info!(
                        "  {}",
                        "No loose disc files found".if_supports_color(Stdout, |t| t.dimmed()),
                    );
                    crate::log_blank();
                    continue;
                }

                // Print plan summary
                if !plan.jobs.is_empty() {
                    log::info!(
                        "  {} {} to create:",
                        format!("{}", plan.jobs.len()).if_supports_color(Stdout, |t| t.green()),
                        if plan.jobs.len() == 1 {
                            "folder"
                        } else {
                            "folders"
                        },
                    );
                    for job in &plan.jobs {
                        let disc_count = job.entry_points.len();
                        let companion_count = job.companion_files.len();
                        let total_files = disc_count + companion_count;
                        log::info!(
                            "    {} ({} {}, {} total files)",
                            job.game_name.if_supports_color(Stdout, |t| t.cyan()),
                            disc_count,
                            if disc_count == 1 { "disc" } else { "discs" },
                            total_files,
                        );
                    }
                }

                if !plan.unmatched.is_empty() {
                    log::warn!(
                        "  {} unmatched:",
                        format!("{}", plan.unmatched.len())
                            .if_supports_color(Stdout, |t| t.yellow()),
                    );
                    for uf in &plan.unmatched {
                        let name = uf.path.file_name().unwrap_or_default().to_string_lossy();
                        log::warn!("    {} — {}", name, uf.reason);
                    }
                }

                if plan.skipped_single_disc > 0 {
                    log::info!(
                        "  {} single-disc games skipped (use --include-single-disc)",
                        format!("{}", plan.skipped_single_disc)
                            .if_supports_color(Stdout, |t| t.dimmed()),
                    );
                }

                total_unmatched += plan.unmatched.len();
                total_skipped += plan.skipped_single_disc;

                // Execute if not dry run
                if !dry_run && !plan.jobs.is_empty() {
                    for job in &plan.jobs {
                        let result = execute_organize_job(job);
                        if result.folder_created {
                            total_jobs += 1;
                        }
                        total_files_moved += result.files_moved;
                        for err in &result.errors {
                            log::error!("    {err}");
                            total_errors.push(err.clone());
                        }
                    }
                }

                crate::log_blank();
            }
            Err(e) => {
                pb.finish_and_clear();
                crate::log_dat_error(
                    console.metadata.platform_name,
                    &cf.folder_name,
                    console.analyzer.short_name(),
                    &e,
                );
            }
        }
    }

    if !found_any {
        log::info!(
            "{}",
            "No disc-based consoles found in library".if_supports_color(Stdout, |t| t.dimmed()),
        );
    }

    // Final summary
    if total_jobs > 0 || total_unmatched > 0 || total_skipped > 0 {
        crate::log_blank();
        if total_jobs > 0 {
            log::info!(
                "{} Created {} folders, moved {} files",
                "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                total_jobs,
                total_files_moved,
            );
        }
        if total_unmatched > 0 {
            log::warn!(
                "{} {} files could not be matched",
                "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                total_unmatched,
            );
        }
        if !total_errors.is_empty() {
            log::error!(
                "{} {} errors occurred",
                "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                total_errors.len(),
            );
        }
    }

    Ok(())
}
