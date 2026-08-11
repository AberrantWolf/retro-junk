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

/// The filter decides both what a list shows and what a bulk button acts on.
/// It matches on the review's target, which is where the path lives for every
/// kind — so one description reaches an incoming package and a stray playable
/// alike.
#[test]
fn a_filter_selects_the_same_rows_a_bulk_action_would_take() {
    use crate::suggestions::SuggestionFilter;

    let row = |id: i64, kind: &str, target: &str| retro_junk_db::work::Suggestion {
        id,
        kind: kind.to_owned(),
        target_kind: "path".to_owned(),
        target_id: target.to_owned(),
        payload_json: "{}".to_owned(),
        confidence: 0.1,
        provenance: "test".to_owned(),
        created_at: String::new(),
        resolved_at: None,
        resolution: String::new(),
    };
    let rows = vec![
        row(1, "adopt_playable", "gc/rvz/readme.txt"),
        row(2, "adopt_playable", "gc/rvz/Zelda.rvz"),
        row(3, "adopt_playable", "psx/notes.txt"),
        row(4, "import", "/incoming/Wipeout"),
    ];
    let ids = |filter: &SuggestionFilter| -> Vec<i64> {
        filter
            .select(rows.clone())
            .into_iter()
            .map(|suggestion| suggestion.id)
            .collect()
    };

    // Extension-shaped, across directories.
    assert_eq!(ids(&SuggestionFilter::new(None, "*.txt")), vec![1, 3]);
    // Directory-shaped.
    assert_eq!(ids(&SuggestionFilter::new(None, "*/rvz/*")), vec![1, 2]);
    // Kind and pattern narrow together, never independently.
    assert_eq!(
        ids(&SuggestionFilter::new(Some("adopt_playable"), "*.txt")),
        vec![1, 3]
    );
    assert!(ids(&SuggestionFilter::new(Some("import"), "*.txt")).is_empty());
    // An empty filter is "everything", not "nothing" — otherwise clearing the
    // filter box would blank the view.
    assert_eq!(ids(&SuggestionFilter::new(None, "  ")), vec![1, 2, 3, 4]);
    assert_eq!(ids(&SuggestionFilter::default()), vec![1, 2, 3, 4]);
}

/// A review surface decides what buttons to show from `offered_actions`, and
/// those buttons then call `apply_suggestion_choice`. If the two disagree, a
/// card either promises an action that errors out or hides one that would have
/// worked.
#[test]
fn what_a_surface_offers_matches_what_the_dispatch_can_do() {
    use crate::adoption::{
        ADOPT_SUGGESTION_KIND, AdoptionCandidate, AdoptionCandidateKind, AdoptionSuggestionPayload,
    };
    use crate::suggestions::{SCRAPE_SUGGESTION_KIND, offered_actions};

    let suggestion = |kind: &str, payload: String| retro_junk_db::work::Suggestion {
        id: 1,
        kind: kind.to_owned(),
        target_kind: "path".to_owned(),
        target_id: "gc/rvz/Zelda.rvz".to_owned(),
        payload_json: payload,
        confidence: 0.3,
        provenance: "test".to_owned(),
        created_at: String::new(),
        resolved_at: None,
        resolution: String::new(),
    };
    let adoption = |candidates: Vec<AdoptionCandidate>| {
        suggestion(
            ADOPT_SUGGESTION_KIND,
            serde_json::to_string(&AdoptionSuggestionPayload {
                relative_path: "gc/rvz/Zelda.rvz".to_owned(),
                status: "ambiguous_archive_master".to_owned(),
                detail: String::new(),
                candidates,
            })
            .unwrap(),
        )
    };
    let candidate = |id: &str| AdoptionCandidate {
        kind: AdoptionCandidateKind::ArchiveMaster,
        id: id.to_owned(),
        label: id.to_owned(),
        archive_release_id: String::new(),
        carrier_id: String::new(),
        platform_id: "gc".to_owned(),
    };

    // Kinds that propose one command are applicable outright.
    assert!(
        offered_actions(&suggestion(
            crate::incoming::IMPORT_SUGGESTION_KIND,
            "[]".to_owned()
        ))
        .applicable
    );
    assert!(offered_actions(&suggestion(SCRAPE_SUGGESTION_KIND, "{}".to_owned())).applicable);

    // An adoption review with nothing to choose from records a decision the
    // user has to make where the files are; offering Apply would promise an
    // action the dispatch refuses.
    let unmatched = offered_actions(&adoption(Vec::new()));
    assert!(!unmatched.applicable);
    assert!(unmatched.choices.is_empty());
    assert!(
        unmatched.ignorable,
        "a stray file can always be ignored for good"
    );

    // One candidate is not a choice — it can be applied as it stands.
    let single = offered_actions(&adoption(vec![candidate("dump-1")]));
    assert!(single.applicable);
    assert!(single.choices.is_empty());

    // Several candidates must be answered before applying, so the surface has
    // to ask rather than pick for the user.
    let several = offered_actions(&adoption(vec![candidate("dump-1"), candidate("dump-2")]));
    assert!(several.applicable);
    assert_eq!(several.choices.len(), 2);

    // A payload written before candidates existed still reads as a review
    // rather than turning into a blank, actionless card.
    let legacy = offered_actions(&suggestion(
        ADOPT_SUGGESTION_KIND,
        r#"{"relative_path":"gc/rvz/Zelda.rvz","status":"unmatched","detail":"no match"}"#
            .to_owned(),
    ));
    assert!(!legacy.applicable);
    assert!(legacy.ignorable);

    let unknown = offered_actions(&suggestion("something-new", "{}".to_owned()));
    assert!(!unknown.applicable);
    assert!(!unknown.ignorable);
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
            "INSERT INTO archive_release_files(id,archive_release_id,category,asset_type,relative_path,file_size,sha256,presence_state,captured_at,manifest_path,manifest_sha256)
             VALUES(?1,'rel','artwork',?2,?3,10,'s','present','2026-01-01','f.toml','h')",
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
        archive: crate::executor::ArchiveScan::default(),
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
        &|_, _, _, _| {},
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
        &|_, _, _, _| {},
        &AtomicBool::new(false),
    )
    .expect("a weak match is not an error");

    let weak = report.needs_review.expect("should be filed for review");
    assert_eq!(weak.archive_release_id, "rel");
    assert_eq!(weak.missing, vec!["covers".to_owned()]);
    assert_eq!(report.published, 0);
}

/// A mod's own bytes are in no scraper's database, so the release is asked
/// about as the work it was made from — with the parent's name and digests,
/// and never the mod's. This is the executor's half of derivation-aware
/// scraping; the GUI and the CLI reach the same rule by other routes.
#[test]
fn a_modded_release_is_looked_up_as_the_work_it_derives_from() {
    let conn = fixture("", false, &[]);
    conn.execute_batch(
        "UPDATE media SET tag='modded',crc32='ff',md5='ee',sha1='dd',
                          rom_name='Test Game (Hard Mode).iso',media_serial='' WHERE id='m';
         INSERT INTO media(id,release_id,dat_source,dat_name,media_serial,rom_name,file_size,crc32,md5,sha1)
         VALUES('parent','catrel','redump','Test Game (USA)','SLUS-00001','Test Game (USA).iso',
                2048,'aa','bb','cc');",
    )
    .unwrap();

    let identity = retro_junk_db::library::query_archived_scrape_identities(&conn, "prof")
        .unwrap()
        .remove("rel")
        .expect("the release still has an identity of its own");
    let derivation = scrape_derivation(&identity.derivation);

    let retro_junk_scraper::Derivation::Parent(parent) = &derivation else {
        panic!("a modded medium under a catalogued work resolves that work");
    };
    assert_eq!(parent.filename, "Test Game (USA).iso");
    assert_eq!(parent.serial, "SLUS-00001");
    assert_eq!(
        parent.hashes.as_ref().map(|hashes| hashes.crc32.as_str()),
        Some("aa")
    );

    // The identity offered carries nothing of the file itself.
    let own = retro_junk_scraper::RomInfo {
        serial: String::new(),
        scraper_serial: String::new(),
        filename: "Test Game (Hard Mode).iso".to_owned(),
        file_size: 4096,
        hashes: retro_junk_scraper::RomHashes::complete("ff", "ee", "dd"),
        platform: retro_junk_core::Platform::Ps1,
        expects_serial: true,
    };
    let asked = derivation
        .identify(&own)
        .expect("a named parent is askable");
    assert_eq!(asked.filename, "Test Game (USA).iso");
    assert_ne!(asked.hashes, own.hashes);

    // And it is as identifiable as its parent. Complete hashes outrank the
    // serial because they identify these exact bytes.
    assert_eq!(
        identity.tier(),
        retro_junk_db::library::ScrapeIdentityTier::Hashes
    );
}

/// Every catalog derivation has to land somewhere in the scrape core's terms;
/// a mod with no parent must land on "nothing to ask" rather than falling back
/// to the file's own bytes.
#[test]
fn every_catalog_derivation_maps_to_a_lookup_decision() {
    use retro_junk_db::CatalogDerivation;

    assert_eq!(
        scrape_derivation(&CatalogDerivation::Own),
        retro_junk_scraper::Derivation::Own
    );
    assert_eq!(
        scrape_derivation(&CatalogDerivation::Homebrew),
        retro_junk_scraper::Derivation::Standalone
    );
    assert_eq!(
        scrape_derivation(&CatalogDerivation::Modded { parent: None }),
        retro_junk_scraper::Derivation::UnknownParent
    );
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
        &|_, _, _, _| {},
        &AtomicBool::new(false),
    )
    .expect_err("a dump target is a programming error, not work");

    assert!(error.to_string().contains("expected a release target"));
}
