use std::path::PathBuf;

use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;
use retro_junk_lib::{AnalysisContext, Platform};

use crate::CliError;

use super::default_catalog_db_path;

/// Enrich catalog releases with GameDataBase metadata.
pub(crate) fn run_catalog_enrich_gdb(
    ctx: &AnalysisContext,
    systems: Vec<String>,
    db_path: Option<PathBuf>,
    limit: Option<u32>,
    gdb_dir: Option<PathBuf>,
) -> Result<(), CliError> {
    use retro_junk_import::gdb_import::{self, GdbEnrichOptions};

    let db_path = db_path.unwrap_or_else(default_catalog_db_path);

    if !db_path.exists() {
        log::warn!("No catalog database found at {}", db_path.display());
        log::info!("Run 'retro-junk catalog import all' first.");
        return Ok(());
    }

    let conn = retro_junk_db::open_database(&db_path)
        .map_err(|e| CliError::database(format!("Failed to open catalog database: {}", e)))?;

    // Resolve systems
    let consoles: Vec<(String, &'static [&'static str])> =
        if systems.len() == 1 && systems[0].eq_ignore_ascii_case("all") {
            ctx.consoles()
                .filter(|c| c.analyzer.has_gdb_support())
                .map(|c| {
                    (
                        c.metadata.short_name.to_string(),
                        c.analyzer.gdb_csv_names(),
                    )
                })
                .collect()
        } else {
            let mut result = Vec::new();
            for s in &systems {
                let p: Platform = s.parse().map_err(|_| {
                    CliError::unknown_system(format!(
                        "Unknown system '{}'. Use a short name like 'nes', 'snes', 'n64'.",
                        s
                    ))
                })?;
                if let Some(console) = ctx.get_by_short_name(p.short_name()) {
                    let csv_names = console.analyzer.gdb_csv_names();
                    if csv_names.is_empty() {
                        log::warn!(
                            "  {} No GDB support for '{}'",
                            "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                            s,
                        );
                    } else {
                        result.push((p.short_name().to_string(), csv_names));
                    }
                }
            }
            result
        };

    if consoles.is_empty() {
        log::warn!("No systems with GDB support specified.");
        return Ok(());
    }

    let mut total_enriched = 0u32;

    for (short_name, csv_names) in &consoles {
        log::info!(
            "\n{} {}",
            "Enriching".if_supports_color(Stdout, |t| t.bold()),
            short_name.if_supports_color(Stdout, |t| t.cyan()),
        );

        let options = GdbEnrichOptions {
            platform_id: short_name.clone(),
            limit,
            gdb_dir: gdb_dir.clone(),
        };

        match gdb_import::enrich_gdb(&conn, csv_names, &options) {
            Ok(stats) => {
                log::info!(
                    "  {} {}: {}/{} matched, {} enriched, {} disagreements",
                    "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                    short_name.if_supports_color(Stdout, |t| t.bold()),
                    stats.matched,
                    stats.media_checked,
                    stats.enriched,
                    stats.disagreements,
                );
                if stats.companies_created > 0 {
                    log::info!("    {} new companies created", stats.companies_created,);
                }
                if stats.skipped_no_hash > 0 {
                    log::info!(
                        "    {} media entries skipped (no SHA1)",
                        stats.skipped_no_hash,
                    );
                }
                total_enriched += stats.enriched;
            }
            Err(e) => {
                log::error!(
                    "  {} {}: {}",
                    "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                    short_name,
                    e,
                );
            }
        }
    }

    log::info!(
        "\n{} Total enriched: {}",
        "Done.".if_supports_color(Stdout, |t| t.bold()),
        total_enriched,
    );

    Ok(())
}
