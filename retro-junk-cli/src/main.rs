//! retro-junk CLI
//!
//! Command-line interface for analyzing retro game ROMs and disc images.

use std::fs;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

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
                eprintln!("Warning: Unknown console '{}', skipping", name);
            }
        }
    }

    // Read the root directory
    let entries = match fs::read_dir(&root_path) {
        Ok(entries) => entries,
        Err(e) => {
            eprintln!("Error reading directory: {}", e);
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
                "Found {} folder: {}",
                console.metadata.platform_name, folder_name
            );

            // Scan for ROM files in the folder
            analyze_folder(&path, console.analyzer.as_ref(), &options);
        }
    }

    if !found_any {
        println!("No matching console folders found in {}", root_path.display());
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
            eprintln!("  Error reading folder: {}", e);
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
                eprintln!("  Error opening {}: {}", file_name, e);
                continue;
            }
        };

        match analyzer.analyze(&mut file, options) {
            Ok(info) => {
                print_analysis(file_name, &info);
            }
            Err(e) => {
                println!("  {}: Analysis not implemented ({})", file_name, e);
            }
        }
    }

    if file_count == 0 {
        println!("  No ROM files found");
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

/// Describe the relationship between actual and expected file size.
fn size_verdict(file_size: u64, expected_size: u64) -> String {
    if file_size == expected_size {
        "OK".into()
    } else if file_size < expected_size {
        let missing = expected_size - file_size;
        format!("TRUNCATED (missing {})", format_bytes(missing))
    } else {
        let excess = file_size - expected_size;
        format!("OVERSIZED (+{})", format_bytes(excess))
    }
}

/// Print the analysis result for a single file.
fn print_analysis(file_name: &str, info: &RomIdentification) {
    println!("  {}:", file_name);

    // Identity fields
    if let Some(ref serial) = info.serial_number {
        println!("    Serial:   {}", serial);
    }
    if let Some(ref name) = info.internal_name {
        println!("    Name:     {}", name);
    }
    if let Some(ref maker) = info.maker_code {
        println!("    Maker:    {}", maker);
    }
    if let Some(ref version) = info.version {
        println!("    Version:  {}", version);
    }

    // Format (from extra, if present)
    if let Some(format) = info.extra.get("format") {
        print!("    Format:   {}", format);
        if let Some(mapper) = info.extra.get("mapper") {
            print!(", Mapper {}", mapper);
            if let Some(mapper_name) = info.extra.get("mapper_name") {
                print!(" ({})", mapper_name);
            }
        }
        println!();
    }

    // Size information
    match (info.file_size, info.expected_size) {
        (Some(actual), Some(expected)) => {
            let verdict = size_verdict(actual, expected);
            println!(
                "    Size:     {} on disk, {} expected [{}]",
                format_bytes(actual),
                format_bytes(expected),
                verdict,
            );
        }
        (Some(actual), None) => {
            println!("    Size:     {}", format_bytes(actual));
        }
        _ => {}
    }

    // Regions
    if !info.regions.is_empty() {
        let region_str: Vec<_> = info.regions.iter().map(|r| r.name()).collect();
        println!("    Region:   {}", region_str.join(", "));
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
            println!("{}:", current_manufacturer);
        }

        let extensions = console.metadata.extensions.join(", ");
        let folders = console.metadata.folder_names.join(", ");
        println!(
            "  {} [{}]",
            console.metadata.short_name, console.metadata.platform_name
        );
        println!("    Extensions: {}", extensions);
        println!("    Folder names: {}", folders);
    }
}
