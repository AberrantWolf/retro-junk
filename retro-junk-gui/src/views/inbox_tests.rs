use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;

use crate::app::RetroJunkApp;
use crate::state::View;

fn database_with_suggestions() -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("catalog.db");
    let mut connection = retro_junk_db::open_database(&path).expect("open database");
    let payload = serde_json::json!([{
        "source": "/incoming/Wipeout",
        "title": "Wipeout",
        "platform_id": "psx",
        "region": "usa",
        "identification": "CatalogVerified",
        "disposition": "Ready",
        "package_sha256": "abc",
    }])
    .to_string();
    retro_junk_db::work::open_suggestion(
        &mut connection,
        &retro_junk_db::work::NewSuggestion {
            kind: "import",
            target_kind: "path",
            target_id: "/incoming/Wipeout",
            payload_json: &payload,
            confidence: 1.0,
            provenance: "daemon",
        },
    )
    .expect("open import suggestion");
    retro_junk_db::work::open_suggestion(
        &mut connection,
        &retro_junk_db::work::NewSuggestion {
            kind: "adopt_playable",
            target_kind: "path",
            target_id: "psx/Unknown Game.chd",
            payload_json: &serde_json::json!({
                "relative_path": "psx/Unknown Game.chd",
                "status": "unmatched",
                "detail": "no catalog match",
            })
            .to_string(),
            confidence: 0.1,
            provenance: "cli-adopt",
        },
    )
    .expect("open adoption suggestion");
    (directory, path)
}

/// The inbox must render both suggestion kinds, and must only offer Apply
/// for the one the tool can actually execute: an adoption review records a
/// decision about files the user has to resolve, and an Apply button there
/// would promise an action that does not exist.
#[test]
fn only_executable_suggestions_offer_apply() {
    let (_directory, path) = database_with_suggestions();
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
    harness.state_mut().ui_state.current_view = View::Inbox;
    // The load is a background thread; run until its reply lands.
    for _ in 0..40 {
        harness.run();
        if !harness.state().ui_state.inbox.items.is_empty() {
            break;
        }
    }

    assert_eq!(
        harness.state().ui_state.inbox.items.len(),
        2,
        "both open suggestions should be listed"
    );
    let applicable: Vec<_> = harness
        .state()
        .ui_state
        .inbox
        .items
        .iter()
        .map(|item| (item.suggestion.kind.clone(), item.applicable))
        .collect();
    assert!(applicable.contains(&("import".to_owned(), true)));
    assert!(applicable.contains(&("adopt_playable".to_owned(), false)));

    harness.run();
    // "Wipeout" appears both as the card headline and inside its source
    // path; the assertion is that the card rendered at all.
    assert!(
        harness
            .query_all_by_label_contains("Wipeout")
            .next()
            .is_some()
    );
    assert!(
        harness
            .query_all_by_label_contains("Unknown Game.chd")
            .next()
            .is_some()
    );
}
