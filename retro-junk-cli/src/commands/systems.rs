use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;
use retro_junk_db::Connection;

use retro_junk_lib::{AnalysisContext, Platform, RegisteredConsole};

use crate::CliError;

/// Capability filter for system resolution.
pub(crate) enum SystemCapability {
    DatSupport,
    GdbSupport,
}

/// Resolve a systems argument to a list of consoles.
///
/// Empty vec or "all" returns all consoles matching the capability filter.
/// Explicit names are validated; unknown names produce an error.
pub(crate) fn resolve_systems<'a>(
    ctx: &'a AnalysisContext,
    systems: &[String],
    capability: SystemCapability,
) -> Result<Vec<&'a RegisteredConsole>, CliError> {
    let cap_filter = |c: &&RegisteredConsole| -> bool {
        match capability {
            SystemCapability::DatSupport => c.analyzer.has_dat_support(),
            SystemCapability::GdbSupport => c.analyzer.has_gdb_support(),
        }
    };

    let cap_name = match capability {
        SystemCapability::DatSupport => Some("DAT"),
        SystemCapability::GdbSupport => Some("GDB"),
    };

    if systems.is_empty() || (systems.len() == 1 && systems[0].eq_ignore_ascii_case("all")) {
        return Ok(ctx.consoles().filter(cap_filter).collect());
    }

    let mut result = Vec::new();
    for name in systems {
        let console = ctx.get_by_short_name(name).or_else(|| {
            // Try parsing as a Platform alias (e.g., "megadrive" -> genesis)
            name.parse::<Platform>()
                .ok()
                .and_then(|p| ctx.get_by_short_name(p.short_name()))
        });

        let console = console.ok_or_else(|| {
            CliError::unknown_system(format!(
                "Unknown system '{}'. Run 'retro-junk systems' to see all available systems.",
                name
            ))
        })?;

        if !cap_filter(&console) {
            if let Some(cap) = cap_name {
                return Err(CliError::unknown_system(format!(
                    "No {} support for '{}'. Run 'retro-junk systems' to see supported systems.",
                    cap, name
                )));
            }
        }

        result.push(console);
    }

    Ok(result)
}

/// Resolve a single system name to a console.
pub(crate) fn resolve_single_system<'a>(
    ctx: &'a AnalysisContext,
    system: &str,
) -> Result<&'a RegisteredConsole, CliError> {
    ctx.get_by_short_name(system)
        .or_else(|| {
            system
                .parse::<Platform>()
                .ok()
                .and_then(|p| ctx.get_by_short_name(p.short_name()))
        })
        .ok_or_else(|| {
            CliError::unknown_system(format!(
                "Unknown system '{}'. Run 'retro-junk systems' to see all available systems.",
                system
            ))
        })
}

/// Resolve systems to DB platform IDs (for enrich, which doesn't use AnalysisContext directly).
///
/// Empty vec or "all" returns all platform IDs with `core_platform` set.
pub(crate) fn resolve_platform_ids(
    conn: &Connection,
    systems: &[String],
) -> Result<Vec<String>, CliError> {
    if systems.is_empty() || (systems.len() == 1 && systems[0].eq_ignore_ascii_case("all")) {
        let platforms = retro_junk_db::list_platforms(conn)
            .map_err(|e| CliError::database(format!("Failed to list platforms: {}", e)))?;
        return Ok(platforms
            .into_iter()
            .filter(|p| !p.core_platform.is_empty())
            .map(|p| p.id)
            .collect());
    }

    let mut ids = Vec::new();
    for s in systems {
        let p: Platform = s.parse().map_err(|_| {
            CliError::unknown_system(format!(
                "Unknown system '{}'. Run 'retro-junk systems' to see all available systems.",
                s
            ))
        })?;
        ids.push(p.short_name().to_string());
    }
    Ok(ids)
}

/// List all supported systems grouped by manufacturer.
pub(crate) fn run_systems(
    ctx: &AnalysisContext,
    manufacturer: Option<String>,
) -> Result<(), CliError> {
    log::info!(
        "{}",
        "Supported systems:".if_supports_color(Stdout, |t| t.bold()),
    );

    // Group consoles by manufacturer
    let manufacturers = ["Nintendo", "Sega", "Sony", "Microsoft"];
    let mut total = 0u32;
    let mut dat_count = 0u32;
    let mut gdb_count = 0u32;

    for mfr in &manufacturers {
        if let Some(ref filter) = manufacturer {
            if !mfr.eq_ignore_ascii_case(filter) {
                continue;
            }
        }

        let consoles: Vec<_> = ctx
            .consoles()
            .filter(|c| c.metadata.manufacturer == *mfr)
            .collect();

        if consoles.is_empty() {
            continue;
        }

        log::info!("");
        log::info!("  {}", mfr.if_supports_color(Stdout, |t| t.bold()),);

        for c in &consoles {
            let has_dat = c.analyzer.has_dat_support();
            let has_gdb = c.analyzer.has_gdb_support();

            let mut tags = String::new();
            if has_dat {
                tags.push_str("DAT");
                dat_count += 1;
            }
            if has_gdb {
                if !tags.is_empty() {
                    tags.push_str("  ");
                }
                tags.push_str("GDB");
                gdb_count += 1;
            }

            log::info!(
                "    {:<12}{:<42} {}",
                c.metadata
                    .short_name
                    .if_supports_color(Stdout, |t| t.cyan()),
                c.metadata.platform_name,
                tags.if_supports_color(Stdout, |t| t.dimmed()),
            );

            total += 1;
        }
    }

    log::info!("");
    log::info!(
        "{} systems ({} with DAT support, {} with GDB support)",
        total,
        dat_count,
        gdb_count,
    );

    Ok(())
}
