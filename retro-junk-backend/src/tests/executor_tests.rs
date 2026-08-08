//! The run's shared archive scan, and what a batch acquires once.

use super::*;

/// A projection action for a console that does not exist — it needs no archive
/// content to reach the executor's coordination, which is what these check.
fn console_action(directory: &str) -> ProposedAction {
    ProposedAction {
        kind: ActionKind::SyncGamelist,
        target: WorkTarget::console("prof", directory),
        profile_id: "prof".to_owned(),
        platform_id: String::new(),
        playable_platform_id: directory.to_owned(),
        label: directory.to_owned(),
        blocked: None,
        build: None,
    }
}

/// An `ExecContext` pointed at a real, empty archive. Only the fields the
/// scan cache touches matter here.
fn context(archive_root: &std::path::Path) -> ExecContext {
    retro_junk_archive::initialize_archive(
        archive_root,
        &retro_junk_archive::ArchiveRootManifest::new("Scan cache"),
    )
    .unwrap();
    let playable_root = archive_root.parent().unwrap().join("playable");
    ExecContext {
        profile: retro_junk_archive::CollectionProfile::for_roots(
            archive_root.to_path_buf(),
            playable_root.clone(),
        ),
        db_path: archive_root.join("catalog.db"),
        tools: ToolPaths::default(),
        scrape: ScrapeSettings::default(),
        roots: FrontendRoots::from_settings(&playable_root, "", ""),
        analyzers: Arc::new(retro_junk_lib::create_default_context()),
        owner: ExecContext::owner_string("test"),
        lock: LockEtiquette::InteractiveWait,
        reconcile: ReconcileMode::AtBatchEnd,
        archive: ArchiveScan::default(),
    }
}

/// The whole point: asking twice walks the archive once. Two actions in the
/// same stage — and the projection stage runs two per release — must not each
/// pay for their own walk.
#[test]
fn asking_twice_returns_the_same_scan() {
    let temp = tempfile::tempdir().unwrap();
    let ctx = context(&temp.path().join("archive"));

    let first = ctx.archive().unwrap();
    let second = ctx.archive().unwrap();

    assert!(
        Arc::ptr_eq(&first, &second),
        "the second ask walked the archive again"
    );
}

/// A scan that outlived a change to the archive would hand the next action a
/// tree that is no longer there, which is the failure this cache could
/// plausibly introduce.
#[test]
fn a_changed_archive_is_scanned_again() {
    let temp = tempfile::tempdir().unwrap();
    let ctx = context(&temp.path().join("archive"));

    let before = ctx.archive().unwrap();
    ctx.archive_changed();
    let after = ctx.archive().unwrap();

    assert!(
        !Arc::ptr_eq(&before, &after),
        "an action changed the archive and the next one read the old scan"
    );
}

/// Reconciling is the act of writing down what is on disk right now, and it
/// runs before the run has marked the old scan stale. It must not be served a
/// cached answer.
#[test]
fn a_rescan_never_accepts_the_cached_answer() {
    let temp = tempfile::tempdir().unwrap();
    let ctx = context(&temp.path().join("archive"));

    let before = ctx.archive().unwrap();
    let fresh = ctx.rescan_archive().unwrap();
    assert!(!Arc::ptr_eq(&before, &fresh), "the rescan reused the cache");

    // ...and the fresh one is kept, so the next action does not walk again.
    assert!(
        Arc::ptr_eq(&fresh, &ctx.archive().unwrap()),
        "the rescan's result was thrown away instead of shared"
    );
}

/// Only the two projection kinds leave the archive as they found it. Everything
/// else appends evidence, publishes a dump, or renames an output — and a kind
/// added later must default to "changed" rather than silently reading a stale
/// tree.
#[test]
fn only_the_projection_kinds_leave_the_archive_alone() {
    for kind in ActionKind::all() {
        let expected = !matches!(kind, ActionKind::ProjectAssets | ActionKind::SyncGamelist);
        assert_eq!(
            changes_the_archive(*kind),
            expected,
            "{kind:?} is on the wrong side of the archive-mutation rule"
        );
    }
}

/// Scraping owns a narrower release-file projection update, so the batch
/// worker must not follow it with the whole-archive reconcile that caused the
/// original one-artwork refresh stall.
#[test]
fn scraping_does_not_request_a_full_reconcile() {
    assert!(changes_the_archive(ActionKind::Scrape));
    assert!(!requires_full_reconcile(ActionKind::Scrape));
    assert!(requires_full_reconcile(ActionKind::VerifyIntegrity));
}

/// The lock is taken for the batch, not for each item. Proven from the outside:
/// with the archive already held by someone else, *every* item reports busy —
/// if the lock were still per item, only the first would, and the rest would
/// each go on to contend separately.
#[test]
fn the_archive_lock_governs_the_whole_batch() {
    let temp = tempfile::tempdir().unwrap();
    let archive_root = temp.path().join("archive");
    let mut ctx = context(&archive_root);
    // The daemon's etiquette, so a busy archive is reported rather than waited
    // on — an interactive wait would block this test forever, which is itself
    // the behavior being relied on.
    ctx.lock = LockEtiquette::DaemonFailFast;

    let held = retro_junk_archive::ArchiveLock::acquire(&archive_root).expect("lock the archive");
    let batch = [console_action("psx"), console_action("snes")];
    let outcomes = execute_actions(
        &ctx,
        &batch,
        &crate::ops::SILENT_PROGRESS,
        &AtomicBool::new(false),
    )
    .expect("the batch reports rather than errors");
    drop(held);

    assert_eq!(outcomes.len(), batch.len(), "one outcome per action");
    assert!(
        outcomes
            .iter()
            .all(|outcome| matches!(outcome, ActionOutcome::ArchiveBusy)),
        "a busy archive stops the batch, not just its first item: {outcomes:?}"
    );
}

/// Cancelling stops the batch where it is and says so for the rest, rather
/// than reporting work that never ran as failed.
#[test]
fn a_cancelled_batch_reports_every_remaining_item() {
    let temp = tempfile::tempdir().unwrap();
    let ctx = context(&temp.path().join("archive"));
    let batch = [console_action("psx"), console_action("snes")];

    let outcomes = execute_actions(
        &ctx,
        &batch,
        &crate::ops::SILENT_PROGRESS,
        &AtomicBool::new(true),
    )
    .expect("cancellation is an outcome, not an error");

    assert_eq!(outcomes.len(), batch.len());
    assert!(
        outcomes
            .iter()
            .all(|outcome| matches!(outcome, ActionOutcome::Cancelled)),
        "{outcomes:?}"
    );
}

/// An empty batch must not open a connection or reach for the lock — the
/// worker hands over an empty stage on every idle daemon tick.
#[test]
fn an_empty_batch_touches_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let mut ctx = context(&temp.path().join("archive"));
    ctx.db_path = temp.path().join("does-not-exist").join("catalog.db");

    let outcomes = execute_actions(
        &ctx,
        &[],
        &crate::ops::SILENT_PROGRESS,
        &AtomicBool::new(false),
    )
    .expect("an empty batch cannot fail");
    assert!(outcomes.is_empty());
}
