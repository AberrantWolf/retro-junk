//! retro-junk CLI
//!
//! Command-line interface for analyzing retro game ROMs and disc images.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use clap::{Args, Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use log::{Level, LevelFilter};
use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;

use retro_junk_lib::rename::{
    RenameOptions, RenamePlan, RenameProgress, SerialWarningKind, execute_renames,
    format_match_method, plan_renames, M3uAction,
};
use retro_junk_lib::repair::{
    RepairOptions, RepairPlan, RepairProgress, execute_repairs, plan_repairs,
};
use retro_junk_lib::{
    AnalysisContext, AnalysisOptions, FolderScanResult, Platform, RomAnalyzer,
    RomIdentification,
};

// -- Custom logger --

struct CliLogger {
    level: LevelFilter,
    logfile: Option<Mutex<fs::File>>,
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

        // Terminal: warn/error to stderr, info to stdout
        if record.level() <= Level::Warn {
            eprintln!("{}", msg);
        } else {
            println!("{}", msg);
        }

        // Logfile: ANSI-stripped
        if let Some(ref file) = self.logfile {
            let stripped = strip_ansi_escapes::strip(&msg);
            let text = String::from_utf8_lossy(&stripped);
            let mut guard = file.lock().unwrap();
            let _ = writeln!(guard, "{}", text);
        }
    }

    fn flush(&self) {
        if let Some(ref file) = self.logfile {
            let _ = std::io::Write::flush(&mut *file.lock().unwrap());
        }
    }
}

// -- CLI types --

#[derive(Parser)]
#[command(name = "retro-junk")]
#[command(about = "Analyze retro game ROMs and disc images", long_about = None)]
struct Cli {
    /// Root path containing console folders (defaults to current directory)
    #[arg(short, long, global = true)]
    root: Option<PathBuf>,

    /// Only show warnings and errors (suppress normal output)
    #[arg(long, global = true)]
    quiet: bool,

    /// Write log output to a file (ANSI codes stripped)
    #[arg(long, global = true)]
    logfile: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

/// Common arguments for commands that process ROM files.
#[derive(Args, Clone)]
struct RomFilterArgs {
    /// Console names or aliases (e.g., snes,n64,ps1,gc,gg)
    #[arg(short, long, value_delimiter = ',')]
    consoles: Option<Vec<Platform>>,

    /// Maximum number of ROMs to process per console
    #[arg(short, long)]
    limit: Option<usize>,
}

#[derive(Subcommand)]
enum Commands {
    /// Analyze ROMs in a directory structure
    Analyze {
        /// Quick mode: read as little data as possible (useful for network shares)
        #[arg(short, long)]
        quick: bool,

        #[command(flatten)]
        roms: RomFilterArgs,
    },

    /// List all supported consoles
    List,

    /// Rename ROM files to NoIntro canonical names
    Rename {
        /// Show planned renames without executing
        #[arg(short = 'n', long)]
        dry_run: bool,

        /// Force CRC32 hash-based matching (reads full files)
        #[arg(long)]
        hash: bool,

        #[command(flatten)]
        roms: RomFilterArgs,

        /// Use DAT files from this directory instead of the cache
        #[arg(long)]
        dat_dir: Option<PathBuf>,
    },

    /// [Experimental] Repair trimmed/truncated ROMs by padding to match DAT checksums
    Repair {
        /// Show planned repairs without executing
        #[arg(short = 'n', long)]
        dry_run: bool,

        /// Don't create .bak backup files
        #[arg(long)]
        no_backup: bool,

        #[command(flatten)]
        roms: RomFilterArgs,

        /// Use DAT files from this directory instead of the cache
        #[arg(long)]
        dat_dir: Option<PathBuf>,
    },

    /// Scrape game metadata and media from ScreenScraper.fr
    Scrape {
        #[command(flatten)]
        roms: RomFilterArgs,

        /// Media types to download (e.g., covers,screenshots,videos,marquees)
        #[arg(long, value_delimiter = ',')]
        media_types: Option<Vec<String>>,

        /// Directory for metadata files (default: <root>-metadata).
        /// Set to the same path as --root to place gamelist.xml inside ROM directories,
        /// which is needed for ES-DE with LegacyGamelistFileLocation enabled
        #[arg(long)]
        metadata_dir: Option<PathBuf>,

        /// Directory for media files (default: <root>-media)
        #[arg(long)]
        media_dir: Option<PathBuf>,

        /// Frontend to generate metadata for
        #[arg(long, default_value = "esde")]
        frontend: String,

        /// Preferred region for names/media (e.g., us, eu, jp)
        #[arg(long, default_value = "us")]
        region: String,

        /// Preferred language for descriptions (e.g., en, fr, match)
        #[arg(long, default_value = "en")]
        language: String,

        /// Fallback language when --language match has no data for the matched language
        #[arg(long, default_value = "en")]
        language_fallback: String,

        /// Hash all files even when serial/filename should suffice
        #[arg(long)]
        force_full_hash: bool,

        /// Show what would be scraped without downloading
        #[arg(short = 'n', long)]
        dry_run: bool,

        /// Skip games that already have metadata
        #[arg(long)]
        skip_existing: bool,

        /// Disable scrape log file
        #[arg(long)]
        no_log: bool,

        /// Disable miximage generation
        #[arg(long)]
        no_miximage: bool,

        /// Force redownload of all media, ignoring existing files
        #[arg(long)]
        force_redownload: bool,

        /// Maximum concurrent API threads (default: server-granted max)
        #[arg(long)]
        threads: Option<usize>,
    },

    /// Manage cached DAT files
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },

    /// Manage ScreenScraper credentials configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Manage the game catalog database
    Catalog {
        #[command(subcommand)]
        action: CatalogAction,
    },
}

#[derive(Subcommand)]
enum CacheAction {
    /// List cached DAT files
    List,

    /// Remove all cached DAT files
    Clear,

    /// Download DAT files for specified systems
    Fetch {
        /// Systems to fetch (e.g., snes,n64) or "all"
        #[arg(value_delimiter = ',')]
        systems: Vec<String>,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Show current credentials and their sources
    Show,

    /// Interactively set up credentials
    Setup,

    /// Test credentials against the ScreenScraper API
    Test,

    /// Print the config file path
    Path,
}

#[derive(Subcommand)]
enum CatalogAction {
    /// Import DAT files into the catalog database
    Import {
        /// Systems to import (e.g., snes,n64) or "all"
        #[arg(value_delimiter = ',')]
        systems: Vec<String>,

        /// Path to catalog YAML data directory (default: ./catalog)
        #[arg(long)]
        catalog_dir: Option<PathBuf>,

        /// Path to the catalog database file (default: ~/.cache/retro-junk/catalog.db)
        #[arg(long)]
        db: Option<PathBuf>,

        /// Use DAT files from this directory instead of the cache
        #[arg(long)]
        dat_dir: Option<PathBuf>,
    },

    /// Enrich catalog releases with ScreenScraper metadata
    Enrich {
        /// Systems to enrich (e.g., nes,snes) or "all"
        #[arg(value_delimiter = ',')]
        systems: Vec<String>,

        /// Path to the catalog database file
        #[arg(long)]
        db: Option<PathBuf>,

        /// Maximum releases to process per system
        #[arg(long)]
        limit: Option<u32>,

        /// Re-enrich releases that already have ScreenScraper data
        #[arg(long)]
        force: bool,

        /// Download media assets
        #[arg(long)]
        download_assets: bool,

        /// Directory for downloaded media assets
        #[arg(long)]
        asset_dir: Option<PathBuf>,

        /// Preferred region for names and media (default: us)
        #[arg(long, default_value = "us")]
        region: String,

        /// Preferred language for descriptions (default: en)
        #[arg(long, default_value = "en")]
        language: String,

        /// Force hash-based lookup even for serial consoles
        #[arg(long)]
        force_hash: bool,
    },

    /// Scan a ROM folder and add matched files to collection
    Scan {
        /// System to scan (e.g., nes, snes, n64)
        system: String,

        /// Path to ROM folder
        folder: PathBuf,

        /// Path to the catalog database file
        #[arg(long)]
        db: Option<PathBuf>,

        /// User ID for collection entries
        #[arg(long, default_value = "default")]
        user_id: String,
    },

    /// Re-verify collection entries against files on disk
    Verify {
        /// System to verify (e.g., nes, snes, n64)
        system: String,

        /// Path to the catalog database file
        #[arg(long)]
        db: Option<PathBuf>,

        /// User ID for collection entries
        #[arg(long, default_value = "default")]
        user_id: String,
    },

    /// List unresolved disagreements between data sources
    Disagreements {
        /// Path to the catalog database file
        #[arg(long)]
        db: Option<PathBuf>,

        /// Filter by system (e.g., nes, snes)
        #[arg(long)]
        system: Option<String>,

        /// Filter by field name (e.g., release_date, title)
        #[arg(long)]
        field: Option<String>,

        /// Maximum number of disagreements to show
        #[arg(long, default_value = "50")]
        limit: u32,
    },

    /// Resolve a disagreement by choosing a value
    Resolve {
        /// Disagreement ID to resolve
        id: i64,

        /// Path to the catalog database file
        #[arg(long)]
        db: Option<PathBuf>,

        /// Choose source A's value
        #[arg(long, group = "choice")]
        source_a: bool,

        /// Choose source B's value
        #[arg(long, group = "choice")]
        source_b: bool,

        /// Provide a custom resolution value
        #[arg(long, group = "choice")]
        custom: Option<String>,
    },

    /// Analyze media asset coverage gaps
    Gaps {
        /// System to analyze (e.g., nes, snes)
        system: String,

        /// Path to the catalog database file
        #[arg(long)]
        db: Option<PathBuf>,

        /// Only analyze releases in your collection
        #[arg(long)]
        collection_only: bool,

        /// Show releases missing this specific asset type (e.g., box-front, screenshot)
        #[arg(long)]
        missing: Option<String>,

        /// Maximum releases to list
        #[arg(long, default_value = "50")]
        limit: u32,
    },

    /// Show catalog database statistics
    Stats {
        /// Path to the catalog database file
        #[arg(long)]
        db: Option<PathBuf>,
    },

    /// Delete and recreate the catalog database
    Reset {
        /// Path to the catalog database file
        #[arg(long)]
        db: Option<PathBuf>,

        /// Confirm database deletion (required)
        #[arg(long)]
        confirm: bool,
    },
}

fn main() {
    let cli = Cli::parse();
    let quiet = cli.quiet;

    // Initialize logger
    let level = if quiet { LevelFilter::Warn } else { LevelFilter::Info };
    let logfile = cli.logfile.map(|p| {
        let file = fs::File::create(&p).unwrap_or_else(|e| {
            eprintln!("Error: could not create logfile {}: {}", p.display(), e);
            std::process::exit(1);
        });
        Mutex::new(file)
    });
    let logger = Box::new(CliLogger { level, logfile });
    log::set_boxed_logger(logger).expect("Failed to set logger");
    log::set_max_level(level);

    let ctx = create_context();

    match cli.command {
        Commands::Analyze { quick, roms } => {
            run_analyze(&ctx, quick, roms.consoles, roms.limit, cli.root);
        }
        Commands::List => {
            run_list(&ctx);
        }
        Commands::Rename {
            dry_run,
            hash,
            roms,
            dat_dir,
        } => {
            run_rename(&ctx, dry_run, hash, roms.consoles, roms.limit, cli.root, dat_dir, quiet);
        }
        Commands::Repair {
            dry_run,
            no_backup,
            roms,
            dat_dir,
        } => {
            run_repair(&ctx, dry_run, no_backup, roms.consoles, roms.limit, cli.root, dat_dir, quiet);
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
            run_scrape(
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
            CacheAction::List => run_cache_list(),
            CacheAction::Clear => run_cache_clear(),
            CacheAction::Fetch { systems } => run_cache_fetch(&ctx, systems),
        },
        Commands::Config { action } => match action {
            ConfigAction::Show => run_config_show(),
            ConfigAction::Setup => run_config_setup(),
            ConfigAction::Test => run_config_test(quiet),
            ConfigAction::Path => run_config_path(),
        },
        Commands::Catalog { action } => match action {
            CatalogAction::Import {
                systems,
                catalog_dir,
                db,
                dat_dir,
            } => {
                run_catalog_import(&ctx, systems, catalog_dir, db, dat_dir);
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
                force_hash,
            } => {
                run_catalog_enrich(
                    systems,
                    db,
                    limit,
                    force,
                    download_assets,
                    asset_dir,
                    region,
                    language,
                    force_hash,
                    quiet,
                );
            }
            CatalogAction::Scan {
                system,
                folder,
                db,
                user_id,
            } => {
                run_catalog_scan(&ctx, system, folder, db, user_id, quiet);
            }
            CatalogAction::Verify {
                system,
                db,
                user_id,
            } => {
                run_catalog_verify(&ctx, system, db, user_id, quiet);
            }
            CatalogAction::Disagreements {
                db,
                system,
                field,
                limit,
            } => {
                run_catalog_disagreements(db, system, field, limit);
            }
            CatalogAction::Resolve {
                id,
                db,
                source_a,
                source_b,
                custom,
            } => {
                run_catalog_resolve(id, db, source_a, source_b, custom);
            }
            CatalogAction::Gaps {
                system,
                db,
                collection_only,
                missing,
                limit,
            } => {
                run_catalog_gaps(system, db, collection_only, missing, limit);
            }
            CatalogAction::Stats { db } => {
                run_catalog_stats(db);
            }
            CatalogAction::Reset { db, confirm } => {
                run_catalog_reset(db, confirm);
            }
        },
    }
}

/// Create the analysis context with all registered consoles.
fn create_context() -> AnalysisContext {
    let mut ctx = AnalysisContext::new();

    // Nintendo
    ctx.register(retro_junk_nintendo::NesAnalyzer::new());
    ctx.register(retro_junk_nintendo::SnesAnalyzer::new());
    ctx.register(retro_junk_nintendo::N64Analyzer::new());
    ctx.register(retro_junk_nintendo::GameCubeAnalyzer::new());
    ctx.register(retro_junk_nintendo::WiiAnalyzer::new());
    ctx.register(retro_junk_nintendo::WiiUAnalyzer::new());
    ctx.register(retro_junk_nintendo::GameBoyAnalyzer::new());
    ctx.register(retro_junk_nintendo::GbaAnalyzer::new());
    ctx.register(retro_junk_nintendo::DsAnalyzer::new());
    ctx.register(retro_junk_nintendo::N3dsAnalyzer::new());

    // Sony
    ctx.register(retro_junk_sony::Ps1Analyzer::new());
    ctx.register(retro_junk_sony::Ps2Analyzer::new());
    ctx.register(retro_junk_sony::Ps3Analyzer::new());
    ctx.register(retro_junk_sony::PspAnalyzer::new());
    ctx.register(retro_junk_sony::VitaAnalyzer::new());

    // Sega
    ctx.register(retro_junk_sega::Sg1000Analyzer::new());
    ctx.register(retro_junk_sega::MasterSystemAnalyzer::new());
    ctx.register(retro_junk_sega::GenesisAnalyzer::new());
    ctx.register(retro_junk_sega::SegaCdAnalyzer::new());
    ctx.register(retro_junk_sega::Sega32xAnalyzer::new());
    ctx.register(retro_junk_sega::SaturnAnalyzer::new());
    ctx.register(retro_junk_sega::DreamcastAnalyzer::new());
    ctx.register(retro_junk_sega::GameGearAnalyzer::new());

    // Microsoft
    ctx.register(retro_junk_microsoft::XboxAnalyzer::new());
    ctx.register(retro_junk_microsoft::Xbox360Analyzer::new());

    ctx
}

/// Scan the root directory for console folders, logging unrecognized ones.
///
/// Shared by all commands that iterate over console folders.
fn scan_folders(
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
fn log_dat_error(platform_name: &str, folder_name: &str, short_name: &str, error: &dyn std::fmt::Display) {
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

/// Run the analyze command.
fn run_analyze(
    ctx: &AnalysisContext,
    quick: bool,
    consoles: Option<Vec<Platform>>,
    limit: Option<usize>,
    root: Option<PathBuf>,
) {
    let root_path =
        root.unwrap_or_else(|| std::env::current_dir().expect("Failed to get current directory"));

    log::info!("Analyzing ROMs in: {}", root_path.display());
    if quick {
        log::info!("Quick mode enabled");
    }
    if let Some(n) = limit {
        log::info!("Limit: {} games per console", n);
    }
    log::info!("");

    let options = AnalysisOptions::new().quick(quick);

    let scan = match scan_folders(ctx, &root_path, &consoles) {
        Some(s) => s,
        None => return,
    };

    for cf in &scan.matches {
        let console = ctx.get_by_platform(cf.platform).unwrap();
        log::info!(
            "{} {} folder: {}",
            "Found".if_supports_color(Stdout, |t| t.bold()),
            console.metadata.platform_name,
            cf.folder_name.if_supports_color(Stdout, |t| t.cyan()),
        );

        analyze_folder(&cf.path, console.analyzer.as_ref(), &options, limit);
    }

    if scan.matches.is_empty() {
        log::info!(
            "{}",
            format!(
                "No matching console folders found in {}",
                root_path.display()
            )
            .if_supports_color(Stdout, |t| t.dimmed()),
        );
        log::info!("");
        log::info!("Tip: Create folders named after consoles (e.g., 'snes', 'n64', 'ps1')");
        log::info!("     and place your ROM files inside them.");
        log::info!("");
        log::info!("Run 'retro-junk list' to see all supported console names.");
    }
}

/// Analyze all ROM files in a folder.
fn analyze_folder(
    folder: &PathBuf,
    analyzer: &dyn RomAnalyzer,
    options: &AnalysisOptions,
    limit: Option<usize>,
) {
    use retro_junk_lib::scanner::{self, GameEntry};

    let extensions = scanner::extension_set(analyzer.file_extensions());

    let mut game_entries = match scanner::scan_game_entries(folder, &extensions) {
        Ok(entries) => entries,
        Err(e) => {
            log::warn!(
                "  {} Error reading folder: {}",
                "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                e,
            );
            return;
        }
    };

    if let Some(max) = limit {
        game_entries.truncate(max);
    }

    let mut any_output = false;
    for entry in &game_entries {
        match entry {
            GameEntry::SingleFile(path) => {
                any_output = true;
                analyze_and_print(path, analyzer, options, "");
            }
            GameEntry::MultiDisc { name, files } => {
                any_output = true;
                log::info!(
                    "  {}",
                    format!("{}:", name).if_supports_color(Stdout, |t| t.bold()),
                );
                for path in files {
                    analyze_and_print(path, analyzer, options, "  ");
                }
            }
        }
    }

    if !any_output {
        log::info!(
            "  {}",
            "No ROM files found".if_supports_color(Stdout, |t| t.dimmed()),
        );
    }
    log::info!("");
}

/// Analyze a single file and print its results.
fn analyze_and_print(
    path: &PathBuf,
    analyzer: &dyn RomAnalyzer,
    options: &AnalysisOptions,
    indent: &str,
) {
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");

    let file_options = AnalysisOptions {
        file_path: Some(path.clone()),
        ..options.clone()
    };

    let mut file = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) => {
            log::warn!(
                "  {}{} Error opening {}: {}",
                indent,
                "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                file_name,
                e,
            );
            return;
        }
    };

    match analyzer.analyze(&mut file, &file_options) {
        Ok(info) => {
            let lines = format_analysis(file_name, &info, indent);
            let has_warnings = lines.iter().any(|(level, _)| *level <= Level::Warn);
            for (i, (level, msg)) in lines.iter().enumerate() {
                // Promote header to warn if this file has warnings (visible in quiet mode)
                let effective_level = if i == 0 && has_warnings { Level::Warn } else { *level };
                log::log!(effective_level, "{}", msg);
            }
        }
        Err(e) => {
            log::warn!(
                "  {}{}: {} Analysis not implemented ({})",
                indent,
                file_name,
                "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                e,
            );
        }
    }
}

/// Format a byte size as a human-readable string.
fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 && bytes % (1024 * 1024) == 0 {
        format!("{} MB", bytes / (1024 * 1024))
    } else if bytes >= 1024 && bytes % 1024 == 0 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{} bytes", bytes)
    }
}

// -- Size verdict logic --

enum SizeVerdict {
    Ok,
    Trimmed { missing: u64 },
    Truncated { missing: u64 },
    CopierHeader,
    Oversized { excess: u64 },
}

fn is_power_of_two(n: u64) -> bool {
    n > 0 && (n & (n - 1)) == 0
}

fn compute_size_verdict(file_size: u64, expected_size: u64) -> SizeVerdict {
    if file_size == expected_size {
        return SizeVerdict::Ok;
    }

    if file_size < expected_size {
        let missing = expected_size - file_size;

        // Likely trimmed: file still has most data AND file size is a power of 2
        // OR the missing amount is a power-of-2 fraction of expected size
        let has_most_data = file_size >= expected_size / 2;
        let file_is_pow2 = is_power_of_two(file_size);
        let missing_is_pow2_fraction =
            is_power_of_two(missing) && is_power_of_two(expected_size) && missing < expected_size;

        if has_most_data && (file_is_pow2 || missing_is_pow2_fraction) {
            SizeVerdict::Trimmed { missing }
        } else {
            SizeVerdict::Truncated { missing }
        }
    } else {
        let excess = file_size - expected_size;
        if excess == 512 {
            SizeVerdict::CopierHeader
        } else {
            SizeVerdict::Oversized { excess }
        }
    }
}

fn print_size_verdict(verdict: &SizeVerdict) -> String {
    match verdict {
        SizeVerdict::Ok => format!(
            "{} {}",
            "\u{2714}".if_supports_color(Stdout, |t| t.green()),
            "OK".if_supports_color(Stdout, |t| t.green()),
        ),
        SizeVerdict::Trimmed { missing } => format!(
            "{} {} (-{}, trailing data stripped)",
            "\u{2702}".if_supports_color(Stdout, |t| t.yellow()),
            "TRIMMED".if_supports_color(Stdout, |t| t.yellow()),
            format_bytes(*missing),
        ),
        SizeVerdict::Truncated { missing } => format!(
            "{} {} (missing {})",
            "\u{2718}".if_supports_color(Stdout, |t| t.bright_red()),
            "TRUNCATED".if_supports_color(Stdout, |t| t.bright_red()),
            format_bytes(*missing),
        ),
        SizeVerdict::CopierHeader => format!(
            "\u{1F4DD} {} (+512 bytes, likely copier header)",
            "OVERSIZED".if_supports_color(Stdout, |t| t.yellow()),
        ),
        SizeVerdict::Oversized { excess } => format!(
            "{} {} (+{})",
            "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
            "OVERSIZED".if_supports_color(Stdout, |t| t.yellow()),
            format_bytes(*excess),
        ),
    }
}

// -- Key prettification --

/// Known acronyms that should stay uppercase when prettifying keys.
const ACRONYMS: &[&str] = &[
    "PRG", "CHR", "RAM", "ROM", "SRAM", "NVRAM", "SGB", "CGB", "TV", "ID",
];

/// Convert a snake_case key to Title Case, keeping known acronyms uppercase.
fn prettify_key(key: &str) -> String {
    key.split('_')
        .filter(|s| !s.is_empty())
        .map(|word| {
            let upper = word.to_uppercase();
            if ACRONYMS.contains(&upper.as_str()) {
                upper
            } else {
                let mut chars = word.chars();
                match chars.next() {
                    Some(c) => {
                        let mut s = c.to_uppercase().to_string();
                        s.extend(chars);
                        s
                    }
                    None => String::new(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// -- Hardware keys (ordered) --

/// Known hardware/technical extra keys, in display order.
const HARDWARE_KEYS: &[&str] = &[
    "mapping",
    "speed",
    "chipset",
    "coprocessor",
    "mirroring",
    "cartridge_type",
    "rom_size",
    "prg_rom_size",
    "chr_rom_size",
    "sram_size",
    "ram_size",
    "prg_ram_size",
    "prg_nvram_size",
    "chr_ram_size",
    "chr_nvram_size",
    "expansion_ram",
    "expansion_device",
    "battery",
    "trainer",
    "sgb",
    "console_type",
    "tv_system",
    "copier_header",
    "checksum_complement_valid",
];

/// Format the analysis result for a single file as level-tagged lines.
/// The first element is always the file header line.
fn format_analysis(file_name: &str, info: &RomIdentification, indent: &str) -> Vec<(Level, String)> {
    let mut lines: Vec<(Level, String)> = Vec::new();
    let mut shown_keys: HashSet<&str> = HashSet::new();

    // Header line (caller may promote to Warn if other lines have warnings)
    lines.push((Level::Info, format!(
        "  {}{}:",
        indent,
        file_name.if_supports_color(Stdout, |t| t.bold()),
    )));

    // (a) Identity fields
    if let Some(ref serial) = info.serial_number {
        lines.push((Level::Info, format!(
            "    {}{}   {}",
            indent,
            "Serial:".if_supports_color(Stdout, |t| t.cyan()),
            serial,
        )));
    }
    if let Some(ref name) = info.internal_name {
        lines.push((Level::Info, format!(
            "    {}{}     {}",
            indent,
            "Name:".if_supports_color(Stdout, |t| t.cyan()),
            name,
        )));
    }
    if let Some(ref maker) = info.maker_code {
        lines.push((Level::Info, format!(
            "    {}{}    {}",
            indent,
            "Maker:".if_supports_color(Stdout, |t| t.cyan()),
            maker,
        )));
    }
    if let Some(ref version) = info.version {
        lines.push((Level::Info, format!(
            "    {}{}  {}",
            indent,
            "Version:".if_supports_color(Stdout, |t| t.cyan()),
            version,
        )));
    }

    // (b) Format line (composed as single string)
    if let Some(format_val) = info.extra.get("format") {
        shown_keys.insert("format");
        let mut format_line = format!(
            "    {}{}   {}",
            indent,
            "Format:".if_supports_color(Stdout, |t| t.cyan()),
            format_val,
        );
        if let Some(mapper) = info.extra.get("mapper") {
            shown_keys.insert("mapper");
            format_line.push_str(&format!(", Mapper {}", mapper));
            if let Some(mapper_name) = info.extra.get("mapper_name") {
                shown_keys.insert("mapper_name");
                format_line.push_str(&format!(" ({})", mapper_name));
            }
        }
        lines.push((Level::Info, format_line));
    }

    // (c) Hardware section
    let hardware_present: Vec<&str> = HARDWARE_KEYS
        .iter()
        .filter(|k| info.extra.contains_key(**k))
        .copied()
        .collect();

    if !hardware_present.is_empty() {
        lines.push((Level::Info, format!(
            "    {}{}",
            indent,
            "Hardware:".if_supports_color(Stdout, |t| t.bright_magenta()),
        )));
        for key in &hardware_present {
            shown_keys.insert(key);
            let value = &info.extra[*key];
            lines.push((Level::Info, format!(
                "      {}{} {}",
                indent,
                format!("{}:", prettify_key(key)).if_supports_color(Stdout, |t| t.cyan()),
                value,
            )));
        }
    }

    // (d) Size verdict
    match (info.file_size, info.expected_size) {
        (Some(actual), Some(expected)) => {
            let verdict = compute_size_verdict(actual, expected);
            let level = if matches!(verdict, SizeVerdict::Ok) { Level::Info } else { Level::Warn };
            lines.push((level, format!(
                "    {}{}     {} on disk, {} expected [{}]",
                indent,
                "Size:".if_supports_color(Stdout, |t| t.cyan()),
                format_bytes(actual),
                format_bytes(expected),
                print_size_verdict(&verdict),
            )));
        }
        (Some(actual), None) => {
            lines.push((Level::Info, format!(
                "    {}{}     {}",
                indent,
                "Size:".if_supports_color(Stdout, |t| t.cyan()),
                format_bytes(actual),
            )));
        }
        _ => {}
    }

    // (e) Checksums
    let mut checksum_keys: Vec<_> = info
        .extra
        .keys()
        .filter(|k| k.starts_with("checksum_status:"))
        .collect();
    checksum_keys.sort();
    for key in &checksum_keys {
        shown_keys.insert(key.as_str());
        let name = &key["checksum_status:".len()..];
        let status = &info.extra[key.as_str()];
        let is_ok = status.starts_with("OK") || status.starts_with("Valid");
        let level = if is_ok { Level::Info } else { Level::Warn };
        if is_ok {
            let colored_status = format!("{}", status.if_supports_color(Stdout, |t| t.green()));
            lines.push((level, format!(
                "    {}{} {}  {}",
                indent,
                "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                format!("{}:", name).if_supports_color(Stdout, |t| t.cyan()),
                colored_status,
            )));
        } else {
            let colored_status = format!("{}", status.if_supports_color(Stdout, |t| t.red()));
            lines.push((level, format!(
                "    {}{} {}  {}",
                indent,
                "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                format!("{}:", name).if_supports_color(Stdout, |t| t.cyan()),
                colored_status,
            )));
        }
    }

    // (f) Region
    if !info.regions.is_empty() {
        let region_str: Vec<_> = info.regions.iter().map(|r| r.name()).collect();
        lines.push((Level::Info, format!(
            "    {}{}   {}",
            indent,
            "Region:".if_supports_color(Stdout, |t| t.cyan()),
            region_str.join(", "),
        )));
    }

    // (g) Remaining extras
    let mut remaining: Vec<_> = info
        .extra
        .keys()
        .filter(|k| !shown_keys.contains(k.as_str()))
        .collect();
    remaining.sort();

    if !remaining.is_empty() {
        lines.push((Level::Info, format!(
            "    {}{}",
            indent,
            "Details:".if_supports_color(Stdout, |t| t.bright_magenta()),
        )));
        for key in &remaining {
            let value = &info.extra[key.as_str()];
            lines.push((Level::Info, format!(
                "      {}{} {}",
                indent,
                format!("{}:", prettify_key(key)).if_supports_color(Stdout, |t| t.cyan()),
                value,
            )));
        }
    }

    lines
}

/// Run the rename command.
#[allow(clippy::too_many_arguments)]
fn run_rename(
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

    let scan = match scan_folders(ctx, &root_path, &consoles) {
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

                let has_work =
                    !plan.renames.is_empty() || !plan.m3u_actions.is_empty();
                if !dry_run && has_work {
                    // Prompt for confirmation (raw print — user interaction)
                    let m3u_count = plan.m3u_actions.len();
                    if m3u_count > 0 {
                        print!(
                            "\n  Proceed with {} renames and {} m3u updates? [y/N] ",
                            plan.renames.len(),
                            m3u_count,
                        );
                    } else {
                        print!("\n  Proceed with {} renames? [y/N] ", plan.renames.len());
                    }
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
                log_dat_error(
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
                log::info!("  {} [{}]", console.metadata.short_name, dat_names.join(", "));
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

/// Run the repair command.
#[allow(clippy::too_many_arguments)]
fn run_repair(
    ctx: &AnalysisContext,
    dry_run: bool,
    no_backup: bool,
    consoles: Option<Vec<Platform>>,
    limit: Option<usize>,
    root: Option<PathBuf>,
    dat_dir: Option<PathBuf>,
    quiet: bool,
) {
    let root_path =
        root.unwrap_or_else(|| std::env::current_dir().expect("Failed to get current directory"));

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
    log::info!("");

    let scan = match scan_folders(ctx, &root_path, &consoles) {
        Some(s) => s,
        None => return,
    };

    let mut total_repaired = 0usize;
    let mut total_already_correct = 0usize;
    let mut total_no_match = 0usize;
    let mut total_errors: Vec<String> = Vec::new();
    let mut found_any = false;

    for cf in &scan.matches {
        let console = ctx.get_by_platform(cf.platform).unwrap();

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
                    .unwrap()
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

                let has_issues =
                    !plan.no_match.is_empty() || !plan.errors.is_empty();
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
                    print!(
                        "\n  Proceed with {} repairs? [y/N] ",
                        plan.repairable.len(),
                    );
                    std::io::stdout().flush().unwrap();

                    let mut input = String::new();
                    std::io::stdin().read_line(&mut input).unwrap();

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
                log_dat_error(
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
        return;
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
}

/// Print the repair plan for a single console.
fn print_repair_plan(plan: &RepairPlan) {
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
            action.method.description().if_supports_color(Stdout, |t| t.dimmed()),
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

/// Print the rename plan for a single console.
fn print_rename_plan(plan: &RenamePlan) {
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
                format!("(matched by CRC32: {crc})")
                    .if_supports_color(Stdout, |t| t.dimmed()),
            ),
            (Some(crc), false) => format!(
                " {}",
                format!("(CRC32: {crc}, no DAT match)")
                    .if_supports_color(Stdout, |t| t.dimmed()),
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
}

/// Print M3U folder rename and playlist actions.
fn print_m3u_actions(actions: &[M3uAction]) {
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

/// Claim a spinner slot from the free list, reset it, and start ticking with the given message.
fn claim_spinner_slot(
    key: usize,
    msg: String,
    spinners: &[ProgressBar],
    free_slots: &mut Vec<usize>,
    slot_assignments: &mut HashMap<usize, usize>,
) {
    if let Some(slot) = free_slots.pop() {
        spinners[slot].reset();
        spinners[slot].enable_steady_tick(std::time::Duration::from_millis(100));
        spinners[slot].set_message(msg);
        slot_assignments.insert(key, slot);
    }
}

/// Release a spinner slot: stop it, clear the line, and return it to the free list.
fn release_spinner_slot(
    key: usize,
    spinners: &[ProgressBar],
    free_slots: &mut Vec<usize>,
    slot_assignments: &mut HashMap<usize, usize>,
) {
    if let Some(slot) = slot_assignments.remove(&key) {
        spinners[slot].finish_and_clear();
        free_slots.push(slot);
    }
}

/// Run the scrape command.
#[allow(clippy::too_many_arguments)]
fn run_scrape(
    ctx: &AnalysisContext,
    consoles: Option<Vec<Platform>>,
    limit: Option<usize>,
    media_types: Option<Vec<String>>,
    metadata_dir: Option<PathBuf>,
    media_dir: Option<PathBuf>,
    _frontend: String,
    region: String,
    language: String,
    language_fallback: String,
    force_full_hash: bool,
    dry_run: bool,
    skip_existing: bool,
    no_log: bool,
    no_miximage: bool,
    force_redownload: bool,
    threads: Option<usize>,
    root: Option<PathBuf>,
    quiet: bool,
) {
    use indicatif::MultiProgress;

    let root_path =
        root.unwrap_or_else(|| std::env::current_dir().expect("Failed to get current directory"));

    // Build scrape options
    let mut options = retro_junk_scraper::ScrapeOptions::new(root_path.clone());
    options.region = region;
    options.language = language;
    options.language_fallback = language_fallback;
    options.force_hash = force_full_hash;
    options.dry_run = dry_run;
    options.skip_existing = skip_existing;
    options.no_log = no_log;
    options.no_miximage = no_miximage;
    options.force_redownload = force_redownload;
    options.limit = limit;

    // Load miximage layout unless disabled
    if !no_miximage {
        match retro_junk_frontend::miximage_layout::MiximageLayout::load_or_create() {
            Ok(layout) => {
                options.miximage_layout = Some(layout);
            }
            Err(e) => {
                log::warn!(
                    "{} Failed to load miximage layout, disabling miximages: {}",
                    "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                    e,
                );
            }
        }
    }

    if let Some(mdir) = metadata_dir {
        options.metadata_dir = mdir;
    }
    if let Some(mdir) = media_dir {
        options.media_dir = mdir;
    }
    if let Some(ref types) = media_types {
        options.media_selection = retro_junk_scraper::MediaSelection::from_names(types);
    }

    log::info!(
        "Scraping ROMs in: {}",
        root_path.display().if_supports_color(Stdout, |t| t.cyan()),
    );
    if dry_run {
        log::info!(
            "{}",
            "Dry run: no files will be downloaded".if_supports_color(Stdout, |t| t.dimmed()),
        );
    }
    if let Some(n) = limit {
        log::info!(
            "{}",
            format!("Limit: {} ROMs per console", n).if_supports_color(Stdout, |t| t.dimmed()),
        );
    }
    log::info!(
        "Metadata: {}",
        options.metadata_dir.display().if_supports_color(Stdout, |t| t.dimmed()),
    );
    log::info!(
        "Media:    {}",
        options.media_dir.display().if_supports_color(Stdout, |t| t.dimmed()),
    );
    log::info!("");

    // Load credentials
    let creds = match retro_junk_scraper::Credentials::load() {
        Ok(c) => c,
        Err(e) => {
            log::warn!(
                "{} Failed to load ScreenScraper credentials: {}",
                "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                e,
            );
            log::warn!("");
            log::warn!("Set credentials via environment variables:");
            log::warn!("  SCREENSCRAPER_DEVID, SCREENSCRAPER_DEVPASSWORD");
            log::warn!("  SCREENSCRAPER_SSID, SCREENSCRAPER_SSPASSWORD (optional)");
            log::warn!("");
            log::warn!("Or create ~/.config/retro-junk/credentials.toml");
            return;
        }
    };

    // Create a tokio runtime for async operations
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");

    rt.block_on(async {
        // Validate credentials
        let pb = if quiet {
            ProgressBar::hidden()
        } else {
            let pb = ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::with_template("  {spinner:.cyan} {msg}")
                    .unwrap()
                    .tick_chars("/-\\|"),
            );
            pb.set_message("Connecting to ScreenScraper...");
            pb.enable_steady_tick(std::time::Duration::from_millis(100));
            pb
        };

        let (client, user_info) = match retro_junk_scraper::ScreenScraperClient::new(creds).await {
            Ok(result) => result,
            Err(e) => {
                pb.finish_and_clear();
                log::warn!(
                    "{} Failed to connect to ScreenScraper: {}",
                    "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                    e,
                );
                return;
            }
        };
        pb.finish_and_clear();

        // Compute worker count: min(user override or server max, cpu count)
        let ss_max = user_info.max_threads() as usize;
        let cpu_max = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        let max_workers = threads
            .map(|t| t.min(ss_max))
            .unwrap_or_else(|| ss_max.min(cpu_max));
        let max_workers = max_workers.max(1);

        log::info!(
            "{} Connected to ScreenScraper (requests today: {}/{}, max threads: {}, using: {})",
            "\u{2714}".if_supports_color(Stdout, |t| t.green()),
            user_info.requests_today(),
            user_info.max_requests_per_day(),
            user_info.max_threads(),
            max_workers,
        );
        log::info!("");

        let scan = match scan_folders(ctx, &root_path, &consoles) {
            Some(s) => s,
            None => return,
        };

        let esde = retro_junk_frontend::esde::EsDeFrontend::new();
        let mut total_games = 0usize;
        let mut total_media = 0usize;
        let mut total_errors = 0usize;
        let mut total_unidentified = 0usize;

        for cf in &scan.matches {
            let console = ctx.get_by_platform(cf.platform).unwrap();
            let path = &cf.path;
            let folder_name = &cf.folder_name;

            // Check if this system has a ScreenScraper ID
            if retro_junk_scraper::screenscraper_system_id(cf.platform).is_none() {
                log::warn!(
                    "  {} Skipping \"{}\" — no ScreenScraper system ID",
                    "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                    folder_name,
                );
                continue;
            }

            log::info!(
                "{} {}",
                console
                    .metadata
                    .platform_name
                    .if_supports_color(Stdout, |t| t.bold()),
                format!("({})", folder_name).if_supports_color(Stdout, |t| t.dimmed()),
            );

            // Set up MultiProgress with N spinner slots
            let mp = if quiet {
                MultiProgress::with_draw_target(indicatif::ProgressDrawTarget::hidden())
            } else {
                MultiProgress::new()
            };

            let spinner_style = ProgressStyle::with_template("  {spinner:.cyan} {msg}")
                .unwrap()
                .tick_chars("/-\\|");

            let mut spinners: Vec<ProgressBar> = (0..max_workers)
                .map(|_| {
                    let pb = mp.add(ProgressBar::new_spinner());
                    pb.set_style(spinner_style.clone());
                    pb.enable_steady_tick(std::time::Duration::from_millis(100));
                    pb
                })
                .collect();

            // Track which spinner slot each game index is using
            let mut slot_assignments: HashMap<usize, usize> = HashMap::new();
            let mut free_slots: Vec<usize> = (0..max_workers).rev().collect();
            let mut scan_total = 0usize;

            let (event_tx, mut event_rx) =
                tokio::sync::mpsc::unbounded_channel::<retro_junk_scraper::ScrapeEvent>();

            let scrape_future = retro_junk_scraper::scrape_folder(
                &client,
                path,
                console.analyzer.as_ref(),
                &options,
                folder_name,
                max_workers,
                event_tx,
            );
            tokio::pin!(scrape_future);
            let mut scrape_result: Option<Result<retro_junk_scraper::ScrapeResult, retro_junk_scraper::ScrapeError>> = None;

            loop {
                tokio::select! {
                    result = &mut scrape_future, if scrape_result.is_none() => {
                        scrape_result = Some(result);
                    }
                    event = event_rx.recv() => {
                        match event {
                            Some(e) => {
                                match e {
                                    retro_junk_scraper::ScrapeEvent::Scanning => {
                                        claim_spinner_slot(usize::MAX, "Scanning for ROM files...".into(), &spinners, &mut free_slots, &mut slot_assignments);
                                    }
                                    retro_junk_scraper::ScrapeEvent::ScanComplete { total } => {
                                        scan_total = total;
                                        release_spinner_slot(usize::MAX, &spinners, &mut free_slots, &mut slot_assignments);
                                    }
                                    retro_junk_scraper::ScrapeEvent::GameStarted { index, ref file } => {
                                        claim_spinner_slot(index, format!(
                                            "[{}/{}] {}",
                                            index + 1, scan_total, file,
                                        ), &spinners, &mut free_slots, &mut slot_assignments);
                                    }
                                    retro_junk_scraper::ScrapeEvent::GameLookingUp { index, ref file } => {
                                        if let Some(&slot) = slot_assignments.get(&index) {
                                            spinners[slot].set_message(format!(
                                                "[{}/{}] Looking up {}",
                                                index + 1, scan_total, file,
                                            ));
                                        }
                                    }
                                    retro_junk_scraper::ScrapeEvent::GameDownloading { index, ref file } => {
                                        if let Some(&slot) = slot_assignments.get(&index) {
                                            spinners[slot].set_message(format!(
                                                "[{}/{}] Downloading media for {}",
                                                index + 1, scan_total, file,
                                            ));
                                        }
                                    }
                                    retro_junk_scraper::ScrapeEvent::GameDownloadingMedia { index, ref file, ref media_type } => {
                                        if let Some(&slot) = slot_assignments.get(&index) {
                                            spinners[slot].set_message(format!(
                                                "[{}/{}] Downloading {} for {}",
                                                index + 1, scan_total, media_type, file,
                                            ));
                                        }
                                    }
                                    retro_junk_scraper::ScrapeEvent::GameSkipped { index, ref file, ref reason } => {
                                        if let Some(&slot) = slot_assignments.get(&index) {
                                            spinners[slot].set_message(format!(
                                                "[{}/{}] Skipped {}: {}",
                                                index + 1, scan_total, file, reason,
                                            ));
                                        }
                                        release_spinner_slot(index, &spinners, &mut free_slots, &mut slot_assignments);
                                    }
                                    retro_junk_scraper::ScrapeEvent::GameCompleted { index, ref file, ref game_name } => {
                                        if let Some(&slot) = slot_assignments.get(&index) {
                                            spinners[slot].set_message(format!(
                                                "[{}/{}] {} -> \"{}\"",
                                                index + 1, scan_total, file, game_name,
                                            ));
                                        }
                                        release_spinner_slot(index, &spinners, &mut free_slots, &mut slot_assignments);
                                    }
                                    retro_junk_scraper::ScrapeEvent::GameFailed { index, ref file, ref reason } => {
                                        if let Some(&slot) = slot_assignments.get(&index) {
                                            spinners[slot].set_message(format!(
                                                "[{}/{}] {} failed: {}",
                                                index + 1, scan_total, file, reason,
                                            ));
                                        }
                                        release_spinner_slot(index, &spinners, &mut free_slots, &mut slot_assignments);
                                    }
                                    retro_junk_scraper::ScrapeEvent::GameGrouped { .. } => {
                                        // Grouped discs happen after the concurrent phase; no spinner
                                    }
                                    retro_junk_scraper::ScrapeEvent::FatalError { ref message } => {
                                        // Clear all spinners, show fatal error
                                        for spinner in &spinners {
                                            spinner.finish_and_clear();
                                        }
                                        log::warn!(
                                            "  {} Fatal: {}",
                                            "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                                            message,
                                        );
                                    }
                                    retro_junk_scraper::ScrapeEvent::Done => {
                                        // Handled when channel closes
                                    }
                                }
                            }
                            None => break, // channel closed, all senders dropped
                        }
                    }
                }
            }

            // Clear all spinners
            for spinner in &mut spinners {
                spinner.finish_and_clear();
            }

            match scrape_result.expect("scrape_folder must have completed") {
                Ok(result) => {
                    let summary = result.log.summary();
                    total_games += summary.total_success + summary.total_partial + summary.total_grouped;
                    total_media += summary.media_downloaded;
                    total_errors += summary.total_errors;
                    total_unidentified += summary.total_unidentified;

                    let has_issues = summary.total_unidentified > 0 || summary.total_errors > 0;

                    // In quiet mode, re-emit console header as warn for context
                    if has_issues && log::max_level() < LevelFilter::Info {
                        log::warn!(
                            "{} ({}):",
                            console.metadata.platform_name,
                            folder_name,
                        );
                    }

                    // Print per-system summary
                    if summary.total_success > 0 {
                        log::info!(
                            "  {} {} games scraped (serial: {}, filename: {}, hash: {})",
                            "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                            summary.total_success,
                            summary.by_serial,
                            summary.by_filename,
                            summary.by_hash,
                        );
                    }
                    if summary.total_grouped > 0 {
                        log::info!(
                            "  {} {} discs grouped with primary",
                            "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                            summary.total_grouped,
                        );
                    }
                    if summary.media_downloaded > 0 {
                        log::info!(
                            "  {} {} media files downloaded",
                            "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                            summary.media_downloaded,
                        );
                    }
                    if summary.total_unidentified > 0 {
                        log::warn!(
                            "  {} {} unidentified",
                            "?".if_supports_color(Stdout, |t| t.yellow()),
                            summary.total_unidentified,
                        );
                    }
                    if summary.total_errors > 0 {
                        log::warn!(
                            "  {} {} errors",
                            "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                            summary.total_errors,
                        );
                    }

                    // Print per-game details for problem entries
                    for entry in result.log.entries() {
                        match entry {
                            retro_junk_scraper::LogEntry::Unidentified {
                                file,
                                serial_tried,
                                scraper_serial_tried,
                                filename_tried,
                                hashes_tried,
                                crc32,
                                md5,
                                sha1,
                                errors,
                            } => {
                                log::warn!("  ? {}: unidentified", file);
                                if let Some(s) = serial_tried {
                                    log::warn!("      ROM serial: {}", s);
                                }
                                if let Some(s) = scraper_serial_tried {
                                    log::warn!("      Scraper serial tried: {}", s);
                                }
                                if *filename_tried {
                                    log::warn!("      Filename lookup: tried");
                                }
                                if *hashes_tried {
                                    log::warn!("      Hash lookup: tried");
                                    if let Some(c) = crc32 {
                                        log::warn!("        CRC32: {}", c);
                                    }
                                    if let Some(m) = md5 {
                                        log::warn!("        MD5:   {}", m);
                                    }
                                    if let Some(s) = sha1 {
                                        log::warn!("        SHA1:  {}", s);
                                    }
                                }
                                for e in errors {
                                    log::warn!("      Error: {}", e);
                                }
                            }
                            retro_junk_scraper::LogEntry::Partial {
                                file,
                                game_name,
                                warnings,
                            } => {
                                log::warn!(
                                    "  {} {}: \"{}\"",
                                    "~".if_supports_color(Stdout, |t| t.green()),
                                    file,
                                    game_name.if_supports_color(Stdout, |t| t.green()),
                                );
                                for w in warnings {
                                    log::warn!("      {}", w);
                                }
                            }
                            retro_junk_scraper::LogEntry::Error { file, message } => {
                                log::warn!("  {} {}: {}", "\u{2718}", file, message);
                            }
                            _ => {}
                        }
                    }

                    // Write metadata
                    if !result.games.is_empty() && !dry_run {
                        let system_metadata_dir = options.metadata_dir.join(folder_name);
                        let system_media_dir = options.media_dir.join(folder_name);

                        use retro_junk_frontend::Frontend;
                        if let Err(e) = esde.write_metadata(
                            &result.games,
                            path,
                            &system_metadata_dir,
                            &system_media_dir,
                        ) {
                            log::warn!(
                                "  {} Error writing metadata: {}",
                                "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                                e,
                            );
                        } else {
                            log::info!(
                                "  {} gamelist.xml written to {}",
                                "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                                system_metadata_dir.display(),
                            );
                        }
                    }

                    // Write scrape log
                    if !no_log && !dry_run {
                        let log_path = options.metadata_dir.join(format!(
                            "scrape-log-{}-{}.txt",
                            folder_name,
                            chrono::Local::now().format("%Y%m%d-%H%M%S"),
                        ));
                        if let Err(e) = std::fs::create_dir_all(&options.metadata_dir) {
                            log::warn!("Warning: could not create metadata dir: {}", e);
                        } else if let Err(e) = result.log.write_to_file(&log_path) {
                            log::warn!("Warning: could not write scrape log: {}", e);
                        }
                    }
                }
                Err(e) => {
                    log::warn!(
                        "  {} Error: {}",
                        "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                        e,
                    );
                    total_errors += 1;
                }
            }
            log::info!("");
        }

        // Print overall summary
        if total_games > 0 || total_errors > 0 || total_unidentified > 0 {
            log::info!("{}", "Summary:".if_supports_color(Stdout, |t| t.bold()));
            if total_games > 0 {
                log::info!(
                    "  {} {} games scraped, {} media files",
                    "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                    total_games,
                    total_media,
                );
            }
            if total_unidentified > 0 {
                log::warn!(
                    "  {} {} unidentified",
                    "?".if_supports_color(Stdout, |t| t.yellow()),
                    total_unidentified,
                );
            }
            if total_errors > 0 {
                log::warn!(
                    "  {} {} errors",
                    "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                    total_errors,
                );
            }

            // Show remaining quota
            if let Some(quota) = client.current_quota().await {
                log::info!(
                    "  Quota: {}/{} requests used today",
                    quota.requests_today(),
                    quota.max_requests_per_day(),
                );
            }
        }
    });
}

/// Run the list command.
fn run_list(ctx: &AnalysisContext) {
    log::info!("Supported consoles:");
    log::info!("");

    let mut current_manufacturer = "";

    for console in ctx.consoles() {
        if console.metadata.manufacturer != current_manufacturer {
            if !current_manufacturer.is_empty() {
                log::info!("");
            }
            current_manufacturer = console.metadata.manufacturer;
            log::info!(
                "{}:",
                current_manufacturer.if_supports_color(Stdout, |t| t.bold()),
            );
        }

        let extensions = console.metadata.extensions.join(", ");
        let folders = console.metadata.folder_names.join(", ");
        let has_dat = console.analyzer.has_dat_support();

        log::info!(
            "  {} [{}]{}",
            console
                .metadata
                .short_name
                .if_supports_color(Stdout, |t| t.bold()),
            console
                .metadata
                .platform_name
                .if_supports_color(Stdout, |t| t.cyan()),
            if has_dat {
                format!(" {}", "(DAT)".if_supports_color(Stdout, |t| t.green()))
            } else {
                String::new()
            },
        );
        log::info!("    Extensions: {}", extensions);
        log::info!("    Folder names: {}", folders);
    }
}

/// List cached DAT files.
fn run_cache_list() {
    match retro_junk_dat::cache::list() {
        Ok(entries) => {
            if entries.is_empty() {
                log::info!(
                    "{}",
                    "No cached DAT files.".if_supports_color(Stdout, |t| t.dimmed()),
                );
                log::info!("Run 'retro-junk cache fetch <system>' to download DAT files.");
                return;
            }

            log::info!(
                "{}",
                "Cached DAT files:".if_supports_color(Stdout, |t| t.bold()),
            );
            log::info!("");

            let mut total_size = 0u64;
            for entry in &entries {
                total_size += entry.file_size;
                log::info!(
                    "  {} [{}]",
                    entry.short_name.if_supports_color(Stdout, |t| t.bold()),
                    entry.dat_name.if_supports_color(Stdout, |t| t.cyan()),
                );
                log::info!(
                    "    Size: {}, Downloaded: {}, Version: {}",
                    format_bytes(entry.file_size),
                    entry.downloaded,
                    entry.dat_version,
                );
            }
            log::info!("");
            log::info!(
                "Total: {} files, {}",
                entries.len(),
                format_bytes(total_size)
            );
        }
        Err(e) => {
            log::warn!(
                "{} Error listing cache: {}",
                "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                e,
            );
        }
    }
}

/// Clear the DAT cache.
fn run_cache_clear() {
    match retro_junk_dat::cache::clear() {
        Ok(freed) => {
            log::info!(
                "{} Cache cleared ({} freed)",
                "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                format_bytes(freed),
            );
        }
        Err(e) => {
            log::warn!(
                "{} Error clearing cache: {}",
                "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                e,
            );
        }
    }
}

/// Fetch DAT files for specified systems.
fn run_cache_fetch(ctx: &AnalysisContext, systems: Vec<String>) {
    use retro_junk_lib::DatSource;

    let to_fetch: Vec<(String, Vec<&str>, &'static [&'static str], DatSource)> =
        if systems.len() == 1 && systems[0].eq_ignore_ascii_case("all") {
            ctx.consoles()
                .filter(|c| c.analyzer.has_dat_support())
                .map(|c| {
                    (
                        c.metadata.short_name.to_string(),
                        c.analyzer.dat_names().to_vec(),
                        c.analyzer.dat_download_ids(),
                        c.analyzer.dat_source(),
                    )
                })
                .collect()
        } else {
            systems
                .into_iter()
                .filter_map(|short_name| {
                    let console = ctx.get_by_short_name(&short_name);
                    match console {
                        Some(c) => {
                            let dat_names = c.analyzer.dat_names();
                            if dat_names.is_empty() {
                                log::warn!(
                                    "  {} No DAT support for '{}'",
                                    "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                                    short_name,
                                );
                                None
                            } else {
                                Some((
                                    short_name,
                                    dat_names.to_vec(),
                                    c.analyzer.dat_download_ids(),
                                    c.analyzer.dat_source(),
                                ))
                            }
                        }
                        None => {
                            log::warn!(
                                "  {} Unknown system '{}'",
                                "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                                short_name,
                            );
                            None
                        }
                    }
                })
                .collect()
        };

    for (short_name, dat_names, download_ids, dat_source) in &to_fetch {
        match retro_junk_dat::cache::fetch(short_name, dat_names, download_ids, *dat_source) {
            Ok(paths) => {
                let total_size: u64 = paths
                    .iter()
                    .filter_map(|p| fs::metadata(p).ok())
                    .map(|m| m.len())
                    .sum();
                if paths.len() == 1 {
                    log::info!(
                        "  {} {} ({})",
                        "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                        short_name.if_supports_color(Stdout, |t| t.bold()),
                        format_bytes(total_size),
                    );
                } else {
                    log::info!(
                        "  {} {} ({} DATs, {})",
                        "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                        short_name.if_supports_color(Stdout, |t| t.bold()),
                        paths.len(),
                        format_bytes(total_size),
                    );
                }
            }
            Err(e) => {
                log::warn!(
                    "  {} {}: {}",
                    "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                    short_name.if_supports_color(Stdout, |t| t.bold()),
                    e,
                );
            }
        }
    }
}

// -- Config subcommands --

/// Mask a string, showing only the first 2 characters.
fn mask_value(s: &str) -> String {
    if s.len() <= 2 {
        "****".to_string()
    } else {
        format!("{}****", &s[..2])
    }
}

/// Show current credentials and their sources.
fn run_config_show() {
    use retro_junk_scraper::CredentialSource;

    let path = retro_junk_scraper::config_path();
    let sources = retro_junk_scraper::credential_sources();

    log::info!(
        "{}",
        "ScreenScraper Configuration".if_supports_color(Stdout, |t| t.bold()),
    );
    log::info!("");

    // Config file status
    match &path {
        Some(p) if p.exists() => {
            log::info!(
                "  Config file: {} {}",
                p.display().if_supports_color(Stdout, |t| t.cyan()),
                "(exists)".if_supports_color(Stdout, |t| t.green()),
            );
        }
        Some(p) => {
            log::info!(
                "  Config file: {} {}",
                p.display().if_supports_color(Stdout, |t| t.cyan()),
                "(not found)".if_supports_color(Stdout, |t| t.dimmed()),
            );
        }
        None => {
            log::info!(
                "  Config file: {}",
                "could not determine path".if_supports_color(Stdout, |t| t.red()),
            );
        }
    }
    log::info!("");

    // Resolve values per-field (Credentials::load() would fail if required fields are missing)
    let creds = retro_junk_scraper::Credentials::load().ok();

    let get_value =
        |source: &CredentialSource, from_creds: Option<String>, is_secret: bool| -> Option<String> {
            match source {
                CredentialSource::Missing => None,
                CredentialSource::Default => Some("retro-junk".to_string()),
                CredentialSource::Embedded => {
                    from_creds.map(|v| if is_secret { mask_value(&v) } else { v })
                }
                CredentialSource::EnvVar(var) => {
                    let v = std::env::var(var).ok()?;
                    Some(if is_secret { mask_value(&v) } else { v })
                }
                CredentialSource::ConfigFile => {
                    from_creds.map(|v| if is_secret { mask_value(&v) } else { v })
                }
            }
        };

    let fields: &[(&str, &CredentialSource, Option<String>)] = &[
        (
            "dev_id",
            &sources.dev_id,
            get_value(&sources.dev_id, creds.as_ref().map(|c| c.dev_id.clone()), false),
        ),
        (
            "dev_password",
            &sources.dev_password,
            get_value(
                &sources.dev_password,
                creds.as_ref().map(|c| c.dev_password.clone()),
                true,
            ),
        ),
        (
            "soft_name",
            &sources.soft_name,
            get_value(
                &sources.soft_name,
                creds.as_ref().map(|c| c.soft_name.clone()),
                false,
            ),
        ),
        (
            "user_id",
            &sources.user_id,
            get_value(
                &sources.user_id,
                creds.as_ref().and_then(|c| c.user_id.clone()),
                false,
            ),
        ),
        (
            "user_password",
            &sources.user_password,
            get_value(
                &sources.user_password,
                creds.as_ref().and_then(|c| c.user_password.clone()),
                true,
            ),
        ),
    ];

    for (name, source, value) in fields {
        let source_str = format!("({})", source);
        match value {
            Some(v) => {
                log::info!(
                    "  {} {} {}",
                    format!("{}:", name).if_supports_color(Stdout, |t| t.cyan()),
                    v,
                    source_str.if_supports_color(Stdout, |t| t.dimmed()),
                );
            }
            None => {
                log::info!(
                    "  {} {} {}",
                    format!("{}:", name).if_supports_color(Stdout, |t| t.cyan()),
                    "not set".if_supports_color(Stdout, |t| t.yellow()),
                    source_str.if_supports_color(Stdout, |t| t.dimmed()),
                );
            }
        }
    }
}

/// Interactively set up credentials.
fn run_config_setup() {
    println!(
        "{}",
        "ScreenScraper Credential Setup".if_supports_color(Stdout, |t| t.bold()),
    );
    println!();

    // Load existing config as defaults
    let existing = retro_junk_scraper::Credentials::load().ok();

    let read_line = |prompt: &str, default: Option<&str>, required: bool| -> Option<String> {
        loop {
            if let Some(def) = default {
                print!("  {} [{}]: ", prompt, def);
            } else {
                print!("  {}: ", prompt);
            }
            std::io::stdout().flush().unwrap();

            let mut input = String::new();
            std::io::stdin().read_line(&mut input).unwrap();
            let trimmed = input.trim().to_string();

            if trimmed.is_empty() {
                if let Some(def) = default {
                    return Some(def.to_string());
                }
                if required {
                    println!(
                        "    {}",
                        "This field is required.".if_supports_color(Stdout, |t| t.yellow()),
                    );
                    continue;
                }
                return None;
            }
            return Some(trimmed);
        }
    };

    let has_embedded = retro_junk_scraper::has_embedded_dev_credentials();

    let (dev_id, dev_password) = if has_embedded {
        println!(
            "  {}",
            "Developer credentials: embedded in binary (no setup needed)"
                .if_supports_color(Stdout, |t| t.dimmed()),
        );
        // Use whatever load() resolved (embedded or overridden)
        let base = existing.as_ref();
        (
            base.map(|c| c.dev_id.clone())
                .unwrap_or_else(|| "embedded".to_string()),
            base.map(|c| c.dev_password.clone())
                .unwrap_or_else(|| "embedded".to_string()),
        )
    } else {
        println!(
            "  {}",
            "Developer credentials (required):".if_supports_color(Stdout, |t| t.dimmed()),
        );
        let dev_id = read_line(
            "dev_id",
            existing.as_ref().map(|c| c.dev_id.as_str()),
            true,
        )
        .unwrap();
        let dev_password = read_line(
            "dev_password",
            existing.as_ref().map(|c| c.dev_password.as_str()),
            true,
        )
        .unwrap();
        (dev_id, dev_password)
    };

    println!();
    println!(
        "  {}",
        "User credentials (optional, press Enter to skip):".if_supports_color(Stdout, |t| t.dimmed()),
    );
    let user_id = read_line(
        "user_id",
        existing.as_ref().and_then(|c| c.user_id.as_deref()),
        false,
    );
    let user_password = read_line(
        "user_password",
        existing
            .as_ref()
            .and_then(|c| c.user_password.as_deref()),
        false,
    );

    let creds = retro_junk_scraper::Credentials {
        dev_id,
        dev_password,
        soft_name: existing
            .map(|c| c.soft_name)
            .unwrap_or_else(|| "retro-junk".to_string()),
        user_id,
        user_password,
    };

    match retro_junk_scraper::save_to_file(&creds) {
        Ok(path) => {
            println!();
            println!(
                "{} Credentials saved to {}",
                "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                path.display().if_supports_color(Stdout, |t| t.cyan()),
            );
        }
        Err(e) => {
            eprintln!();
            eprintln!(
                "{} Failed to save credentials: {}",
                "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                e,
            );
        }
    }
}

/// Test credentials against the ScreenScraper API.
fn run_config_test(quiet: bool) {
    let creds = match retro_junk_scraper::Credentials::load() {
        Ok(c) => c,
        Err(e) => {
            log::warn!(
                "{} Failed to load credentials: {}",
                "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                e,
            );
            log::warn!("");
            log::warn!("Run 'retro-junk config setup' to configure credentials.");
            return;
        }
    };

    log::info!("Testing credentials against ScreenScraper API...");

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");

    rt.block_on(async {
        let pb = if quiet {
            ProgressBar::hidden()
        } else {
            let pb = ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::with_template("  {spinner:.cyan} {msg}")
                    .unwrap()
                    .tick_chars("/-\\|"),
            );
            pb.set_message("Connecting...");
            pb.enable_steady_tick(std::time::Duration::from_millis(100));
            pb
        };

        match retro_junk_scraper::ScreenScraperClient::new(creds).await {
            Ok((_client, user_info)) => {
                pb.finish_and_clear();
                log::info!(
                    "{} Credentials are valid!",
                    "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                );
                log::info!("");
                log::info!(
                    "  Requests today: {}/{}",
                    user_info.requests_today(),
                    user_info.max_requests_per_day(),
                );
                log::info!("  Max threads:    {}", user_info.max_threads());
            }
            Err(e) => {
                pb.finish_and_clear();
                log::warn!(
                    "{} Credential validation failed: {}",
                    "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                    e,
                );
            }
        }
    });
}

/// Print the config file path.
fn run_config_path() {
    match retro_junk_scraper::config_path() {
        Some(path) => log::info!("{}", path.display()),
        None => {
            log::warn!("Could not determine config directory");
            std::process::exit(1);
        }
    }
}

// -- Catalog subcommands --

/// Default path for the catalog database.
fn default_catalog_db_path() -> PathBuf {
    retro_junk_dat::cache::cache_dir()
        .unwrap_or_else(|_| PathBuf::from(".cache"))
        .join("catalog.db")
}

/// Default path for catalog YAML data.
fn default_catalog_dir() -> PathBuf {
    // Look for catalog/ relative to the current directory
    PathBuf::from("catalog")
}

/// Import DAT files into the catalog database.
fn run_catalog_import(
    ctx: &AnalysisContext,
    systems: Vec<String>,
    catalog_dir: Option<PathBuf>,
    db_path: Option<PathBuf>,
    dat_dir: Option<PathBuf>,
) {
    use retro_junk_import::{ImportStats, dat_source_str, import_dat, log_import};

    let db_path = db_path.unwrap_or_else(default_catalog_db_path);
    let catalog_dir = catalog_dir.unwrap_or_else(default_catalog_dir);

    // Open or create the database
    let conn = match retro_junk_db::open_database(&db_path) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to open catalog database at {}: {}", db_path.display(), e);
            std::process::exit(1);
        }
    };

    // Seed platforms and companies from YAML
    if catalog_dir.exists() {
        match retro_junk_db::seed_from_catalog(&conn, &catalog_dir) {
            Ok(stats) => {
                log::info!(
                    "Seeded {} platforms, {} companies, {} overrides from {}",
                    stats.platforms,
                    stats.companies,
                    stats.overrides,
                    catalog_dir.display(),
                );
            }
            Err(e) => {
                log::warn!("Warning: failed to seed from catalog YAML: {}", e);
            }
        }
    } else {
        log::warn!(
            "Catalog directory not found at {}; skipping YAML seed",
            catalog_dir.display(),
        );
    }

    // Determine which consoles to import
    let to_import: Vec<_> = if systems.len() == 1 && systems[0].eq_ignore_ascii_case("all") {
        ctx.consoles()
            .filter(|c| c.analyzer.has_dat_support())
            .collect()
    } else {
        systems
            .iter()
            .filter_map(|name| {
                let console = ctx.get_by_short_name(name);
                if console.is_none() {
                    log::warn!(
                        "  {} Unknown system '{}'",
                        "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                        name,
                    );
                }
                console
            })
            .filter(|c| {
                if !c.analyzer.has_dat_support() {
                    log::warn!(
                        "  {} No DAT support for '{}'",
                        "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                        c.metadata.short_name,
                    );
                    return false;
                }
                true
            })
            .collect()
    };

    if to_import.is_empty() {
        log::warn!("No systems to import.");
        return;
    }

    log::info!(
        "{}",
        format!("Importing {} system(s) into {}", to_import.len(), db_path.display())
            .if_supports_color(Stdout, |t| t.bold()),
    );

    let mut total_stats = ImportStats::default();

    for console in &to_import {
        let short_name = console.metadata.short_name;
        let dat_names = console.analyzer.dat_names();
        let download_ids = console.analyzer.dat_download_ids();
        let source = console.analyzer.dat_source();
        let source_str = dat_source_str(&source);

        // Load DAT files (from custom dir or cache, auto-downloading if needed)
        let dats = match retro_junk_dat::cache::load_dats(
            short_name,
            dat_names,
            download_ids,
            dat_dir.as_deref(),
            source,
        ) {
            Ok(d) => d,
            Err(e) => {
                log::warn!(
                    "  {} {}: {}",
                    "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                    short_name.if_supports_color(Stdout, |t| t.bold()),
                    e,
                );
                continue;
            }
        };

        // Import each DAT
        for dat in &dats {
            let progress = CliImportProgress::new(short_name);
            let stats = match import_dat(&conn, dat, console.metadata.platform, source_str, Some(&progress)) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!(
                        "  {} {}: import failed: {}",
                        "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                        short_name.if_supports_color(Stdout, |t| t.bold()),
                        e,
                    );
                    continue;
                }
            };

            // Log the import
            if let Err(e) = log_import(&conn, source_str, &dat.name, Some(&dat.version), &stats) {
                log::warn!("Failed to log import: {}", e);
            }

            log::info!(
                "  {} {} — {} games: {} works, {} releases, {} media ({} new, {} updated, {} unchanged), {} skipped",
                "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                short_name.if_supports_color(Stdout, |t| t.bold()),
                stats.total_games,
                stats.works_created + stats.works_existing,
                stats.releases_created + stats.releases_existing,
                stats.media_created + stats.media_updated + stats.media_unchanged,
                stats.media_created,
                stats.media_updated,
                stats.media_unchanged,
                stats.skipped_bad,
            );

            total_stats.works_created += stats.works_created;
            total_stats.works_existing += stats.works_existing;
            total_stats.releases_created += stats.releases_created;
            total_stats.releases_existing += stats.releases_existing;
            total_stats.media_created += stats.media_created;
            total_stats.media_updated += stats.media_updated;
            total_stats.media_unchanged += stats.media_unchanged;
            total_stats.skipped_bad += stats.skipped_bad;
            total_stats.total_games += stats.total_games;
            total_stats.disagreements_found += stats.disagreements_found;
        }
    }

    // Apply overrides after all imports
    let overrides_applied = if catalog_dir.exists() {
        match retro_junk_catalog::yaml::load_overrides(&catalog_dir.join("overrides")) {
            Ok(overrides) if !overrides.is_empty() => {
                match retro_junk_import::apply_overrides(&conn, &overrides) {
                    Ok(count) => {
                        if count > 0 {
                            log::info!(
                                "  {} Applied {} override(s)",
                                "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                                count,
                            );
                        }
                        count
                    }
                    Err(e) => {
                        log::warn!("Failed to apply overrides: {}", e);
                        0
                    }
                }
            }
            Ok(_) => 0,
            Err(e) => {
                log::warn!("Failed to load overrides: {}", e);
                0
            }
        }
    } else {
        0
    };

    log::info!("");
    log::info!(
        "{}",
        "Import complete".if_supports_color(Stdout, |t| t.bold()),
    );
    log::info!(
        "  Works: {} new, {} existing",
        total_stats.works_created,
        total_stats.works_existing,
    );
    log::info!(
        "  Releases: {} new, {} existing",
        total_stats.releases_created,
        total_stats.releases_existing,
    );
    log::info!(
        "  Media: {} new, {} updated, {} unchanged, {} bad dumps skipped",
        total_stats.media_created,
        total_stats.media_updated,
        total_stats.media_unchanged,
        total_stats.skipped_bad,
    );
    if total_stats.disagreements_found > 0 {
        log::info!("  Disagreements: {}", total_stats.disagreements_found);
    }
    if overrides_applied > 0 {
        log::info!("  Overrides applied: {}", overrides_applied);
    }
    log::info!("  Database: {}", db_path.display());
}

/// CLI progress reporter for DAT imports.
struct CliImportProgress {
    system: String,
}

impl CliImportProgress {
    fn new(system: &str) -> Self {
        Self {
            system: system.to_string(),
        }
    }
}

impl retro_junk_import::ImportProgress for CliImportProgress {
    fn on_game(&self, current: usize, total: usize, _name: &str) {
        // Log progress every 1000 games to avoid spam
        if current % 1000 == 0 && current < total {
            log::info!(
                "    {} [{}/{}]",
                self.system.if_supports_color(Stdout, |t| t.dimmed()),
                current,
                total,
            );
        }
    }

    fn on_phase(&self, message: &str) {
        log::info!("{}", message);
    }

    fn on_complete(&self, message: &str) {
        log::info!("{}", message);
    }
}

/// Analyze media asset coverage gaps.
fn run_catalog_gaps(
    system: String,
    db_path: Option<PathBuf>,
    collection_only: bool,
    missing: Option<String>,
    limit: u32,
) {
    let db_path = db_path.unwrap_or_else(default_catalog_db_path);

    if !db_path.exists() {
        log::warn!("No catalog database found at {}", db_path.display());
        log::info!("Run 'retro-junk catalog import all' first.");
        return;
    }

    let conn = match retro_junk_db::open_database(&db_path) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to open catalog database: {}", e);
            std::process::exit(1);
        }
    };

    let scope = if collection_only {
        "collection"
    } else {
        "catalog"
    };

    // If --missing is specified, list releases missing that asset type
    if let Some(ref asset_type) = missing {
        log::info!(
            "{}",
            format!(
                "Releases in {} {} missing '{}' assets:",
                scope, system, asset_type,
            )
            .if_supports_color(Stdout, |t| t.bold()),
        );

        match retro_junk_db::releases_missing_asset_type(
            &conn,
            &system,
            asset_type,
            collection_only,
            Some(limit),
        ) {
            Ok(releases) => {
                if releases.is_empty() {
                    log::info!("  No gaps found — all releases have '{}' assets.", asset_type);
                } else {
                    for (id, title, region) in &releases {
                        log::info!(
                            "  {} {} ({})",
                            "\u{2022}".if_supports_color(Stdout, |t| t.dimmed()),
                            title,
                            region.if_supports_color(Stdout, |t| t.dimmed()),
                        );
                        log::debug!("    {}", id);
                    }
                    if releases.len() as u32 == limit {
                        log::info!(
                            "  ... (showing first {}, use --limit to see more)",
                            limit,
                        );
                    }
                }
            }
            Err(e) => {
                log::error!("Failed to query gaps: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }

    // Default: show coverage summary
    log::info!(
        "{}",
        format!("Asset coverage for {} ({}):", system, scope)
            .if_supports_color(Stdout, |t| t.bold()),
    );

    // Coverage summary
    match retro_junk_db::asset_coverage_summary(&conn, &system, collection_only) {
        Ok((total, with_assets, asset_count)) => {
            let pct = if total > 0 {
                (with_assets as f64 / total as f64 * 100.0) as u32
            } else {
                0
            };
            log::info!("  Total releases:       {:>6}", total);
            log::info!("  With any asset:       {:>6} ({}%)", with_assets, pct);
            log::info!("  Without any asset:    {:>6}", total - with_assets);
            log::info!("  Total assets:         {:>6}", asset_count);
        }
        Err(e) => {
            log::error!("Failed to query coverage: {}", e);
            std::process::exit(1);
        }
    }

    // Asset counts by type
    match retro_junk_db::asset_counts_by_type(&conn, &system, collection_only) {
        Ok(counts) => {
            if !counts.is_empty() {
                log::info!("");
                log::info!(
                    "{}",
                    "  Assets by type:".if_supports_color(Stdout, |t| t.bold()),
                );
                for (asset_type, count) in &counts {
                    log::info!("    {:<20} {:>6}", asset_type, count);
                }
            }
        }
        Err(e) => {
            log::error!("Failed to query asset types: {}", e);
        }
    }

    // Show releases with no assets at all
    match retro_junk_db::releases_with_no_assets(&conn, &system, collection_only, Some(10)) {
        Ok(releases) => {
            if !releases.is_empty() {
                log::info!("");
                log::info!(
                    "{}",
                    format!("  Releases with no assets (first {}):", releases.len())
                        .if_supports_color(Stdout, |t| t.dimmed()),
                );
                for (_id, title, region) in &releases {
                    log::info!(
                        "    {} {} ({})",
                        "\u{2022}".if_supports_color(Stdout, |t| t.dimmed()),
                        title,
                        region,
                    );
                }
            }
        }
        Err(e) => {
            log::error!("Failed to query releases without assets: {}", e);
        }
    }

    log::info!("");
    log::info!(
        "Use {} to see releases missing a specific type.",
        "--missing <type>".if_supports_color(Stdout, |t| t.bold()),
    );
    log::info!(
        "  Asset types: box-front, box-back, screenshot, title-screen, wheel, fanart, cart-front"
    );
}

/// Show catalog database statistics.
fn run_catalog_stats(db_path: Option<PathBuf>) {
    let db_path = db_path.unwrap_or_else(default_catalog_db_path);

    if !db_path.exists() {
        log::warn!("No catalog database found at {}", db_path.display());
        log::info!("Run 'retro-junk catalog import all' to create one.");
        return;
    }

    let conn = match retro_junk_db::open_database(&db_path) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to open catalog database: {}", e);
            std::process::exit(1);
        }
    };

    match retro_junk_db::catalog_stats(&conn) {
        Ok(stats) => {
            log::info!(
                "{}",
                "Catalog Database Statistics".if_supports_color(Stdout, |t| t.bold()),
            );
            log::info!("  Database: {}", db_path.display());
            log::info!("");
            log::info!("  Platforms:      {:>8}", stats.platforms);
            log::info!("  Companies:      {:>8}", stats.companies);
            log::info!("  Works:          {:>8}", stats.works);
            log::info!("  Releases:       {:>8}", stats.releases);
            log::info!("  Media entries:  {:>8}", stats.media);
            log::info!("  Assets:         {:>8}", stats.assets);
            log::info!("  Owned (coll.):  {:>8}", stats.collection_owned);
            log::info!(
                "  Disagreements:  {:>8} (unresolved)",
                stats.unresolved_disagreements,
            );
        }
        Err(e) => {
            log::error!("Failed to query catalog stats: {}", e);
            std::process::exit(1);
        }
    }
}

/// Delete and recreate the catalog database.
fn run_catalog_reset(db_path: Option<PathBuf>, confirm: bool) {
    let db_path = db_path.unwrap_or_else(default_catalog_db_path);

    if !confirm {
        log::warn!(
            "This will permanently delete the catalog database at:\n  {}",
            db_path.display(),
        );
        log::info!("Re-run with --confirm to proceed:");
        log::info!("  retro-junk catalog reset --confirm");
        return;
    }

    if !db_path.exists() {
        log::info!("No catalog database found at {}", db_path.display());
        log::info!("Nothing to reset.");
        return;
    }

    let file_size = std::fs::metadata(&db_path)
        .map(|m| m.len())
        .unwrap_or(0);

    match std::fs::remove_file(&db_path) {
        Ok(()) => {
            let size_mb = file_size as f64 / (1024.0 * 1024.0);
            log::info!(
                "{}",
                "Catalog database deleted.".if_supports_color(Stdout, |t| t.bold()),
            );
            log::info!("  Path: {}", db_path.display());
            log::info!("  Freed: {:.1} MB", size_mb);
            log::info!("");
            log::info!("Run 'retro-junk catalog import all' to rebuild.");
        }
        Err(e) => {
            log::error!("Failed to delete {}: {}", db_path.display(), e);
            std::process::exit(1);
        }
    }
}

/// Enrich catalog releases with ScreenScraper metadata.
fn run_catalog_enrich(
    systems: Vec<String>,
    db_path: Option<PathBuf>,
    limit: Option<u32>,
    force: bool,
    download_assets: bool,
    asset_dir: Option<PathBuf>,
    region: String,
    language: String,
    force_hash: bool,
    quiet: bool,
) {
    use retro_junk_import::scraper_import::{self, EnrichOptions, EnrichProgress, EnrichStats};
    use retro_junk_scraper::lookup::LookupMethod;

    let db_path = db_path.unwrap_or_else(default_catalog_db_path);

    if !db_path.exists() {
        log::warn!("No catalog database found at {}", db_path.display());
        log::info!("Run 'retro-junk catalog import all' first.");
        return;
    }

    let conn = match retro_junk_db::open_database(&db_path) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to open catalog database: {}", e);
            std::process::exit(1);
        }
    };

    // Resolve "all" to all platforms with core_platform set
    let platform_ids = if systems.len() == 1 && systems[0].eq_ignore_ascii_case("all") {
        match retro_junk_db::list_platforms(&conn) {
            Ok(platforms) => platforms
                .into_iter()
                .filter(|p| p.core_platform.is_some())
                .map(|p| p.id)
                .collect(),
            Err(e) => {
                log::error!("Failed to list platforms: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        systems
    };

    if platform_ids.is_empty() {
        log::warn!("No systems specified.");
        return;
    }

    let options = EnrichOptions {
        platform_ids,
        limit,
        skip_existing: !force,
        download_assets,
        asset_dir,
        preferred_region: region,
        preferred_language: language,
        force_hash,
    };

    // Load credentials and create client
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");

    rt.block_on(async {
        let creds = match retro_junk_scraper::credentials::Credentials::load() {
            Ok(c) => c,
            Err(e) => {
                log::error!("Failed to load ScreenScraper credentials: {}", e);
                log::info!("Run 'retro-junk config setup' to configure credentials.");
                std::process::exit(1);
            }
        };

        let (client, user_info) = match retro_junk_scraper::client::ScreenScraperClient::new(creds).await {
            Ok(r) => r,
            Err(e) => {
                log::error!("Failed to connect to ScreenScraper: {}", e);
                std::process::exit(1);
            }
        };

        log::info!(
            "{}",
            format!(
                "Connected to ScreenScraper (requests today: {}/{})",
                user_info.requests_today(),
                user_info.max_requests_per_day(),
            )
            .if_supports_color(Stdout, |t| t.bold()),
        );

        struct CliEnrichProgress {
            quiet: bool,
        }

        impl EnrichProgress for CliEnrichProgress {
            fn on_release(&self, current: usize, total: usize, title: &str) {
                if !self.quiet {
                    log::info!(
                        "  [{}/{}] {}",
                        current,
                        total,
                        title.if_supports_color(Stdout, |t| t.dimmed()),
                    );
                }
            }

            fn on_found(&self, _current: usize, title: &str, ss_name: &str, method: &LookupMethod) {
                log::info!(
                    "    {} {} (via {}, SS: \"{}\")",
                    "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                    title.if_supports_color(Stdout, |t| t.bold()),
                    method,
                    ss_name,
                );
            }

            fn on_not_found(&self, _current: usize, title: &str) {
                log::info!(
                    "    {} {}",
                    "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                    title,
                );
            }

            fn on_asset_downloaded(&self, _current: usize, _title: &str, asset_type: &str) {
                log::debug!("      Downloaded {}", asset_type);
            }

            fn on_error(&self, _current: usize, title: &str, error: &str) {
                log::warn!(
                    "    {} {}: {}",
                    "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                    title,
                    error,
                );
            }

            fn on_complete(&self, stats: &EnrichStats) {
                log::info!("");
                log::info!(
                    "{}",
                    "Enrichment complete".if_supports_color(Stdout, |t| t.bold()),
                );
                log::info!("  Processed:     {:>6}", stats.releases_processed);
                log::info!("  Enriched:      {:>6}", stats.releases_enriched);
                log::info!("  Not found:     {:>6}", stats.releases_not_found);
                log::info!("  Skipped:       {:>6}", stats.releases_skipped);
                log::info!("  Assets:        {:>6}", stats.assets_downloaded);
                log::info!("  Companies:     {:>6} (new)", stats.companies_created);
                log::info!("  Disagreements: {:>6}", stats.disagreements_found);
                if stats.errors > 0 {
                    log::info!("  Errors:        {:>6}", stats.errors);
                }
            }
        }

        let progress = CliEnrichProgress { quiet };

        match scraper_import::enrich_releases(&client, &conn, &options, Some(&progress)).await {
            Ok(_) => {}
            Err(e) => {
                log::error!("Enrichment failed: {}", e);
                std::process::exit(1);
            }
        }
    });
}

/// Scan a ROM folder and add matched files to the collection.
fn run_catalog_scan(
    ctx: &AnalysisContext,
    system: String,
    folder: PathBuf,
    db_path: Option<PathBuf>,
    user_id: String,
    quiet: bool,
) {
    use retro_junk_import::scan_import::{ScanOptions, ScanProgress, ScanStats};

    let db_path = db_path.unwrap_or_else(default_catalog_db_path);

    if !db_path.exists() {
        log::warn!("No catalog database found at {}", db_path.display());
        log::info!("Run 'retro-junk catalog import all' first.");
        return;
    }

    if !folder.exists() {
        log::error!("ROM folder not found: {}", folder.display());
        std::process::exit(1);
    }

    let console = match ctx.get_by_short_name(&system) {
        Some(c) => c,
        None => {
            log::error!("Unknown system '{}'. Use a short name like 'nes', 'snes', 'n64'.", system);
            std::process::exit(1);
        }
    };

    let conn = match retro_junk_db::open_database(&db_path) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to open catalog database: {}", e);
            std::process::exit(1);
        }
    };

    let options = ScanOptions {
        user_id,
    };

    struct CliScanProgress {
        quiet: bool,
    }

    impl ScanProgress for CliScanProgress {
        fn on_file(&self, current: usize, total: usize, filename: &str) {
            if !self.quiet {
                log::debug!("  [{}/{}] {}", current, total, filename);
            }
        }

        fn on_match(&self, filename: &str, title: &str) {
            log::info!(
                "  {} {} -> {}",
                "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                filename,
                title.if_supports_color(Stdout, |t| t.bold()),
            );
        }

        fn on_no_match(&self, filename: &str) {
            if !self.quiet {
                log::info!(
                    "  {} {}",
                    "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                    filename.if_supports_color(Stdout, |t| t.dimmed()),
                );
            }
        }

        fn on_error(&self, filename: &str, error: &str) {
            log::warn!(
                "  {} {}: {}",
                "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                filename,
                error,
            );
        }

        fn on_complete(&self, stats: &ScanStats) {
            log::info!("");
            log::info!(
                "{}",
                "Scan complete".if_supports_color(Stdout, |t| t.bold()),
            );
            log::info!("  Files scanned: {:>6}", stats.files_scanned);
            log::info!("  Matched:       {:>6}", stats.matched);
            log::info!("  Already owned: {:>6}", stats.already_owned);
            log::info!("  Unmatched:     {:>6}", stats.unmatched);
            if stats.errors > 0 {
                log::info!("  Errors:        {:>6}", stats.errors);
            }
        }
    }

    log::info!(
        "{}",
        format!(
            "Scanning {} ROMs in {}",
            console.metadata.short_name,
            folder.display()
        )
        .if_supports_color(Stdout, |t| t.bold()),
    );

    let progress = CliScanProgress { quiet };

    match retro_junk_import::scan_folder(
        &conn,
        &folder,
        console.analyzer.as_ref(),
        console.metadata.platform,
        &options,
        Some(&progress),
    ) {
        Ok(result) => {
            if !result.unmatched.is_empty() && !quiet {
                log::info!("");
                log::info!(
                    "{}",
                    format!("{} unmatched files:", result.unmatched.len())
                        .if_supports_color(Stdout, |t| t.dimmed()),
                );
                for f in &result.unmatched {
                    log::info!(
                        "  {} (CRC32: {}, size: {})",
                        f.path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(),
                        f.crc32,
                        f.file_size,
                    );
                }
            }
        }
        Err(e) => {
            log::error!("Scan failed: {}", e);
            std::process::exit(1);
        }
    }
}

/// Re-verify collection entries against files on disk.
fn run_catalog_verify(
    ctx: &AnalysisContext,
    system: String,
    db_path: Option<PathBuf>,
    user_id: String,
    _quiet: bool,
) {
    let db_path = db_path.unwrap_or_else(default_catalog_db_path);

    if !db_path.exists() {
        log::warn!("No catalog database found at {}", db_path.display());
        log::info!("Run 'retro-junk catalog import all' first.");
        return;
    }

    let console = match ctx.get_by_short_name(&system) {
        Some(c) => c,
        None => {
            log::error!("Unknown system '{}'. Use a short name like 'nes', 'snes', 'n64'.", system);
            std::process::exit(1);
        }
    };

    let conn = match retro_junk_db::open_database(&db_path) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to open catalog database: {}", e);
            std::process::exit(1);
        }
    };

    log::info!(
        "{}",
        format!(
            "Verifying {} collection entries",
            console.metadata.short_name
        )
        .if_supports_color(Stdout, |t| t.bold()),
    );

    match retro_junk_import::verify_collection(
        &conn,
        console.analyzer.as_ref(),
        console.metadata.platform,
        &user_id,
    ) {
        Ok(stats) => {
            log::info!("");
            log::info!(
                "{}",
                "Verification complete".if_supports_color(Stdout, |t| t.bold()),
            );
            log::info!("  Checked:        {:>6}", stats.checked);
            log::info!("  Verified:       {:>6}", stats.verified);
            log::info!("  Missing:        {:>6}", stats.missing);
            log::info!("  Hash mismatch:  {:>6}", stats.hash_mismatch);
            log::info!("  No path:        {:>6}", stats.no_path);
            if stats.errors > 0 {
                log::info!("  Errors:         {:>6}", stats.errors);
            }
        }
        Err(e) => {
            log::error!("Verification failed: {}", e);
            std::process::exit(1);
        }
    }
}

/// List unresolved disagreements between data sources.
fn run_catalog_disagreements(
    db_path: Option<PathBuf>,
    system: Option<String>,
    field: Option<String>,
    limit: u32,
) {
    use retro_junk_db::DisagreementFilter;

    let db_path = db_path.unwrap_or_else(default_catalog_db_path);

    if !db_path.exists() {
        log::warn!("No catalog database found at {}", db_path.display());
        log::info!("Run 'retro-junk catalog import all' first.");
        return;
    }

    let conn = match retro_junk_db::open_database(&db_path) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to open catalog database: {}", e);
            std::process::exit(1);
        }
    };

    let filter = DisagreementFilter {
        platform_id: system.as_deref(),
        field: field.as_deref(),
        limit: Some(limit),
        ..Default::default()
    };

    match retro_junk_db::list_unresolved_disagreements(&conn, &filter) {
        Ok(disagreements) => {
            if disagreements.is_empty() {
                log::info!("No unresolved disagreements found.");
                return;
            }

            log::info!(
                "{}",
                format!("{} unresolved disagreement(s):", disagreements.len())
                    .if_supports_color(Stdout, |t| t.bold()),
            );
            log::info!("");

            for d in &disagreements {
                log::info!(
                    "  #{} {} {} [{}]",
                    format!("{}", d.id).if_supports_color(Stdout, |t| t.bold()),
                    d.entity_type,
                    d.entity_id.if_supports_color(Stdout, |t| t.dimmed()),
                    d.field.if_supports_color(Stdout, |t| t.cyan()),
                );
                log::info!(
                    "    {} {}: {}",
                    "\u{25B6}".if_supports_color(Stdout, |t| t.blue()),
                    d.source_a,
                    d.value_a.as_deref().unwrap_or("(empty)"),
                );
                log::info!(
                    "    {} {}: {}",
                    "\u{25B6}".if_supports_color(Stdout, |t| t.yellow()),
                    d.source_b,
                    d.value_b.as_deref().unwrap_or("(empty)"),
                );
                log::info!("");
            }

            log::info!(
                "Resolve with: retro-junk catalog resolve <id> --source-a | --source-b | --custom <value>"
            );
        }
        Err(e) => {
            log::error!("Failed to query disagreements: {}", e);
            std::process::exit(1);
        }
    }
}

/// Resolve a disagreement by choosing a value.
fn run_catalog_resolve(
    id: i64,
    db_path: Option<PathBuf>,
    source_a: bool,
    source_b: bool,
    custom: Option<String>,
) {
    let db_path = db_path.unwrap_or_else(default_catalog_db_path);

    if !db_path.exists() {
        log::warn!("No catalog database found at {}", db_path.display());
        return;
    }

    let conn = match retro_junk_db::open_database(&db_path) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to open catalog database: {}", e);
            std::process::exit(1);
        }
    };

    // Fetch the disagreement first
    let disagreement = match retro_junk_db::get_disagreement(&conn, id) {
        Ok(Some(d)) => d,
        Ok(None) => {
            log::error!("Disagreement #{} not found.", id);
            std::process::exit(1);
        }
        Err(e) => {
            log::error!("Failed to fetch disagreement: {}", e);
            std::process::exit(1);
        }
    };

    if disagreement.resolved {
        log::warn!(
            "Disagreement #{} is already resolved (resolution: {}).",
            id,
            disagreement.resolution.as_deref().unwrap_or("unknown"),
        );
        return;
    }

    // Determine resolution
    let (resolution, chosen_value) = if source_a {
        ("source_a".to_string(), disagreement.value_a.clone())
    } else if source_b {
        ("source_b".to_string(), disagreement.value_b.clone())
    } else if let Some(ref val) = custom {
        (format!("custom: {val}"), Some(val.clone()))
    } else {
        log::error!("Must specify --source-a, --source-b, or --custom <value>.");
        std::process::exit(1);
    };

    // Apply the chosen value to the entity
    if let Some(ref value) = chosen_value {
        if let Err(e) = retro_junk_db::apply_disagreement_resolution(
            &conn,
            &disagreement.entity_type,
            &disagreement.entity_id,
            &disagreement.field,
            value,
        ) {
            log::error!("Failed to apply resolution: {}", e);
            std::process::exit(1);
        }
    }

    // Mark as resolved
    match retro_junk_db::resolve_disagreement(&conn, id, &resolution) {
        Ok(()) => {
            log::info!(
                "{} Resolved disagreement #{}",
                "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                id,
            );
            log::info!(
                "  {} {} [{}] = {}",
                disagreement.entity_type,
                disagreement.entity_id,
                disagreement.field,
                chosen_value.as_deref().unwrap_or("(empty)"),
            );
        }
        Err(e) => {
            log::error!("Failed to resolve disagreement: {}", e);
            std::process::exit(1);
        }
    }
}

