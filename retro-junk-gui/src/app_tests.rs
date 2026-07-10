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
