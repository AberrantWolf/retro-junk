use std::path::PathBuf;

use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;
use retro_junk_lib::Platform;

use crate::CliError;

use super::default_catalog_db_path;

/// Clear enrichment status for releases matching the given criteria.
pub(crate) fn run_catalog_unenrich(
    system: String,
    after: Option<String>,
    db_path: Option<PathBuf>,
    confirm: bool,
) -> Result<(), CliError> {
    // Parse system as a Platform enum (accepts aliases like "megadrive", "MD", etc.)
    let core_platform: Platform = system.parse().map_err(|_| {
        CliError::unknown_system(format!(
            "Unknown system '{}'. Use a short name like 'nes', 'snes', 'n64'.",
            system
        ))
    })?;
    let short_name = core_platform.short_name();

    let db_path = db_path.unwrap_or_else(default_catalog_db_path);

    let conn = retro_junk_db::open_database(&db_path)
        .map_err(|e| CliError::database(format!("Failed to open database: {}", e)))?;

    // Find the DB platform by matching short_name
    let platforms = retro_junk_db::list_platforms(&conn)
        .map_err(|e| CliError::database(format!("Failed to list platforms: {}", e)))?;

    let platform = platforms
        .iter()
        .find(|p| p.short_name == short_name)
        .ok_or_else(|| {
            CliError::unknown_system(format!(
                "System '{}' not found in catalog database. Run 'retro-junk catalog import all' first.",
                short_name
            ))
        })?;

    let after_ref = after.as_deref();

    // Count how many releases would be affected
    let count = retro_junk_db::count_enriched_releases(&conn, &platform.id, after_ref)
        .map_err(|e| CliError::database(format!("Failed to count enriched releases: {}", e)))?;

    if count == 0 {
        log::info!(
            "No enriched releases found for {} matching criteria.",
            platform.display_name
        );
        return Ok(());
    }

    let scope = if let Some(ref after_val) = after {
        format!("with titles >= \"{}\"", after_val)
    } else {
        "all".to_string()
    };

    if !confirm {
        log::info!(
            "Would clear enrichment status for {} {} releases ({}).",
            count.if_supports_color(Stdout, |t| t.bold()),
            platform
                .display_name
                .if_supports_color(Stdout, |t| t.bold()),
            scope,
        );
        log::info!("Re-run with --confirm to proceed:");
        let mut cmd = format!("  retro-junk catalog unenrich {}", short_name);
        if let Some(ref after_val) = after {
            cmd.push_str(&format!(" --after \"{}\"", after_val));
        }
        cmd.push_str(" --confirm");
        log::info!("{}", cmd);
        return Ok(());
    }

    // Execute the unenrich
    let changed = retro_junk_db::unenrich_releases(&conn, &platform.id, after_ref)
        .map_err(|e| CliError::database(format!("Failed to unenrich releases: {}", e)))?;

    log::info!(
        "{} Cleared enrichment status for {} {} releases ({}).",
        "\u{2714}".if_supports_color(Stdout, |t| t.green()),
        changed.if_supports_color(Stdout, |t| t.bold()),
        platform.display_name,
        scope,
    );
    log::info!(
        "Run 'retro-junk catalog enrich {}' to re-enrich.",
        short_name
    );

    Ok(())
}
