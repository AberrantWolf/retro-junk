//! retro-junk CLI
//!
//! Command-line interface for analyzing retro game ROMs and disc images.

mod cli_types;
mod commands;
mod error;
mod spinner;

pub(crate) use error::CliError;

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

use clap::Parser;
use log::LevelFilter;
use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;

use retro_junk_lib::{AnalysisContext, FolderScanResult, Platform};

use cli_types::{
    CacheAction, CatalogAction, Cli, Commands, CredentialsAction, DatCacheAction, GdbCacheAction,
    SettingsAction,
};

// -- Custom logger --

struct CliLogger {
    level: LevelFilter,
    verbose: bool,
    logfile: Option<Mutex<fs::File>>,
}

impl CliLogger {
    /// Format a timestamp as HH:MM:SS.mmm for log output.
    fn timestamp() -> String {
        let now = SystemTime::now();
        let dur = now
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        let total_secs = dur.as_secs();
        let millis = dur.subsec_millis();
        let hours = (total_secs / 3600) % 24;
        let minutes = (total_secs / 60) % 60;
        let seconds = total_secs % 60;
        format!("{hours:02}:{minutes:02}:{seconds:02}.{millis:03}")
    }
}

impl log::Log for CliLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let msg = record.args().to_string();

        if self.verbose {
            // Verbose mode: include timestamp, level, and module
            let ts = Self::timestamp();
            let level = record.level();
            let module = record.module_path().unwrap_or("?");
            let formatted = format!("[{ts} {level:5} {module}] {msg}");
            if record.level() <= log::Level::Warn {
                eprintln!("{formatted}");
            } else {
                println!("{formatted}");
            }
            // Logfile: ANSI-stripped
            if let Some(ref file) = self.logfile {
                let stripped = strip_ansi_escapes::strip(&formatted);
                let text = String::from_utf8_lossy(&stripped);
                let mut guard = file.lock().unwrap();
                let _ = writeln!(guard, "{text}");
            }
        } else {
            // Normal mode: no timestamps for terminal
            if record.level() <= log::Level::Warn {
                eprintln!("{msg}");
            } else {
                println!("{msg}");
            }
            // Logfile always gets timestamps
            if let Some(ref file) = self.logfile {
                let ts = Self::timestamp();
                let level = record.level();
                let stripped = strip_ansi_escapes::strip(&msg);
                let text = String::from_utf8_lossy(&stripped);
                let mut guard = file.lock().unwrap();
                let _ = writeln!(guard, "[{ts} {level:5}] {text}");
            }
        }
    }

    fn flush(&self) {
        if let Some(ref file) = self.logfile {
            let _ = std::io::Write::flush(&mut *file.lock().unwrap());
        }
    }
}

// -- Helpers --

/// Log an empty line. Wrapper around `log::info!("")` for consistency.
pub(crate) fn log_blank() {
    log::info!("");
}

// -- Main --

fn main() {
    let cli = Cli::parse();
    let quiet = cli.quiet;
    let verbose = cli.verbose;

    // Initialize logger
    let level = if quiet {
        LevelFilter::Warn
    } else if verbose {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    };
    let logfile = cli.logfile.map(|p| {
        let file = fs::File::create(&p).unwrap_or_else(|e| {
            eprintln!("Error: could not create logfile {}: {}", p.display(), e);
            std::process::exit(1);
        });
        Mutex::new(file)
    });
    let logger = Box::new(CliLogger {
        level,
        verbose,
        logfile,
    });
    log::set_boxed_logger(logger).expect("Failed to set logger");
    log::set_max_level(level);

    let ctx = create_context();
    let command = cli.command;
    let library_path = cli.library_path;

    if let Err(e) = run(command, library_path, quiet, &ctx) {
        log::error!("{e}");
        std::process::exit(1);
    }
}

fn run(
    command: Commands,
    library_path_override: Option<PathBuf>,
    quiet: bool,
    ctx: &AnalysisContext,
) -> Result<(), CliError> {
    // Commands that need the library path resolve it once here.
    let needs_library_path = matches!(
        command,
        Commands::Analyze(_)
            | Commands::Rename(_)
            | Commands::Organize(_)
            | Commands::FixCue(_)
            | Commands::Compress(_)
            | Commands::Repair(_)
            | Commands::Scrape(_)
    );
    let library_path = if needs_library_path {
        retro_junk_lib::settings::resolve_library_path(library_path_override)
    } else {
        // Unused — give a dummy value that won't be accessed.
        PathBuf::new()
    };

    match command {
        Commands::Archive { action } => commands::archive::run_archive(action, ctx)?,
        Commands::Sync(args) => commands::sync::run_sync(args)?,
        Commands::RebuildPlayable(args) => commands::sync::run_rebuild_playable(args)?,
        Commands::RenamePlayables(args) => commands::sync::run_rename_playables(args)?,
        Commands::Status(args) => commands::sync::run_status(args)?,
        Commands::Daemon { action } => commands::daemon::run_daemon(action)?,
        Commands::Suggestions { action } => commands::suggestions::run_suggestions(action)?,
        Commands::Analyze(args) => {
            commands::analyze::run_analyze(ctx, &args, &library_path)?;
        }
        Commands::Rename(args) => {
            commands::rename::run_rename(ctx, args, &library_path, quiet)?;
        }
        Commands::Organize(args) => {
            commands::organize::run_organize(ctx, args, &library_path, quiet)?;
        }
        Commands::FixCue(args) => {
            commands::fix_cue::run_fix_cue(ctx, &args, &library_path, quiet)?;
        }
        Commands::Compress(args) => {
            commands::compress::run_compress(ctx, &args, &library_path, quiet)?;
        }
        Commands::Repair(args) => {
            commands::repair::run_repair(ctx, args, &library_path, quiet)?;
        }
        Commands::Scrape(args) => {
            commands::scrape::run_scrape(ctx, args, &library_path, quiet)?;
        }
        Commands::Cache { action } => match action {
            CacheAction::Dat { action } => match action {
                DatCacheAction::List => commands::cache::run_cache_list(),
                DatCacheAction::Clear => commands::cache::run_cache_clear(),
                DatCacheAction::Fetch { systems, force } => {
                    commands::cache::run_cache_fetch(ctx, systems, force);
                }
            },
            CacheAction::Gdb { action } => match action {
                GdbCacheAction::List => commands::cache::run_gdb_cache_list(),
                GdbCacheAction::Clear => commands::cache::run_gdb_cache_clear(),
                GdbCacheAction::Fetch { systems, force } => {
                    commands::cache::run_gdb_cache_fetch(ctx, systems, force);
                }
            },
        },
        Commands::Credentials { action } => match action {
            CredentialsAction::Show => commands::credentials::run_credentials_show(),
            CredentialsAction::Setup => commands::credentials::run_credentials_setup()?,
            CredentialsAction::Test => commands::credentials::run_credentials_test(quiet)?,
            CredentialsAction::Path => commands::credentials::run_credentials_path()?,
        },
        Commands::Settings { action } => match action {
            SettingsAction::Show => commands::settings::run_config_show(),
            SettingsAction::LibraryPath { path, clear } => {
                commands::settings::run_config_library_path(path, clear)?;
            }
        },
        Commands::Systems { manufacturer } => {
            commands::systems::run_systems(ctx, &manufacturer);
        }
        Commands::Catalog { action } => run_catalog(action, quiet, ctx)?,
    }

    Ok(())
}

/// Dispatch `catalog` subcommands.
// Flat dispatch match over every catalog subcommand; one arm per command.
#[allow(clippy::too_many_lines)]
fn run_catalog(action: CatalogAction, quiet: bool, ctx: &AnalysisContext) -> Result<(), CliError> {
    match action {
        CatalogAction::Deduplicate {
            platform,
            apply,
            json,
            db,
        } => {
            let path = db.unwrap_or_else(retro_junk_lib::settings::catalog_database_path);
            let connection = retro_junk_db::open_database(&path)
                .map_err(|error| CliError::database(error.to_string()))?;
            let report = if apply {
                retro_junk_db::deduplicate_catalog(&connection, platform.as_deref())
            } else {
                retro_junk_db::analyze_catalog_duplicates(&connection, platform.as_deref())
            }
            .map_err(|error| CliError::database(error.to_string()))?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report)
                        .map_err(|error| CliError::other(error.to_string()))?
                );
            } else {
                log::info!(
                    "{} exact duplicate group(s), {} affected reference(s), {} suspected non-identical group(s){}",
                    report.exact_groups.len(),
                    report.affected_references,
                    report.suspected_groups,
                    if report.applied {
                        " merged"
                    } else {
                        " (dry run)"
                    }
                );
                for group in report.exact_groups {
                    log::info!(
                        "{} <- {}",
                        group.canonical_media_id,
                        group.duplicate_media_ids.join(", ")
                    );
                }
            }
        }
        CatalogAction::Import {
            systems,
            catalog_dir,
            db,
            dat_dir,
        } => {
            commands::catalog::import::run_catalog_import(
                ctx,
                &systems,
                &catalog_dir,
                db,
                dat_dir.as_deref(),
            )?;
        }
        CatalogAction::EnrichGdb {
            systems,
            db,
            limit,
            gdb_dir,
        } => {
            commands::catalog::enrich_gdb::run_catalog_enrich_gdb(
                ctx,
                &systems,
                db,
                limit,
                gdb_dir.as_deref(),
            )?;
        }
        CatalogAction::Enrich(args) => {
            commands::catalog::enrich::run_catalog_enrich(args, quiet)?;
        }
        CatalogAction::Scan {
            system,
            folder,
            db,
            user_id,
        } => {
            commands::catalog::scan::run_catalog_scan(ctx, &system, &folder, db, user_id, quiet)?;
        }
        CatalogAction::Verify {
            system,
            db,
            user_id,
        } => {
            commands::catalog::verify::run_catalog_verify(ctx, &system, db, &user_id, quiet)?;
        }
        CatalogAction::Disagreements {
            db,
            system,
            field,
            limit,
        } => {
            commands::catalog::disagreements::run_catalog_disagreements(
                db, &system, &field, limit,
            )?;
        }
        CatalogAction::Resolve {
            id,
            db,
            source_a,
            source_b,
            custom,
        } => {
            commands::catalog::disagreements::run_catalog_resolve(
                id,
                db,
                source_a,
                source_b,
                custom.as_deref(),
            )?;
        }
        CatalogAction::Gaps {
            system,
            db,
            collection_only,
            missing,
            limit,
        } => {
            commands::catalog::gaps::run_catalog_gaps(
                ctx,
                &system,
                db,
                collection_only,
                missing.as_deref(),
                limit,
            )?;
        }
        CatalogAction::Lookup(args) => {
            commands::catalog::lookup::run_catalog_lookup(args)?;
        }
        CatalogAction::Reconcile {
            systems,
            db,
            dry_run,
        } => {
            commands::catalog::reconcile::run_catalog_reconcile(&systems, db, dry_run)?;
        }
        CatalogAction::Stats { db } => {
            commands::catalog::stats::run_catalog_stats(db)?;
        }
        CatalogAction::Unenrich {
            system,
            after,
            db,
            confirm,
        } => {
            commands::catalog::unenrich::run_catalog_unenrich(
                ctx,
                &system,
                after.as_deref(),
                db,
                confirm,
            )?;
        }
        CatalogAction::Reset { db, confirm } => {
            commands::catalog::reset::run_catalog_reset(db, confirm)?;
        }
    }
    Ok(())
}

/// Create the analysis context with all registered consoles.
fn create_context() -> AnalysisContext {
    retro_junk_lib::create_default_context()
}

/// Scan the root directory for console folders, logging unrecognized ones.
///
/// Shared by all commands that iterate over console folders.
pub(crate) fn scan_folders(
    ctx: &AnalysisContext,
    root: &std::path::Path,
    consoles: Option<&[Platform]>,
) -> Option<FolderScanResult> {
    match ctx.scan_console_folders(root, consoles) {
        Ok(result) => {
            for name in &result.unrecognized {
                log::info!(
                    "  {} Skipping \"{}\" — not a recognized console folder",
                    "\u{2014}".if_supports_color(Stdout, |t| t.dimmed()),
                    name.if_supports_color(Stdout, |t| t.dimmed()),
                );
            }
            Some(result)
        }
        Err(e) => {
            log::warn!(
                "{} Error reading directory: {}",
                "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                e,
            );
            None
        }
    }
}

/// Log a DAT loading error with a `cache dat fetch` hint.
pub(crate) fn log_dat_error(
    platform_name: &str,
    folder_name: &str,
    short_name: &str,
    error: &dyn std::fmt::Display,
) {
    log::warn!(
        "{} {}: {} Error: {}",
        platform_name.if_supports_color(Stdout, |t| t.bold()),
        format!("({folder_name})").if_supports_color(Stdout, |t| t.dimmed()),
        "\u{2718}".if_supports_color(Stdout, |t| t.red()),
        error,
    );
    log::warn!(
        "  {} Try: retro-junk cache dat fetch {}",
        "\u{2139}".if_supports_color(Stdout, |t| t.cyan()),
        short_name,
    );
}
