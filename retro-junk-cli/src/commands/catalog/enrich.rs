use std::path::PathBuf;

use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;

use crate::commands::scrape::connect_screenscraper;
use crate::spinner;

use super::default_catalog_db_path;

/// Enrich catalog releases with ScreenScraper metadata.
pub(crate) fn run_catalog_enrich(
    systems: Vec<String>,
    db_path: Option<PathBuf>,
    limit: Option<u32>,
    force: bool,
    download_assets: bool,
    asset_dir: Option<PathBuf>,
    region: String,
    language: String,
    force_hash: bool,
    threads: Option<usize>,
    quiet: bool,
) {
    use retro_junk_import::scraper_import::{self, EnrichEvent, EnrichOptions};

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

    // Resolve "all" to all platforms with core_platform set
    let platform_ids = if systems.len() == 1 && systems[0].eq_ignore_ascii_case("all") {
        match retro_junk_db::list_platforms(&conn) {
            Ok(platforms) => platforms
                .into_iter()
                .filter(|p| p.core_platform.is_some())
                .map(|p| p.id)
                .collect(),
            Err(e) => {
                log::error!("Failed to list platforms: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        systems
    };

    if platform_ids.is_empty() {
        log::warn!("No systems specified.");
        return;
    }

    let options = EnrichOptions {
        platform_ids,
        limit,
        skip_existing: !force,
        download_assets,
        asset_dir,
        preferred_region: region,
        preferred_language: language,
        force_hash,
    };

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");

    rt.block_on(async {
        let (client, max_workers) = match connect_screenscraper(threads, quiet).await {
            Some(r) => r,
            None => std::process::exit(1),
        };

        let mut pool = spinner::SpinnerPool::new(max_workers, quiet, false);
        let mut platform_total = 0usize;

        let (event_tx, event_rx) =
            tokio::sync::mpsc::unbounded_channel::<EnrichEvent>();

        let enrich_future = scraper_import::enrich_releases(
            client, &conn, &options, max_workers, event_tx,
        );

        let enrich_result = retro_junk_lib::async_util::run_with_events(
            enrich_future,
            event_rx,
            |e| match e {
                EnrichEvent::PlatformStarted { platform_name, total, .. } => {
                    pool.clear_all();
                    platform_total = total;
                    pool.println(&format!(
                        "Enriching {} releases for {}",
                        total,
                        platform_name.if_supports_color(Stdout, |t| t.bold()),
                    ));
                }
                EnrichEvent::ReleaseStarted { index, total, ref title } => {
                    pool.claim(index, format!("[{}/{}] {}", index + 1, total, title));
                }
                EnrichEvent::ReleaseLookingUp { index, ref title } => {
                    pool.update(index, format!("[{}/{}] Looking up {}", index + 1, platform_total, title));
                }
                EnrichEvent::ReleaseDownloadingAsset { index, ref title, ref asset_type } => {
                    pool.update(index, format!("[{}/{}] Downloading {} for {}", index + 1, platform_total, asset_type, title));
                }
                EnrichEvent::ReleaseFound { index, ref title, ref ss_name, ref method } => {
                    pool.release_with_message(index, &format!(
                        "  {} {} (via {}, SS: \"{}\")",
                        "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                        title.if_supports_color(Stdout, |t| t.bold()),
                        method,
                        ss_name,
                    ));
                }
                EnrichEvent::ReleaseNotFound { index, ref title } => {
                    pool.release_with_message(index, &format!(
                        "  {} {}",
                        "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                        title,
                    ));
                }
                EnrichEvent::ReleaseSkipped { index } => {
                    pool.release(index);
                }
                EnrichEvent::ReleaseError { index, ref title, ref error } => {
                    pool.release_with_message(index, &format!(
                        "  {} {}: {}",
                        "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                        title,
                        error,
                    ));
                }
                EnrichEvent::FatalError { ref message } => {
                    pool.clear_all();
                    pool.println(&format!(
                        "  {} Fatal: {}",
                        "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                        message,
                    ));
                }
                EnrichEvent::PlatformDone { .. } => {
                    pool.clear_all();
                }
                EnrichEvent::Done { ref stats } => {
                    pool.clear_all();
                    pool.println("");
                    pool.println(&format!(
                        "{}",
                        "Enrichment complete".if_supports_color(Stdout, |t| t.bold()),
                    ));
                    pool.println(&format!("  Processed:     {:>6}", stats.releases_processed));
                    pool.println(&format!("  Enriched:      {:>6}", stats.releases_enriched));
                    pool.println(&format!("  Not found:     {:>6}", stats.releases_not_found));
                    pool.println(&format!("  Skipped:       {:>6}", stats.releases_skipped));
                    pool.println(&format!("  Assets:        {:>6}", stats.assets_downloaded));
                    pool.println(&format!("  Companies:     {:>6} (new)", stats.companies_created));
                    pool.println(&format!("  Disagreements: {:>6}", stats.disagreements_found));
                    if stats.errors > 0 {
                        pool.println(&format!("  Errors:        {:>6}", stats.errors));
                    }
                }
            },
        ).await;

        pool.clear_all();

        match enrich_result {
            Ok(_) => {}
            Err(e) => {
                log::error!("Enrichment failed: {}", e);
                std::process::exit(1);
            }
        }
    });
}
