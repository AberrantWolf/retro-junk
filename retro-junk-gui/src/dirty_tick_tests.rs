//! The cross-process refresh signal (B7).

use egui_kittest::Harness;

use super::RetroJunkApp;

/// Startup must not treat the database's existing change count as news.
///
/// Every write since the database was created has already bumped the tick,
/// so a first poll that compared against zero would schedule a full
/// projection refresh on every launch — undoing the paint-from-committed-
/// projections startup path Phase A exists to protect.
#[test]
fn the_first_poll_records_the_tick_without_refreshing() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("catalog.db");
    let mut writer = retro_junk_db::open_database(&path).expect("open database");
    // Stand in for writes an earlier session (or the daemon) already made.
    for index in 0..3 {
        retro_junk_db::work::open_suggestion(
            &mut writer,
            &retro_junk_db::work::NewSuggestion {
                kind: "import",
                target_kind: "path",
                target_id: &format!("/incoming/{index}"),
                payload_json: "{}",
                confidence: 1.0,
                provenance: "test",
            },
        )
        .expect("open suggestion");
    }
    let observed_before = retro_junk_db::work::read_runtime_state(&writer)
        .expect("runtime state")
        .dirty_tick;
    assert!(observed_before > 0, "the fixture must have dirty writes");

    let connection = retro_junk_db::open_database(&path).expect("open app connection");
    let app_path = path.clone();
    let mut harness = Harness::new_eframe(move |cc| {
        RetroJunkApp::with_parts(
            &cc.egui_ctx,
            crate::settings::AppSettings::default(),
            Some(connection),
            Some(app_path.clone()),
        )
    });
    // A backlog scope is what a refresh would act on; leaving it set makes
    // "did it refresh?" observable through `backlog_loading`.
    harness.state_mut().ui_state.backlog_scope = Some(retro_junk_db::convergence::Scope::Profile(
        "prof".to_owned(),
    ));
    harness.run();

    assert_eq!(
        harness.state().ui_state.dirty_tick,
        Some(observed_before),
        "the first poll should adopt the database's current tick"
    );
    assert!(
        !harness.state().ui_state.backlog_loading,
        "the first poll must not schedule a refresh"
    );

    // Another process commits: the next poll must notice.
    retro_junk_db::work::open_suggestion(
        &mut writer,
        &retro_junk_db::work::NewSuggestion {
            kind: "import",
            target_kind: "path",
            target_id: "/incoming/daemon",
            payload_json: "{}",
            confidence: 1.0,
            provenance: "daemon",
        },
    )
    .expect("open suggestion");
    // Skip the 1 Hz throttle rather than sleeping through it.
    harness.state_mut().ui_state.last_dirty_poll = None;
    harness.run();

    assert!(
        harness.state().ui_state.dirty_tick > Some(observed_before),
        "a write by another process should advance the observed tick"
    );
}
