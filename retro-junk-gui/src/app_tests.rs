//! Headless GUI smoke tests driven through `egui_kittest`.
//!
//! These build the app via `RetroJunkApp::with_parts` with default settings
//! and no catalog DB, so they never touch the user's config or cache dirs.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::RetroJunkApp;
use crate::state::View;
use crate::widgets::icons;

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
    crate::test_support::settle(&mut harness);

    // Sidebar navigation entries (icon + label, per `widgets::icons`)
    harness.get_by_label(&icons::labeled(icons::LIBRARY, "Library"));
    harness.get_by_label(&icons::labeled(icons::SETTINGS, "Settings"));
    harness.get_by_label(&icons::labeled(icons::TOOLS, "Tools"));

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
    crate::test_support::settle(&mut harness);

    harness.get_by_label_contains("Settings").click();
    crate::test_support::settle(&mut harness);

    assert_eq!(harness.state().ui_state.current_view, View::Settings);
    harness.get_by_label("Current root:");
}

#[test]
fn collection_row_opens_resizable_details_with_log_viewer_open() {
    let temp = tempfile::tempdir().unwrap();
    let archive_root = temp.path().join("archive");
    let playable_root = temp.path().join("roms");
    let workspace_root = temp.path().join("work");
    let archive_manifest = retro_junk_archive::ArchiveRootManifest::new("Test collection");
    retro_junk_archive::initialize_archive(&archive_root, &archive_manifest).unwrap();
    let profile = retro_junk_archive::CollectionProfile {
        profile_id: archive_manifest.profile_id,
        display_name: "Test collection".to_owned(),
        archive_root: archive_root.clone(),
        playable_root: playable_root.clone(),
        workspace_root: workspace_root.clone(),
        network_mode: true,
        platform_defaults: Vec::new(),
        incoming_roots: Vec::new(),
        watch_backend: retro_junk_archive::WatchBackend::default(),
    };
    let mut settings = crate::settings::AppSettings::default();
    settings.library.current_profile = Some(profile.profile_id);
    settings.library.profiles.push(profile);

    let conn = retro_junk_db::open_memory().unwrap();
    conn.execute("INSERT INTO platforms(id,display_name,short_name,manufacturer,generation,media_type,release_year,description,core_platform) VALUES('nes','NES','NES','Nintendo',3,'cartridge',1983,'','Nes')", []).unwrap();
    conn.execute(
        "INSERT INTO works(id,canonical_name) VALUES('work','Game')",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO releases(id,work_id,platform_id,region,title) VALUES('catalog-release','work','nes','usa','Game')", []).unwrap();
    conn.execute("INSERT INTO media(id,release_id,dat_source) VALUES('catalog-media','catalog-release','no-intro')", []).unwrap();
    conn.execute(
        "INSERT INTO archive_profiles(id,display_name,manifest_path,manifest_sha256,archive_root,playable_root,workspace_root)
         VALUES(?1,'Test collection','retro-junk-archive.toml','hash',?2,?3,?4)",
        (
            archive_manifest.profile_id.to_string(),
            archive_root.to_string_lossy().into_owned(),
            playable_root.to_string_lossy().into_owned(),
            workspace_root.to_string_lossy().into_owned(),
        ),
    )
    .unwrap();
    conn.execute(
        "INSERT INTO archive_releases(id,profile_id,catalog_release_id,platform_id,title,region,manifest_path,manifest_sha256,binding_state)
         VALUES('archive-release',?1,'catalog-release','nes','Game','usa','nes/game/release.toml','hash','resolved')",
        [archive_manifest.profile_id.to_string()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO physical_copies(id,archive_release_id,copy_number,manifest_path,manifest_sha256)
         VALUES('physical-copy','archive-release',1,'nes/game/physical/copy-01/physical-copy.toml','hash')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO carriers(id,physical_copy_id,catalog_media_id,kind,manifest_path,manifest_sha256,binding_state)
         VALUES('carrier','physical-copy','catalog-media','cartridge','nes/game/physical/copy-01/carrier/carrier.toml','hash','resolved')",
        [],
    )
    .unwrap();

    let mut harness = Harness::new_eframe(|cc| {
        RetroJunkApp::with_parts(&cc.egui_ctx, settings, Some(conn), None)
    });
    harness.state_mut().ui_state.current_view = View::Collection;
    // Table-row responses report a placeholder rect to accessibility on their
    // first frame; run a second frame so the synthetic click below lands on
    // the row's real rect.
    harness.run_steps(2);
    harness.get_by_label("Game (usa)").click();
    // The hermetic harness has no library store, so the details pane keeps
    // its loading spinner (and repaint requests) alive; step a bounded number
    // of frames instead of running to quiescence.
    harness.run_steps(4);

    assert_eq!(
        harness
            .state()
            .ui_state
            .collection_editor
            .as_ref()
            .map(|editor| editor.archive_release_id.as_str()),
        Some("archive-release")
    );
    harness.get_by_label("Physical copy and playable policy");

    harness.state_mut().ui_state.log_viewer.open = true;
    harness.run_steps(4);
    harness.get_by_label("Physical copy and playable policy");
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
fn archive_detail_owns_grouped_playable_assets() {
    use std::path::PathBuf;

    use retro_junk_db::{
        ArchiveReleaseSummary, ArchivedLibraryListItem, ArchivedPlayableLibraryEntry,
        LibraryAvailabilityCounts, LibraryEntryCounts, LibraryEntryListPage,
    };
    use retro_junk_lib::scanner::GameEntry;

    let ctx = egui::Context::default();
    let mut app =
        RetroJunkApp::with_parts(&ctx, crate::settings::AppSettings::default(), None, None);
    let entry = crate::test_support::test_entry(GameEntry::SingleFile(PathBuf::from("game.chd")));
    let entry_id = entry.id.unwrap();
    let console = crate::test_support::test_console("psx", vec![entry]);
    let console_id = console.id.unwrap();
    app.browser.consoles.push(console);
    app.browser.active_page = Some(LibraryEntryListPage {
        console_id,
        console_revision: 0,
        total_count: 1,
        logical_count: 1,
        counts: LibraryEntryCounts::default(),
        availability_counts: LibraryAvailabilityCounts::default(),
        archived_playable_gaps: Vec::new(),
        archived_releases: vec![ArchivedLibraryListItem {
            summary: ArchiveReleaseSummary {
                archive_release_id: "archive-release".to_owned(),
                catalog_release_id: None,
                platform_id: "psx".to_owned(),
                title: "Game".to_owned(),
                region: "usa".to_owned(),
                revision: String::new(),
                physical_copy_count: 1,
                carrier_count: 1,
                dump_count: 1,
                preservation_count: 1,
                preservation_present_count: 1,
                playable_count: 1,
                playable_present_count: 1,
                playable_missing_count: 0,
                desired_playable_count: 1,
                satisfied_playable_count: 1,
                integrity_verified_count: 1,
                reproduction_verified_count: 1,
                catalog_verified_count: 1,
                round_trip_verified_count: 0,
            },
            action: None,
            playable_representations: Vec::new(),
            playable_library_entries: vec![ArchivedPlayableLibraryEntry {
                id: entry_id,
                display_name: "game.chd".to_owned(),
                playable_format: "chd".to_owned(),
            }],
            archived_assets: Vec::new(),
            scrape_identity: None,
            facts: retro_junk_db::facts::ReleaseFacts {
                archive_release_id: "archive-release".to_owned(),
                platform_id: "psx".to_owned(),
                title: "Game".to_owned(),
                region: "usa".to_owned(),
                revision: String::new(),
                catalog_release_id: None,
                catalog_work_id: None,
                expected_discs: None,
                carriers: Vec::new(),
                desired_playables: 1,
                satisfied_playables: 1,
                missing_playables: 0,
                archived_asset_types: Vec::new(),
                playable_names: Vec::new(),
            },
        }],
        offset: 0,
        rows: Vec::new(),
    });
    app.ui_state.current_view = View::Library;
    app.ui_state.detail_panel_open = true;
    app.ui_state.selected_console = Some(console_id);
    app.ui_state.focused_entry = None;
    app.ui_state.focused_archive_release = Some("archive-release".to_owned());
    app.ui_state
        .selected_archive_releases
        .insert("archive-release".to_owned());
    app.ui_state.selected_entries.insert(entry_id);

    app.reconcile_detail_assets(&ctx);

    assert_eq!(app.selected_library_row_count(), 1);
    assert_eq!(app.browser.detail_asset_entry, Some(entry_id));

    app.ui_state
        .selected_entries
        .insert(retro_junk_db::LibraryEntryId(999));
    app.reconcile_detail_assets(&ctx);
    assert_eq!(app.selected_library_row_count(), 2);
    assert_eq!(app.browser.detail_asset_entry, None);
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
    crate::test_support::settle(&mut harness);

    // Navigate to Tools, then the Data tab.
    harness.get_by_label_contains("Tools").click();
    crate::test_support::settle(&mut harness);
    harness.get_by_label("Data").click();
    crate::test_support::settle(&mut harness);

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
