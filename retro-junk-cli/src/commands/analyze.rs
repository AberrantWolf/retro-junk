use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use log::Level;
use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;

use retro_junk_lib::{AnalysisContext, AnalysisOptions, Platform, RomAnalyzer, RomIdentification};

use crate::CliError;
use crate::scan_folders;

/// Run the analyze command.
pub(crate) fn run_analyze(
    ctx: &AnalysisContext,
    quick: bool,
    consoles: Option<Vec<Platform>>,
    limit: Option<usize>,
    root: Option<PathBuf>,
) -> Result<(), CliError> {
    let root_path =
        root.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    log::info!("Analyzing ROMs in: {}", root_path.display());
    if quick {
        log::info!("Quick mode enabled");
    }
    if let Some(n) = limit {
        log::info!("Limit: {} games per console", n);
    }
    crate::log_blank();

    let options = AnalysisOptions::new().quick(quick);

    let scan = match scan_folders(ctx, &root_path, &consoles) {
        Some(s) => s,
        None => return Ok(()),
    };

    for cf in &scan.matches {
        let console = ctx.get_by_platform(cf.platform).ok_or_else(|| {
            CliError::unknown_system(format!("No analyzer for platform {:?}", cf.platform))
        })?;
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
        crate::log_blank();
        log::info!("Tip: Create folders named after consoles (e.g., 'snes', 'n64', 'ps1')");
        log::info!("     and place your ROM files inside them.");
        crate::log_blank();
        log::info!("Run 'retro-junk list' to see all supported console names.");
    }

    Ok(())
}

/// Analyze all ROM files in a folder.
fn analyze_folder(
    folder: &Path,
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
    crate::log_blank();
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
                let effective_level = if i == 0 && has_warnings {
                    Level::Warn
                } else {
                    *level
                };
                log::log!(effective_level, "{}", msg);
            }
        }
        Err(e) => {
            log::warn!(
                "  {}{}: {} Analysis failed ({})",
                indent,
                file_name,
                "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                e,
            );
        }
    }
}

/// Format a byte size as a human-readable string.
pub(crate) fn format_bytes(bytes: u64) -> String {
    retro_junk_lib::util::format_bytes(bytes)
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
fn format_analysis(
    file_name: &str,
    info: &RomIdentification,
    indent: &str,
) -> Vec<(Level, String)> {
    let mut lines: Vec<(Level, String)> = Vec::new();
    let mut shown_keys: HashSet<&str> = HashSet::new();

    // Header line (caller may promote to Warn if other lines have warnings)
    lines.push((
        Level::Info,
        format!(
            "  {}{}:",
            indent,
            file_name.if_supports_color(Stdout, |t| t.bold()),
        ),
    ));

    // (a) Identity fields
    if let Some(ref serial) = info.serial_number {
        lines.push((
            Level::Info,
            format!(
                "    {}{}   {}",
                indent,
                "Serial:".if_supports_color(Stdout, |t| t.cyan()),
                serial,
            ),
        ));
    }
    if let Some(ref name) = info.internal_name {
        lines.push((
            Level::Info,
            format!(
                "    {}{}     {}",
                indent,
                "Name:".if_supports_color(Stdout, |t| t.cyan()),
                name,
            ),
        ));
    }
    if let Some(ref maker) = info.maker_code {
        lines.push((
            Level::Info,
            format!(
                "    {}{}    {}",
                indent,
                "Maker:".if_supports_color(Stdout, |t| t.cyan()),
                maker,
            ),
        ));
    }
    if let Some(ref version) = info.version {
        lines.push((
            Level::Info,
            format!(
                "    {}{}  {}",
                indent,
                "Version:".if_supports_color(Stdout, |t| t.cyan()),
                version,
            ),
        ));
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
        lines.push((
            Level::Info,
            format!(
                "    {}{}",
                indent,
                "Hardware:".if_supports_color(Stdout, |t| t.bright_magenta()),
            ),
        ));
        for key in &hardware_present {
            shown_keys.insert(key);
            let value = &info.extra[*key];
            lines.push((
                Level::Info,
                format!(
                    "      {}{} {}",
                    indent,
                    format!("{}:", prettify_key(key)).if_supports_color(Stdout, |t| t.cyan()),
                    value,
                ),
            ));
        }
    }

    // (d) Size verdict
    match (info.file_size, info.expected_size) {
        (Some(actual), Some(expected)) => {
            let verdict = compute_size_verdict(actual, expected);
            let level = if matches!(verdict, SizeVerdict::Ok) {
                Level::Info
            } else {
                Level::Warn
            };
            lines.push((
                level,
                format!(
                    "    {}{}     {} on disk, {} expected [{}]",
                    indent,
                    "Size:".if_supports_color(Stdout, |t| t.cyan()),
                    format_bytes(actual),
                    format_bytes(expected),
                    print_size_verdict(&verdict),
                ),
            ));
        }
        (Some(actual), None) => {
            lines.push((
                Level::Info,
                format!(
                    "    {}{}     {}",
                    indent,
                    "Size:".if_supports_color(Stdout, |t| t.cyan()),
                    format_bytes(actual),
                ),
            ));
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
            lines.push((
                level,
                format!(
                    "    {}{} {}  {}",
                    indent,
                    "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                    format!("{}:", name).if_supports_color(Stdout, |t| t.cyan()),
                    colored_status,
                ),
            ));
        } else {
            let colored_status = format!("{}", status.if_supports_color(Stdout, |t| t.red()));
            lines.push((
                level,
                format!(
                    "    {}{} {}  {}",
                    indent,
                    "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                    format!("{}:", name).if_supports_color(Stdout, |t| t.cyan()),
                    colored_status,
                ),
            ));
        }
    }

    // (f) Region
    if !info.regions.is_empty() {
        let region_str: Vec<_> = info.regions.iter().map(|r| r.name()).collect();
        lines.push((
            Level::Info,
            format!(
                "    {}{}   {}",
                indent,
                "Region:".if_supports_color(Stdout, |t| t.cyan()),
                region_str.join(", "),
            ),
        ));
    }

    // (g) Remaining extras
    let mut remaining: Vec<_> = info
        .extra
        .keys()
        .filter(|k| !shown_keys.contains(k.as_str()))
        .collect();
    remaining.sort();

    if !remaining.is_empty() {
        lines.push((
            Level::Info,
            format!(
                "    {}{}",
                indent,
                "Details:".if_supports_color(Stdout, |t| t.bright_magenta()),
            ),
        ));
        for key in &remaining {
            let value = &info.extra[key.as_str()];
            lines.push((
                Level::Info,
                format!(
                    "      {}{} {}",
                    indent,
                    format!("{}:", prettify_key(key)).if_supports_color(Stdout, |t| t.cyan()),
                    value,
                ),
            ));
        }
    }

    lines
}
