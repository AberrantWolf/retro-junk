use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;

use crate::CliError;
use crate::cli_types::CatalogEnrichArgs;
use crate::commands::scrape::connect_screenscraper;
use crate::commands::systems::resolve_platform_ids;

use super::default_catalog_db_path;

/// Enrich catalog releases with `ScreenScraper` metadata.
// Linear enrich orchestration: options setup, async event-rendering loop, summary.
#[allow(clippy::too_many_lines)]
pub(crate) fn run_catalog_enrich(args: CatalogEnrichArgs, quiet: bool) -> Result<(), CliError> {
    use retro_junk_import::scraper_import::{self, EnrichEvent, EnrichOptions};

    let CatalogEnrichArgs {
        systems,
        db: db_path,
        limit,
        force,
        download_assets,
        asset_dir,
        region,
        language,
        threads,
        no_reconcile,
    } = args;

    let db_path = db_path.unwrap_or_else(default_catalog_db_path);

    if !db_path.exists() {
        log::warn!("No catalog database found at {}", db_path.display());
        log::info!("Run 'retro-junk catalog import all' first.");
        return Ok(());
    }

    let conn = retro_junk_db::open_database(&db_path)
        .map_err(|e| CliError::database(format!("Failed to open catalog database: {e}")))?;

    let platform_ids = resolve_platform_ids(&conn, &systems)?;

    let reconcile_platform_ids = platform_ids.clone();

    let options = EnrichOptions {
        platform_ids,
        limit,
        skip_existing: !force,
        download_assets,
        asset_dir,
        preferred_region: region,
        preferred_language: language,
    };

    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| CliError::runtime(format!("Failed to create tokio runtime: {e}")))?;

    rt.block_on(async {
        let (client, max_workers) = connect_screenscraper(threads, quiet).await?;

        let (event_tx, event_rx) = tokio::sync::mpsc::channel::<EnrichEvent>(1024);

        let enrich_future =
            scraper_import::enrich_releases(client, &conn, &options, max_workers, event_tx);

        let enrich_result =
            retro_junk_lib::async_util::run_with_events(enrich_future, event_rx, |e| {
                if quiet {
                    return;
                }
                match e {
                    EnrichEvent::PlatformStarted {
                        platform_name,
                        total,
                        ..
                    } => {
                        log::info!(
                            "Enriching {} releases for {}",
                            total,
                            platform_name.if_supports_color(Stdout, |t| t.bold()),
                        );
                    }
                    EnrichEvent::ReleaseFound {
                        ref title,
                        ref ss_name,
                        ref method,
                        ..
                    } => {
                        log::info!(
                            "  {} {} (via {}, SS: \"{}\")",
                            "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                            title.if_supports_color(Stdout, |t| t.bold()),
                            method,
                            ss_name,
                        );
                    }
                    EnrichEvent::ReleaseNotFound { ref title, .. } => {
                        log::info!(
                            "  {} {}",
                            "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                            title,
                        );
                    }
                    EnrichEvent::ReleaseError {
                        ref title,
                        ref error,
                        ..
                    } => {
                        log::info!(
                            "  {} {}: {}",
                            "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                            title,
                            error,
                        );
                    }
                    EnrichEvent::FatalError { ref message } => {
                        log::info!(
                            "  {} Fatal: {}",
                            "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                            message,
                        );
                    }
                    EnrichEvent::Done { ref stats } => {
                        crate::log_blank();
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
                    _ => {} // PlatformDone, ReleaseSkipped — no output
                }
            })
            .await;

        enrich_result.map_err(|e| CliError::other(format!("Enrichment failed: {e}")))?;

        Ok::<(), CliError>(())
    })?;

    // Auto-reconcile after enrichment
    if !no_reconcile {
        super::reconcile::run_reconcile_on_conn(&conn, &reconcile_platform_ids, false)?;
    }

    Ok(())
}
