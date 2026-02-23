use std::path::PathBuf;

use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;

use super::default_catalog_db_path;

pub(crate) fn run_catalog_stats(db_path: Option<PathBuf>) {
    let db_path = db_path.unwrap_or_else(default_catalog_db_path);

    if !db_path.exists() {
        log::warn!("No catalog database found at {}", db_path.display());
        log::info!("Run 'retro-junk catalog import all' to create one.");
        return;
    }

    let conn = match retro_junk_db::open_database(&db_path) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to open catalog database: {}", e);
            std::process::exit(1);
        }
    };

    match retro_junk_db::catalog_stats(&conn) {
        Ok(stats) => {
            log::info!(
                "{}",
                "Catalog Database Statistics".if_supports_color(Stdout, |t| t.bold()),
            );
            log::info!("  Database: {}", db_path.display());
            log::info!("");
            log::info!("  Platforms:      {:>8}", stats.platforms);
            log::info!("  Companies:      {:>8}", stats.companies);
            log::info!("  Works:          {:>8}", stats.works);
            log::info!("  Releases:       {:>8}", stats.releases);
            log::info!("  Media entries:  {:>8}", stats.media);
            log::info!("  Assets:         {:>8}", stats.assets);
            log::info!("  Owned (coll.):  {:>8}", stats.collection_owned);
            log::info!(
                "  Disagreements:  {:>8} (unresolved)",
                stats.unresolved_disagreements,
            );
        }
        Err(e) => {
            log::error!("Failed to query catalog stats: {}", e);
            std::process::exit(1);
        }
    }
}
