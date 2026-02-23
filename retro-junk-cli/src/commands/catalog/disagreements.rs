use std::path::PathBuf;

use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;

use super::default_catalog_db_path;

/// List unresolved disagreements between data sources.
pub(crate) fn run_catalog_disagreements(
    db_path: Option<PathBuf>,
    system: Option<String>,
    field: Option<String>,
    limit: u32,
) {
    use retro_junk_db::DisagreementFilter;

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

    let filter = DisagreementFilter {
        platform_id: system.as_deref(),
        field: field.as_deref(),
        limit: Some(limit),
        ..Default::default()
    };

    match retro_junk_db::list_unresolved_disagreements(&conn, &filter) {
        Ok(disagreements) => {
            if disagreements.is_empty() {
                log::info!("No unresolved disagreements found.");
                return;
            }

            log::info!(
                "{}",
                format!("{} unresolved disagreement(s):", disagreements.len())
                    .if_supports_color(Stdout, |t| t.bold()),
            );
            log::info!("");

            for d in &disagreements {
                log::info!(
                    "  #{} {} {} [{}]",
                    format!("{}", d.id).if_supports_color(Stdout, |t| t.bold()),
                    d.entity_type,
                    d.entity_id.if_supports_color(Stdout, |t| t.dimmed()),
                    d.field.if_supports_color(Stdout, |t| t.cyan()),
                );
                log::info!(
                    "    {} {}: {}",
                    "\u{25B6}".if_supports_color(Stdout, |t| t.blue()),
                    d.source_a,
                    d.value_a.as_deref().unwrap_or("(empty)"),
                );
                log::info!(
                    "    {} {}: {}",
                    "\u{25B6}".if_supports_color(Stdout, |t| t.yellow()),
                    d.source_b,
                    d.value_b.as_deref().unwrap_or("(empty)"),
                );
                log::info!("");
            }

            log::info!(
                "Resolve with: retro-junk catalog resolve <id> --source-a | --source-b | --custom <value>"
            );
        }
        Err(e) => {
            log::error!("Failed to query disagreements: {}", e);
            std::process::exit(1);
        }
    }
}

/// Resolve a disagreement by choosing a value.
pub(crate) fn run_catalog_resolve(
    id: i64,
    db_path: Option<PathBuf>,
    source_a: bool,
    source_b: bool,
    custom: Option<String>,
) {
    let db_path = db_path.unwrap_or_else(default_catalog_db_path);

    if !db_path.exists() {
        log::warn!("No catalog database found at {}", db_path.display());
        return;
    }

    let conn = match retro_junk_db::open_database(&db_path) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to open catalog database: {}", e);
            std::process::exit(1);
        }
    };

    // Fetch the disagreement first
    let disagreement = match retro_junk_db::get_disagreement(&conn, id) {
        Ok(Some(d)) => d,
        Ok(None) => {
            log::error!("Disagreement #{} not found.", id);
            std::process::exit(1);
        }
        Err(e) => {
            log::error!("Failed to fetch disagreement: {}", e);
            std::process::exit(1);
        }
    };

    if disagreement.resolved {
        log::warn!(
            "Disagreement #{} is already resolved (resolution: {}).",
            id,
            disagreement.resolution.as_deref().unwrap_or("unknown"),
        );
        return;
    }

    // Determine resolution
    let (resolution, chosen_value) = if source_a {
        ("source_a".to_string(), disagreement.value_a.clone())
    } else if source_b {
        ("source_b".to_string(), disagreement.value_b.clone())
    } else if let Some(ref val) = custom {
        (format!("custom: {val}"), Some(val.clone()))
    } else {
        log::error!("Must specify --source-a, --source-b, or --custom <value>.");
        std::process::exit(1);
    };

    // Apply the chosen value to the entity
    if let Some(ref value) = chosen_value {
        if let Err(e) = retro_junk_db::apply_disagreement_resolution(
            &conn,
            &disagreement.entity_type,
            &disagreement.entity_id,
            &disagreement.field,
            value,
        ) {
            log::error!("Failed to apply resolution: {}", e);
            std::process::exit(1);
        }
    }

    // Mark as resolved
    match retro_junk_db::resolve_disagreement(&conn, id, &resolution) {
        Ok(()) => {
            log::info!(
                "{} Resolved disagreement #{}",
                "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                id,
            );
            log::info!(
                "  {} {} [{}] = {}",
                disagreement.entity_type,
                disagreement.entity_id,
                disagreement.field,
                chosen_value.as_deref().unwrap_or("(empty)"),
            );
        }
        Err(e) => {
            log::error!("Failed to resolve disagreement: {}", e);
            std::process::exit(1);
        }
    }
}
