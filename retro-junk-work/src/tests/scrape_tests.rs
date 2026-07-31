use super::*;

/// ES-DE keys a multi-disc game's media by the `.m3u` directory name, not by
/// the individual disc files. Naming downloaded artwork after a disc would
/// leave the frontend showing nothing for every multi-disc game.
#[test]
fn a_multi_disc_release_uses_its_playlist_directory_name() {
    assert_eq!(
        frontend_stem("psx/Final Fantasy VII (USA).m3u/Final Fantasy VII (USA) (Disc 1).chd")
            .as_deref(),
        Some("Final Fantasy VII (USA).m3u")
    );
}

#[test]
fn a_single_disc_release_uses_its_file_stem() {
    assert_eq!(
        frontend_stem("psx/Castlevania - Symphony of the Night (USA).chd").as_deref(),
        Some("Castlevania - Symphony of the Night (USA)")
    );
}

/// A directory that merely mentions m3u is not a playlist directory.
#[test]
fn only_a_playlist_directory_overrides_the_file_stem() {
    assert_eq!(
        frontend_stem("psx/m3u-backups/Game (USA).chd").as_deref(),
        Some("Game (USA)")
    );
}

/// The Inbox decides whether to show Apply from `is_applicable`, and the
/// button then calls `apply_suggestion`. If those two disagree, a card either
/// promises an action that errors out or hides one that would have worked.
#[test]
fn every_applicable_kind_has_a_dispatch_arm() {
    use crate::suggestions::{SCRAPE_SUGGESTION_KIND, is_applicable};

    assert!(is_applicable(crate::incoming::IMPORT_SUGGESTION_KIND));
    assert!(is_applicable(SCRAPE_SUGGESTION_KIND));

    // Adoption reviews record a decision the user makes elsewhere; offering
    // Apply for them would promise an action that does not exist yet.
    assert!(!is_applicable("adopt_playable"));
    assert!(!is_applicable("something-new"));
}

/// Build a projection holding one archived release, optionally bound to a
/// catalog medium with the given serial and hashes.
fn fixture(serial: &str, hashes: bool, artwork: &[&str]) -> retro_junk_db::Connection {
    let conn = retro_junk_db::open_memory().unwrap();
    conn.execute_batch(
        "INSERT INTO platforms(id,display_name,short_name,manufacturer,generation,media_type,release_year,description,core_platform)
         VALUES('psx','PS1','PS1','Sony',5,'optical',1994,'','Ps1');
         INSERT INTO archive_profiles(id,display_name,manifest_path,manifest_sha256,archive_root,playable_root,workspace_root)
         VALUES('prof','Test','a.toml','h','/archive','/roms','/work');
         INSERT INTO archive_releases(id,profile_id,platform_id,title,region,manifest_path,manifest_sha256,binding_state)
         VALUES('rel','prof','psx','Test Game','usa','rel/release.toml','h','resolved');
         INSERT INTO physical_copies(id,archive_release_id,copy_number,manifest_path,manifest_sha256)
         VALUES('copy','rel',1,'pc.toml','h');
         INSERT INTO works(id,canonical_name) VALUES('w','Test Game');
         INSERT INTO releases(id,work_id,platform_id,region,title) VALUES('catrel','w','psx','usa','Test Game');",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO media(id,release_id,dat_source,dat_name,media_serial,rom_name,file_size,crc32,md5,sha1)
         VALUES('m','catrel','redump','Test Game (USA)',?1,'Test Game (USA).iso',2048,?2,?3,?4)",
        (
            serial,
            if hashes { "aa" } else { "" },
            if hashes { "bb" } else { "" },
            if hashes { "cc" } else { "" },
        ),
    )
    .unwrap();
    conn.execute(
        "INSERT INTO carriers(id,physical_copy_id,sequence_number,catalog_media_id,manifest_path,manifest_sha256)
         VALUES('car','copy',0,'m','c.toml','h')",
        [],
    )
    .unwrap();
    for (index, asset_type) in artwork.iter().enumerate() {
        conn.execute(
            "INSERT INTO archive_release_files(id,archive_release_id,category,asset_type,relative_path,file_size,sha256,captured_at,manifest_path,manifest_sha256)
             VALUES(?1,'rel','artwork',?2,?3,10,'s','2026-01-01','f.toml','h')",
            (format!("af{index}"), asset_type, format!("artwork/{index}.png")),
        )
        .unwrap();
    }
    conn
}

fn context(only_when_unambiguous: bool) -> ExecContext {
    ExecContext {
        profile: retro_junk_archive::CollectionProfile::for_roots(
            std::path::PathBuf::from("/archive"),
            std::path::PathBuf::from("/roms"),
        ),
        db_path: std::path::PathBuf::from("/nonexistent.db"),
        tools: crate::executor::ToolPaths::default(),
        scrape: crate::executor::ScrapeSettings {
            expected_assets: retro_junk_frontend::AssetSelection {
                types: vec![retro_junk_frontend::AssetType::Cover],
            },
            only_when_unambiguous,
            daily_request_reserve: 0,
        },
        roots: retro_junk_lib::archive_ops::FrontendRoots::from_settings(
            std::path::Path::new("/roms"),
            "",
            "",
        ),
        analyzers: std::sync::Arc::new(retro_junk_lib::create_default_context()),
        owner: "test".to_owned(),
        lock: crate::executor::LockEtiquette::DaemonFailFast,
        reconcile: crate::executor::ReconcileMode::AtBatchEnd,
    }
}

fn scrape_action() -> ProposedAction {
    ProposedAction {
        kind: retro_junk_db::convergence::ActionKind::Scrape,
        target: WorkTarget::Release("rel".to_owned()),
        profile_id: "prof".to_owned(),
        platform_id: "psx".to_owned(),
        playable_platform_id: "psx".to_owned(),
        label: "Test Game (usa)".to_owned(),
        blocked: None,
        build: None,
    }
}

/// Derivation and execution are separate passes, so a gap can close in
/// between — another run, or the user. Executing anyway would spend a request
/// to re-download artwork the archive already holds.
#[test]
fn a_gap_closed_since_derivation_does_no_work() {
    let conn = fixture("SLUS-00001", true, &["cover"]);

    let report = scrape_release_artwork(
        &context(true),
        &scrape_action(),
        &conn,
        &|_, _, _| {},
        &AtomicBool::new(false),
    )
    .expect("a satisfied release is not an error");

    assert_eq!(report.published, 0);
    assert!(report.needs_review.is_none());
}

/// A filename-only match is a guess. Publishing a guess into the archive
/// unattended would make it durable, so it becomes a review card instead —
/// and crucially not an error, which would back the release off for hours.
#[test]
fn a_weak_match_is_filed_for_review_rather_than_guessed_at() {
    let conn = fixture("", false, &[]);

    let report = scrape_release_artwork(
        &context(true),
        &scrape_action(),
        &conn,
        &|_, _, _| {},
        &AtomicBool::new(false),
    )
    .expect("a weak match is not an error");

    let weak = report.needs_review.expect("should be filed for review");
    assert_eq!(weak.archive_release_id, "rel");
    assert_eq!(weak.missing, vec!["covers".to_owned()]);
    assert_eq!(report.published, 0);
}

/// A dump target cannot be scraped: artwork belongs to the release, not to
/// one of its discs.
#[test]
fn a_non_release_target_is_rejected() {
    let conn = fixture("SLUS-00001", true, &[]);
    let action = ProposedAction {
        target: WorkTarget::Dump("dump".to_owned()),
        ..scrape_action()
    };

    let error = scrape_release_artwork(
        &context(true),
        &action,
        &conn,
        &|_, _, _| {},
        &AtomicBool::new(false),
    )
    .expect_err("a dump target is a programming error, not work");

    assert!(error.to_string().contains("expected a release target"));
}
