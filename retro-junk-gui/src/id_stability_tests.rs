//! Regression tests for widget-id stability across panel toggles.
//!
//! Toggling a conditional panel (log viewer, detail pane) changes the sibling
//! count on the parent `Ui`, which would shift the auto-id of a plain
//! `CentralPanel` and re-id every widget inside it for a frame. egui's debug
//! `warn_if_rect_changes_id` check then flashes red rectangles and logs a
//! "changed id between passes" warning per widget. `util::stable_central_panel`
//! pins those ids; these tests assert the warnings stay gone.
//!
//! The `log` logger is process-global and set-once, so a single test owns it and
//! all id-stability scenarios run here sequentially.

use std::sync::Mutex;

use egui_kittest::Harness;

use super::RetroJunkApp;
use crate::state::{BrowseTable, FocusedPanel, ToolsTab, View};

static CAPTURED: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct CaptureLogger;
impl log::Log for CaptureLogger {
    fn enabled(&self, _m: &log::Metadata) -> bool {
        true
    }
    fn log(&self, record: &log::Record) {
        if record.level() <= log::Level::Warn {
            CAPTURED.lock().unwrap().push(format!("{}", record.args()));
        }
    }
    fn flush(&self) {}
}

static LOGGER: CaptureLogger = CaptureLogger;

/// Number of "changed id between passes" warnings captured since the last clear.
fn clashes() -> usize {
    CAPTURED
        .lock()
        .unwrap()
        .iter()
        .filter(|m| m.contains("changed id between passes"))
        .count()
}

fn reset() {
    CAPTURED.lock().unwrap().clear();
}

#[test]
#[cfg(debug_assertions)]
fn panel_toggles_do_not_reassign_widget_ids() {
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Warn);

    let conn = retro_junk_db::open_memory().expect("open memory db");
    let mut harness = Harness::new_eframe(|cc| {
        RetroJunkApp::with_parts(
            &cc.egui_ctx,
            crate::settings::AppSettings::default(),
            Some(conn),
            None,
        )
    });
    // Production disables this heuristic because it cannot distinguish a
    // recycled virtual-list viewport slot from an unstable widget. Re-enable
    // it here so the deterministic panel/layout cases remain guarded.
    harness
        .ctx
        .all_styles_mut(|style| style.debug.warn_if_rect_changes_id = true);
    harness.run();

    // ── Tools/Browse: toggling the bottom log viewer must not re-id the view ──
    harness.state_mut().ui_state.current_view = View::Tools;
    harness.state_mut().ui_state.tools_state.active_tab = ToolsTab::Browse;
    for _ in 0..4 {
        harness.run();
    }

    reset();
    for _ in 0..4 {
        let cur = harness.state().ui_state.log_viewer.open;
        harness.state_mut().ui_state.log_viewer.open = !cur;
        for _ in 0..3 {
            harness.run();
        }
    }
    assert_eq!(
        clashes(),
        0,
        "toggling the log viewer re-assigned widget ids in the Tools view"
    );

    // ── Switching browse tables (with the log open) must stay clean too ──
    harness.state_mut().ui_state.log_viewer.open = true;
    for _ in 0..3 {
        harness.run();
    }
    reset();
    for &table in BrowseTable::ALL {
        harness.state_mut().ui_state.tools_state.browse.active_table = table;
        harness
            .state_mut()
            .ui_state
            .tools_state
            .browse
            .table_state
            .needs_query = true;
        for _ in 0..3 {
            harness.run();
        }
    }
    assert_eq!(clashes(), 0, "cycling browse tables re-assigned widget ids");

    // ── Library: toggling the right-hand detail panel must not re-id the table ──
    harness.state_mut().ui_state.log_viewer.open = false;
    harness.state_mut().ui_state.current_view = View::Library;
    harness.state_mut().root_path = Some(std::path::PathBuf::from("/nonexistent/library"));
    for _ in 0..4 {
        harness.run();
    }
    reset();
    for _ in 0..4 {
        let cur = harness.state().ui_state.detail_panel_open;
        harness.state_mut().ui_state.detail_panel_open = !cur;
        for _ in 0..3 {
            harness.run();
        }
    }
    assert_eq!(
        clashes(),
        0,
        "toggling the detail panel re-assigned widget ids in the Library view"
    );

    // ── Library: virtualized rows must not flash red when the viewport moves ──
    let console = crate::test_support::test_console("psx", Vec::new());
    let console_id = console.id.unwrap();
    let rows = (0..300)
        .map(|index| retro_junk_db::LibraryEntryListItem {
            id: retro_junk_db::LibraryEntryId(index + 1),
            display_name: format!("Game {index:03}.bin"),
            status: "unknown".into(),
            tag: String::new(),
            region_override: String::new(),
            data_size: 0,
            crc32: String::new(),
            dat_game_name: String::new(),
            serial: String::new(),
            internal_name: String::new(),
            detected_regions: Vec::new(),
            has_hash_warnings: false,
            has_broken_references: false,
            has_cue_compat_issues: false,
            revision: 0,
            source_revision: 0,
            archived: false,
            archive_complete: false,
            playable_format: "bin".into(),
            preferred_format: None,
            archive_release_id: None,
        })
        .collect();
    harness.state_mut().browser.consoles = vec![console];
    harness.state_mut().browser.active_page = Some(retro_junk_db::LibraryEntryListPage {
        console_id,
        console_revision: 0,
        total_count: 300,
        logical_count: 300,
        counts: Default::default(),
        availability_counts: Default::default(),
        archived_playable_gaps: Vec::new(),
        archived_releases: Vec::new(),
        offset: 0,
        rows,
    });
    harness.state_mut().ui_state.selected_console = Some(console_id);
    harness.state_mut().ui_state.focused_entry = Some(retro_junk_db::LibraryEntryId(1));
    harness.state_mut().ui_state.focused_panel = FocusedPanel::GameTable;
    harness.state_mut().ui_state.detail_panel_open = false;
    for _ in 0..3 {
        harness.run();
    }
    reset();
    harness.event(egui::Event::PointerMoved(egui::pos2(600.0, 400.0)));
    for _ in 0..8 {
        harness.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            // Five 22-point rows: this makes different virtual rows occupy
            // exactly the same screen rectangles on consecutive frames.
            delta: egui::vec2(0.0, -110.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        });
        harness.run();
    }
    assert_eq!(
        clashes(),
        0,
        "scrolling a large game table re-assigned widget ids at viewport slots"
    );
}
