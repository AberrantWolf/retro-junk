//! Tools → Data tab: catalog data-gathering operations.
//!
//! Surfaces the CLI's catalog-building pipeline in the GUI — fetch DAT/GDB
//! reference data, import DATs into the catalog DB, and enrich releases from
//! GameDataBase and ScreenScraper. All work is delegated to
//! [`crate::backend::catalog_ops`]; this module is purely the UI.

use retro_junk_core::util::format_bytes;
use retro_junk_lib::Platform;

use crate::app::RetroJunkApp;
use crate::backend::catalog_ops::{self, CacheKind, SsEnrichOptions};

/// A deferred action captured during rendering, dispatched after the immutable
/// UI borrows are released (each op needs `&mut app`).
enum Action {
    Import,
    GdbEnrich,
    SsEnrich,
    FetchCache(CacheKind),
    ClearCache(CacheKind),
}

pub fn show(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    let ctx = ui.ctx().clone();

    // Refresh cache listings when first shown or after a cache op.
    if app.tools_state.data.needs_cache_refresh {
        catalog_ops::load_cache_lists(app, &ctx);
        app.tools_state.data.needs_cache_refresh = false;
    }

    let busy = app.tools_state.data.op_in_flight;
    let mut action: Option<Action> = None;

    egui::ScrollArea::vertical().show(ui, |ui| {
        show_system_selector(ui, app);
        ui.add_space(12.0);

        if busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Operation running — watch the activity bar and log panel for progress.");
            });
            ui.add_space(8.0);
        }

        show_import_card(ui, app, busy, &mut action);
        ui.add_space(8.0);
        show_gdb_card(ui, app, busy, &mut action);
        ui.add_space(8.0);
        show_ss_card(ui, app, busy, &mut action);
        ui.add_space(8.0);
        show_cache_card(ui, app, busy, &mut action);
    });

    // Dispatch the captured action now that UI borrows are released.
    if let Some(action) = action {
        match action {
            Action::Import => catalog_ops::run_import(app, &ctx),
            Action::GdbEnrich => catalog_ops::run_gdb_enrich(app, &ctx),
            Action::SsEnrich => {
                let d = &app.tools_state.data;
                let opts = SsEnrichOptions {
                    force: d.ss_force,
                    download_assets: d.ss_download_assets,
                    region: d.ss_region.clone(),
                    language: d.ss_language.clone(),
                    limit: catalog_ops::parse_limit(&d.ss_limit),
                    reconcile: d.ss_reconcile,
                };
                catalog_ops::run_ss_enrich(app, &ctx, opts);
            }
            Action::FetchCache(kind) => catalog_ops::run_cache_fetch(app, &ctx, kind),
            Action::ClearCache(kind) => catalog_ops::run_cache_clear(app, &ctx, kind),
        }
    }
}

/// System multi-select. An empty selection means "all capable systems".
fn show_system_selector(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    // Snapshot the DAT-capable consoles (superset of GDB-capable) up front so
    // we don't hold a borrow of `app.context` while mutating the selection set.
    let context = app.context.clone();
    let consoles: Vec<(Platform, &'static str, &'static str)> = context
        .consoles()
        .filter(|c| c.analyzer.has_dat_support())
        .map(|c| {
            (
                c.metadata.platform,
                c.metadata.short_name,
                c.metadata.platform_name,
            )
        })
        .collect();

    let selected = &mut app.tools_state.data.selected_systems;

    let summary = if selected.is_empty() {
        format!("All systems ({})", consoles.len())
    } else {
        format!("{} selected", selected.len())
    };

    egui::CollapsingHeader::new(format!("Systems: {}", summary))
        .id_salt("data_system_selector")
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("All").clicked() {
                    selected.clear();
                }
                if ui.button("None").clicked() {
                    // "None" is meaningless for operations, but lets the user
                    // start a fresh explicit selection. Select the first system.
                    selected.clear();
                    if let Some((p, _, _)) = consoles.first() {
                        selected.insert(*p);
                    }
                }
            });
            ui.add_space(4.0);

            egui::Grid::new("data_system_grid")
                .num_columns(3)
                .spacing([16.0, 2.0])
                .show(ui, |ui| {
                    for (i, (platform, short, name)) in consoles.iter().enumerate() {
                        let mut on = selected.contains(platform);
                        if ui
                            .checkbox(&mut on, format!("{} ({})", name, short))
                            .changed()
                        {
                            if on {
                                selected.insert(*platform);
                            } else {
                                selected.remove(platform);
                            }
                        }
                        if (i + 1).is_multiple_of(3) {
                            ui.end_row();
                        }
                    }
                });
        });
}

/// A titled card wrapping operation controls.
fn card(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.strong(title);
        ui.add_space(4.0);
        add_contents(ui);
    });
}

fn show_import_card(
    ui: &mut egui::Ui,
    _app: &RetroJunkApp,
    busy: bool,
    action: &mut Option<Action>,
) {
    card(ui, "Import catalog from DATs", |ui| {
        ui.label(
            "Build or refresh the catalog database from Redump/No-Intro DAT files \
             (downloaded automatically if not cached).",
        );
        ui.add_space(4.0);
        if ui.add_enabled(!busy, egui::Button::new("Import")).clicked() {
            *action = Some(Action::Import);
        }
    });
}

fn show_gdb_card(
    ui: &mut egui::Ui,
    app: &mut RetroJunkApp,
    busy: bool,
    action: &mut Option<Action>,
) {
    card(ui, "Enrich from GameDataBase", |ui| {
        ui.label(
            "Add Japanese titles, developer/publisher, genre and player counts \
             from PigSaint's GameDataBase (matched by SHA1).",
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Limit per system:");
            ui.add(
                egui::TextEdit::singleline(&mut app.tools_state.data.gdb_limit)
                    .desired_width(80.0)
                    .hint_text("all"),
            );
        });
        ui.add_space(4.0);
        if ui
            .add_enabled(!busy, egui::Button::new("Enrich (GDB)"))
            .clicked()
        {
            *action = Some(Action::GdbEnrich);
        }
    });
}

fn show_ss_card(
    ui: &mut egui::Ui,
    app: &mut RetroJunkApp,
    busy: bool,
    action: &mut Option<Action>,
) {
    card(ui, "Enrich from ScreenScraper", |ui| {
        ui.label(
            "Fetch metadata (and optionally media assets) from ScreenScraper.fr. \
             Requires configured credentials.",
        );
        ui.add_space(4.0);
        let d = &mut app.tools_state.data;
        egui::Grid::new("ss_enrich_opts")
            .num_columns(2)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                ui.label("Region:");
                ui.add(egui::TextEdit::singleline(&mut d.ss_region).desired_width(60.0));
                ui.end_row();
                ui.label("Language:");
                ui.add(egui::TextEdit::singleline(&mut d.ss_language).desired_width(60.0));
                ui.end_row();
                ui.label("Limit per system:");
                ui.add(
                    egui::TextEdit::singleline(&mut d.ss_limit)
                        .desired_width(80.0)
                        .hint_text("all"),
                );
                ui.end_row();
            });
        ui.checkbox(&mut d.ss_force, "Re-enrich already-enriched releases");
        ui.checkbox(&mut d.ss_download_assets, "Download media assets");
        ui.checkbox(&mut d.ss_reconcile, "Reconcile duplicate works afterward");
        ui.add_space(4.0);
        if ui
            .add_enabled(!busy, egui::Button::new("Enrich (ScreenScraper)"))
            .clicked()
        {
            *action = Some(Action::SsEnrich);
        }
    });
}

fn show_cache_card(ui: &mut egui::Ui, app: &RetroJunkApp, busy: bool, action: &mut Option<Action>) {
    card(ui, "Reference-data cache", |ui| {
        ui.label("Pre-download or clear DAT and GDB reference files.");
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("Fetch DATs"))
                .clicked()
            {
                *action = Some(Action::FetchCache(CacheKind::Dat));
            }
            if ui
                .add_enabled(!busy, egui::Button::new("Fetch GDB"))
                .clicked()
            {
                *action = Some(Action::FetchCache(CacheKind::Gdb));
            }
            if ui
                .add_enabled(!busy, egui::Button::new("Clear DATs"))
                .clicked()
            {
                *action = Some(Action::ClearCache(CacheKind::Dat));
            }
            if ui
                .add_enabled(!busy, egui::Button::new("Clear GDB"))
                .clicked()
            {
                *action = Some(Action::ClearCache(CacheKind::Gdb));
            }
        });

        let d = &app.tools_state.data;
        let dat_total: u64 = d.dat_cache_entries.iter().map(|e| e.file_size).sum();
        let gdb_total: u64 = d.gdb_cache_entries.iter().map(|e| e.file_size).sum();
        ui.add_space(4.0);
        ui.weak(format!(
            "Cached: {} DAT file(s) ({}), {} GDB file(s) ({})",
            d.dat_cache_entries.len(),
            format_bytes(dat_total),
            d.gdb_cache_entries.len(),
            format_bytes(gdb_total),
        ));
    });
}
