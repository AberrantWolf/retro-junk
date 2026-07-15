//! Headless GUI smoke tests driven through egui_kittest.
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
fn sidebar_click_switches_to_settings_view() {
    let mut harness = harness();
    harness.run();

    harness.get_by_label("Settings").click();
    harness.run();

    assert_eq!(harness.state().current_view, View::Settings);
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
        Some("X".to_string()),
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
        Some("Z".to_string()),
        ProgressDisplay::Count,
    ));
    assert!(!app.chd_compress_busy("Z"));
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
        None,
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

    assert_eq!(harness.state().tools_state.active_tab, ToolsTab::Data);

    // Each data-gathering operation card should be present.
    harness.get_by_label("Import catalog from DATs");
    harness.get_by_label("Enrich from GameDataBase");
    harness.get_by_label("Enrich from ScreenScraper");
    harness.get_by_label("Reference-data cache");
}
