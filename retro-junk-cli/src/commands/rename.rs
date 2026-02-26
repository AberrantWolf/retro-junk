use std::io::Write;
use std::path::PathBuf;

use indicatif::{ProgressBar, ProgressStyle};
use log::Level;
use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;

use retro_junk_lib::rename::{
    M3uAction, RenameOptions, RenamePlan, RenameProgress, SerialWarningKind, execute_renames,
    format_match_method, plan_renames,
};
use retro_junk_lib::{AnalysisContext, Platform};

pub(crate) fn run_rename(
    ctx: &AnalysisContext,
    dry_run: bool,
    hash_mode: bool,
    consoles: Option<Vec<Platform>>,
    limit: Option<usize>,
    root: Option<PathBuf>,
    dat_dir: Option<PathBuf>,
    quiet: bool,
) {
    let root_path =
        root.unwrap_or_else(|| std::env::current_dir().expect("Failed to get current directory"));

    let rename_options = RenameOptions {
        hash_mode,
        dat_dir,
        limit,
        ..Default::default()
    };

    log::info!(
        "Scanning ROMs in: {}",
        root_path.display().if_supports_color(Stdout, |t| t.cyan()),
    );
    if hash_mode {
        log::info!(
            "{}",
            "Hash mode: computing CRC32 for all files".if_supports_color(Stdout, |t| t.dimmed()),
        );
    }
    if dry_run {
        log::info!(
            "{}",
            "Dry run: no files will be renamed".if_supports_color(Stdout, |t| t.dimmed()),
        );
    }
    if let Some(n) = limit {
        log::info!(
            "{}",
            format!("Limit: {} ROMs per console", n).if_supports_color(Stdout, |t| t.dimmed()),
        );
    }
    log::info!("");

    let scan = match crate::scan_folders(ctx, &root_path, &consoles) {
        Some(s) => s,
        None => return,
    };

    let mut total_renamed = 0usize;
    let mut total_already_correct = 0usize;
    let mut total_unmatched = 0usize;
    let mut total_errors: Vec<String> = Vec::new();
    let mut total_conflicts: Vec<String> = Vec::new();
    let mut found_any = false;

    for cf in &scan.matches {
        let console = ctx.get_by_platform(cf.platform).unwrap();

        // Check if this system has DAT support via the analyzer trait
        if !console.analyzer.has_dat_support() {
            log::warn!(
                "  {} Skipping \"{}\" — no DAT support yet",
                "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                cf.folder_name,
            );
            continue;
        }

        found_any = true;

        // Set up progress bar (hidden in quiet mode)
        let pb = if quiet {
            ProgressBar::hidden()
        } else {
            let pb = ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::with_template("  {spinner:.cyan} {msg}")
                    .unwrap()
                    .tick_chars("/-\\|"),
            );
            pb
        };

        let progress_callback = |progress: RenameProgress| match progress {
            RenameProgress::ScanningConsole { file_count, .. } => {
                pb.set_message(format!("Found {file_count} ROM files"));
                pb.tick();
            }
            RenameProgress::MatchingFile {
                ref file_name,
                file_index,
                total,
            } => {
                pb.set_message(format!(
                    "[{}/{}] Matching {}",
                    file_index + 1,
                    total,
                    file_name
                ));
                pb.tick();
            }
            RenameProgress::Hashing {
                ref file_name,
                bytes_done,
                bytes_total,
            } => {
                if bytes_total > 0 {
                    let pct = (bytes_done * 100) / bytes_total;
                    pb.set_message(format!("Hashing {} ({pct}%)", file_name));
                }
                pb.tick();
            }
            RenameProgress::Done => {
                pb.finish_and_clear();
            }
        };

        match plan_renames(
            &cf.path,
            console.analyzer.as_ref(),
            &rename_options,
            &progress_callback,
        ) {
            Ok(plan) => {
                pb.finish_and_clear();

                // Determine if plan has issues (affects header level in quiet mode)
                let has_issues = !plan.unmatched.is_empty()
                    || !plan.conflicts.is_empty()
                    || !plan.discrepancies.is_empty()
                    || !plan.serial_warnings.is_empty();

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

                print_rename_plan(&plan);

                let has_work = !plan.renames.is_empty()
                    || !plan.m3u_actions.is_empty()
                    || !plan.broken_cue_files.is_empty()
                    || !plan.broken_m3u_files.is_empty()
                    || !plan.misnamed_m3u_playlists.is_empty();
                if !dry_run && has_work {
                    // Prompt for confirmation (raw print — user interaction)
                    let m3u_count = plan.m3u_actions.len();
                    let cue_count = plan.broken_cue_files.len();
                    let m3u_fix_count = plan.broken_m3u_files.len();
                    let m3u_rename_count = plan.misnamed_m3u_playlists.len();
                    let mut parts = Vec::new();
                    if !plan.renames.is_empty() {
                        parts.push(format!("{} renames", plan.renames.len()));
                    }
                    if m3u_count > 0 || m3u_rename_count > 0 {
                        let total = m3u_count + m3u_rename_count;
                        parts.push(format!("{} m3u updates", total));
                    }
                    if cue_count > 0 || m3u_fix_count > 0 {
                        let total = cue_count + m3u_fix_count;
                        parts.push(format!("{} reference fixes", total));
                    }
                    print!("\n  Proceed with {}? [y/N] ", parts.join(" and "));
                    std::io::stdout().flush().unwrap();

                    let mut input = String::new();
                    std::io::stdin().read_line(&mut input).unwrap();

                    if input.trim().eq_ignore_ascii_case("y") {
                        let summary = execute_renames(&plan);
                        total_renamed += summary.renamed;
                        total_already_correct += summary.already_correct;
                        total_errors.extend(summary.errors);
                        total_conflicts.extend(summary.conflicts);

                        log::info!(
                            "  {} {} files renamed",
                            "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                            summary.renamed,
                        );
                        if summary.m3u_folders_renamed > 0 {
                            log::info!(
                                "  {} {} m3u folders renamed",
                                "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                                summary.m3u_folders_renamed,
                            );
                        }
                        if summary.m3u_playlists_written > 0 {
                            log::info!(
                                "  {} {} m3u playlists written",
                                "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                                summary.m3u_playlists_written,
                            );
                        }
                        if summary.m3u_playlists_renamed > 0 {
                            log::info!(
                                "  {} {} m3u playlists renamed",
                                "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                                summary.m3u_playlists_renamed,
                            );
                        }
                        let ref_fixes = summary.cue_files_updated + summary.m3u_references_updated;
                        if ref_fixes > 0 {
                            log::info!(
                                "  {} {} file references fixed",
                                "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                                ref_fixes,
                            );
                        }
                    } else {
                        log::info!("  {}", "Skipped".if_supports_color(Stdout, |t| t.dimmed()));
                    }
                } else {
                    total_already_correct += plan.already_correct.len();
                    total_unmatched += plan.unmatched.len();
                    total_conflicts.extend(
                        plan.conflicts
                            .iter()
                            .map(|(_, msg): &(PathBuf, String)| msg.clone()),
                    );
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
        log::info!("");
    }

    if scan.matches.is_empty() || !found_any {
        log::info!(
            "{}",
            "No console folders with DAT support found.".if_supports_color(Stdout, |t| t.dimmed()),
        );
        log::info!("");
        log::info!("Supported systems for rename:");
        for console in ctx.consoles() {
            let dat_names = console.analyzer.dat_names();
            if !dat_names.is_empty() {
                log::info!(
                    "  {} [{}]",
                    console.metadata.short_name,
                    dat_names.join(", ")
                );
            }
        }
        return;
    }

    // Print overall summary
    log::info!("{}", "Summary:".if_supports_color(Stdout, |t| t.bold()));
    if total_renamed > 0 {
        log::info!(
            "  {} {} files renamed",
            "\u{2714}".if_supports_color(Stdout, |t| t.green()),
            total_renamed,
        );
    }
    if total_already_correct > 0 {
        log::info!(
            "  {} {} already correctly named",
            "\u{2714}".if_supports_color(Stdout, |t| t.green()),
            total_already_correct,
        );
    }
    if total_unmatched > 0 {
        log::warn!(
            "  {} {} unmatched",
            "?".if_supports_color(Stdout, |t| t.yellow()),
            total_unmatched,
        );
    }
    for conflict in &total_conflicts {
        log::warn!(
            "  {} {}",
            "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
            conflict,
        );
    }
    for error in &total_errors {
        log::warn!(
            "  {} {}",
            "\u{2718}".if_supports_color(Stdout, |t| t.red()),
            error,
        );
    }
}

/// Print the rename plan for a single console.
pub(crate) fn print_rename_plan(plan: &RenamePlan) {
    // Renames
    for rename in &plan.renames {
        let source_name = rename
            .source
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?");
        let target_name = rename
            .target
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?");

        let method_str = format_match_method(&rename.matched_by);

        log::info!(
            "  {} {} {} {} {}",
            "\u{2192}".if_supports_color(Stdout, |t| t.green()),
            source_name.if_supports_color(Stdout, |t| t.dimmed()),
            "\u{2192}".if_supports_color(Stdout, |t| t.green()),
            target_name.if_supports_color(Stdout, |t| t.bold()),
            format!("[{method_str}]").if_supports_color(Stdout, |t| t.dimmed()),
        );
    }

    // Already correct
    if !plan.already_correct.is_empty() {
        log::info!(
            "  {} {} already correctly named",
            "\u{2714}".if_supports_color(Stdout, |t| t.green()),
            plan.already_correct.len(),
        );
    }

    // Unmatched
    for uf in &plan.unmatched {
        let name = uf.file.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        if let Some(ref crc) = uf.crc32 {
            log::warn!(
                "  {} {} (no match, CRC32: {})",
                "?".if_supports_color(Stdout, |t| t.yellow()),
                name.if_supports_color(Stdout, |t| t.dimmed()),
                crc,
            );
        } else {
            log::warn!(
                "  {} {} (no match)",
                "?".if_supports_color(Stdout, |t| t.yellow()),
                name.if_supports_color(Stdout, |t| t.dimmed()),
            );
        }
    }

    // Conflicts
    for (_, msg) in &plan.conflicts {
        log::warn!(
            "  {} {}",
            "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
            msg,
        );
    }

    // Discrepancies (--hash mode: serial and hash matched different games)
    for d in &plan.discrepancies {
        let file_name = d.file.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        log::warn!(
            "  {} {} serial=\"{}\" hash=\"{}\"",
            "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
            format!("{file_name}: serial/hash mismatch").if_supports_color(Stdout, |t| t.yellow()),
            d.serial_game,
            d.hash_game,
        );
    }

    // Serial warnings
    for w in &plan.serial_warnings {
        let file_name = w.file.file_name().and_then(|n| n.to_str()).unwrap_or("?");

        // Build hash suffix: "(matched by CRC32: abc123)" or "(CRC32: abc123, no DAT match)"
        let hash_suffix = match (&w.crc32, w.matched_by_hash) {
            (Some(crc), true) => format!(
                " {}",
                format!("(matched by CRC32: {crc})").if_supports_color(Stdout, |t| t.dimmed()),
            ),
            (Some(crc), false) => format!(
                " {}",
                format!("(CRC32: {crc}, no DAT match)").if_supports_color(Stdout, |t| t.dimmed()),
            ),
            _ => String::new(),
        };

        match &w.kind {
            SerialWarningKind::NoMatch {
                full_serial,
                game_code,
            } => {
                if let Some(code) = game_code {
                    log::warn!(
                        "  {} {}: serial \"{}\" (looked up as \"{}\") not found in DAT{}",
                        "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                        file_name.if_supports_color(Stdout, |t| t.dimmed()),
                        full_serial,
                        code,
                        hash_suffix,
                    );
                } else {
                    log::warn!(
                        "  {} {}: serial \"{}\" not found in DAT{}",
                        "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                        file_name.if_supports_color(Stdout, |t| t.dimmed()),
                        full_serial,
                        hash_suffix,
                    );
                }
            }
            SerialWarningKind::Ambiguous {
                full_serial,
                game_code: _,
                candidates,
            } => {
                let candidate_list = candidates.join(", ");
                let lookup_serial = full_serial;
                log::warn!(
                    "  {} {}: serial \"{}\" matches {} DAT entries (falling back to hash): {}{}",
                    "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                    file_name.if_supports_color(Stdout, |t| t.dimmed()),
                    lookup_serial,
                    candidates.len(),
                    candidate_list,
                    hash_suffix,
                );
            }
            SerialWarningKind::Missing => {
                log::warn!(
                    "  {} {}: no serial found (expected for this platform){}",
                    "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                    file_name.if_supports_color(Stdout, |t| t.dimmed()),
                    hash_suffix,
                );
            }
        }
    }

    // M3U actions
    print_m3u_actions(&plan.m3u_actions);

    // Broken CUE files
    for cue_path in &plan.broken_cue_files {
        let name = cue_path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        log::info!(
            "  {} {} (broken FILE references)",
            "\u{1F527}".if_supports_color(Stdout, |t| t.yellow()),
            name.if_supports_color(Stdout, |t| t.bold()),
        );
    }

    // Broken M3U playlists
    for m3u_path in &plan.broken_m3u_files {
        let name = m3u_path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        log::info!(
            "  {} {} (broken playlist entries)",
            "\u{1F527}".if_supports_color(Stdout, |t| t.yellow()),
            name.if_supports_color(Stdout, |t| t.bold()),
        );
    }

    // Misnamed M3U playlists
    for (source, target) in &plan.misnamed_m3u_playlists {
        let source_name = source.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        let target_name = target.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        log::info!(
            "  {} {} {} {} {}",
            "\u{2192}".if_supports_color(Stdout, |t| t.green()),
            source_name.if_supports_color(Stdout, |t| t.dimmed()),
            "\u{2192}".if_supports_color(Stdout, |t| t.green()),
            target_name.if_supports_color(Stdout, |t| t.bold()),
            "(playlist)".if_supports_color(Stdout, |t| t.dimmed()),
        );
    }
}

/// Print M3U folder rename and playlist actions.
pub(crate) fn print_m3u_actions(actions: &[M3uAction]) {
    if actions.is_empty() {
        return;
    }

    for action in actions {
        let source_name = action
            .source_folder
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?");
        let target_name = action
            .target_folder
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?");

        // Folder rename (if different)
        if action.source_folder != action.target_folder {
            log::info!(
                "  {} {} {} {} {}",
                "\u{1F4C1}".if_supports_color(Stdout, |t| t.green()),
                source_name.if_supports_color(Stdout, |t| t.dimmed()),
                "\u{2192}".if_supports_color(Stdout, |t| t.green()),
                target_name.if_supports_color(Stdout, |t| t.bold()),
                "(folder)".if_supports_color(Stdout, |t| t.dimmed()),
            );
        }

        // Playlist write
        if !action.playlist_entries.is_empty() {
            let playlist_name = format!("{}.m3u", action.game_name);
            log::info!(
                "  {} Write {} ({} discs)",
                "\u{1F4DD}".if_supports_color(Stdout, |t| t.green()),
                playlist_name.if_supports_color(Stdout, |t| t.bold()),
                action.playlist_entries.len(),
            );
        }
    }
}
