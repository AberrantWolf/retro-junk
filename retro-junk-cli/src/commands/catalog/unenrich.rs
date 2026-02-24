use std::path::PathBuf;

use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;
use retro_junk_lib::Platform;

use super::default_catalog_db_path;

/// Clear enrichment status for releases matching the given criteria.
pub(crate) fn run_catalog_unenrich(
    system: String,
    after: Option<String>,
    db_path: Option<PathBuf>,
    confirm: bool,
) {
    // Parse system as a Platform enum (accepts aliases like "megadrive", "MD", etc.)
    let core_platform: Platform = match system.parse() {
        Ok(p) => p,
        Err(_) => {
            log::error!("Unknown system '{}'. Use a short name like 'nes', 'snes', 'n64'.", system);
            std::process::exit(1);
        }
    };
    let short_name = core_platform.short_name();

    let db_path = db_path.unwrap_or_else(default_catalog_db_path);

    let conn = match retro_junk_db::open_database(&db_path) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to open database: {}", e);
            std::process::exit(1);
        }
    };

    // Find the DB platform by matching short_name
    let platforms = match retro_junk_db::list_platforms(&conn) {
        Ok(p) => p,
        Err(e) => {
            log::error!("Failed to list platforms: {}", e);
            std::process::exit(1);
        }
    };

    let platform = platforms.iter().find(|p| p.short_name == short_name);
    let platform = match platform {
        Some(p) => p,
        None => {
            log::error!("System '{}' not found in catalog database.", short_name);
            log::info!("Run 'retro-junk catalog import all' first.");
            std::process::exit(1);
        }
    };

    let after_ref = after.as_deref();

    // Count how many releases would be affected
    let count = match retro_junk_db::count_enriched_releases(&conn, &platform.id, after_ref) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to count enriched releases: {}", e);
            std::process::exit(1);
        }
    };

    if count == 0 {
        log::info!("No enriched releases found for {} matching criteria.", platform.display_name);
        return;
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
            platform.display_name.if_supports_color(Stdout, |t| t.bold()),
            scope,
        );
        log::info!("Re-run with --confirm to proceed:");
        let mut cmd = format!("  retro-junk catalog unenrich {}", short_name);
        if let Some(ref after_val) = after {
            cmd.push_str(&format!(" --after \"{}\"", after_val));
        }
        cmd.push_str(" --confirm");
        log::info!("{}", cmd);
        return;
    }

    // Execute the unenrich
    match retro_junk_db::unenrich_releases(&conn, &platform.id, after_ref) {
        Ok(changed) => {
            log::info!(
                "{} Cleared enrichment status for {} {} releases ({}).",
                "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                changed.if_supports_color(Stdout, |t| t.bold()),
                platform.display_name,
                scope,
            );
            log::info!("Run 'retro-junk catalog enrich {}' to re-enrich.", short_name);
        }
        Err(e) => {
            log::error!("Failed to unenrich releases: {}", e);
            std::process::exit(1);
        }
    }
}
