use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;

use crate::app::RetroJunkApp;
use crate::backend::inbox::InboxSort;
use crate::state::View;

/// A database holding one import suggestion and `strays` adoption reviews,
/// half of them stray text files under `gc/rvz`.
fn database_with_suggestions(strays: usize) -> (tempfile::TempDir, std::path::PathBuf) {
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

    for index in 0..strays {
        let (relative, status) = if index % 2 == 0 {
            (format!("gc/rvz/notes-{index}.txt"), "unmatched")
        } else {
            (format!("psx/Unknown Game {index}.chd"), "unmatched")
        };
        retro_junk_db::work::open_suggestion(
            &mut connection,
            &retro_junk_db::work::NewSuggestion {
                kind: "adopt_playable",
                target_kind: "path",
                target_id: &relative,
                payload_json: &serde_json::json!({
                    "relative_path": relative,
                    "status": status,
                    "detail": "no catalog match",
                })
                .to_string(),
                confidence: 0.1,
                provenance: "cli-adopt",
            },
        )
        .expect("open adoption suggestion");
    }
    (directory, path)
}

fn app_with(path: &std::path::Path) -> Harness<'static, RetroJunkApp> {
    let connection = retro_junk_db::open_database(path).expect("open app connection");
    let app_path = path.to_path_buf();
    let mut harness = Harness::new_eframe(move |cc| {
        RetroJunkApp::with_parts(
            &cc.egui_ctx,
            crate::settings::AppSettings::default(),
            Some(connection),
            Some(app_path.clone()),
        )
    });
    harness.state_mut().ui_state.current_view = View::Inbox;
    // The load is a background thread; step frames until its reply lands.
    // A spinner is on screen the whole time, so this cannot wait for the UI
    // to go quiescent. The sleep matters under a fully loaded machine (the
    // whole workspace's suites in parallel): bare steps can spin through
    // their budget before the reader thread ever gets scheduled. The loop
    // breaks the moment the reply lands, so the healthy case stays fast.
    for _ in 0..400 {
        harness.step();
        if !harness.state().ui_state.inbox.items.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    harness
}

/// The inbox must render every kind of review, and must only offer Apply for
/// the ones the tool can actually execute: an adoption review with no
/// candidate records a decision the user has to make where the files are, and
/// an Apply button there would promise an action that does not exist.
#[test]
fn only_executable_suggestions_offer_apply() {
    let (_directory, path) = database_with_suggestions(2);
    let mut harness = app_with(&path);

    assert_eq!(
        harness.state().ui_state.inbox.items.len(),
        3,
        "all open suggestions should be listed"
    );
    let applicable: Vec<_> = harness
        .state()
        .ui_state
        .inbox
        .items
        .iter()
        .map(|item| (item.suggestion.kind.clone(), item.actions.applicable))
        .collect();
    assert!(applicable.contains(&("import".to_owned(), true)));
    assert!(applicable.contains(&("adopt_playable".to_owned(), false)));

    crate::test_support::settle(&mut harness);
    // "Wipeout" appears both as the card headline and inside its source
    // path; the assertion is that the row rendered at all.
    assert!(
        harness
            .query_all_by_label_contains("Wipeout")
            .next()
            .is_some()
    );
    assert!(
        harness
            .query_all_by_label_contains("Unknown Game 1.chd")
            .next()
            .is_some()
    );
}

/// The reason this view is shaped the way it is: a backlog of hundreds must
/// cost the same to draw as a backlog of ten. Only the rows inside the
/// viewport may be laid out, so a widget count that grows with the backlog
/// means the list stopped being virtualized.
#[test]
fn only_the_visible_rows_are_drawn() {
    let strays = 400;
    let (_directory, path) = database_with_suggestions(strays);
    let mut harness = app_with(&path);
    crate::test_support::settle(&mut harness);

    assert_eq!(
        harness.state().ui_state.inbox.items.len(),
        strays + 1,
        "the whole backlog is loaded"
    );
    // Every stray row's name contains "notes-" or "Unknown Game"; counting the
    // ones that reached the screen counts what was actually laid out.
    let drawn = harness.query_all_by_label_contains("notes-").count()
        + harness.query_all_by_label_contains("Unknown Game").count();
    assert!(
        drawn > 0,
        "the list rendered nothing at all, so this proves nothing"
    );
    assert!(
        drawn < strays / 4,
        "{drawn} of {strays} rows were laid out; the list is drawing rows nobody can see"
    );
}

/// The filter is what makes a bulk button safe: the count in its label has to
/// be the same set the list is showing, or "dismiss 412" means nothing.
#[test]
fn the_filter_selects_what_a_bulk_action_would_take() {
    use retro_junk_backend::suggestions::SuggestionFilter;

    let (_directory, path) = database_with_suggestions(4);
    let harness = app_with(&path);
    let inbox = &harness.state().ui_state.inbox;

    let everything = inbox.visible(&SuggestionFilter::default(), InboxSort::default());
    assert_eq!(everything.len(), 5);

    let text_files = inbox.visible(&SuggestionFilter::new(None, "*.txt"), InboxSort::default());
    assert_eq!(text_files.len(), 2, "two of the four strays are .txt");
    assert!(text_files.iter().all(|item| {
        item.suggestion
            .target_id
            .to_ascii_lowercase()
            .ends_with(".txt")
    }));

    let by_folder = inbox.visible(
        &SuggestionFilter::new(None, "*/rvz/*"),
        InboxSort::default(),
    );
    assert_eq!(by_folder.len(), 2, "the same two, described by directory");

    let imports = inbox.visible(
        &SuggestionFilter::new(Some("import"), ""),
        InboxSort::default(),
    );
    assert_eq!(imports.len(), 1);
}

/// Newest-first is the default because a fresh arrival must not be buried
/// under hundreds of old rows that were already decided against.
#[test]
fn the_default_order_puts_new_arrivals_on_top() {
    use retro_junk_backend::suggestions::SuggestionFilter;

    let (_directory, path) = database_with_suggestions(3);
    let harness = app_with(&path);
    let inbox = &harness.state().ui_state.inbox;

    let newest = inbox.visible(&SuggestionFilter::default(), InboxSort::Newest);
    let oldest = inbox.visible(&SuggestionFilter::default(), InboxSort::Oldest);
    assert_eq!(
        newest.first().map(|item| item.suggestion.id),
        oldest.last().map(|item| item.suggestion.id),
        "the two orders must be reverses of each other"
    );
    // The import was filed first, so it is last under the default order.
    assert_eq!(
        oldest.first().map(|item| item.suggestion.kind.as_str()),
        Some("import")
    );
}
