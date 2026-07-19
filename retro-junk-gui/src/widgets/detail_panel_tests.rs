//! Regression test: long values in the detail panel must wrap instead of
//! widening the panel.
//!
//! egui persists a side panel's width as the laid-out width of its content
//! (`PanelState`), so a single non-wrapping label permanently grows the panel
//! frame by frame until it hits its max width — overriding any width the user
//! dragged and clipping the rest of the content. Render an entry whose
//! filename/hashes/warnings are far wider than the panel and assert the
//! stored panel width stays at its default.

use std::path::PathBuf;

use egui_kittest::Harness;

use crate::app::RetroJunkApp;
use crate::state::{LibraryBrowserState, View};
use crate::test_support::{test_console, test_entry};

#[test]
fn long_values_wrap_instead_of_widening_the_panel() {
    let mut harness = Harness::new_eframe(|cc| {
        RetroJunkApp::with_parts(
            &cc.egui_ctx,
            crate::settings::AppSettings::default(),
            None,
            None,
        )
    });

    let long_name = "Some Absurdly Long Game Title That Definitely Exceeds The Panel \
                     Width (USA) (Disc 1) (Rev 2) [Translation Patch v1.2.3].cue";
    let mut entry = test_entry(retro_junk_lib::scanner::GameEntry::SingleFile(
        PathBuf::from("/roms/psx").join(long_name),
    ));
    entry.hashes = Some(retro_junk_dat::FileHashes {
        crc32: "deadbeef".to_string(),
        sha1: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
        md5: Some("0123456789abcdef0123456789abcdef".to_string()),
        data_size: 123_456_789,
        warnings: vec![
            "an unbroken-ish long warning message that also has to wrap instead of \
             pushing the panel wider and wider every frame"
                .to_string(),
        ],
    });
    // Media-scan sentinel: pretend assets were already scanned so the panel
    // doesn't spawn a background loader thread against the fake root path.
    entry.asset_paths = Some(std::collections::HashMap::new());

    let state = harness.state_mut();
    state.ui_state.current_view = View::Library;
    state.root_path = Some(PathBuf::from("/roms"));
    state.browser = LibraryBrowserState {
        consoles: vec![test_console("psx", vec![entry])],
        root_id: None,
        active_page: None,
        entry_counts: std::collections::HashMap::new(),
        console_statuses: std::collections::HashMap::new(),
        stale_consoles: std::collections::HashSet::new(),
        asset_discovery_in_flight: std::collections::HashSet::new(),
    };
    state.ui_state.selected_console = state.browser.consoles[0].id;
    state.ui_state.focused_entry = state.browser.consoles[0].entries[0].id;
    state.ui_state.detail_panel_open = true;

    // Give the panel several frames: the buggy feedback loop (content lays
    // out wider -> panel persists the wider size -> repeat) grows one step
    // per frame.
    for _ in 0..8 {
        harness.run();
    }

    let panel =
        egui::containers::panel::PanelState::load(&harness.ctx, egui::Id::new("detail_panel"))
            .expect("detail panel should have stored its state");
    // `.default_size(280.0)` in views/library.rs; anything meaningfully wider
    // means some content refused to wrap and pushed the panel out.
    assert!(
        panel.outer_rect.width() <= 281.0,
        "detail panel grew to {:.0}px — some content is not wrapping",
        panel.outer_rect.width()
    );
}
