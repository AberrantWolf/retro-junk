//! Headless GUI smoke tests driven through `egui_kittest`.
//!
//! These build the app via `RetroJunkApp::with_parts` with default settings
//! and no catalog DB, so they never touch the user's config or cache dirs.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::RetroJunkApp;
use crate::state::View;

fn harness<'a>() -> Harness<'a, RetroJunkApp> {
    Harness::new_eframe(|cc| {
        RetroJunkApp::with_parts(
            &cc.egui_ctx,
            crate::settings::AppSettings::default(),
            None,
            None,
        )
    })
}

#[test]
fn first_launch_shows_sidebar_and_welcome() {
    let mut harness = harness();
    harness.run();

    // Sidebar navigation entries
    harness.get_by_label("Library");
    harness.get_by_label("Settings");
    harness.get_by_label("Tools");

    // No library root configured yet, so the welcome screen shows
    harness.get_by_label("retro-junk Library Manager");
}

#[test]
#[cfg(debug_assertions)]
fn app_disables_virtual_list_incompatible_rect_id_warning() {
    let harness = harness();
    assert!(
        !harness
            .ctx
            .style_of(egui::Theme::Dark)
            .debug
            .warn_if_rect_changes_id
            && !harness
                .ctx
                .style_of(egui::Theme::Light)
                .debug
                .warn_if_rect_changes_id,
        "virtualized tables must not enable egui's cross-frame rect/id heuristic"
    );
    assert!(
        harness.ctx.options(|options| options.warn_on_id_clash),
        "true same-frame widget ID collisions must remain visible"
    );
}

#[test]
fn sidebar_click_switches_to_settings_view() {
    let mut harness = harness();
    harness.run();

    harness.get_by_label("Settings").click();
    harness.run();

    assert_eq!(harness.state().ui_state.current_view, View::Settings);
    harness.get_by_label("Current root:");
}

#[test]
fn chd_compress_busy_scopes_to_operation_kind_and_folder() {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use crate::state::{BackgroundOperation, OperationKind, ProgressDisplay};

    let mut app = RetroJunkApp::with_parts(
        &egui::Context::default(),
        crate::settings::AppSettings::default(),
        None,
        None,
    );

    assert!(!app.chd_compress_busy("X"));

    app.operations.push(BackgroundOperation::new(
        1,
        "Compressing".to_string(),
        Arc::new(AtomicBool::new(false)),
        OperationKind::ChdCompress,
        "X".to_string(),
        ProgressDisplay::Percent,
    ));

    assert!(app.chd_compress_busy("X"));
    assert!(!app.chd_compress_busy("Y"));

    // A differently-kinded op scoped to the same folder must not trip the guard.
    app.operations.push(BackgroundOperation::new(
        2,
        "Scanning".to_string(),
        Arc::new(AtomicBool::new(false)),
        OperationKind::Scan,
        "Z".to_string(),
        ProgressDisplay::Count,
    ));
    assert!(!app.chd_compress_busy("Z"));
}

#[test]
fn selected_entry_identity_survives_entry_reorder() {
    use std::path::PathBuf;

    use retro_junk_lib::scanner::GameEntry;

    let mut app = RetroJunkApp::with_parts(
        &egui::Context::default(),
        crate::settings::AppSettings::default(),
        None,
        None,
    );
    let first = crate::test_support::test_entry(GameEntry::SingleFile(PathBuf::from("a.rom")));
    let second = crate::test_support::test_entry(GameEntry::SingleFile(PathBuf::from("b.rom")));
    let selected_id = second.id.expect("test entries have durable IDs");
    let console = crate::test_support::test_console("psx", vec![first, second]);
    app.ui_state.selected_console = console.id;
    app.ui_state.selected_entries.insert(selected_id);
    app.browser.consoles.push(console);

    assert_eq!(app.selected_entry_indices(), vec![1]);
    app.browser.consoles[0].entries.reverse();
    assert_eq!(app.selected_entry_indices(), vec![0]);
    assert!(app.ui_state.selected_entries.contains(&selected_id));
}

#[test]
fn changing_detail_focus_releases_every_previous_asset_path() {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use retro_junk_frontend::AssetType;
    use retro_junk_lib::scanner::GameEntry;

    let ctx = egui::Context::default();
    let mut app =
        RetroJunkApp::with_parts(&ctx, crate::settings::AppSettings::default(), None, None);
    let mut first = crate::test_support::test_entry(GameEntry::SingleFile(PathBuf::from("a.rom")));
    let mut second = crate::test_support::test_entry(GameEntry::SingleFile(PathBuf::from("b.rom")));
    let first_id = first.id.unwrap();
    let second_id = second.id.unwrap();
    first.asset_paths = Some(HashMap::from([(
        AssetType::Cover,
        PathBuf::from("/media/a.png"),
    )]));
    second.asset_paths = Some(HashMap::from([(
        AssetType::Cover,
        PathBuf::from("/media/b.png"),
    )]));
    app.browser.consoles.push(crate::test_support::test_console(
        "psx",
        vec![first, second],
    ));
    app.browser.detail_asset_entry = Some(first_id);
    app.ui_state.current_view = View::Library;
    app.ui_state.detail_panel_open = true;
    app.ui_state.focused_entry = Some(second_id);

    app.reconcile_detail_assets(&ctx);

    assert_eq!(app.browser.detail_asset_entry, Some(second_id));
    assert!(
        app.browser.consoles[0]
            .entries
            .iter()
            .all(|entry| entry.asset_paths.is_none()),
        "selection changes must drop paths for old and not-yet-loaded detail entries"
    );

    app.browser.consoles[0].entries[1].asset_paths = Some(HashMap::from([(
        AssetType::Screenshot,
        PathBuf::from("/media/b-shot.png"),
    )]));
    app.ui_state.focused_entry = Some(first_id);
    app.reconcile_detail_assets(&ctx);
    assert!(app.browser.consoles[0].entries[1].asset_paths.is_none());
}

#[test]
fn on_exit_cancels_and_joins_all_operation_threads() {
    // Exercises `RetroJunkApp::cancel_and_join_all_operations`, the part of
    // `on_exit` under test (D2). Calling `on_exit()` itself would write the
    // user's real settings.toml to disk, which this test suite must not do.
    use crate::state::{BackgroundOperation, OperationKind, ProgressDisplay, next_operation_id};

    let mut app = RetroJunkApp::with_parts(
        &egui::Context::default(),
        crate::settings::AppSettings::default(),
        None,
        None,
    );

    let op_id = next_operation_id();
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    app.operations.push(BackgroundOperation::new(
        op_id,
        "test op".to_string(),
        cancel.clone(),
        OperationKind::Other,
        String::new(),
        ProgressDisplay::Count,
    ));
    let handle = std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(5));
    });
    app.op_threads.insert(op_id, handle);

    app.cancel_and_join_all_operations();

    assert!(
        cancel.load(std::sync::atomic::Ordering::Relaxed),
        "on_exit must cancel every in-flight operation"
    );
    assert!(
        app.op_threads.is_empty(),
        "on_exit must join every operation thread"
    );
}

#[test]
fn tools_data_tab_renders_operation_cards() {
    use crate::state::ToolsTab;

    let mut harness = harness();
    harness.run();

    // Navigate to Tools, then the Data tab.
    harness.get_by_label("Tools").click();
    harness.run();
    harness.get_by_label("Data").click();
    harness.run();

    assert_eq!(
        harness.state().ui_state.tools_state.active_tab,
        ToolsTab::Data
    );

    // Each data-gathering operation card should be present.
    harness.get_by_label("Import catalog from DATs");
    harness.get_by_label("Enrich from GameDataBase");
    harness.get_by_label("Enrich from ScreenScraper");
    harness.get_by_label("Reference-data cache");
}
