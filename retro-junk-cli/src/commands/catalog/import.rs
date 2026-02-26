use std::path::PathBuf;

use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;

use retro_junk_lib::AnalysisContext;

use super::{default_catalog_db_path, default_catalog_dir};

/// Import DAT files into the catalog database.
pub(crate) fn run_catalog_import(
    ctx: &AnalysisContext,
    systems: Vec<String>,
    catalog_dir: Option<PathBuf>,
    db_path: Option<PathBuf>,
    dat_dir: Option<PathBuf>,
) {
    use retro_junk_import::{ImportStats, dat_source_str, import_dat, log_import};

    let db_path = db_path.unwrap_or_else(default_catalog_db_path);
    let catalog_dir = catalog_dir.unwrap_or_else(default_catalog_dir);

    // Open or create the database
    let conn = match retro_junk_db::open_database(&db_path) {
        Ok(c) => c,
        Err(e) => {
            log::error!(
                "Failed to open catalog database at {}: {}",
                db_path.display(),
                e
            );
            std::process::exit(1);
        }
    };

    // Seed platforms and companies from YAML
    if catalog_dir.exists() {
        match retro_junk_db::seed_from_catalog(&conn, &catalog_dir) {
            Ok(stats) => {
                log::info!(
                    "Seeded {} platforms, {} companies, {} overrides from {}",
                    stats.platforms,
                    stats.companies,
                    stats.overrides,
                    catalog_dir.display(),
                );
            }
            Err(e) => {
                log::warn!("Warning: failed to seed from catalog YAML: {}", e);
            }
        }
    } else {
        log::warn!(
            "Catalog directory not found at {}; skipping YAML seed",
            catalog_dir.display(),
        );
    }

    // Determine which consoles to import
    let to_import: Vec<_> = if systems.len() == 1 && systems[0].eq_ignore_ascii_case("all") {
        ctx.consoles()
            .filter(|c| c.analyzer.has_dat_support())
            .collect()
    } else {
        systems
            .iter()
            .filter_map(|name| {
                let console = ctx.get_by_short_name(name);
                if console.is_none() {
                    log::warn!(
                        "  {} Unknown system '{}'",
                        "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                        name,
                    );
                }
                console
            })
            .filter(|c| {
                if !c.analyzer.has_dat_support() {
                    log::warn!(
                        "  {} No DAT support for '{}'",
                        "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                        c.metadata.short_name,
                    );
                    return false;
                }
                true
            })
            .collect()
    };

    if to_import.is_empty() {
        log::warn!("No systems to import.");
        return;
    }

    log::info!(
        "{}",
        format!(
            "Importing {} system(s) into {}",
            to_import.len(),
            db_path.display()
        )
        .if_supports_color(Stdout, |t| t.bold()),
    );

    let mut total_stats = ImportStats::default();

    for console in &to_import {
        let short_name = console.metadata.short_name;
        let dat_names = console.analyzer.dat_names();
        let download_ids = console.analyzer.dat_download_ids();
        let source = console.analyzer.dat_source();
        let source_str = dat_source_str(&source);

        // Load DAT files (from custom dir or cache, auto-downloading if needed)
        let dats = match retro_junk_dat::cache::load_dats(
            short_name,
            dat_names,
            download_ids,
            dat_dir.as_deref(),
            source,
        ) {
            Ok(d) => d,
            Err(e) => {
                log::warn!(
                    "  {} {}: {}",
                    "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                    short_name.if_supports_color(Stdout, |t| t.bold()),
                    e,
                );
                continue;
            }
        };

        // Import each DAT
        for dat in &dats {
            let progress = CliImportProgress::new(short_name);
            let stats = match import_dat(
                &conn,
                dat,
                console.metadata.platform,
                source_str,
                Some(&progress),
            ) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!(
                        "  {} {}: import failed: {}",
                        "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                        short_name.if_supports_color(Stdout, |t| t.bold()),
                        e,
                    );
                    continue;
                }
            };

            // Log the import
            if let Err(e) = log_import(&conn, source_str, &dat.name, Some(&dat.version), &stats) {
                log::warn!("Failed to log import: {}", e);
            }

            log::info!(
                "  {} {} — {} games: {} works, {} releases, {} media ({} new, {} updated, {} unchanged), {} skipped",
                "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                short_name.if_supports_color(Stdout, |t| t.bold()),
                stats.total_games,
                stats.works_created + stats.works_existing,
                stats.releases_created + stats.releases_existing,
                stats.media_created + stats.media_updated + stats.media_unchanged,
                stats.media_created,
                stats.media_updated,
                stats.media_unchanged,
                stats.skipped_bad,
            );

            total_stats.works_created += stats.works_created;
            total_stats.works_existing += stats.works_existing;
            total_stats.releases_created += stats.releases_created;
            total_stats.releases_existing += stats.releases_existing;
            total_stats.media_created += stats.media_created;
            total_stats.media_updated += stats.media_updated;
            total_stats.media_unchanged += stats.media_unchanged;
            total_stats.skipped_bad += stats.skipped_bad;
            total_stats.total_games += stats.total_games;
            total_stats.disagreements_found += stats.disagreements_found;
        }
    }

    // Apply overrides after all imports
    let overrides_applied = if catalog_dir.exists() {
        match retro_junk_catalog::yaml::load_overrides(&catalog_dir.join("overrides")) {
            Ok(overrides) if !overrides.is_empty() => {
                match retro_junk_import::apply_overrides(&conn, &overrides) {
                    Ok(count) => {
                        if count > 0 {
                            log::info!(
                                "  {} Applied {} override(s)",
                                "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                                count,
                            );
                        }
                        count
                    }
                    Err(e) => {
                        log::warn!("Failed to apply overrides: {}", e);
                        0
                    }
                }
            }
            Ok(_) => 0,
            Err(e) => {
                log::warn!("Failed to load overrides: {}", e);
                0
            }
        }
    } else {
        0
    };

    log::info!("");
    log::info!(
        "{}",
        "Import complete".if_supports_color(Stdout, |t| t.bold()),
    );
    log::info!(
        "  Works: {} new, {} existing",
        total_stats.works_created,
        total_stats.works_existing,
    );
    log::info!(
        "  Releases: {} new, {} existing",
        total_stats.releases_created,
        total_stats.releases_existing,
    );
    log::info!(
        "  Media: {} new, {} updated, {} unchanged, {} bad dumps skipped",
        total_stats.media_created,
        total_stats.media_updated,
        total_stats.media_unchanged,
        total_stats.skipped_bad,
    );
    if total_stats.disagreements_found > 0 {
        log::info!("  Disagreements: {}", total_stats.disagreements_found);
    }
    if overrides_applied > 0 {
        log::info!("  Overrides applied: {}", overrides_applied);
    }
    log::info!("  Database: {}", db_path.display());
}

/// CLI progress reporter for DAT imports.
struct CliImportProgress {
    system: String,
}

impl CliImportProgress {
    fn new(system: &str) -> Self {
        Self {
            system: system.to_string(),
        }
    }
}

impl retro_junk_import::ImportProgress for CliImportProgress {
    fn on_game(&self, current: usize, total: usize, _name: &str) {
        // Log progress every 1000 games to avoid spam
        if current.is_multiple_of(1000) && current < total {
            log::info!(
                "    {} [{}/{}]",
                self.system.if_supports_color(Stdout, |t| t.dimmed()),
                current,
                total,
            );
        }
    }

    fn on_phase(&self, message: &str) {
        log::info!("{}", message);
    }

    fn on_complete(&self, message: &str) {
        log::info!("{}", message);
    }
}
