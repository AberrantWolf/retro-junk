use std::path::PathBuf;

use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;
use retro_junk_lib::{AnalysisContext, Platform};

use super::default_catalog_db_path;

/// Enrich catalog releases with GameDataBase metadata.
pub(crate) fn run_catalog_enrich_gdb(
    ctx: &AnalysisContext,
    systems: Vec<String>,
    db_path: Option<PathBuf>,
    limit: Option<u32>,
    gdb_dir: Option<PathBuf>,
) {
    use retro_junk_import::gdb_import::{self, GdbEnrichOptions};

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
            systems
                .iter()
                .filter_map(|s| {
                    let p: Platform = match s.parse() {
                        Ok(p) => p,
                        Err(_) => {
                            log::error!(
                                "Unknown system '{}'. Use a short name like 'nes', 'snes', 'n64'.",
                                s
                            );
                            std::process::exit(1);
                        }
                    };
                    let console = ctx.get_by_short_name(p.short_name())?;
                    let csv_names = console.analyzer.gdb_csv_names();
                    if csv_names.is_empty() {
                        log::warn!(
                            "  {} No GDB support for '{}'",
                            "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                            s,
                        );
                        None
                    } else {
                        Some((p.short_name().to_string(), csv_names))
                    }
                })
                .collect()
        };

    if consoles.is_empty() {
        log::warn!("No systems with GDB support specified.");
        return;
    }

    let mut total_enriched = 0u32;

    for (short_name, csv_names) in &consoles {
        println!(
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
                println!(
                    "  {} {}: {}/{} matched, {} enriched, {} disagreements",
                    "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                    short_name.if_supports_color(Stdout, |t| t.bold()),
                    stats.matched,
                    stats.media_checked,
                    stats.enriched,
                    stats.disagreements,
                );
                if stats.companies_created > 0 {
                    println!("    {} new companies created", stats.companies_created,);
                }
                if stats.skipped_no_hash > 0 {
                    println!(
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

    println!(
        "\n{} Total enriched: {}",
        "Done.".if_supports_color(Stdout, |t| t.bold()),
        total_enriched,
    );
}
