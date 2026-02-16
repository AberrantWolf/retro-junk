//! retro-junk CLI
//!
//! Command-line interface for analyzing retro game ROMs and disc images.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;

use retro_junk_lib::rename::{
    RenameOptions, RenamePlan, RenameProgress, execute_renames, format_match_method, plan_renames,
};
use retro_junk_lib::{AnalysisContext, AnalysisOptions, Platform, RomAnalyzer, RomIdentification};

#[derive(Parser)]
#[command(name = "retro-junk")]
#[command(about = "Analyze retro game ROMs and disc images", long_about = None)]
struct Cli {
    /// Root path containing console folders (defaults to current directory)
    #[arg(short, long, global = true)]
    root: Option<PathBuf>,

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
            run_rename(&ctx, dry_run, hash, roms.consoles, roms.limit, cli.root, dat_dir);
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
                cli.root,
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
            ConfigAction::Test => run_config_test(),
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

    println!("Analyzing ROMs in: {}", root_path.display());
    if quick {
        println!("Quick mode enabled");
    }
    if let Some(n) = limit {
        println!("Limit: {} ROMs per console", n);
    }
    println!();

    let options = AnalysisOptions::new().quick(quick);

    // Read the root directory
    let entries = match fs::read_dir(&root_path) {
        Ok(entries) => entries,
        Err(e) => {
            eprintln!(
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
            println!(
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
        println!(
            "{}",
            format!(
                "No matching console folders found in {}",
                root_path.display()
            )
            .if_supports_color(Stdout, |t| t.dimmed()),
        );
        println!();
        println!("Tip: Create folders named after consoles (e.g., 'snes', 'n64', 'ps1')");
        println!("     and place your ROM files inside them.");
        println!();
        println!("Run 'retro-junk list' to see all supported console names.");
    }
}

/// Analyze all ROM files in a folder.
fn analyze_folder(
    folder: &PathBuf,
    analyzer: &dyn RomAnalyzer,
    options: &AnalysisOptions,
    limit: Option<usize>,
) {
    let extensions: std::collections::HashSet<_> = analyzer
        .file_extensions()
        .iter()
        .map(|e| e.to_lowercase())
        .collect();

    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(e) => {
            eprintln!(
                "  {} Error reading folder: {}",
                "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                e,
            );
            return;
        }
    };

    let mut files: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();
        if extensions.contains(&ext) {
            files.push(path);
        }
    }
    files.sort();
    if let Some(max) = limit {
        files.truncate(max);
    }

    for path in &files {
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");

        // Try to analyze the file
        let mut file = match fs::File::open(path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!(
                    "  {} Error opening {}: {}",
                    "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                    file_name,
                    e,
                );
                continue;
            }
        };

        match analyzer.analyze(&mut file, options) {
            Ok(info) => {
                print_analysis(file_name, &info);
            }
            Err(e) => {
                println!(
                    "  {}: {} Analysis not implemented ({})",
                    file_name,
                    "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                    e,
                );
            }
        }
    }

    if files.is_empty() {
        println!(
            "  {}",
            "No ROM files found".if_supports_color(Stdout, |t| t.dimmed()),
        );
    }
    println!();
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

/// Print the analysis result for a single file.
fn print_analysis(file_name: &str, info: &RomIdentification) {
    let mut shown_keys: HashSet<&str> = HashSet::new();

    println!("  {}:", file_name.if_supports_color(Stdout, |t| t.bold()),);

    // (a) Identity fields
    if let Some(ref serial) = info.serial_number {
        println!(
            "    {}   {}",
            "Serial:".if_supports_color(Stdout, |t| t.cyan()),
            serial,
        );
    }
    if let Some(ref name) = info.internal_name {
        println!(
            "    {}     {}",
            "Name:".if_supports_color(Stdout, |t| t.cyan()),
            name,
        );
    }
    if let Some(ref maker) = info.maker_code {
        println!(
            "    {}    {}",
            "Maker:".if_supports_color(Stdout, |t| t.cyan()),
            maker,
        );
    }
    if let Some(ref version) = info.version {
        println!(
            "    {}  {}",
            "Version:".if_supports_color(Stdout, |t| t.cyan()),
            version,
        );
    }

    // (b) Format line
    if let Some(format) = info.extra.get("format") {
        shown_keys.insert("format");
        print!(
            "    {}   {}",
            "Format:".if_supports_color(Stdout, |t| t.cyan()),
            format,
        );
        if let Some(mapper) = info.extra.get("mapper") {
            shown_keys.insert("mapper");
            print!(", Mapper {}", mapper);
            if let Some(mapper_name) = info.extra.get("mapper_name") {
                shown_keys.insert("mapper_name");
                print!(" ({})", mapper_name);
            }
        }
        println!();
    }

    // (c) Hardware section
    let hardware_present: Vec<&str> = HARDWARE_KEYS
        .iter()
        .filter(|k| info.extra.contains_key(**k))
        .copied()
        .collect();

    if !hardware_present.is_empty() {
        println!(
            "    {}",
            "Hardware:".if_supports_color(Stdout, |t| t.bright_magenta()),
        );
        for key in &hardware_present {
            shown_keys.insert(key);
            let value = &info.extra[*key];
            println!(
                "      {} {}",
                format!("{}:", prettify_key(key)).if_supports_color(Stdout, |t| t.cyan()),
                value,
            );
        }
    }

    // (d) Size verdict
    match (info.file_size, info.expected_size) {
        (Some(actual), Some(expected)) => {
            let verdict = compute_size_verdict(actual, expected);
            println!(
                "    {}     {} on disk, {} expected [{}]",
                "Size:".if_supports_color(Stdout, |t| t.cyan()),
                format_bytes(actual),
                format_bytes(expected),
                print_size_verdict(&verdict),
            );
        }
        (Some(actual), None) => {
            println!(
                "    {}     {}",
                "Size:".if_supports_color(Stdout, |t| t.cyan()),
                format_bytes(actual),
            );
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
        let (emoji, colored_status) = if status.starts_with("OK") || status.starts_with("Valid") {
            (
                "\u{2714}",
                format!("{}", status.if_supports_color(Stdout, |t| t.green())),
            )
        } else {
            (
                "\u{2718}",
                format!("{}", status.if_supports_color(Stdout, |t| t.red())),
            )
        };
        let is_ok = status.starts_with("OK") || status.starts_with("Valid");
        if is_ok {
            println!(
                "    {} {}  {}",
                emoji.if_supports_color(Stdout, |t| t.green()),
                format!("{}:", name).if_supports_color(Stdout, |t| t.cyan()),
                colored_status,
            );
        } else {
            println!(
                "    {} {}  {}",
                emoji.if_supports_color(Stdout, |t| t.red()),
                format!("{}:", name).if_supports_color(Stdout, |t| t.cyan()),
                colored_status,
            );
        }
    }

    // (f) Region
    if !info.regions.is_empty() {
        let region_str: Vec<_> = info.regions.iter().map(|r| r.name()).collect();
        println!(
            "    {}   {}",
            "Region:".if_supports_color(Stdout, |t| t.cyan()),
            region_str.join(", "),
        );
    }

    // (g) Remaining extras
    let mut remaining: Vec<_> = info
        .extra
        .keys()
        .filter(|k| !shown_keys.contains(k.as_str()))
        .collect();
    remaining.sort();

    if !remaining.is_empty() {
        println!(
            "    {}",
            "Details:".if_supports_color(Stdout, |t| t.bright_magenta()),
        );
        for key in &remaining {
            let value = &info.extra[key.as_str()];
            println!(
                "      {} {}",
                format!("{}:", prettify_key(key)).if_supports_color(Stdout, |t| t.cyan()),
                value,
            );
        }
    }
}

/// Run the rename command.
fn run_rename(
    ctx: &AnalysisContext,
    dry_run: bool,
    hash_mode: bool,
    consoles: Option<Vec<Platform>>,
    limit: Option<usize>,
    root: Option<PathBuf>,
    dat_dir: Option<PathBuf>,
) {
    let root_path =
        root.unwrap_or_else(|| std::env::current_dir().expect("Failed to get current directory"));

    let rename_options = RenameOptions {
        hash_mode,
        dat_dir,
        limit,
        ..Default::default()
    };

    println!(
        "Scanning ROMs in: {}",
        root_path.display().if_supports_color(Stdout, |t| t.cyan()),
    );
    if hash_mode {
        println!(
            "{}",
            "Hash mode: computing CRC32 for all files".if_supports_color(Stdout, |t| t.dimmed()),
        );
    }
    if dry_run {
        println!(
            "{}",
            "Dry run: no files will be renamed".if_supports_color(Stdout, |t| t.dimmed()),
        );
    }
    if let Some(n) = limit {
        println!(
            "{}",
            format!("Limit: {} ROMs per console", n).if_supports_color(Stdout, |t| t.dimmed()),
        );
    }
    println!();

    // Read root directory and find console folders
    let entries = match fs::read_dir(&root_path) {
        Ok(entries) => entries,
        Err(e) => {
            eprintln!(
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
            if console.analyzer.dat_name().is_none() {
                eprintln!(
                    "  {} Skipping \"{}\" — no DAT support yet",
                    "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                    folder_name,
                );
                continue;
            }

            found_any = true;

            println!(
                "{} {}",
                console
                    .metadata
                    .platform_name
                    .if_supports_color(Stdout, |t| t.bold()),
                format!("({})", folder_name).if_supports_color(Stdout, |t| t.dimmed()),
            );

            // Set up progress bar
            let pb = ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::with_template("  {spinner:.cyan} {msg}")
                    .unwrap()
                    .tick_chars("/-\\|"),
            );

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
                    print_rename_plan(&plan);

                    if !dry_run && !plan.renames.is_empty() {
                        // Prompt for confirmation
                        print!("\n  Proceed with {} renames? [y/N] ", plan.renames.len(),);
                        std::io::stdout().flush().unwrap();

                        let mut input = String::new();
                        std::io::stdin().read_line(&mut input).unwrap();

                        if input.trim().eq_ignore_ascii_case("y") {
                            let summary = execute_renames(&plan);
                            total_renamed += summary.renamed;
                            total_already_correct += summary.already_correct;
                            total_errors.extend(summary.errors);
                            total_conflicts.extend(summary.conflicts);

                            println!(
                                "  {} {} files renamed",
                                "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                                summary.renamed,
                            );
                        } else {
                            println!("  {}", "Skipped".if_supports_color(Stdout, |t| t.dimmed()),);
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
                    eprintln!(
                        "  {} Error: {}",
                        "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                        e,
                    );
                }
            }
            println!();
        }
    }

    if !found_any {
        println!(
            "{}",
            "No console folders with DAT support found.".if_supports_color(Stdout, |t| t.dimmed()),
        );
        println!();
        println!("Supported systems for rename:");
        for console in ctx.consoles() {
            if let Some(dat_name) = console.analyzer.dat_name() {
                println!("  {} [{}]", console.metadata.short_name, dat_name,);
            }
        }
        return;
    }

    // Print overall summary
    println!("{}", "Summary:".if_supports_color(Stdout, |t| t.bold()),);
    if total_renamed > 0 {
        println!(
            "  {} {} files renamed",
            "\u{2714}".if_supports_color(Stdout, |t| t.green()),
            total_renamed,
        );
    }
    if total_already_correct > 0 {
        println!(
            "  {} {} already correctly named",
            "\u{2714}".if_supports_color(Stdout, |t| t.green()),
            total_already_correct,
        );
    }
    if total_unmatched > 0 {
        println!(
            "  {} {} unmatched",
            "?".if_supports_color(Stdout, |t| t.yellow()),
            total_unmatched,
        );
    }
    for conflict in &total_conflicts {
        println!(
            "  {} {}",
            "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
            conflict,
        );
    }
    for error in &total_errors {
        println!(
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

        println!(
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
        println!(
            "  {} {} already correctly named",
            "\u{2714}".if_supports_color(Stdout, |t| t.green()),
            plan.already_correct.len(),
        );
    }

    // Unmatched
    for path in &plan.unmatched {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        println!(
            "  {} {} (no match)",
            "?".if_supports_color(Stdout, |t| t.yellow()),
            name.if_supports_color(Stdout, |t| t.dimmed()),
        );
    }

    // Conflicts
    for (_, msg) in &plan.conflicts {
        println!(
            "  {} {}",
            "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
            msg,
        );
    }

    // Discrepancies (--hash mode: serial and hash matched different games)
    for d in &plan.discrepancies {
        let file_name = d.file.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        println!(
            "  {} {} serial=\"{}\" hash=\"{}\"",
            "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
            format!("{file_name}: serial/hash mismatch").if_supports_color(Stdout, |t| t.yellow()),
            d.serial_game,
            d.hash_game,
        );
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
    root: Option<PathBuf>,
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
    options.limit = limit;

    if let Some(mdir) = metadata_dir {
        options.metadata_dir = mdir;
    }
    if let Some(mdir) = media_dir {
        options.media_dir = mdir;
    }
    if let Some(ref types) = media_types {
        options.media_selection = retro_junk_scraper::MediaSelection::from_names(types);
    }

    println!(
        "Scraping ROMs in: {}",
        root_path.display().if_supports_color(Stdout, |t| t.cyan()),
    );
    if dry_run {
        println!(
            "{}",
            "Dry run: no files will be downloaded".if_supports_color(Stdout, |t| t.dimmed()),
        );
    }
    if let Some(n) = limit {
        println!(
            "{}",
            format!("Limit: {} ROMs per console", n).if_supports_color(Stdout, |t| t.dimmed()),
        );
    }
    println!(
        "Metadata: {}",
        options.metadata_dir.display().if_supports_color(Stdout, |t| t.dimmed()),
    );
    println!(
        "Media:    {}",
        options.media_dir.display().if_supports_color(Stdout, |t| t.dimmed()),
    );
    println!();

    // Load credentials
    let creds = match retro_junk_scraper::Credentials::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "{} Failed to load ScreenScraper credentials: {}",
                "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                e,
            );
            eprintln!();
            eprintln!("Set credentials via environment variables:");
            eprintln!("  SCREENSCRAPER_DEVID, SCREENSCRAPER_DEVPASSWORD");
            eprintln!("  SCREENSCRAPER_SSID, SCREENSCRAPER_SSPASSWORD (optional)");
            eprintln!();
            eprintln!("Or create ~/.config/retro-junk/credentials.toml");
            return;
        }
    };

    // Create a tokio runtime for async operations
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");

    rt.block_on(async {
        // Validate credentials
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::with_template("  {spinner:.cyan} {msg}")
                .unwrap()
                .tick_chars("/-\\|"),
        );
        pb.set_message("Connecting to ScreenScraper...");
        pb.enable_steady_tick(std::time::Duration::from_millis(100));

        let (client, user_info) = match retro_junk_scraper::ScreenScraperClient::new(creds).await {
            Ok(result) => result,
            Err(e) => {
                pb.finish_and_clear();
                eprintln!(
                    "{} Failed to connect to ScreenScraper: {}",
                    "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                    e,
                );
                return;
            }
        };
        pb.finish_and_clear();

        println!(
            "{} Connected to ScreenScraper (requests today: {}/{}, max threads: {})",
            "\u{2714}".if_supports_color(Stdout, |t| t.green()),
            user_info.requests_today(),
            user_info.max_requests_per_day(),
            user_info.max_threads(),
        );
        println!();

        // Scan root directory for console folders
        let entries = match fs::read_dir(&root_path) {
            Ok(entries) => entries,
            Err(e) => {
                eprintln!(
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
                let short_name = platform.short_name();

                // Check if this system has a ScreenScraper ID
                if retro_junk_scraper::screenscraper_system_id(platform).is_none() {
                    eprintln!(
                        "  {} Skipping \"{}\" — no ScreenScraper system ID",
                        "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                        folder_name,
                    );
                    continue;
                }

                println!(
                    "{} {}",
                    console
                        .metadata
                        .platform_name
                        .if_supports_color(Stdout, |t| t.bold()),
                    format!("({})", folder_name).if_supports_color(Stdout, |t| t.dimmed()),
                );

                let pb = ProgressBar::new_spinner();
                pb.set_style(
                    ProgressStyle::with_template("  {spinner:.cyan} {msg}")
                        .unwrap()
                        .tick_chars("/-\\|"),
                );
                pb.enable_steady_tick(std::time::Duration::from_millis(100));

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

                        // Print per-system summary
                        if summary.total_success > 0 {
                            println!(
                                "  {} {} games scraped (serial: {}, filename: {}, hash: {})",
                                "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                                summary.total_success,
                                summary.by_serial,
                                summary.by_filename,
                                summary.by_hash,
                            );
                        }
                        if summary.media_downloaded > 0 {
                            println!(
                                "  {} {} media files downloaded",
                                "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                                summary.media_downloaded,
                            );
                        }
                        if summary.total_unidentified > 0 {
                            println!(
                                "  {} {} unidentified",
                                "?".if_supports_color(Stdout, |t| t.yellow()),
                                summary.total_unidentified,
                            );
                        }
                        if summary.total_errors > 0 {
                            println!(
                                "  {} {} errors",
                                "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                                summary.total_errors,
                            );
                        }

                        // Write metadata
                        if !result.games.is_empty() && !dry_run {
                            let system_metadata_dir = options.metadata_dir.join(short_name);
                            let system_media_dir = options.media_dir.join(short_name);

                            use retro_junk_frontend::Frontend;
                            if let Err(e) = esde.write_metadata(
                                &result.games,
                                &path,
                                &system_metadata_dir,
                                &system_media_dir,
                            ) {
                                eprintln!(
                                    "  {} Error writing metadata: {}",
                                    "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                                    e,
                                );
                            } else {
                                println!(
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
                                short_name,
                                chrono::Local::now().format("%Y%m%d-%H%M%S"),
                            ));
                            if let Err(e) = std::fs::create_dir_all(&options.metadata_dir) {
                                eprintln!("Warning: could not create metadata dir: {}", e);
                            } else if let Err(e) = result.log.write_to_file(&log_path) {
                                eprintln!("Warning: could not write scrape log: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        pb.finish_and_clear();
                        eprintln!(
                            "  {} Error: {}",
                            "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                            e,
                        );
                        total_errors += 1;
                    }
                }
                println!();
            }
        }

        // Print overall summary
        if total_games > 0 || total_errors > 0 || total_unidentified > 0 {
            println!("{}", "Summary:".if_supports_color(Stdout, |t| t.bold()));
            if total_games > 0 {
                println!(
                    "  {} {} games scraped, {} media files",
                    "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                    total_games,
                    total_media,
                );
            }
            if total_unidentified > 0 {
                println!(
                    "  {} {} unidentified",
                    "?".if_supports_color(Stdout, |t| t.yellow()),
                    total_unidentified,
                );
            }
            if total_errors > 0 {
                println!(
                    "  {} {} errors",
                    "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                    total_errors,
                );
            }

            // Show remaining quota
            if let Some(quota) = client.current_quota().await {
                println!(
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
    println!("Supported consoles:");
    println!();

    let mut current_manufacturer = "";

    for console in ctx.consoles() {
        if console.metadata.manufacturer != current_manufacturer {
            if !current_manufacturer.is_empty() {
                println!();
            }
            current_manufacturer = console.metadata.manufacturer;
            println!(
                "{}:",
                current_manufacturer.if_supports_color(Stdout, |t| t.bold()),
            );
        }

        let extensions = console.metadata.extensions.join(", ");
        let folders = console.metadata.folder_names.join(", ");
        let has_dat = console.analyzer.dat_name().is_some();

        println!(
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
        println!("    Extensions: {}", extensions);
        println!("    Folder names: {}", folders);
    }
}

/// List cached DAT files.
fn run_cache_list() {
    match retro_junk_dat::cache::list() {
        Ok(entries) => {
            if entries.is_empty() {
                println!(
                    "{}",
                    "No cached DAT files.".if_supports_color(Stdout, |t| t.dimmed()),
                );
                println!("Run 'retro-junk cache fetch <system>' to download DAT files.");
                return;
            }

            println!(
                "{}",
                "Cached DAT files:".if_supports_color(Stdout, |t| t.bold()),
            );
            println!();

            let mut total_size = 0u64;
            for entry in &entries {
                total_size += entry.file_size;
                println!(
                    "  {} [{}]",
                    entry.short_name.if_supports_color(Stdout, |t| t.bold()),
                    entry.dat_name.if_supports_color(Stdout, |t| t.cyan()),
                );
                println!(
                    "    Size: {}, Downloaded: {}, Version: {}",
                    format_bytes(entry.file_size),
                    entry.downloaded,
                    entry.dat_version,
                );
            }
            println!();
            println!(
                "Total: {} files, {}",
                entries.len(),
                format_bytes(total_size)
            );
        }
        Err(e) => {
            eprintln!(
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
            println!(
                "{} Cache cleared ({} freed)",
                "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                format_bytes(freed),
            );
        }
        Err(e) => {
            eprintln!(
                "{} Error clearing cache: {}",
                "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                e,
            );
        }
    }
}

/// Fetch DAT files for specified systems.
fn run_cache_fetch(ctx: &AnalysisContext, systems: Vec<String>) {
    let to_fetch: Vec<(String, String)> =
        if systems.len() == 1 && systems[0].eq_ignore_ascii_case("all") {
            ctx.consoles()
                .filter_map(|c| {
                    c.analyzer
                        .dat_name()
                        .map(|dat_name| (c.metadata.short_name.to_string(), dat_name.to_string()))
                })
                .collect()
        } else {
            systems
                .into_iter()
                .filter_map(|short_name| {
                    let console = ctx.get_by_short_name(&short_name);
                    match console {
                        Some(c) => match c.analyzer.dat_name() {
                            Some(dat_name) => Some((short_name, dat_name.to_string())),
                            None => {
                                eprintln!(
                                    "  {} No DAT support for '{}'",
                                    "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                                    short_name,
                                );
                                None
                            }
                        },
                        None => {
                            eprintln!(
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

    for (short_name, dat_name) in &to_fetch {
        print!(
            "  Fetching {}... ",
            short_name.if_supports_color(Stdout, |t| t.bold()),
        );
        std::io::stdout().flush().unwrap();

        match retro_junk_dat::cache::fetch(short_name, dat_name) {
            Ok(path) => {
                let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                println!(
                    "{} ({})",
                    "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                    format_bytes(size),
                );
            }
            Err(e) => {
                println!(
                    "{} {}",
                    "\u{2718}".if_supports_color(Stdout, |t| t.red()),
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

    println!(
        "{}",
        "ScreenScraper Configuration".if_supports_color(Stdout, |t| t.bold()),
    );
    println!();

    // Config file status
    match &path {
        Some(p) if p.exists() => {
            println!(
                "  Config file: {} {}",
                p.display().if_supports_color(Stdout, |t| t.cyan()),
                "(exists)".if_supports_color(Stdout, |t| t.green()),
            );
        }
        Some(p) => {
            println!(
                "  Config file: {} {}",
                p.display().if_supports_color(Stdout, |t| t.cyan()),
                "(not found)".if_supports_color(Stdout, |t| t.dimmed()),
            );
        }
        None => {
            println!(
                "  Config file: {}",
                "could not determine path".if_supports_color(Stdout, |t| t.red()),
            );
        }
    }
    println!();

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
                println!(
                    "  {} {} {}",
                    format!("{}:", name).if_supports_color(Stdout, |t| t.cyan()),
                    v,
                    source_str.if_supports_color(Stdout, |t| t.dimmed()),
                );
            }
            None => {
                println!(
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
fn run_config_test() {
    let creds = match retro_junk_scraper::Credentials::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "{} Failed to load credentials: {}",
                "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                e,
            );
            eprintln!();
            eprintln!("Run 'retro-junk config setup' to configure credentials.");
            return;
        }
    };

    println!("Testing credentials against ScreenScraper API...");

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");

    rt.block_on(async {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::with_template("  {spinner:.cyan} {msg}")
                .unwrap()
                .tick_chars("/-\\|"),
        );
        pb.set_message("Connecting...");
        pb.enable_steady_tick(std::time::Duration::from_millis(100));

        match retro_junk_scraper::ScreenScraperClient::new(creds).await {
            Ok((_client, user_info)) => {
                pb.finish_and_clear();
                println!(
                    "{} Credentials are valid!",
                    "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                );
                println!();
                println!(
                    "  Requests today: {}/{}",
                    user_info.requests_today(),
                    user_info.max_requests_per_day(),
                );
                println!("  Max threads:    {}", user_info.max_threads());
            }
            Err(e) => {
                pb.finish_and_clear();
                eprintln!(
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
        Some(path) => println!("{}", path.display()),
        None => {
            eprintln!("Could not determine config directory");
            std::process::exit(1);
        }
    }
}
