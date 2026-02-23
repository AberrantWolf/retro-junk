use std::path::PathBuf;

use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;

use super::default_catalog_db_path;

/// Analyze media asset coverage gaps.
pub(crate) fn run_catalog_gaps(
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
