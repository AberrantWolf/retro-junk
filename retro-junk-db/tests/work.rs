//! Claim mechanics, suggestion supersession, incoming-package states, and
//! the v21 → v22 migration.

use retro_junk_db::work::{
    ClaimOutcome, NewSuggestion, daemon_heartbeat, daemon_started, get_incoming_package,
    get_suggestion, has_recent_error, held_claim, list_incoming_packages, list_open_suggestions,
    observe_incoming_package, open_suggestion, open_suggestion_counts, read_runtime_state,
    refresh_claim, release_claim, remove_incoming_package, reopen_suggestions, resolve_suggestion,
    resolve_suggestions, set_incoming_error, set_incoming_imported, set_incoming_ready, try_claim,
};

fn conn() -> retro_junk_db::Connection {
    retro_junk_db::schema::open_memory().unwrap()
}

#[test]
fn fresh_claim_blocks_other_owners_until_released() {
    let mut db = conn();
    assert!(try_claim(&db, "build", "release", "r1", "daemon").unwrap());
    // A fresh claim is exclusive.
    assert!(!try_claim(&db, "build", "release", "r1", "gui").unwrap());
    // Same target, different action: independent.
    assert!(try_claim(&db, "verify_integrity", "release", "r1", "gui").unwrap());
    let held = held_claim(&db, "build", "release", "r1").unwrap().unwrap();
    assert_eq!(held.owner, "daemon");

    release_claim(
        &mut db,
        "build",
        "release",
        "r1",
        "daemon",
        &ClaimOutcome::Success,
    )
    .unwrap();
    assert!(held_claim(&db, "build", "release", "r1").unwrap().is_none());
    assert!(try_claim(&db, "build", "release", "r1", "gui").unwrap());
}

#[test]
fn stale_claims_are_reaped_and_refresh_extends_only_the_owner() {
    let db = conn();
    assert!(try_claim(&db, "build", "release", "r1", "crashed").unwrap());
    // Refresh by a non-owner must not touch the heartbeat.
    refresh_claim(&db, "build", "release", "r1", "somebody-else").unwrap();
    // Age the claim past the timeout; anyone may take it over.
    db.execute(
        "UPDATE work_claims SET since=datetime('now','-10 minutes')",
        [],
    )
    .unwrap();
    assert!(held_claim(&db, "build", "release", "r1").unwrap().is_none());
    assert!(try_claim(&db, "build", "release", "r1", "daemon").unwrap());
    let held = held_claim(&db, "build", "release", "r1").unwrap().unwrap();
    assert_eq!(held.owner, "daemon");
}

/// A process whose claim went stale and was taken over may finish late and
/// release on its way out; that release must not delete the new owner's live
/// claim, or a third process could claim the same target mid-work.
#[test]
fn release_by_a_former_owner_leaves_the_new_owners_claim() {
    let mut db = conn();
    assert!(try_claim(&db, "build", "release", "r1", "stalled").unwrap());
    db.execute(
        "UPDATE work_claims SET since=datetime('now','-10 minutes')",
        [],
    )
    .unwrap();
    assert!(try_claim(&db, "build", "release", "r1", "daemon").unwrap());
    release_claim(
        &mut db,
        "build",
        "release",
        "r1",
        "stalled",
        &ClaimOutcome::Cancelled,
    )
    .unwrap();
    let held = held_claim(&db, "build", "release", "r1").unwrap().unwrap();
    assert_eq!(held.owner, "daemon");
}

#[test]
fn failed_release_records_error_and_success_clears_it() {
    let mut db = conn();
    let tick_before = read_runtime_state(&db).unwrap().dirty_tick;
    assert!(try_claim(&db, "build", "release", "r1", "daemon").unwrap());
    release_claim(
        &mut db,
        "build",
        "release",
        "r1",
        "daemon",
        &ClaimOutcome::Failed {
            error: "chdman exploded".to_owned(),
        },
    )
    .unwrap();
    assert!(has_recent_error(&db, "build", "release", "r1", 6).unwrap());
    // A backoff window that has already elapsed is not "recent".
    db.execute(
        "UPDATE work_errors SET occurred_at=datetime('now','-7 hours')",
        [],
    )
    .unwrap();
    assert!(!has_recent_error(&db, "build", "release", "r1", 6).unwrap());
    assert!(has_recent_error(&db, "build", "release", "r1", 8).unwrap());

    // Success clears the record entirely.
    assert!(try_claim(&db, "build", "release", "r1", "daemon").unwrap());
    release_claim(
        &mut db,
        "build",
        "release",
        "r1",
        "daemon",
        &ClaimOutcome::Success,
    )
    .unwrap();
    assert!(!has_recent_error(&db, "build", "release", "r1", 24).unwrap());

    // Cancelled releases record nothing.
    assert!(try_claim(&db, "build", "release", "r1", "daemon").unwrap());
    release_claim(
        &mut db,
        "build",
        "release",
        "r1",
        "daemon",
        &ClaimOutcome::Cancelled,
    )
    .unwrap();
    assert!(!has_recent_error(&db, "build", "release", "r1", 24).unwrap());

    let tick_after = read_runtime_state(&db).unwrap().dirty_tick;
    assert!(
        tick_after > tick_before,
        "coordination commits bump the tick"
    );
}

#[test]
fn reopening_a_suggestion_supersedes_the_old_row_and_keeps_history() {
    let mut db = conn();
    let first = open_suggestion(
        &mut db,
        &NewSuggestion {
            kind: "import",
            target_kind: "path",
            target_id: "/incoming/game.bin",
            payload_json: r#"{"plan":1}"#,
            confidence: 0.9,
            provenance: "daemon",
        },
    )
    .unwrap();
    let second = open_suggestion(
        &mut db,
        &NewSuggestion {
            kind: "import",
            target_kind: "path",
            target_id: "/incoming/game.bin",
            payload_json: r#"{"plan":2}"#,
            confidence: 0.95,
            provenance: "daemon",
        },
    )
    .unwrap();
    let open = list_open_suggestions(&db, None).unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].id, second);
    assert_eq!(open[0].payload_json, r#"{"plan":2}"#);
    let superseded = get_suggestion(&db, first).unwrap().unwrap();
    assert_eq!(superseded.resolution, "superseded");
    assert!(superseded.resolved_at.is_some());

    assert!(resolve_suggestion(&mut db, second, "applied").unwrap());
    // Resolving twice is a lost race, not an error.
    assert!(!resolve_suggestion(&mut db, second, "dismissed").unwrap());
    assert!(list_open_suggestions(&db, None).unwrap().is_empty());
}

/// The daemon re-derives every pass and proposes the same thing each time.
/// An identical proposal must keep the open row — its id is what undo holds —
/// and must not bump the dirty tick, which every other process reads as
/// "refresh now".
#[test]
fn refiling_an_identical_suggestion_keeps_the_open_row() {
    let mut db = conn();
    let proposal = NewSuggestion {
        kind: "scrape_review",
        target_kind: "release",
        target_id: "r1",
        payload_json: r#"{"match":"filename"}"#,
        confidence: 0.4,
        provenance: "daemon",
    };
    let first = open_suggestion(&mut db, &proposal).unwrap();
    let tick_before = read_runtime_state(&db).unwrap().dirty_tick;
    let second = open_suggestion(&mut db, &proposal).unwrap();
    assert_eq!(first, second);
    let open = list_open_suggestions(&db, None).unwrap();
    assert_eq!(open.len(), 1);
    let tick_after = read_runtime_state(&db).unwrap().dirty_tick;
    assert_eq!(
        tick_before, tick_after,
        "an unchanged proposal is not a change other processes must refresh for"
    );
}

/// Open one adoption review per path, the way an adoption sweep does.
fn file_reviews(db: &mut retro_junk_db::Connection, paths: &[&str]) -> Vec<i64> {
    paths
        .iter()
        .map(|path| {
            open_suggestion(
                db,
                &NewSuggestion {
                    kind: "adopt_playable",
                    target_kind: "path",
                    target_id: path,
                    payload_json: "{}",
                    confidence: 0.1,
                    provenance: "cli-adopt",
                },
            )
            .unwrap()
        })
        .collect()
}

/// Bulk dismissal reports exactly which rows it closed, which is what makes it
/// undoable: the caller keeps that list rather than trying to reconstruct
/// "whatever the filter matched a moment ago".
#[test]
fn bulk_dismissal_reports_what_it_actually_closed() {
    let mut db = conn();
    let ids = file_reviews(&mut db, &["gc/a.txt", "gc/b.txt", "gc/c.rvz"]);
    // Somebody else got to one of them first.
    assert!(resolve_suggestion(&mut db, ids[1], "applied").unwrap());

    let closed = resolve_suggestions(&mut db, &ids, "dismissed").unwrap();
    assert_eq!(
        closed,
        vec![ids[0], ids[2]],
        "a row someone else resolved is absent, not a failure of the batch"
    );
    assert!(list_open_suggestions(&db, None).unwrap().is_empty());
    assert_eq!(
        get_suggestion(&db, ids[1]).unwrap().unwrap().resolution,
        "applied",
        "the earlier resolution must stand"
    );

    let outcome = reopen_suggestions(&mut db, &closed).unwrap();
    assert!(outcome.is_complete());
    assert_eq!(outcome.reopened, closed);
    assert_eq!(list_open_suggestions(&db, None).unwrap().len(), 2);
}

/// The store allows only one open row per target, so undoing a dismissal after
/// a sweep re-filed the same path would collide. The newer row is the better
/// answer — the undo must say so rather than failing, or blowing up the whole
/// batch on one path.
#[test]
fn undo_yields_to_a_freshly_filed_suggestion() {
    let mut db = conn();
    let ids = file_reviews(&mut db, &["gc/a.txt", "gc/b.txt"]);
    let closed = resolve_suggestions(&mut db, &ids, "dismissed").unwrap();
    // A later sweep files one of them again.
    let refiled = file_reviews(&mut db, &["gc/a.txt"]);

    let outcome = reopen_suggestions(&mut db, &[closed[0], closed[1], 9999]).unwrap();
    assert_eq!(outcome.superseded, vec![closed[0]]);
    assert_eq!(outcome.reopened, vec![closed[1]]);
    assert_eq!(outcome.missing, vec![9999]);
    assert!(!outcome.is_complete());

    let open: Vec<i64> = list_open_suggestions(&db, None)
        .unwrap()
        .into_iter()
        .map(|suggestion| suggestion.id)
        .collect();
    assert_eq!(open, vec![closed[1], refiled[0]]);

    // Undoing twice is harmless: already-open rows count as reopened.
    let again = reopen_suggestions(&mut db, &[closed[1]]).unwrap();
    assert_eq!(again.reopened, vec![closed[1]]);
    assert!(again.is_complete());
}

#[test]
fn counts_by_kind_describe_the_backlog_without_listing_it() {
    let mut db = conn();
    file_reviews(&mut db, &["gc/a.txt", "gc/b.txt"]);
    let import = open_suggestion(
        &mut db,
        &NewSuggestion {
            kind: "import",
            target_kind: "path",
            target_id: "/incoming/game.bin",
            payload_json: "{}",
            confidence: 1.0,
            provenance: "daemon",
        },
    )
    .unwrap();

    assert_eq!(
        open_suggestion_counts(&db).unwrap(),
        vec![("adopt_playable".to_owned(), 2), ("import".to_owned(), 1)]
    );

    assert!(resolve_suggestion(&mut db, import, "applied").unwrap());
    assert_eq!(
        open_suggestion_counts(&db).unwrap(),
        vec![("adopt_playable".to_owned(), 2)],
        "a resolved kind drops out rather than showing zero"
    );
}

#[test]
fn incoming_package_lifecycle_tracks_fingerprint_changes() {
    let mut db = conn();
    // First sighting owes a planning pass.
    assert!(observe_incoming_package(&mut db, "/in/a.bin", "p1", "1000:111").unwrap());
    // Unchanged re-sighting doesn't.
    assert!(!observe_incoming_package(&mut db, "/in/a.bin", "p1", "1000:111").unwrap());

    set_incoming_ready(&mut db, "/in/a.bin", "Game (USA)", r#"{"candidate":1}"#).unwrap();
    let package = get_incoming_package(&db, "/in/a.bin").unwrap().unwrap();
    assert_eq!(package.state, "ready");
    assert_eq!(package.plan_json, r#"{"candidate":1}"#);

    // The file changed under the plan: back to pending, stale plan dropped.
    assert!(observe_incoming_package(&mut db, "/in/a.bin", "p1", "2000:222").unwrap());
    let package = get_incoming_package(&db, "/in/a.bin").unwrap().unwrap();
    assert_eq!(package.state, "pending");
    assert!(package.plan_json.is_empty());

    set_incoming_error(&mut db, "/in/a.bin", "no catalog match").unwrap();
    assert_eq!(list_incoming_packages(&db, Some("error")).unwrap().len(), 1);

    set_incoming_imported(&mut db, "/in/a.bin").unwrap();
    let package = get_incoming_package(&db, "/in/a.bin").unwrap().unwrap();
    assert_eq!(package.state, "imported");

    remove_incoming_package(&mut db, "/in/a.bin").unwrap();
    assert!(get_incoming_package(&db, "/in/a.bin").unwrap().is_none());
}

#[test]
fn daemon_runtime_state_round_trips() {
    let db = conn();
    let state = read_runtime_state(&db).unwrap();
    assert!(state.daemon_pid.is_none());
    daemon_started(&db, 4242).unwrap();
    daemon_heartbeat(&db).unwrap();
    let state = read_runtime_state(&db).unwrap();
    assert_eq!(state.daemon_pid, Some(4242));
    assert!(state.daemon_started_at.is_some());
    assert!(state.daemon_heartbeat_at.is_some());
}

#[test]
fn migration_from_v21_creates_coordination_tables_and_seeds_runtime() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("catalog.db");
    {
        // Start from a fully current database, then rewind it to v21 by
        // dropping exactly what the v21 → v22 migration owns.
        let db = retro_junk_db::open_database(&path).unwrap();
        db.execute_batch(
            "DROP TABLE work_claims; DROP TABLE work_errors; DROP TABLE suggestions;
             DROP TABLE incoming_packages; DROP TABLE runtime_state;
             DELETE FROM schema_version;",
        )
        .unwrap();
        db.execute("INSERT INTO schema_version(version) VALUES(21)", [])
            .unwrap();
    }
    let db = retro_junk_db::open_database(&path).unwrap();
    // Tables exist, the singleton is seeded, and claims work immediately.
    assert_eq!(read_runtime_state(&db).unwrap().dirty_tick, 0);
    assert!(try_claim(&db, "build", "release", "r1", "daemon").unwrap());
    let version: i32 = db
        .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(version, retro_junk_db::schema::CURRENT_VERSION);
}
