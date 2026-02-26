use std::path::PathBuf;

use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;

use retro_junk_lib::AnalysisContext;

use super::default_catalog_db_path;

/// Scan a ROM folder and add matched files to the collection.
pub(crate) fn run_catalog_scan(
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
            log::error!(
                "Unknown system '{}'. Use a short name like 'nes', 'snes', 'n64'.",
                system
            );
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

    let options = ScanOptions { user_id };

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
                        f.path
                            .file_name()
                            .map(|n| n.to_string_lossy())
                            .unwrap_or_default(),
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
