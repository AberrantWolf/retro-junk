//! retro-junk CLI
//!
//! Command-line interface for analyzing retro game ROMs and disc images.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;

use retro_junk_lib::{AnalysisContext, AnalysisOptions, RomAnalyzer, RomIdentification};

#[derive(Parser)]
#[command(name = "retro-junk")]
#[command(about = "Analyze retro game ROMs and disc images", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Analyze ROMs in a directory structure
    Analyze {
        /// Quick mode: read as little data as possible (useful for network shares)
        #[arg(short, long)]
        quick: bool,

        /// Console short names to analyze (e.g., snes, n64, ps1)
        /// If not specified, all matching console folders will be analyzed
        #[arg(short, long, value_delimiter = ',')]
        consoles: Option<Vec<String>>,

        /// Root path containing console folders (defaults to current directory)
        #[arg(short, long)]
        root: Option<PathBuf>,
    },

    /// List all supported consoles
    List,
}

fn main() {
    let cli = Cli::parse();
    let ctx = create_context();

    match cli.command {
        Commands::Analyze {
            quick,
            consoles,
            root,
        } => {
            run_analyze(&ctx, quick, consoles, root);
        }
        Commands::List => {
            run_list(&ctx);
        }
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
    consoles: Option<Vec<String>>,
    root: Option<PathBuf>,
) {
    let root_path = root.unwrap_or_else(|| std::env::current_dir().expect("Failed to get current directory"));

    println!("Analyzing ROMs in: {}", root_path.display());
    if quick {
        println!("Quick mode enabled");
    }
    println!();

    let options = AnalysisOptions::new().quick(quick);

    // If specific consoles were requested, validate them
    if let Some(ref console_list) = consoles {
        for name in console_list {
            if ctx.get_by_short_name(name).is_none() {
                eprintln!(
                    "  {} Unknown console '{}', skipping",
                    "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                    name,
                );
            }
        }
    }

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
                .filter(|c| {
                    filter
                        .iter()
                        .any(|f| f.eq_ignore_ascii_case(c.metadata.short_name))
                })
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
            analyze_folder(&path, console.analyzer.as_ref(), &options);
        }
    }

    if !found_any {
        println!(
            "{}",
            format!("No matching console folders found in {}", root_path.display())
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
fn analyze_folder(folder: &PathBuf, analyzer: &dyn RomAnalyzer, options: &AnalysisOptions) {
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

    let mut file_count = 0;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        // Check extension
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        if !extensions.contains(&ext) {
            continue;
        }

        file_count += 1;
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");

        // Try to analyze the file
        let mut file = match fs::File::open(&path) {
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

    if file_count == 0 {
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

    println!(
        "  {}:",
        file_name.if_supports_color(Stdout, |t| t.bold()),
    );

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
                format!("{}:", prettify_key(key))
                    .if_supports_color(Stdout, |t| t.cyan()),
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
                format!("{}:", prettify_key(key))
                    .if_supports_color(Stdout, |t| t.cyan()),
                value,
            );
        }
    }
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
        println!(
            "  {} [{}]",
            console
                .metadata
                .short_name
                .if_supports_color(Stdout, |t| t.bold()),
            console
                .metadata
                .platform_name
                .if_supports_color(Stdout, |t| t.cyan()),
        );
        println!("    Extensions: {}", extensions);
        println!("    Folder names: {}", folders);
    }
}
