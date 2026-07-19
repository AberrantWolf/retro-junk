//! Catalog data-gathering background operations for the Tools → Data tab.
//!
//! These are the GUI counterparts of the CLI's `cache dat/gdb fetch`,
//! `catalog import`, `catalog enrich-gdb`, and `catalog enrich` commands. They
//! are thin presentation adapters: all the heavy lifting is delegated to the
//! same `retro_junk_import` / `retro_junk_dat` / `retro_junk_db` library
//! functions the CLI uses (see `retro-junk-cli/src/commands/catalog/*`).
//!
//! Each operation runs on a background thread via [`spawn_background_op`],
//! reports coarse progress through [`AppMessage::OperationProgress`], and prints
//! per-item results with `log::info!/warn!` (surfaced in the GUI log viewer,
//! exactly like the CLI's stdout). On completion a worker sends
//! [`AppMessage::CatalogDataChanged`] so the Dashboard/Browse tabs refresh and
//! the Data-tab in-flight guard clears.

#[cfg(test)]
#[path = "catalog_ops_tests.rs"]
mod tests;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use retro_junk_lib::async_util::{cancellable, run_with_events};
use retro_junk_lib::context::RegisteredConsole;
use retro_junk_lib::{AnalysisContext, Platform};

use crate::app::RetroJunkApp;
use crate::backend::worker::spawn_background_op;
use crate::state::{AppMessage, OperationKind, ProgressDisplay};

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

/// Options for a `ScreenScraper` enrichment run, collected from the UI widgets.
pub struct SsEnrichOptions {
    pub force: bool,
    pub download_assets: bool,
    pub region: String,
    pub language: String,
    pub limit: Option<u32>,
    pub reconcile: bool,
}

/// Resolve the catalog seed-data directory from settings, falling back to
/// `./catalog` (the CLI default in `commands/catalog/mod.rs::default_catalog_dir`).
fn catalog_data_dir(setting: &str) -> PathBuf {
    if setting.trim().is_empty() {
        PathBuf::from("catalog")
    } else {
        PathBuf::from(setting)
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

/// Shared plumbing handed to every background worker: the operation id, the
/// cancellation flag, the message channel back to the UI thread, and the egui
/// context for repaint requests.
struct WorkerCtx<'a> {
    op_id: u64,
    cancel: &'a AtomicBool,
    tx: &'a mpsc::Sender<AppMessage>,
    ctx: &'a egui::Context,
}

impl WorkerCtx<'_> {
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Send a progress tick for this operation and request a repaint.
    fn progress(&self, current: u64, total: u64) {
        let _ = self.tx.send(AppMessage::OperationProgress {
            op_id: self.op_id,
            current,
            total,
        });
        self.ctx.request_repaint();
    }

    /// Send the terminal message pair: remove the progress bar, then signal
    /// that catalog data changed (refresh + clear the in-flight guard).
    fn finish(&self) {
        finish(self.tx, self.op_id, self.ctx);
    }
}

/// Send the terminal message pair: remove the progress bar, then signal that
/// catalog data changed (refresh + clear the in-flight guard).
fn finish(tx: &mpsc::Sender<AppMessage>, op_id: u64, ctx: &egui::Context) {
    let _ = tx.send(AppMessage::OperationComplete { op_id });
    let _ = tx.send(AppMessage::CatalogDataChanged);
    ctx.request_repaint();
}

// ── Catalog import ──────────────────────────────────────────────────────────

/// GUI progress adapter for DAT imports. Mirrors `CliImportProgress` in
/// `commands/catalog/import.rs`, but forwards to the activity-bar progress bar.
struct GuiImportProgress {
    op_id: u64,
    tx: mpsc::Sender<AppMessage>,
    ctx: egui::Context,
}

impl retro_junk_import::ImportProgress for GuiImportProgress {
    fn on_game(&self, current: usize, total: usize, _name: &str) {
        // Throttle to keep the channel light; always send the final tick.
        if current.is_multiple_of(250) || current == total {
            let _ = self.tx.send(AppMessage::OperationProgress {
                op_id: self.op_id,
                current: current as u64,
                total: total as u64,
            });
            self.ctx.request_repaint();
        }
    }

    fn on_phase(&self, message: &str) {
        log::info!("{message}");
    }

    fn on_complete(&self, message: &str) {
        log::info!("{message}");
    }
}

/// Import DATs into the catalog DB for the selected systems (auto-downloading
/// DATs as needed). Mirrors `commands/catalog/import.rs::run_catalog_import`.
pub fn run_import(app: &mut RetroJunkApp, ctx: &egui::Context) {
    let Some(db_path) = app.db_path.clone() else {
        log::warn!("No catalog database path; cannot import");
        return;
    };
    let context = app.context.clone();
    let selected = app.ui_state.tools_state.data.selected_systems.clone();
    let catalog_dir = catalog_data_dir(&app.settings.general.catalog_data_dir);
    let egui_ctx = ctx.clone();
    app.ui_state.tools_state.data.op_in_flight = true;

    spawn_background_op(
        app,
        "Importing catalog".to_string(),
        OperationKind::Other,
        String::new(),
        ProgressDisplay::Count,
        move |op_id, cancel, tx| {
            let w = WorkerCtx {
                op_id,
                cancel: cancel.as_ref(),
                tx: &tx,
                ctx: &egui_ctx,
            };
            import_worker(&w, &context, &selected, &db_path, &catalog_dir);
            w.finish();
        },
    );
}

fn import_worker(
    w: &WorkerCtx<'_>,
    context: &AnalysisContext,
    selected: &HashSet<Platform>,
    db_path: &Path,
    catalog_dir: &Path,
) {
    use retro_junk_import::{ImportStats, dat_source_str, import_dat, log_import};

    let conn = match retro_junk_db::open_database(db_path) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to open catalog database: {e}");
            return;
        }
    };

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
        if w.cancelled() {
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
            if w.cancelled() {
                break;
            }
            let progress = GuiImportProgress {
                op_id: w.op_id,
                tx: w.tx.clone(),
                ctx: w.ctx.clone(),
            };
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
}

// ── GDB enrichment ──────────────────────────────────────────────────────────

/// Enrich catalog releases from `GameDataBase` CSVs for the selected systems.
/// Mirrors `commands/catalog/enrich_gdb.rs::run_catalog_enrich_gdb`.
pub fn run_gdb_enrich(app: &mut RetroJunkApp, ctx: &egui::Context) {
    let Some(db_path) = app.db_path.clone() else {
        return;
    };
    let context = app.context.clone();
    let selected = app.ui_state.tools_state.data.selected_systems.clone();
    let limit = parse_limit(&app.ui_state.tools_state.data.gdb_limit);
    let egui_ctx = ctx.clone();
    app.ui_state.tools_state.data.op_in_flight = true;

    spawn_background_op(
        app,
        "Enriching from GameDataBase".to_string(),
        OperationKind::Other,
        String::new(),
        ProgressDisplay::Count,
        move |op_id, cancel, tx| {
            let w = WorkerCtx {
                op_id,
                cancel: cancel.as_ref(),
                tx: &tx,
                ctx: &egui_ctx,
            };
            gdb_worker(&w, &context, &selected, &db_path, limit);
            w.finish();
        },
    );
}

fn gdb_worker(
    w: &WorkerCtx<'_>,
    context: &AnalysisContext,
    selected: &HashSet<Platform>,
    db_path: &Path,
    limit: Option<u32>,
) {
    use retro_junk_import::gdb_import::{GdbEnrichOptions, enrich_gdb};

    let conn = match retro_junk_db::open_database(db_path) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to open catalog database: {e}");
            return;
        }
    };

    let to_enrich = targets(context, selected, Cap::Gdb);
    let total = to_enrich.len() as u64;
    for (i, console) in to_enrich.iter().enumerate() {
        if w.cancelled() {
            log::info!("GDB enrichment cancelled");
            break;
        }
        w.progress(i as u64, total);

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
}

// ── ScreenScraper enrichment ────────────────────────────────────────────────

/// Enrich catalog releases from `ScreenScraper` for the selected systems, with
/// optional asset download and post-enrichment reconciliation. Mirrors
/// `commands/catalog/enrich.rs::run_catalog_enrich`.
pub fn run_ss_enrich(app: &mut RetroJunkApp, ctx: &egui::Context, opts: SsEnrichOptions) {
    let Some(db_path) = app.db_path.clone() else {
        return;
    };
    let selected = app.ui_state.tools_state.data.selected_systems.clone();
    let egui_ctx = ctx.clone();
    app.ui_state.tools_state.data.op_in_flight = true;

    spawn_background_op(
        app,
        "Enriching from ScreenScraper".to_string(),
        OperationKind::Other,
        String::new(),
        ProgressDisplay::Count,
        move |op_id, cancel, tx| {
            let w = WorkerCtx {
                op_id,
                cancel: cancel.as_ref(),
                tx: &tx,
                ctx: &egui_ctx,
            };
            ss_worker(&w, &selected, &db_path, &opts);
            w.finish();
        },
    );
}

fn ss_worker(
    w: &WorkerCtx<'_>,
    selected: &HashSet<Platform>,
    db_path: &Path,
    opts: &SsEnrichOptions,
) {
    use retro_junk_import::scraper_import::{self, EnrichEvent, EnrichOptions};

    let conn = match retro_junk_db::open_database(db_path) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to open catalog database: {e}");
            return;
        }
    };

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

    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            log::error!("Failed to create async runtime: {e}");
            return;
        }
    };

    rt.block_on(async {
        // Connect to ScreenScraper (cancel-aware — the handshake can be slow).
        let (client, max_workers) =
            match cancellable(retro_junk_scraper::create_client(None), w.cancel).await {
                None => return,
                Some(Ok(r)) => r,
                Some(Err(e)) => {
                    log::error!("ScreenScraper connection failed: {e}");
                    return;
                }
            };

        let (event_tx, event_rx) = tokio::sync::mpsc::channel::<EnrichEvent>(1024);
        let enrich_future =
            scraper_import::enrich_releases(client, &conn, &options, max_workers, event_tx);

        // Progress is tracked per-platform (reset on PlatformStarted).
        let op_id = w.op_id;
        let tx_ev = w.tx.clone();
        let ctx_ev = w.ctx.clone();
        let mut processed: u64 = 0;
        let mut total: u64 = 0;
        let result = run_with_events(enrich_future, event_rx, |e| match e {
            EnrichEvent::PlatformStarted {
                platform_name,
                total: t,
                ..
            } => {
                total = t as u64;
                processed = 0;
                log::info!("Enriching {t} releases for {platform_name}");
                let _ = tx_ev.send(AppMessage::OperationProgress {
                    op_id,
                    current: 0,
                    total,
                });
                ctx_ev.request_repaint();
            }
            EnrichEvent::ReleaseFound { title, ss_name, .. } => {
                processed += 1;
                log::info!("  \u{2714} {title} (SS: \"{ss_name}\")");
                let _ = tx_ev.send(AppMessage::OperationProgress {
                    op_id,
                    current: processed,
                    total,
                });
                ctx_ev.request_repaint();
            }
            EnrichEvent::ReleaseNotFound { title, .. } => {
                processed += 1;
                log::info!("  \u{2718} {title}");
                let _ = tx_ev.send(AppMessage::OperationProgress {
                    op_id,
                    current: processed,
                    total,
                });
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
            log::error!("Enrichment failed: {e}");
            return;
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
    });
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
pub fn run_cache_fetch(app: &mut RetroJunkApp, ctx: &egui::Context, kind: CacheKind) {
    let context = app.context.clone();
    let selected = app.ui_state.tools_state.data.selected_systems.clone();
    let egui_ctx = ctx.clone();
    let desc = match kind {
        CacheKind::Dat => "Fetching DAT files",
        CacheKind::Gdb => "Fetching GDB files",
    };
    app.ui_state.tools_state.data.op_in_flight = true;

    spawn_background_op(
        app,
        desc.to_string(),
        OperationKind::Other,
        String::new(),
        ProgressDisplay::Count,
        move |op_id, cancel, tx| {
            let w = WorkerCtx {
                op_id,
                cancel: cancel.as_ref(),
                tx: &tx,
                ctx: &egui_ctx,
            };
            cache_fetch_worker(&w, &context, &selected, kind);
            w.finish();
        },
    );
}

fn cache_fetch_worker(
    w: &WorkerCtx<'_>,
    context: &AnalysisContext,
    selected: &HashSet<Platform>,
    kind: CacheKind,
) {
    let cap = match kind {
        CacheKind::Dat => Cap::Dat,
        CacheKind::Gdb => Cap::Gdb,
    };
    let to_fetch = targets(context, selected, cap);
    let total = to_fetch.len() as u64;

    for (i, console) in to_fetch.iter().enumerate() {
        if w.cancelled() {
            log::info!("Fetch cancelled");
            break;
        }
        w.progress(i as u64, total);

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

/// Clear the DAT or GDB download cache.
pub fn run_cache_clear(app: &mut RetroJunkApp, ctx: &egui::Context, kind: CacheKind) {
    let egui_ctx = ctx.clone();
    app.ui_state.tools_state.data.op_in_flight = true;

    spawn_background_op(
        app,
        "Clearing cache".to_string(),
        OperationKind::Other,
        String::new(),
        ProgressDisplay::Count,
        move |op_id, _cancel, tx| {
            let result = match kind {
                CacheKind::Dat => retro_junk_dat::cache::clear(),
                CacheKind::Gdb => retro_junk_dat::gdb_cache::clear(),
            };
            match result {
                Ok(freed) => log::info!("Cache cleared ({freed} bytes freed)"),
                Err(e) => log::warn!("Failed to clear cache: {e}"),
            }
            finish(&tx, op_id, &egui_ctx);
        },
    );
}

/// Reload the DAT/GDB cache listings on a background thread and send them back
/// via [`AppMessage::CacheListsLoaded`]. Called when the Data tab is shown or
/// after a cache-mutating operation.
pub fn load_cache_lists(app: &RetroJunkApp, ctx: &egui::Context) {
    let tx = app.message_tx.clone();
    let egui_ctx = ctx.clone();
    std::thread::spawn(move || {
        let dat = retro_junk_dat::cache::list().unwrap_or_default();
        let gdb = retro_junk_dat::gdb_cache::list().unwrap_or_default();
        let _ = tx.send(AppMessage::CacheListsLoaded { dat, gdb });
        egui_ctx.request_repaint();
    });
}

/// Parse a limit text field into `Option<u32>` (empty/invalid = no limit).
pub fn parse_limit(s: &str) -> Option<u32> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        t.parse::<u32>().ok()
    }
}
