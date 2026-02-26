use std::path::PathBuf;

use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;

use retro_junk_lib::AnalysisContext;

use super::default_catalog_db_path;

/// Re-verify collection entries against files on disk.
pub(crate) fn run_catalog_verify(
    ctx: &AnalysisContext,
    system: String,
    db_path: Option<PathBuf>,
    user_id: String,
    _quiet: bool,
) {
    let db_path = db_path.unwrap_or_else(default_catalog_db_path);

    if !db_path.exists() {
        log::warn!("No catalog database found at {}", db_path.display());
        log::info!("Run 'retro-junk catalog import all' first.");
        return;
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

    log::info!(
        "{}",
        format!(
            "Verifying {} collection entries",
            console.metadata.short_name
        )
        .if_supports_color(Stdout, |t| t.bold()),
    );

    match retro_junk_import::verify_collection(
        &conn,
        console.analyzer.as_ref(),
        console.metadata.platform,
        &user_id,
    ) {
        Ok(stats) => {
            log::info!("");
            log::info!(
                "{}",
                "Verification complete".if_supports_color(Stdout, |t| t.bold()),
            );
            log::info!("  Checked:        {:>6}", stats.checked);
            log::info!("  Verified:       {:>6}", stats.verified);
            log::info!("  Missing:        {:>6}", stats.missing);
            log::info!("  Hash mismatch:  {:>6}", stats.hash_mismatch);
            log::info!("  No path:        {:>6}", stats.no_path);
            if stats.errors > 0 {
                log::info!("  Errors:         {:>6}", stats.errors);
            }
        }
        Err(e) => {
            log::error!("Verification failed: {}", e);
            std::process::exit(1);
        }
    }
}
