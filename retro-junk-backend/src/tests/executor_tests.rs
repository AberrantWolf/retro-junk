//! The run's shared archive scan, and when it must be thrown away.

use super::*;

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
