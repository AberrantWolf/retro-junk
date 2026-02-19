//! retro-junk CLI
//!
//! Command-line interface for analyzing retro game ROMs and disc images.

use std::collections::HashSet;
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
use retro_junk_lib::{AnalysisContext, AnalysisOptions, Platform, RomAnalyzer, RomIdentification};

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

    /// Scrape game metadata and media from ScreenScraper.fr
    Scrape {
        #[command(flatten)]
        roms: RomFilterArgs,

        /// Media types to download (e.g., covers,screenshots,videos,marquees)
        #[arg(long, value_delimiter = ',')]
        media_types: Option<Vec<String>>,

        /// Directory for metadata files (default: <root>-metadata)
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

    // Read the root directory
    let entries = match fs::read_dir(&root_path) {
        Ok(entries) => entries,
        Err(e) => {
            log::warn!(
                "{} Error reading directory: {}",
                "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                e,
            );
            return;
        }
    };

    let mut found_any = false;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let folder_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name,
            None => continue,
        };

        // Find matching consoles for this folder
        let matching_consoles = ctx.find_by_folder(folder_name);

        if matching_consoles.is_empty() {
            continue;
        }

        // Filter by requested consoles if specified
        let consoles_to_use: Vec<_> = if let Some(ref filter) = consoles {
            matching_consoles
                .into_iter()
                .filter(|c| filter.contains(&c.metadata.platform))
                .collect()
        } else {
            matching_consoles
        };

        if consoles_to_use.is_empty() {
            continue;
        }

        found_any = true;

        for console in consoles_to_use {
            log::info!(
                "{} {} folder: {}",
                "Found".if_supports_color(Stdout, |t| t.bold()),
                console.metadata.platform_name,
                folder_name.if_supports_color(Stdout, |t| t.cyan()),
            );

            // Scan for ROM files in the folder
            analyze_folder(&path, console.analyzer.as_ref(), &options, limit);
        }
    }

    if !found_any {
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

    // Read root directory and find console folders
    let entries = match fs::read_dir(&root_path) {
        Ok(entries) => entries,
        Err(e) => {
            log::warn!(
                "{} Error reading directory: {}",
                "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                e,
            );
            return;
        }
    };

    let mut total_renamed = 0usize;
    let mut total_already_correct = 0usize;
    let mut total_unmatched = 0usize;
    let mut total_errors: Vec<String> = Vec::new();
    let mut total_conflicts: Vec<String> = Vec::new();
    let mut found_any = false;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let folder_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name,
            None => continue,
        };

        let matching_consoles = ctx.find_by_folder(folder_name);
        if matching_consoles.is_empty() {
            continue;
        }

        // Filter by requested consoles
        let consoles_to_use: Vec<_> = if let Some(ref filter) = consoles {
            matching_consoles
                .into_iter()
                .filter(|c| filter.contains(&c.metadata.platform))
                .collect()
        } else {
            matching_consoles
        };

        if consoles_to_use.is_empty() {
            continue;
        }

        for console in consoles_to_use {
            // Check if this system has DAT support via the analyzer trait
            if !console.analyzer.has_dat_support() {
                log::warn!(
                    "  {} Skipping \"{}\" — no DAT support yet",
                    "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                    folder_name,
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
                &path,
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
                        format!("({})", folder_name).if_supports_color(Stdout, |t| t.dimmed()),
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
                    log::warn!(
                        "{} {}: {} Error: {}",
                        console
                            .metadata
                            .platform_name
                            .if_supports_color(Stdout, |t| t.bold()),
                        format!("({})", folder_name).if_supports_color(Stdout, |t| t.dimmed()),
                        "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                        e,
                    );
                }
            }
            log::info!("");
        }
    }

    if !found_any {
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
    for path in &plan.unmatched {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        log::warn!(
            "  {} {} (no match)",
            "?".if_supports_color(Stdout, |t| t.yellow()),
            name.if_supports_color(Stdout, |t| t.dimmed()),
        );
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
        match &w.kind {
            SerialWarningKind::NoMatch {
                full_serial,
                game_code,
            } => {
                if let Some(code) = game_code {
                    log::warn!(
                        "  {} {}: serial \"{}\" (looked up as \"{}\") not found in DAT",
                        "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                        file_name.if_supports_color(Stdout, |t| t.dimmed()),
                        full_serial,
                        code,
                    );
                } else {
                    log::warn!(
                        "  {} {}: serial \"{}\" not found in DAT",
                        "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                        file_name.if_supports_color(Stdout, |t| t.dimmed()),
                        full_serial,
                    );
                }
            }
            SerialWarningKind::Missing => {
                log::warn!(
                    "  {} {}: no serial found (expected for this platform)",
                    "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                    file_name.if_supports_color(Stdout, |t| t.dimmed()),
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
    root: Option<PathBuf>,
    quiet: bool,
) {
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

        log::info!(
            "{} Connected to ScreenScraper (requests today: {}/{}, max threads: {})",
            "\u{2714}".if_supports_color(Stdout, |t| t.green()),
            user_info.requests_today(),
            user_info.max_requests_per_day(),
            user_info.max_threads(),
        );
        log::info!("");

        // Scan root directory for console folders
        let entries = match fs::read_dir(&root_path) {
            Ok(entries) => entries,
            Err(e) => {
                log::warn!(
                    "{} Error reading directory: {}",
                    "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                    e,
                );
                return;
            }
        };

        let esde = retro_junk_frontend::esde::EsDeFrontend::new();
        let mut total_games = 0usize;
        let mut total_media = 0usize;
        let mut total_errors = 0usize;
        let mut total_unidentified = 0usize;

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let folder_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name,
                None => continue,
            };

            let matching_consoles = ctx.find_by_folder(folder_name);
            if matching_consoles.is_empty() {
                continue;
            }

            // Filter by requested consoles
            let consoles_to_use: Vec<_> = if let Some(ref filter) = consoles {
                matching_consoles
                    .into_iter()
                    .filter(|c| filter.contains(&c.metadata.platform))
                    .collect()
            } else {
                matching_consoles
            };

            if consoles_to_use.is_empty() {
                continue;
            }

            for console in consoles_to_use {
                let platform = console.metadata.platform;

                // Check if this system has a ScreenScraper ID
                if retro_junk_scraper::screenscraper_system_id(platform).is_none() {
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

                let pb = if quiet {
                    ProgressBar::hidden()
                } else {
                    let pb = ProgressBar::new_spinner();
                    pb.set_style(
                        ProgressStyle::with_template("  {spinner:.cyan} {msg}")
                            .unwrap()
                            .tick_chars("/-\\|"),
                    );
                    pb.enable_steady_tick(std::time::Duration::from_millis(100));
                    pb
                };

                let progress_callback = |progress: retro_junk_scraper::ScrapeProgress| {
                    match progress {
                        retro_junk_scraper::ScrapeProgress::Scanning => {
                            pb.set_message("Scanning for ROM files...");
                        }
                        retro_junk_scraper::ScrapeProgress::LookingUp { ref file, index, total } => {
                            pb.set_message(format!("[{}/{}] Looking up {}", index + 1, total, file));
                        }
                        retro_junk_scraper::ScrapeProgress::Downloading { ref media_type, ref file } => {
                            pb.set_message(format!("Downloading {} for {}", media_type, file));
                        }
                        retro_junk_scraper::ScrapeProgress::Skipped { ref file, ref reason } => {
                            pb.set_message(format!("Skipped {}: {}", file, reason));
                        }
                        retro_junk_scraper::ScrapeProgress::Done => {
                            pb.finish_and_clear();
                        }
                    }
                };

                match retro_junk_scraper::scrape_folder(
                    &client,
                    &path,
                    console.analyzer.as_ref(),
                    &options,
                    folder_name,
                    &progress_callback,
                )
                .await
                {
                    Ok(result) => {
                        pb.finish_and_clear();

                        let summary = result.log.summary();
                        total_games += summary.total_success + summary.total_partial;
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
                                    log::warn!("  ~ {}: \"{}\"", file, game_name);
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
                                &path,
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
                        pb.finish_and_clear();
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
