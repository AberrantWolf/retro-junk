//! Catalog data-gathering commands: DAT import, GameDataBase enrichment,
//! ScreenScraper enrichment, and download-cache management.
//!
//! These are the same operations as the CLI's `cache dat/gdb fetch`,
//! `catalog import`, `catalog enrich-gdb`, and `catalog enrich` commands. All
//! the heavy lifting is delegated to the `retro_junk_import` /
//! `retro_junk_dat` / `retro_junk_db` library functions the CLI uses (see
//! `retro-junk-cli/src/commands/catalog/*`); this module adds the shared
//! progress/cancellation plumbing so every frontend runs the identical pass.
//!
//! Per-item results are reported with `log::info!/warn!` — the GUI surfaces
//! the log in its log viewer, the CLI prints it — and coarse progress flows
//! through [`OpCtx::progress`].

#[cfg(test)]
#[path = "catalog_ops_tests.rs"]
mod tests;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use retro_junk_io::ProgressUnit;
use retro_junk_lib::async_util::{cancellable, run_with_events};
use retro_junk_lib::context::RegisteredConsole;
use retro_junk_lib::{AnalysisContext, Platform};

use super::OpCtx;

/// Which download cache a fetch/clear operation targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheKind {
    Dat,
    Gdb,
}

/// Capability an operation requires of a console.
#[derive(Debug, Clone, Copy)]
enum Cap {
    Dat,
    Gdb,
}

/// Options for a `ScreenScraper` enrichment run.
pub struct SsEnrichOptions {
    pub force: bool,
    pub download_assets: bool,
    pub region: String,
    pub language: String,
    pub limit: Option<u32>,
    pub reconcile: bool,
}

/// Resolve the catalog seed-data directory from a settings string, falling
/// back to `./catalog` (the CLI default in
/// `commands/catalog/mod.rs::default_catalog_dir`).
#[must_use]
pub fn catalog_data_dir(setting: &str) -> PathBuf {
    if setting.trim().is_empty() {
        PathBuf::from("catalog")
    } else {
        PathBuf::from(setting)
    }
}

/// Parse a limit text field into `Option<u32>` (empty/invalid = no limit).
#[must_use]
pub fn parse_limit(s: &str) -> Option<u32> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        t.parse::<u32>().ok()
    }
}

/// Registered consoles matching a capability and the user's system selection.
/// An empty `selected` set means "all consoles with this capability".
fn targets<'a>(
    context: &'a AnalysisContext,
    selected: &HashSet<Platform>,
    cap: Cap,
) -> Vec<&'a RegisteredConsole> {
    context
        .consoles()
        .filter(|c| {
            let capable = match cap {
                Cap::Dat => c.analyzer.has_dat_support(),
                Cap::Gdb => c.analyzer.has_gdb_support(),
            };
            capable && (selected.is_empty() || selected.contains(&c.metadata.platform))
        })
        .collect()
}

// ── Catalog import ──────────────────────────────────────────────────────────

/// Import-progress adapter that forwards per-game ticks to an [`OpCtx`].
/// Mirrors `CliImportProgress` in `commands/catalog/import.rs`.
struct OpImportProgress<'a, 'b> {
    ctx: &'a OpCtx<'b>,
}

impl retro_junk_import::ImportProgress for OpImportProgress<'_, '_> {
    fn on_game(&self, current: usize, total: usize, _name: &str) {
        // Throttle to keep the reporting channel light; always send the
        // final tick.
        if current.is_multiple_of(250) || current == total {
            (self.ctx.progress)(
                "Importing catalog",
                ProgressUnit::Items,
                current as u64,
                total as u64,
            );
        }
    }

    fn on_phase(&self, message: &str) {
        log::info!("{message}");
    }

    fn on_complete(&self, message: &str) {
        log::info!("{message}");
    }
}

/// Import DATs into the catalog DB for the selected systems, auto-downloading
/// DATs as needed and seeding/applying the YAML catalog data in `catalog_dir`.
/// Mirrors `commands/catalog/import.rs::run_catalog_import`.
pub fn run_import(
    context: &AnalysisContext,
    selected: &HashSet<Platform>,
    db_path: &Path,
    catalog_dir: &Path,
    ctx: &OpCtx,
) -> Result<(), String> {
    use retro_junk_import::{ImportStats, dat_source_str, import_dat, log_import};

    let conn = retro_junk_db::open_database(db_path)
        .map_err(|e| format!("Failed to open catalog database: {e}"))?;

    // Seed platforms/companies/overrides from YAML (matches CLI import).
    if catalog_dir.exists() {
        match retro_junk_db::seed_from_catalog(&conn, catalog_dir) {
            Ok(stats) => log::info!(
                "Seeded {} platforms, {} companies, {} overrides from {}",
                stats.platforms,
                stats.companies,
                stats.overrides,
                catalog_dir.display(),
            ),
            Err(e) => log::warn!("Failed to seed from catalog YAML: {e}"),
        }
    } else {
        log::warn!(
            "Catalog seed directory not found at {} — platforms may be missing. \
             Set 'catalog_data_dir' in settings to point at the catalog/ folder.",
            catalog_dir.display(),
        );
    }

    let to_import = targets(context, selected, Cap::Dat);
    log::info!(
        "Importing {} system(s) into {}",
        to_import.len(),
        db_path.display()
    );

    let mut total = ImportStats::default();
    for console in &to_import {
        if ctx.cancelled() {
            log::info!("Import cancelled");
            break;
        }
        let short_name = console.metadata.short_name;
        let source = console.analyzer.dat_source();
        let source_str = dat_source_str(&source);

        let dats = match retro_junk_dat::cache::load_dats(
            short_name,
            console.analyzer.dat_names(),
            console.analyzer.dat_download_ids(),
            None,
            source,
            false,
        ) {
            Ok(d) => d,
            Err(e) => {
                log::warn!("{short_name}: {e}");
                continue;
            }
        };

        for dat in &dats {
            if ctx.cancelled() {
                break;
            }
            let progress = OpImportProgress { ctx };
            match import_dat(&conn, dat, console.metadata.platform, source_str, &progress) {
                Ok(stats) => {
                    if let Err(e) = log_import(&conn, source_str, &dat.name, &dat.version, &stats) {
                        log::warn!("Failed to log import: {e}");
                    }
                    log::info!(
                        "\u{2714} {} — {} games: {} works, {} releases, {} media ({} new)",
                        short_name,
                        stats.total_games,
                        stats.works_created + stats.works_existing,
                        stats.releases_created + stats.releases_existing,
                        stats.media_created + stats.media_updated + stats.media_unchanged,
                        stats.media_created,
                    );
                    total.works_created += stats.works_created;
                    total.releases_created += stats.releases_created;
                    total.media_created += stats.media_created;
                    total.total_games += stats.total_games;
                    total.disagreements_found += stats.disagreements_found;
                }
                Err(e) => log::warn!("\u{2718} {short_name}: import failed: {e}"),
            }
        }
    }

    // Apply overrides after all imports (matches CLI import).
    if catalog_dir.exists() {
        match retro_junk_catalog::yaml::load_overrides(&catalog_dir.join("overrides")) {
            Ok(overrides) if !overrides.is_empty() => {
                match retro_junk_import::apply_overrides(&conn, &overrides) {
                    Ok(count) if count > 0 => log::info!("\u{2714} Applied {count} override(s)"),
                    Ok(_) => {}
                    Err(e) => log::warn!("Failed to apply overrides: {e}"),
                }
            }
            Ok(_) => {}
            Err(e) => log::warn!("Failed to load overrides: {e}"),
        }
    }

    log::info!(
        "Import complete: {} works, {} releases, {} media, {} games processed",
        total.works_created,
        total.releases_created,
        total.media_created,
        total.total_games,
    );
    Ok(())
}

// ── GDB enrichment ──────────────────────────────────────────────────────────

/// Enrich catalog releases from `GameDataBase` CSVs for the selected systems.
/// Mirrors `commands/catalog/enrich_gdb.rs::run_catalog_enrich_gdb`.
pub fn run_gdb_enrich(
    context: &AnalysisContext,
    selected: &HashSet<Platform>,
    db_path: &Path,
    limit: Option<u32>,
    ctx: &OpCtx,
) -> Result<(), String> {
    use retro_junk_import::gdb_import::{GdbEnrichOptions, enrich_gdb};

    let conn = retro_junk_db::open_database(db_path)
        .map_err(|e| format!("Failed to open catalog database: {e}"))?;

    let to_enrich = targets(context, selected, Cap::Gdb);
    let total = to_enrich.len() as u64;
    for (i, console) in to_enrich.iter().enumerate() {
        if ctx.cancelled() {
            log::info!("GDB enrichment cancelled");
            break;
        }
        (ctx.progress)(
            "Enriching from GameDataBase",
            ProgressUnit::Items,
            i as u64,
            total,
        );

        let short_name = console.metadata.short_name;
        let options = GdbEnrichOptions {
            platform_id: short_name.to_string(),
            limit,
            gdb_dir: None,
        };
        match enrich_gdb(&conn, console.analyzer.gdb_csv_names(), &options) {
            Ok(stats) => log::info!(
                "\u{2714} {}: {}/{} matched, {} enriched, {} disagreements",
                short_name,
                stats.matched,
                stats.media_checked,
                stats.enriched,
                stats.disagreements,
            ),
            Err(e) => log::error!("\u{2718} {short_name}: {e}"),
        }
    }
    log::info!("GDB enrichment done");
    Ok(())
}

// ── ScreenScraper enrichment ────────────────────────────────────────────────

/// Enrich catalog releases from `ScreenScraper` for the selected systems, with
/// optional asset download and post-enrichment reconciliation. Mirrors
/// `commands/catalog/enrich.rs::run_catalog_enrich`.
pub fn run_ss_enrich(
    selected: &HashSet<Platform>,
    db_path: &Path,
    opts: &SsEnrichOptions,
    ctx: &OpCtx,
) -> Result<(), String> {
    use retro_junk_import::scraper_import::{self, EnrichEvent, EnrichOptions};

    let conn = retro_junk_db::open_database(db_path)
        .map_err(|e| format!("Failed to open catalog database: {e}"))?;

    // Selected platforms as short-name IDs; empty = all platforms in the DB.
    let platform_ids: Vec<String> = selected
        .iter()
        .map(|p| p.short_name().to_string())
        .collect();

    let options = EnrichOptions {
        platform_ids: platform_ids.clone(),
        limit: opts.limit,
        skip_existing: !opts.force,
        download_assets: opts.download_assets,
        asset_dir: None,
        preferred_region: opts.region.clone(),
        preferred_language: opts.language.clone(),
    };

    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| format!("Failed to create async runtime: {e}"))?;

    rt.block_on(async {
        // Connect to ScreenScraper (cancel-aware — the handshake can be slow).
        let (client, max_workers) =
            match cancellable(retro_junk_scraper::create_client(None), ctx.cancel).await {
                None => return Ok(()),
                Some(Ok(r)) => r,
                Some(Err(e)) => {
                    return Err(format!("ScreenScraper connection failed: {e}"));
                }
            };

        let (event_tx, event_rx) = tokio::sync::mpsc::channel::<EnrichEvent>(1024);
        let enrich_future =
            scraper_import::enrich_releases(client, &conn, &options, max_workers, event_tx);

        // Progress is tracked per-platform (reset on PlatformStarted).
        let mut processed: u64 = 0;
        let mut total: u64 = 0;
        let report = |current: u64, total: u64| {
            (ctx.progress)(
                "Enriching from ScreenScraper",
                ProgressUnit::Items,
                current,
                total,
            );
        };
        let result = run_with_events(enrich_future, event_rx, |e| match e {
            EnrichEvent::PlatformStarted {
                platform_name,
                total: t,
                ..
            } => {
                total = t as u64;
                processed = 0;
                log::info!("Enriching {t} releases for {platform_name}");
                report(0, total);
            }
            EnrichEvent::ReleaseFound { title, ss_name, .. } => {
                processed += 1;
                log::info!("  \u{2714} {title} (SS: \"{ss_name}\")");
                report(processed, total);
            }
            EnrichEvent::ReleaseNotFound { title, .. } => {
                processed += 1;
                log::info!("  \u{2718} {title}");
                report(processed, total);
            }
            EnrichEvent::ReleaseError { title, error, .. } => {
                processed += 1;
                log::warn!("  {title}: {error}");
            }
            EnrichEvent::ReleaseSkipped { .. } => {
                processed += 1;
            }
            EnrichEvent::FatalError { message } => log::error!("Fatal: {message}"),
            EnrichEvent::Done { stats } => log::info!(
                "Enrichment complete: {} enriched, {} not found, {} skipped, {} assets",
                stats.releases_enriched,
                stats.releases_not_found,
                stats.releases_skipped,
                stats.assets_downloaded,
            ),
            EnrichEvent::PlatformDone { .. } => {}
        })
        .await;

        if let Err(e) = result {
            return Err(format!("Enrichment failed: {e}"));
        }

        // Auto-reconcile duplicate works after enrichment (matches CLI).
        if opts.reconcile {
            let ids = if platform_ids.is_empty() {
                retro_junk_db::list_platforms(&conn)
                    .map(|ps| {
                        ps.into_iter()
                            .filter(|p| !p.core_platform.is_empty())
                            .map(|p| p.id)
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                platform_ids.clone()
            };
            run_reconcile(&conn, &ids);
        }
        Ok(())
    })
}

/// Merge duplicate works sharing a `ScreenScraper` ID. Mirrors
/// `commands/catalog/reconcile.rs::run_reconcile_on_conn` (non-dry-run).
fn run_reconcile(conn: &retro_junk_db::Connection, platform_ids: &[String]) {
    use retro_junk_import::reconcile::{ReconcileOptions, reconcile_works};

    let options = ReconcileOptions {
        platform_ids: platform_ids.to_vec(),
        dry_run: false,
    };
    match reconcile_works(conn, &options) {
        Ok(result) if result.stats.groups_found > 0 => {
            log::info!(
                "Reconciled {} duplicate work group(s)",
                result.stats.groups_found
            );
        }
        Ok(_) => {}
        Err(e) => log::warn!("Reconciliation failed: {e}"),
    }
}

// ── Cache management ─────────────────────────────────────────────────────────

/// Download DAT or GDB files for the selected systems into the local cache.
pub fn run_cache_fetch(
    context: &AnalysisContext,
    selected: &HashSet<Platform>,
    kind: CacheKind,
    ctx: &OpCtx,
) {
    let (cap, phase) = match kind {
        CacheKind::Dat => (Cap::Dat, "Fetching DAT files"),
        CacheKind::Gdb => (Cap::Gdb, "Fetching GDB files"),
    };
    let to_fetch = targets(context, selected, cap);
    let total = to_fetch.len() as u64;

    for (i, console) in to_fetch.iter().enumerate() {
        if ctx.cancelled() {
            log::info!("Fetch cancelled");
            break;
        }
        (ctx.progress)(phase, ProgressUnit::Items, i as u64, total);

        let short_name = console.metadata.short_name;
        match kind {
            CacheKind::Dat => {
                match retro_junk_dat::cache::fetch(
                    short_name,
                    console.analyzer.dat_names(),
                    console.analyzer.dat_download_ids(),
                    console.analyzer.dat_source(),
                    false,
                ) {
                    Ok(paths) => {
                        log::info!("\u{2714} {} ({} DAT file(s))", short_name, paths.len());
                    }
                    Err(e) => log::warn!("\u{2718} {short_name}: {e}"),
                }
            }
            CacheKind::Gdb => {
                for csv_name in console.analyzer.gdb_csv_names() {
                    match retro_junk_dat::gdb_cache::fetch_gdb(csv_name, false) {
                        Ok(_) => log::info!("\u{2714} {short_name} [{csv_name}]"),
                        Err(e) => log::warn!("\u{2718} {short_name} [{csv_name}]: {e}"),
                    }
                }
            }
        }
    }
    log::info!("Fetch done");
}

/// Clear the DAT or GDB download cache. Returns the number of bytes freed.
pub fn run_cache_clear(kind: CacheKind) -> Result<u64, String> {
    match kind {
        CacheKind::Dat => retro_junk_dat::cache::clear(),
        CacheKind::Gdb => retro_junk_dat::gdb_cache::clear(),
    }
    .map_err(|e| e.to_string())
}

/// List the current DAT and GDB download-cache contents. Cache metadata lives
/// on disk, so callers should not run this on a render thread.
#[must_use]
pub fn cache_lists() -> (
    Vec<retro_junk_dat::cache::CacheEntry>,
    Vec<retro_junk_dat::gdb_cache::GdbCacheEntry>,
) {
    (
        retro_junk_dat::cache::list().unwrap_or_default(),
        retro_junk_dat::gdb_cache::list().unwrap_or_default(),
    )
}

/// Answers: "which catalog media rows are byte-identical duplicates?" without
/// changing anything. Read-only, but it walks the whole catalog, so it belongs
/// off a render thread.
pub fn analyze_duplicates(
    db_path: &Path,
) -> Result<retro_junk_db::CatalogDeduplicationReport, String> {
    let connection = crate::queries::open_catalog(db_path)?;
    retro_junk_db::analyze_catalog_duplicates(&connection, None).map_err(|e| e.to_string())
}

/// Collapse byte-identical duplicate media onto one canonical row, repointing
/// everything that referenced a duplicate. Returns the same report shape as
/// [`analyze_duplicates`], describing what was actually merged.
pub fn deduplicate(db_path: &Path) -> Result<retro_junk_db::CatalogDeduplicationReport, String> {
    let connection = crate::queries::open_catalog(db_path)?;
    retro_junk_db::deduplicate_catalog(&connection, None).map_err(|e| e.to_string())
}

/// One reviewer decision about a disagreement between catalog sources.
pub struct DisagreementResolution<'a> {
    pub disagreement_id: retro_junk_catalog::types::DisagreementId,
    pub entity_type: &'a str,
    pub entity_id: &'a str,
    pub field: &'a str,
    /// Which side was accepted, recorded verbatim on the disagreement row.
    pub resolution: &'a str,
    /// The value to write onto the entity. `None` accepts an empty value,
    /// which leaves the entity alone and only records the decision.
    pub chosen_value: Option<&'a str>,
}

/// Record a reviewer's decision: write the accepted value onto the entity,
/// then mark the disagreement resolved.
///
/// A failure to write the value is logged but does not stop the disagreement
/// from being closed — the decision itself is worth keeping, and re-showing a
/// row the user already judged just makes them judge it again.
pub fn resolve_disagreement(
    conn: &retro_junk_db::Connection,
    resolution: &DisagreementResolution<'_>,
) -> Result<(), String> {
    if let Some(value) = resolution.chosen_value
        && let Err(error) = retro_junk_db::apply_disagreement_resolution(
            conn,
            resolution.entity_type,
            resolution.entity_id,
            resolution.field,
            value,
        )
    {
        log::warn!("Failed to apply resolution: {error}");
    }
    retro_junk_db::resolve_disagreement(conn, resolution.disagreement_id, resolution.resolution)
        .map_err(|error| error.to_string())
}
