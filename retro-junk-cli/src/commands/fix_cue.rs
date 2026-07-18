use std::io::Write;
use std::path::Path;

use indicatif::{ProgressBar, ProgressStyle};
use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;

use retro_junk_lib::AnalysisContext;
use retro_junk_lib::cue_fix::{CueFixPlan, CueFixProgress, execute_cue_fixes, plan_cue_fixes};

use crate::CliError;
use crate::cli_types::FixCueArgs;

/// Run the fix-cue command.
// Linear per-console cue-fix orchestration (plan, report, execute, summarize).
#[allow(clippy::too_many_lines)]
pub(crate) fn run_fix_cue(
    ctx: &AnalysisContext,
    args: &FixCueArgs,
    library_path: &Path,
    quiet: bool,
) -> Result<(), CliError> {
    let dry_run = args.dry_run;
    let no_backup = args.no_backup;
    let consoles = &args.roms.consoles;
    let root_path = library_path;

    log::info!(
        "Scanning CUE sheets in: {}",
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
    crate::log_blank();

    let Some(scan) = crate::scan_folders(ctx, root_path, consoles.as_deref()) else {
        return Ok(());
    };

    let mut total_fixed = 0usize;
    let mut total_already_standard = 0usize;
    let mut total_unfixable = 0usize;
    let mut total_errors: Vec<String> = Vec::new();
    let mut found_any = false;

    for cf in &scan.matches {
        let console = ctx.get_by_platform(cf.platform).ok_or_else(|| {
            CliError::unknown_system(format!("No analyzer for platform {:?}", cf.platform))
        })?;

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

        let progress_callback = |progress: CueFixProgress| match progress {
            CueFixProgress::Scanning { file_count } => {
                pb.set_message(format!("Found {file_count} CUE files"));
                pb.tick();
            }
            CueFixProgress::Checking {
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
            CueFixProgress::Done => {
                pb.finish_and_clear();
            }
        };

        let plan = plan_cue_fixes(&cf.path, &progress_callback);
        pb.finish_and_clear();

        // Skip consoles with no CUE files at all
        if plan.fixable.is_empty()
            && plan.already_standard.is_empty()
            && plan.unfixable.is_empty()
            && plan.errors.is_empty()
        {
            continue;
        }

        let has_issues = !plan.unfixable.is_empty() || !plan.errors.is_empty();
        let header_level = if has_issues {
            log::Level::Warn
        } else {
            log::Level::Info
        };
        log::log!(
            header_level,
            "{} {}",
            console
                .metadata
                .platform_name
                .if_supports_color(Stdout, |t| t.bold()),
            format!("({})", cf.folder_name).if_supports_color(Stdout, |t| t.dimmed()),
        );

        print_cue_fix_plan(&plan);

        if !dry_run && !plan.fixable.is_empty() {
            print!(
                "\n  Proceed with {} CUE fix(es)? [y/N] ",
                plan.fixable.len(),
            );
            std::io::stdout().flush()?;

            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;

            if input.trim().eq_ignore_ascii_case("y") {
                let summary = execute_cue_fixes(&plan, !no_backup);
                total_fixed += summary.fixed;
                total_errors.extend(summary.errors);

                log::info!(
                    "  {} {} CUE files fixed",
                    "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                    summary.fixed,
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
            total_already_standard += plan.already_standard.len();
            total_unfixable += plan.unfixable.len();
        }

        crate::log_blank();
    }

    if scan.matches.is_empty() || !found_any {
        log::info!(
            "{}",
            "No console folders found.".if_supports_color(Stdout, |t| t.dimmed()),
        );
        return Ok(());
    }

    // Print overall summary
    log::info!("{}", "Summary:".if_supports_color(Stdout, |t| t.bold()));
    if total_fixed > 0 {
        log::info!(
            "  {} {} CUE files fixed",
            "\u{2714}".if_supports_color(Stdout, |t| t.green()),
            total_fixed,
        );
    }
    if total_already_standard > 0 {
        log::info!(
            "  {} {} already standard",
            "\u{2714}".if_supports_color(Stdout, |t| t.green()),
            total_already_standard,
        );
    }
    if total_unfixable > 0 {
        log::warn!(
            "  {} {} cannot be auto-fixed",
            "?".if_supports_color(Stdout, |t| t.yellow()),
            total_unfixable,
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

/// Print the CUE fix plan for a single console folder.
fn print_cue_fix_plan(plan: &CueFixPlan) {
    // Fixable files
    for action in &plan.fixable {
        let file_name = action
            .cue_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?");
        log::info!(
            "  {} {} [{}]",
            "\u{1F527}".if_supports_color(Stdout, |t| t.green()),
            file_name.if_supports_color(Stdout, |t| t.bold()),
            action
                .report
                .summary()
                .if_supports_color(Stdout, |t| t.dimmed()),
        );
    }

    // Already standard
    if !plan.already_standard.is_empty() {
        log::info!(
            "  {} {} already standard format",
            "\u{2714}".if_supports_color(Stdout, |t| t.green()),
            plan.already_standard.len(),
        );
    }

    // Unfixable
    for (path, reason) in &plan.unfixable {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        log::warn!(
            "  {} {} ({})",
            "?".if_supports_color(Stdout, |t| t.yellow()),
            name.if_supports_color(Stdout, |t| t.dimmed()),
            reason,
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
