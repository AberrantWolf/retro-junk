use std::path::PathBuf;

use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;

use crate::CliError;

use super::default_catalog_db_path;

/// List unresolved disagreements between data sources.
pub(crate) fn run_catalog_disagreements(
    db_path: Option<PathBuf>,
    system: Option<String>,
    field: Option<String>,
    limit: u32,
) -> Result<(), CliError> {
    use retro_junk_db::DisagreementFilter;

    let db_path = db_path.unwrap_or_else(default_catalog_db_path);

    if !db_path.exists() {
        log::warn!("No catalog database found at {}", db_path.display());
        log::info!("Run 'retro-junk catalog import all' first.");
        return Ok(());
    }

    let conn = retro_junk_db::open_database(&db_path)
        .map_err(|e| CliError::database(format!("Failed to open catalog database: {}", e)))?;

    let filter = DisagreementFilter {
        platform_id: system.as_deref(),
        field: field.as_deref(),
        limit: Some(limit),
        ..Default::default()
    };

    let disagreements = retro_junk_db::list_unresolved_disagreements(&conn, &filter)
        .map_err(|e| CliError::database(format!("Failed to query disagreements: {}", e)))?;

    if disagreements.is_empty() {
        log::info!("No unresolved disagreements found.");
        return Ok(());
    }

    log::info!(
        "{}",
        format!("{} unresolved disagreement(s):", disagreements.len())
            .if_supports_color(Stdout, |t| t.bold()),
    );
    crate::log_blank();

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
        crate::log_blank();
    }

    log::info!(
        "Resolve with: retro-junk catalog resolve <id> --source-a | --source-b | --custom <value>"
    );

    Ok(())
}

/// Resolve a disagreement by choosing a value.
pub(crate) fn run_catalog_resolve(
    id: i64,
    db_path: Option<PathBuf>,
    source_a: bool,
    source_b: bool,
    custom: Option<String>,
) -> Result<(), CliError> {
    let db_path = db_path.unwrap_or_else(default_catalog_db_path);

    if !db_path.exists() {
        log::warn!("No catalog database found at {}", db_path.display());
        return Ok(());
    }

    let conn = retro_junk_db::open_database(&db_path)
        .map_err(|e| CliError::database(format!("Failed to open catalog database: {}", e)))?;

    // Fetch the disagreement first
    let disagreement = retro_junk_db::get_disagreement(&conn, id)
        .map_err(|e| CliError::database(format!("Failed to fetch disagreement: {}", e)))?
        .ok_or_else(|| CliError::other(format!("Disagreement #{} not found.", id)))?;

    if disagreement.resolved {
        log::warn!(
            "Disagreement #{} is already resolved (resolution: {}).",
            id,
            disagreement.resolution.as_deref().unwrap_or("unknown"),
        );
        return Ok(());
    }

    // Determine resolution
    let (resolution, chosen_value) = if source_a {
        ("source_a".to_string(), disagreement.value_a.clone())
    } else if source_b {
        ("source_b".to_string(), disagreement.value_b.clone())
    } else if let Some(ref val) = custom {
        (format!("custom: {val}"), Some(val.clone()))
    } else {
        return Err(CliError::other(
            "Must specify --source-a, --source-b, or --custom <value>.",
        ));
    };

    // Apply the chosen value to the entity
    if let Some(ref value) = chosen_value
        && let Err(e) = retro_junk_db::apply_disagreement_resolution(
            &conn,
            &disagreement.entity_type,
            &disagreement.entity_id,
            &disagreement.field,
            value,
        )
    {
        return Err(CliError::database(format!(
            "Failed to apply resolution: {}",
            e
        )));
    }

    // Mark as resolved
    retro_junk_db::resolve_disagreement(&conn, id, &resolution)
        .map_err(|e| CliError::database(format!("Failed to resolve disagreement: {}", e)))?;

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

    Ok(())
}
