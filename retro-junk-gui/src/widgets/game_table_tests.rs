//! Interaction guards for the `egui_table` game table.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;

use crate::app::RetroJunkApp;
use crate::state::View;

fn projected_rows(count: u64) -> Vec<retro_junk_db::LibraryEntryListItem> {
    (0..count)
        .map(|index| retro_junk_db::LibraryEntryListItem {
            id: retro_junk_db::LibraryEntryId(index + 1),
            display_name: format!("Game {index:03}.bin"),
            status: "unknown".into(),
            tag: String::new(),
            region_override: String::new(),
            data_size: 0,
            dat_game_name: String::new(),
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
        .collect()
}

fn library_harness<'a>(row_count: u64) -> Harness<'a, RetroJunkApp> {
    let conn = retro_junk_db::open_memory().expect("open memory db");
    let mut harness = Harness::new_eframe(|cc| {
        RetroJunkApp::with_parts(
            &cc.egui_ctx,
            crate::settings::AppSettings::default(),
            Some(conn),
            None,
        )
    });
    let console = crate::test_support::test_console("psx", Vec::new());
    let console_id = console.id.unwrap();
    harness.state_mut().browser.consoles = vec![console];
    harness.state_mut().browser.active_page = Some(retro_junk_db::LibraryEntryListPage {
        console_id,
        console_revision: 0,
        total_count: row_count,
        logical_count: row_count,
        counts: retro_junk_db::LibraryEntryCounts::default(),
        availability_counts: retro_junk_db::LibraryAvailabilityCounts::default(),
        archived_playable_gaps: Vec::new(),
        archived_releases: Vec::new(),
        offset: 0,
        rows: projected_rows(row_count),
    });
    harness.state_mut().ui_state.current_view = View::Library;
    harness.state_mut().ui_state.selected_console = Some(console_id);
    harness.state_mut().ui_state.detail_panel_open = false;
    harness.state_mut().root_path = Some(std::path::PathBuf::from("/nonexistent/library"));
    for _ in 0..3 {
        crate::test_support::settle(&mut harness);
    }
    harness
}

/// Rect of a column header. Sticky columns render their header once per
/// scroll region, so take the first match.
fn header_rect(harness: &Harness<'_, RetroJunkApp>, header: &str) -> egui::Rect {
    harness
        .query_all_by_label(header)
        .next()
        .unwrap_or_else(|| panic!("no '{header}' column header rendered"))
        .rect()
}

fn click_at(harness: &mut Harness<'_, RetroJunkApp>, position: egui::Pos2) {
    harness.event(egui::Event::PointerMoved(position));
    crate::test_support::settle(harness);
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
    }
    crate::test_support::settle(harness);
}

/// `egui_table` splits a row across two scroll regions when there are sticky
/// columns: the frozen columns render in one pass, the scrolling ones in
/// another, each with its own row `Ui`. A click has to select the row from
/// either side — de-duplicating row interaction across the two passes
/// silently swallows every click that lands on the frozen columns.
#[test]
fn clicking_either_the_frozen_or_the_scrolling_region_selects_the_row() {
    let mut harness = library_harness(40);
    // First data row, just below the sticky header.
    let y = header_rect(&harness, "Name").bottom() + 8.0;

    for (region, header) in [("frozen", "Name"), ("scrolling", "Availability")] {
        harness.state_mut().ui_state.selected_entries.clear();
        harness.state_mut().ui_state.focused_entry = None;
        crate::test_support::settle(&mut harness);

        let x = header_rect(&harness, header).center().x;
        click_at(&mut harness, egui::pos2(x, y));

        assert_eq!(
            harness.state().ui_state.selected_entries.len(),
            1,
            "a click on the {region} region of a row must select exactly that row"
        );
    }
}
