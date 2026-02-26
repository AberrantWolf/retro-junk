use std::path::PathBuf;

use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;

use super::default_catalog_db_path;

/// Run the `catalog reconcile` command.
pub(crate) fn run_catalog_reconcile(systems: Vec<String>, db_path: Option<PathBuf>, dry_run: bool) {
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

    run_reconcile_on_conn(&conn, &systems, dry_run);
}

/// Shared reconciliation logic, usable from both standalone command and post-enrich.
pub(crate) fn run_reconcile_on_conn(
    conn: &retro_junk_db::Connection,
    systems: &[String],
    dry_run: bool,
) {
    use retro_junk_import::reconcile::{ReconcileOptions, reconcile_works};

    let options = ReconcileOptions {
        platform_ids: systems.to_vec(),
        dry_run,
    };

    log::info!(
        "\n{}",
        "Reconciling works...".if_supports_color(Stdout, |t| t.bold()),
    );

    let result = match reconcile_works(conn, &options) {
        Ok(r) => r,
        Err(e) => {
            log::error!("Reconciliation failed: {}", e);
            std::process::exit(1);
        }
    };

    if result.stats.groups_found == 0 {
        log::info!("  No duplicate works found.");
        return;
    }

    // Group details by platform
    let mut by_platform: std::collections::BTreeMap<&str, Vec<_>> =
        std::collections::BTreeMap::new();
    for detail in &result.details {
        by_platform
            .entry(&detail.platform_id)
            .or_default()
            .push(detail);
    }

    let verb = if dry_run { "Would merge" } else { "Merged" };

    for (platform_id, details) in &by_platform {
        log::info!(
            "\n  {}: {} groups with duplicate works",
            platform_id.if_supports_color(Stdout, |t| t.bold()),
            details.len(),
        );
        for detail in details {
            let all_names: Vec<&str> = detail.absorbed_names.iter().map(|s| s.as_str()).collect();
            log::info!(
                "    {} \"{}\" + \"{}\" -> \"{}\" ({} releases)",
                verb,
                all_names.join("\", \""),
                detail.surviving_name,
                detail
                    .surviving_name
                    .if_supports_color(Stdout, |t| t.green()),
                detail.total_releases,
            );
        }
    }

    log::info!("");
    if dry_run {
        log::info!(
            "{}",
            "Dry run: no changes made.".if_supports_color(Stdout, |t| t.yellow()),
        );
    } else {
        log::info!(
            "{}",
            "Reconciliation complete".if_supports_color(Stdout, |t| t.bold()),
        );
    }
    log::info!("  Groups found:     {:>6}", result.stats.groups_found);
    log::info!("  Works merged:     {:>6}", result.stats.works_merged);
    log::info!("  Works deleted:    {:>6}", result.stats.works_deleted);
    log::info!(
        "  Releases moved:   {:>6}",
        result.stats.releases_reassigned
    );
    log::info!("  Collisions:       {:>6}", result.stats.releases_merged);
    if result.stats.media_moved > 0 {
        log::info!("  Media moved:      {:>6}", result.stats.media_moved);
    }
}
