//! retro-junk CLI
//!
//! Command-line interface for analyzing retro game ROMs and disc images.

mod cli_types;
mod commands;
mod spinner;

use std::fs;
use std::io::Write;
use std::sync::Mutex;
use std::time::SystemTime;

use clap::Parser;
use log::LevelFilter;
use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;

use retro_junk_lib::{AnalysisContext, FolderScanResult, Platform};

use cli_types::*;

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
        let dur = now.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
        let total_secs = dur.as_secs();
        let millis = dur.subsec_millis();
        let hours = (total_secs / 3600) % 24;
        let minutes = (total_secs / 60) % 60;
        let seconds = total_secs % 60;
        format!("{:02}:{:02}:{:02}.{:03}", hours, minutes, seconds, millis)
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
            let formatted = format!("[{} {:5} {}] {}", ts, level, module, msg);
            if record.level() <= log::Level::Warn {
                eprintln!("{}", formatted);
            } else {
                println!("{}", formatted);
            }
            // Logfile: ANSI-stripped
            if let Some(ref file) = self.logfile {
                let stripped = strip_ansi_escapes::strip(&formatted);
                let text = String::from_utf8_lossy(&stripped);
                let mut guard = file.lock().unwrap();
                let _ = writeln!(guard, "{}", text);
            }
        } else {
            // Normal mode: no timestamps for terminal
            if record.level() <= log::Level::Warn {
                eprintln!("{}", msg);
            } else {
                println!("{}", msg);
            }
            // Logfile always gets timestamps
            if let Some(ref file) = self.logfile {
                let ts = Self::timestamp();
                let level = record.level();
                let stripped = strip_ansi_escapes::strip(&msg);
                let text = String::from_utf8_lossy(&stripped);
                let mut guard = file.lock().unwrap();
                let _ = writeln!(guard, "[{} {:5}] {}", ts, level, text);
            }
        }
    }

    fn flush(&self) {
        if let Some(ref file) = self.logfile {
            let _ = std::io::Write::flush(&mut *file.lock().unwrap());
        }
    }
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
    let logger = Box::new(CliLogger { level, verbose, logfile });
    log::set_boxed_logger(logger).expect("Failed to set logger");
    log::set_max_level(level);

    let ctx = create_context();

    match cli.command {
        Commands::Analyze { quick, roms } => {
            commands::analyze::run_analyze(&ctx, quick, roms.consoles, roms.limit, cli.root);
        }
        Commands::Rename {
            dry_run,
            hash,
            roms,
            dat_dir,
        } => {
            commands::rename::run_rename(&ctx, dry_run, hash, roms.consoles, roms.limit, cli.root, dat_dir, quiet);
        }
        Commands::Repair {
            dry_run,
            no_backup,
            roms,
            dat_dir,
        } => {
            commands::repair::run_repair(&ctx, dry_run, no_backup, roms.consoles, roms.limit, cli.root, dat_dir, quiet);
        }
        Commands::Scrape {
            roms,
            media_types,
            metadata_dir,
            media_dir,
            frontend,
            region,
            language,
            language_fallback,
            force_full_hash,
            dry_run,
            skip_existing,
            no_log,
            no_miximage,
            force_redownload,
            threads,
        } => {
            commands::scrape::run_scrape(
                &ctx,
                roms.consoles,
                roms.limit,
                media_types,
                metadata_dir,
                media_dir,
                frontend,
                region,
                language,
                language_fallback,
                force_full_hash,
                dry_run,
                skip_existing,
                no_log,
                no_miximage,
                force_redownload,
                threads,
                cli.root,
                quiet,
            );
        }
        Commands::Cache { action } => match action {
            CacheAction::List => commands::cache::run_cache_list(),
            CacheAction::Clear => commands::cache::run_cache_clear(),
            CacheAction::Fetch { systems } => commands::cache::run_cache_fetch(&ctx, systems),
            CacheAction::GdbList => commands::cache::run_gdb_cache_list(),
            CacheAction::GdbClear => commands::cache::run_gdb_cache_clear(),
            CacheAction::GdbFetch { systems } => commands::cache::run_gdb_cache_fetch(&ctx, systems),
        },
        Commands::Config { action } => match action {
            ConfigAction::Show => commands::config::run_config_show(),
            ConfigAction::Setup => commands::config::run_config_setup(),
            ConfigAction::Test => commands::config::run_config_test(quiet),
            ConfigAction::Path => commands::config::run_config_path(),
        },
        Commands::Catalog { action } => match action {
            CatalogAction::Import {
                systems,
                catalog_dir,
                db,
                dat_dir,
            } => {
                commands::catalog::import::run_catalog_import(&ctx, systems, catalog_dir, db, dat_dir);
            }
            CatalogAction::EnrichGdb {
                systems,
                db,
                limit,
                gdb_dir,
            } => {
                commands::catalog::enrich_gdb::run_catalog_enrich_gdb(&ctx, systems, db, limit, gdb_dir);
            }
            CatalogAction::Enrich {
                systems,
                db,
                limit,
                force,
                download_assets,
                asset_dir,
                region,
                language,
                threads,
                no_reconcile,
            } => {
                commands::catalog::enrich::run_catalog_enrich(
                    systems,
                    db,
                    limit,
                    force,
                    download_assets,
                    asset_dir,
                    region,
                    language,
                    threads,
                    no_reconcile,
                    quiet,
                );
            }
            CatalogAction::Scan {
                system,
                folder,
                db,
                user_id,
            } => {
                commands::catalog::scan::run_catalog_scan(&ctx, system, folder, db, user_id, quiet);
            }
            CatalogAction::Verify {
                system,
                db,
                user_id,
            } => {
                commands::catalog::verify::run_catalog_verify(&ctx, system, db, user_id, quiet);
            }
            CatalogAction::Disagreements {
                db,
                system,
                field,
                limit,
            } => {
                commands::catalog::disagreements::run_catalog_disagreements(db, system, field, limit);
            }
            CatalogAction::Resolve {
                id,
                db,
                source_a,
                source_b,
                custom,
            } => {
                commands::catalog::disagreements::run_catalog_resolve(id, db, source_a, source_b, custom);
            }
            CatalogAction::Gaps {
                system,
                db,
                collection_only,
                missing,
                limit,
            } => {
                commands::catalog::gaps::run_catalog_gaps(system, db, collection_only, missing, limit);
            }
            CatalogAction::Lookup {
                query,
                r#type,
                platform,
                manufacturer,
                crc,
                sha1,
                md5,
                serial,
                limit,
                offset,
                group,
                db,
            } => {
                commands::catalog::lookup::run_catalog_lookup(query, platform, r#type, manufacturer, crc, sha1, md5, serial, limit, offset, group, db);
            }
            CatalogAction::Reconcile {
                systems,
                db,
                dry_run,
            } => {
                commands::catalog::reconcile::run_catalog_reconcile(systems, db, dry_run);
            }
            CatalogAction::Stats { db } => {
                commands::catalog::stats::run_catalog_stats(db);
            }
            CatalogAction::Unenrich {
                system,
                after,
                db,
                confirm,
            } => {
                commands::catalog::unenrich::run_catalog_unenrich(system, after, db, confirm);
            }
            CatalogAction::Reset { db, confirm } => {
                commands::catalog::reset::run_catalog_reset(db, confirm);
            }
        },
    }
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
    consoles: &Option<Vec<Platform>>,
) -> Option<FolderScanResult> {
    let filter = consoles.as_deref();
    match ctx.scan_console_folders(root, filter) {
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

/// Log a DAT loading error with a `cache fetch` hint.
pub(crate) fn log_dat_error(platform_name: &str, folder_name: &str, short_name: &str, error: &dyn std::fmt::Display) {
    log::warn!(
        "{} {}: {} Error: {}",
        platform_name.if_supports_color(Stdout, |t| t.bold()),
        format!("({})", folder_name).if_supports_color(Stdout, |t| t.dimmed()),
        "\u{2718}".if_supports_color(Stdout, |t| t.red()),
        error,
    );
    log::warn!(
        "  {} Try: retro-junk cache fetch {}",
        "\u{2139}".if_supports_color(Stdout, |t| t.cyan()),
        short_name,
    );
}
