//! Thin dispatch to `retro_junk_backend::ops::catalog_ops`. Scheduling,
//! progress forwarding, and message delivery only — DAT import, GDB and
//! ScreenScraper enrichment, and cache management all run in the backend on
//! the same library path the CLI takes (`retro-junk-cli/src/commands/catalog/*`).
//!
//! Each operation runs on a background thread via [`spawn_background_op`] and
//! reports through the activity bar; per-item results arrive as `log::info!/
//! warn!` lines in the GUI log viewer, exactly like the CLI's stdout. On
//! completion a worker sends [`AppMessage::CatalogDataChanged`] so the
//! Dashboard/Browse tabs refresh and the Data-tab in-flight guard clears.

use retro_junk_backend::ops::OpCtx;
use retro_junk_backend::ops::catalog_ops as backend;

use crate::app::RetroJunkApp;
use crate::backend::worker::{forward_phases, spawn_background_op};
use crate::state::{AppMessage, OperationKind, ProgressDisplay};

pub use retro_junk_backend::ops::catalog_ops::{CacheKind, SsEnrichOptions, parse_limit};

/// Send the terminal message pair: remove the progress bar, then signal that
/// catalog data changed (refresh + clear the in-flight guard).
fn finish(tx: &crate::state::AppMessageSender, op_id: u64, ctx: &egui::Context) {
    let _ = tx.send(AppMessage::OperationComplete { op_id });
    let _ = tx.send(AppMessage::CatalogDataChanged);
    ctx.request_repaint();
}

/// Spawn one catalog data operation with the shared plumbing: mark the Data
/// tab in-flight, forward backend phase reports to the activity bar, log a
/// returned error, and finish on the catalog-data-changed path.
fn spawn_catalog_op(
    app: &mut RetroJunkApp,
    ctx: &egui::Context,
    description: &str,
    work: impl FnOnce(&OpCtx) -> Result<(), String> + Send + 'static,
) {
    let egui_ctx = ctx.clone();
    app.ui_state.tools_state.data.op_in_flight = true;

    spawn_background_op(
        app,
        description.to_string(),
        OperationKind::Other,
        String::new(),
        ProgressDisplay::Count,
        move |op_id, cancel, tx| {
            let progress = forward_phases(op_id, tx.clone());
            if let Err(e) = work(&OpCtx::new(&cancel, &progress)) {
                log::error!("{e}");
            }
            finish(&tx, op_id, &egui_ctx);
        },
    );
}

/// Import DATs into the catalog DB for the selected systems.
pub fn run_import(app: &mut RetroJunkApp, ctx: &egui::Context) {
    let Some(db_path) = app.db_path.clone() else {
        log::warn!("No catalog database path; cannot import");
        return;
    };
    let context = app.context.clone();
    let selected = app.ui_state.tools_state.data.selected_systems.clone();
    let catalog_dir = backend::catalog_data_dir(&app.settings.general.catalog_data_dir);

    spawn_catalog_op(app, ctx, "Importing catalog", move |op_ctx| {
        backend::run_import(&context, &selected, &db_path, &catalog_dir, op_ctx)
    });
}

/// Enrich catalog releases from `GameDataBase` CSVs for the selected systems.
pub fn run_gdb_enrich(app: &mut RetroJunkApp, ctx: &egui::Context) {
    let Some(db_path) = app.db_path.clone() else {
        return;
    };
    let context = app.context.clone();
    let selected = app.ui_state.tools_state.data.selected_systems.clone();
    let limit = parse_limit(&app.ui_state.tools_state.data.gdb_limit);

    spawn_catalog_op(app, ctx, "Enriching from GameDataBase", move |op_ctx| {
        backend::run_gdb_enrich(&context, &selected, &db_path, limit, op_ctx)
    });
}

/// Enrich catalog releases from `ScreenScraper` for the selected systems.
pub fn run_ss_enrich(app: &mut RetroJunkApp, ctx: &egui::Context, opts: SsEnrichOptions) {
    let Some(db_path) = app.db_path.clone() else {
        return;
    };
    let selected = app.ui_state.tools_state.data.selected_systems.clone();

    spawn_catalog_op(app, ctx, "Enriching from ScreenScraper", move |op_ctx| {
        backend::run_ss_enrich(&selected, &db_path, &opts, op_ctx)
    });
}

/// Download DAT or GDB files for the selected systems into the local cache.
pub fn run_cache_fetch(app: &mut RetroJunkApp, ctx: &egui::Context, kind: CacheKind) {
    let context = app.context.clone();
    let selected = app.ui_state.tools_state.data.selected_systems.clone();
    let desc = match kind {
        CacheKind::Dat => "Fetching DAT files",
        CacheKind::Gdb => "Fetching GDB files",
    };

    spawn_catalog_op(app, ctx, desc, move |op_ctx| {
        backend::run_cache_fetch(&context, &selected, kind, op_ctx);
        Ok(())
    });
}

/// Clear the DAT or GDB download cache.
pub fn run_cache_clear(app: &mut RetroJunkApp, ctx: &egui::Context, kind: CacheKind) {
    spawn_catalog_op(app, ctx, "Clearing cache", move |_op_ctx| {
        backend::run_cache_clear(kind)
            .map(|freed| log::info!("Cache cleared ({freed} bytes freed)"))
            .map_err(|e| format!("Failed to clear cache: {e}"))
    });
}

/// Reload the DAT/GDB cache listings on a background thread and send them back
/// via [`AppMessage::CacheListsLoaded`]. Called when the Data tab is shown or
/// after a cache-mutating operation.
pub fn load_cache_lists(app: &RetroJunkApp, ctx: &egui::Context) {
    let tx = app.message_tx.clone();
    let egui_ctx = ctx.clone();
    std::thread::spawn(move || {
        let (dat, gdb) = backend::cache_lists();
        let _ = tx.send(AppMessage::CacheListsLoaded { dat, gdb });
        egui_ctx.request_repaint();
    });
}
