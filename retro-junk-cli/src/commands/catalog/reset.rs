use std::path::PathBuf;

use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;

use crate::CliError;

use super::default_catalog_db_path;

/// Delete and recreate the catalog database.
pub(crate) fn run_catalog_reset(db_path: Option<PathBuf>, confirm: bool) -> Result<(), CliError> {
    let db_path = db_path.unwrap_or_else(default_catalog_db_path);

    if !confirm {
        log::warn!(
            "This will permanently delete the catalog database at:\n  {}",
            db_path.display(),
        );
        log::info!("Re-run with --confirm to proceed:");
        log::info!("  retro-junk catalog reset --confirm");
        return Ok(());
    }

    if !db_path.exists() {
        log::info!("No catalog database found at {}", db_path.display());
        log::info!("Nothing to reset.");
        return Ok(());
    }

    let file_size = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);

    std::fs::remove_file(&db_path)
        .map_err(|e| CliError::other(format!("Failed to delete {}: {}", db_path.display(), e)))?;

    let size_mb = file_size as f64 / (1024.0 * 1024.0);
    log::info!(
        "{}",
        "Catalog database deleted.".if_supports_color(Stdout, |t| t.bold()),
    );
    log::info!("  Path: {}", db_path.display());
    log::info!("  Freed: {:.1} MB", size_mb);
    crate::log_blank();
    log::info!("Run 'retro-junk catalog import all' to rebuild.");

    Ok(())
}
