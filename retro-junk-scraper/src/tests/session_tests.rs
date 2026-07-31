use super::*;

fn present(types: &[(AssetType, &str)]) -> HashMap<AssetType, PathBuf> {
    types
        .iter()
        .map(|(asset_type, path)| (*asset_type, PathBuf::from(path)))
        .collect()
}

/// The behavior this consolidation exists to fix: the folder scrape used to
/// skip a game outright once it had a screenshot, so a library scraped before
/// 3D boxes were selected could never acquire them.
#[test]
fn a_partially_scraped_game_still_fetches_what_it_lacks() {
    let selection = AssetSelection {
        types: vec![AssetType::Screenshot, AssetType::Cover, AssetType::Cover3D],
    };
    let on_disk = present(&[(AssetType::Screenshot, "screenshots/game.png")]);

    let wanted = types_to_fetch(&selection, &on_disk, false);

    assert_eq!(wanted.types, vec![AssetType::Cover, AssetType::Cover3D]);
}

/// A fully-covered game costs no request at all — the quota saving the old
/// blanket skip was really providing.
#[test]
fn a_fully_scraped_game_asks_for_nothing() {
    let selection = AssetSelection {
        types: vec![AssetType::Screenshot, AssetType::Cover],
    };
    let on_disk = present(&[
        (AssetType::Screenshot, "screenshots/game.png"),
        (AssetType::Cover, "covers/game.jpg"),
    ]);

    assert!(types_to_fetch(&selection, &on_disk, false).types.is_empty());
}

#[test]
fn force_redownload_ignores_what_is_on_disk() {
    let selection = AssetSelection {
        types: vec![AssetType::Screenshot, AssetType::Cover],
    };
    let on_disk = present(&[
        (AssetType::Screenshot, "screenshots/game.png"),
        (AssetType::Cover, "covers/game.jpg"),
    ]);

    assert_eq!(
        types_to_fetch(&selection, &on_disk, true).types,
        selection.types
    );
}

/// Three surfaces kept their own copy of this list and had already drifted —
/// the enrichment path omitted `InvalidCredentials`, so a bad password
/// retried once per release instead of stopping.
#[test]
fn only_account_level_errors_stop_a_run() {
    assert!(is_fatal(&ScrapeError::QuotaExceeded { used: 5, max: 5 }));
    assert!(is_fatal(&ScrapeError::InvalidCredentials("bad".to_owned())));
    assert!(is_fatal(&ScrapeError::ServerClosed("closed".to_owned())));

    assert!(!is_fatal(&ScrapeError::NotFound {
        warnings: Vec::new()
    }));
    assert!(!is_fatal(&ScrapeError::RateLimit { retry_after: None }));
    assert!(!is_fatal(&ScrapeError::Api("transient".to_owned())));
}

/// Publication failure must not leave a target reported as scraped: the
/// frontend would show media the archive never accepted, and the next
/// convergence pass would derive the same work again.
#[test]
fn publication_failure_fails_the_targets_it_covered() {
    let mut outcomes = vec![
        TargetOutcome {
            key: 1,
            state: TargetState::Scraped {
                game: Box::default(),
                method: LookupMethod::Serial,
                warnings: Vec::new(),
                assets: HashMap::new(),
            },
        },
        TargetOutcome {
            key: 2,
            state: TargetState::Skipped {
                reason: "media already exists".to_owned(),
                assets: HashMap::new(),
            },
        },
    ];
    let pending = vec![PendingPublication {
        key: 1,
        release_id: ArchiveReleaseId::new(),
        asset_type: AssetType::Cover,
        path: PathBuf::from("scratch/original.png"),
        source_url: String::new(),
        rom_stem: "Game".to_owned(),
        media_dir: PathBuf::from("media"),
    }];

    mark_publication_failure(&mut outcomes, &pending, "archive is locked");

    assert!(matches!(outcomes[0].state, TargetState::Failed { .. }));
    assert!(matches!(outcomes[1].state, TargetState::Skipped { .. }));
}

/// An empty target list must not take the archive lock or emit work; the
/// convergence executor calls with whatever derivation produced.
#[tokio::test]
async fn an_empty_run_completes_without_touching_anything() {
    let (events, mut received) = mpsc::unbounded_channel();
    let cancel = AtomicBool::new(false);
    let options = ScrapeSessionOptions::default();
    let selection = AssetSelection::default();
    let client = crate::client::ScreenScraperClient::offline_for_tests();

    let result = run_scrape(&ScrapeRequest {
        client: &client,
        system_id: 1,
        targets: &[],
        selection: &selection,
        options: &options,
        archive: None,
        max_workers: 4,
        events: &events,
        cancel: &cancel,
    })
    .await;

    assert!(result.outcomes.is_empty());
    assert!(result.fatal.is_none());
    assert_eq!(result.published, 0);
    assert!(matches!(received.recv().await, Some(ScrapeEvent::Done)));
}
