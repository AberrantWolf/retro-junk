use std::io::Write;
use std::path::PathBuf;

use indicatif::{ProgressBar, ProgressStyle};
use log::Level;
use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;

use retro_junk_lib::repair::{
    RepairOptions, RepairPlan, RepairProgress, execute_repairs, plan_repairs,
};
use retro_junk_lib::{AnalysisContext, Platform};

use crate::CliError;

/// Run the repair command.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_repair(
    ctx: &AnalysisContext,
    dry_run: bool,
    no_backup: bool,
    consoles: Option<Vec<Platform>>,
    limit: Option<usize>,
    root: Option<PathBuf>,
    dat_dir: Option<PathBuf>,
    quiet: bool,
) -> Result<(), CliError> {
    let root_path =
        root.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let repair_options = RepairOptions {
        dat_dir,
        limit,
        create_backup: !no_backup,
    };

    log::warn!(
        "{}",
        "The repair command is experimental and may not work correctly for all ROMs."
            .if_supports_color(Stdout, |t| t.yellow()),
    );

    log::info!(
        "Scanning ROMs in: {}",
        root_path.display().if_supports_color(Stdout, |t| t.cyan()),
    );
    if dry_run {
        log::info!(
            "{}",
            "Dry run: no files will be modified".if_supports_color(Stdout, |t| t.dimmed()),
        );
    }
    if no_backup {
        log::info!(
            "{}",
            "Backups disabled".if_supports_color(Stdout, |t| t.dimmed()),
        );
    }
    if let Some(n) = limit {
        log::info!(
            "{}",
            format!("Limit: {} ROMs per console", n).if_supports_color(Stdout, |t| t.dimmed()),
        );
    }
    crate::log_blank();

    let scan = match crate::scan_folders(ctx, &root_path, &consoles) {
        Some(s) => s,
        None => return Ok(()),
    };

    let mut total_repaired = 0usize;
    let mut total_already_correct = 0usize;
    let mut total_no_match = 0usize;
    let mut total_errors: Vec<String> = Vec::new();
    let mut found_any = false;

    for cf in &scan.matches {
        let console = ctx.get_by_platform(cf.platform).ok_or_else(|| {
            CliError::unknown_system(format!("No analyzer for platform {:?}", cf.platform))
        })?;

        if !console.analyzer.has_dat_support() {
            log::warn!(
                "  {} Skipping \"{}\" — no DAT support yet",
                "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                cf.folder_name,
            );
            continue;
        }

        found_any = true;

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

        let progress_callback = |progress: RepairProgress| match progress {
            RepairProgress::Scanning { file_count } => {
                pb.set_message(format!("Found {file_count} ROM files"));
                pb.tick();
            }
            RepairProgress::Checking {
                ref file_name,
                file_index,
                total,
            } => {
                pb.set_message(format!(
                    "[{}/{}] Checking {}",
                    file_index + 1,
                    total,
                    file_name
                ));
                pb.tick();
            }
            RepairProgress::TryingRepair {
                ref file_name,
                ref strategy_desc,
            } => {
                pb.set_message(format!("{}: {}", file_name, strategy_desc));
                pb.tick();
            }
            RepairProgress::Done => {
                pb.finish_and_clear();
            }
        };

        match plan_repairs(
            &cf.path,
            console.analyzer.as_ref(),
            &repair_options,
            &progress_callback,
        ) {
            Ok(plan) => {
                pb.finish_and_clear();

                let has_issues = !plan.no_match.is_empty() || !plan.errors.is_empty();
                let header_level = if has_issues { Level::Warn } else { Level::Info };
                log::log!(
                    header_level,
                    "{} {}",
                    console
                        .metadata
                        .platform_name
                        .if_supports_color(Stdout, |t| t.bold()),
                    format!("({})", cf.folder_name).if_supports_color(Stdout, |t| t.dimmed()),
                );

                print_repair_plan(&plan);

                if !dry_run && !plan.repairable.is_empty() {
                    print!("\n  Proceed with {} repairs? [y/N] ", plan.repairable.len(),);
                    std::io::stdout().flush()?;

                    let mut input = String::new();
                    std::io::stdin().read_line(&mut input)?;

                    if input.trim().eq_ignore_ascii_case("y") {
                        let summary = execute_repairs(&plan, repair_options.create_backup);
                        total_repaired += summary.repaired;
                        total_already_correct += summary.already_correct;
                        total_errors.extend(summary.errors);

                        log::info!(
                            "  {} {} files repaired",
                            "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                            summary.repaired,
                        );
                        if summary.backups_created > 0 {
                            log::info!(
                                "  {} {} backups created",
                                "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                                summary.backups_created,
                            );
                        }
                    } else {
                        log::info!("  {}", "Skipped".if_supports_color(Stdout, |t| t.dimmed()));
                    }
                } else {
                    total_already_correct += plan.already_correct.len();
                    total_no_match += plan.no_match.len();
                }
            }
            Err(e) => {
                pb.finish_and_clear();
                crate::log_dat_error(
                    console.metadata.platform_name,
                    &cf.folder_name,
                    console.metadata.short_name,
                    &e,
                );
            }
        }
        crate::log_blank();
    }

    if scan.matches.is_empty() || !found_any {
        log::info!(
            "{}",
            "No console folders with DAT support found.".if_supports_color(Stdout, |t| t.dimmed()),
        );
        return Ok(());
    }

    // Print overall summary
    log::info!("{}", "Summary:".if_supports_color(Stdout, |t| t.bold()));
    if total_repaired > 0 {
        log::info!(
            "  {} {} files repaired",
            "\u{2714}".if_supports_color(Stdout, |t| t.green()),
            total_repaired,
        );
    }
    if total_already_correct > 0 {
        log::info!(
            "  {} {} already correct",
            "\u{2714}".if_supports_color(Stdout, |t| t.green()),
            total_already_correct,
        );
    }
    if total_no_match > 0 {
        log::warn!(
            "  {} {} no matching repair",
            "?".if_supports_color(Stdout, |t| t.yellow()),
            total_no_match,
        );
    }
    for error in &total_errors {
        log::warn!(
            "  {} {}",
            "\u{2718}".if_supports_color(Stdout, |t| t.red()),
            error,
        );
    }

    Ok(())
}

/// Print the repair plan for a single console.
pub(crate) fn print_repair_plan(plan: &RepairPlan) {
    // Repairable files
    for action in &plan.repairable {
        let file_name = action
            .file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?");
        log::info!(
            "  {} {} {} \"{}\" [{}]",
            "\u{1F527}".if_supports_color(Stdout, |t| t.green()),
            file_name.if_supports_color(Stdout, |t| t.bold()),
            "\u{2192}".if_supports_color(Stdout, |t| t.green()),
            action.game_name,
            action
                .method
                .description()
                .if_supports_color(Stdout, |t| t.dimmed()),
        );
    }

    // Already correct
    if !plan.already_correct.is_empty() {
        log::info!(
            "  {} {} already correct",
            "\u{2714}".if_supports_color(Stdout, |t| t.green()),
            plan.already_correct.len(),
        );
    }

    // No match
    for path in &plan.no_match {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        log::warn!(
            "  {} {} (no matching repair)",
            "?".if_supports_color(Stdout, |t| t.yellow()),
            name.if_supports_color(Stdout, |t| t.dimmed()),
        );
    }

    // Errors
    for (path, msg) in &plan.errors {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        log::warn!(
            "  {} {}: {}",
            "\u{2718}".if_supports_color(Stdout, |t| t.red()),
            name.if_supports_color(Stdout, |t| t.dimmed()),
            msg,
        );
    }
}
