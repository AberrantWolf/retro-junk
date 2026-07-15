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
use crate::state::{BrowseTable, ToolsTab, View};

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
    harness.run();

    // ── Tools/Browse: toggling the bottom log viewer must not re-id the view ──
    harness.state_mut().current_view = View::Tools;
    harness.state_mut().tools_state.active_tab = ToolsTab::Browse;
    for _ in 0..4 {
        harness.run();
    }

    reset();
    for _ in 0..4 {
        let cur = harness.state().log_viewer.open;
        harness.state_mut().log_viewer.open = !cur;
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
    harness.state_mut().log_viewer.open = true;
    for _ in 0..3 {
        harness.run();
    }
    reset();
    for &table in BrowseTable::ALL {
        harness.state_mut().tools_state.browse.active_table = table;
        harness
            .state_mut()
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
    harness.state_mut().log_viewer.open = false;
    harness.state_mut().current_view = View::Library;
    harness.state_mut().root_path = Some(std::path::PathBuf::from("/nonexistent/library"));
    for _ in 0..4 {
        harness.run();
    }
    reset();
    for _ in 0..4 {
        let cur = harness.state().detail_panel_open;
        harness.state_mut().detail_panel_open = !cur;
        for _ in 0..3 {
            harness.run();
        }
    }
    assert_eq!(
        clashes(),
        0,
        "toggling the detail panel re-assigned widget ids in the Library view"
    );
}
