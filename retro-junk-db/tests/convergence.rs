//! Convergence derivation over projection fixtures: one fixture row-set per
//! matrix starting state, asserting the exact action set (including blocked
//! reasons) that falls out.

use retro_junk_db::convergence::{
    ActionKind, BlockedReason, Scope, WorkTarget, derive_convergence, summarize_convergence,
};

struct Fixture {
    conn: retro_junk_db::Connection,
    next: u32,
}

impl Fixture {
    fn new() -> Self {
        let conn = retro_junk_db::open_memory().unwrap();
        conn.execute_batch(
            "INSERT INTO platforms(id,display_name,short_name,manufacturer,generation,media_type,release_year,description,core_platform)
             VALUES('psx','PS1','PS1','Sony',5,'optical',1994,'','Ps1');
             INSERT INTO archive_profiles(id,display_name,manifest_path,manifest_sha256,archive_root,playable_root,workspace_root)
             VALUES('prof','Test','retro-junk-archive.toml','h','/archive','/roms','/work');",
        )
        .unwrap();
        Self { conn, next: 0 }
    }

    fn id(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}{}", self.next)
    }

    /// One release with one physical copy; returns `(release_id, copy_id)`.
    fn release(&mut self, title: &str) -> (String, String) {
        let release = self.id("rel");
        let copy = self.id("copy");
        self.conn
            .execute(
                "INSERT INTO archive_releases(id,profile_id,platform_id,title,region,manifest_path,manifest_sha256,binding_state)
                 VALUES(?1,'prof','psx',?2,'usa',?1||'/release.toml','h','resolved')",
                (&release, title),
            )
            .unwrap();
        self.conn
            .execute(
                "INSERT INTO physical_copies(id,archive_release_id,copy_number,manifest_path,manifest_sha256)
                 VALUES(?1,?2,1,'pc.toml','h')",
                (&copy, &release),
            )
            .unwrap();
        (release, copy)
    }

    /// A carrier with its newest dump. Returns `(carrier_id, dump_id)`.
    #[allow(clippy::too_many_arguments)]
    fn carrier_with_dump(
        &mut self,
        copy: &str,
        media_id: Option<&str>,
        sequence: u32,
        format: &str,
        integrity: &str,
        catalog: &str,
        file_count: u32,
    ) -> (String, String) {
        let carrier = self.id("car");
        let dump = self.id("dump");
        let representation = self.id("repm");
        self.conn
            .execute(
                "INSERT INTO carriers(id,physical_copy_id,catalog_media_id,kind,sequence_number,manifest_path,manifest_sha256,binding_state)
                 VALUES(?1,?2,?3,'optical_disc',?4,'c.toml','h',?5)",
                (
                    &carrier,
                    copy,
                    media_id,
                    sequence,
                    if media_id.is_some() { "resolved" } else { "unresolved" },
                ),
            )
            .unwrap();
        self.conn
            .execute(
                "INSERT INTO dump_events(id,carrier_id,representation_id,format,captured_at,manifest_path,manifest_sha256,integrity_state,catalog_state)
                 VALUES(?1,?2,?3,?4,'2026-01-01','d.toml','msha',?5,?6)",
                (&dump, &carrier, &representation, format, integrity, catalog),
            )
            .unwrap();
        self.conn
            .execute(
                "INSERT INTO representations(id,carrier_id,dump_id,role,format,location_role,relative_path,presence_state,input_manifest_sha256)
                 VALUES(?1,?2,?3,'preservation_master',?4,'archive',?1||'/raw','present','msha')",
                (&representation, &carrier, &dump, format),
            )
            .unwrap();
        for index in 0..file_count {
            self.conn
                .execute(
                    "INSERT INTO representation_files(representation_id,relative_path,file_size,sha256)
                     VALUES(?1,?2,100,'fsha')",
                    (&representation, format!("file{index}.bin")),
                )
                .unwrap();
        }
        (carrier, dump)
    }

    /// Mark the newest dump catalog-verified (matching the reconcile
    /// projection: `dump_events.catalog_state` + a verification event).
    fn catalog_verify(&mut self, dump: &str) {
        self.conn
            .execute(
                "UPDATE dump_events SET catalog_state='verified' WHERE id=?1",
                [dump],
            )
            .unwrap();
        let event = self.id("ver");
        self.conn
            .execute(
                "INSERT INTO verification_events(id,representation_id,kind,outcome,performed_at,input_manifest_sha256,evidence_path,complete_track_set)
                 SELECT ?1,de.representation_id,'catalog','verified','2026-01-02',de.manifest_sha256,'e.json',1
                 FROM dump_events de WHERE de.id=?2",
                (&event, dump),
            )
            .unwrap();
    }

    fn policy(&mut self, carrier: &str, format: &str) {
        self.conn
            .execute(
                "INSERT INTO playable_policies(scope_type,scope_id,format,retain_intermediate,allow_unverified,options_json)
                 VALUES('carrier',?1,?2,0,0,'{}')",
                (carrier, format),
            )
            .unwrap();
    }

    fn playable_present(&mut self, carrier: &str, format: &str) {
        self.playable(carrier, format, "present");
    }

    /// A built playable whose recorded file is not where the evidence says.
    fn playable_missing(&mut self, carrier: &str, format: &str) {
        self.playable(carrier, format, "missing");
    }

    /// A present playable filed under a named frontend folder — the folder the
    /// gamelist derivation groups on.
    fn playable_in(&mut self, carrier: &str, format: &str, directory: &str) {
        let representation = self.id("repp");
        self.conn
            .execute(
                "INSERT INTO representations(id,carrier_id,role,format,location_role,relative_path,presence_state,input_manifest_sha256)
                 VALUES(?1,?2,'playable',?3,'playable',?4||'/'||?1||'.chd','present','msha')",
                (&representation, carrier, format, directory),
            )
            .unwrap();
    }

    fn playable(&mut self, carrier: &str, format: &str, presence: &str) {
        let representation = self.id("repp");
        self.conn
            .execute(
                "INSERT INTO representations(id,carrier_id,role,format,location_role,relative_path,presence_state,input_manifest_sha256)
                 VALUES(?1,?2,'playable',?3,'playable',?1||'/game.chd',?4,'msha')",
                (&representation, carrier, format, presence),
            )
            .unwrap();
    }

    /// A scanned playable file under the profile's playable root that no
    /// carrier claims, carrying `media_id`'s digests — the shape a collection
    /// assembled before the archive leaves behind.
    fn unbound_library_file(&mut self, media_id: &str) {
        // The medium needs digests for anything to match against; the base
        // fixture leaves them blank because most tests match by id alone.
        self.conn
            .execute(
                "UPDATE media SET sha1='a2aee128',crc32='42fc324d',file_size=652028496
                 WHERE id=?1",
                [media_id],
            )
            .unwrap();
        self.unbound_library_file_with_digest("a2aee128", "42fc324d", 652_028_496);
    }

    fn unbound_library_file_with_digest(&mut self, sha1: &str, crc32: &str, size: i64) {
        let key = self.id("file");
        self.conn
            .execute_batch(
                "INSERT INTO library_roots(id,root_path)
                 SELECT 1,'/roms' WHERE NOT EXISTS(SELECT 1 FROM library_roots WHERE id=1);
                 INSERT INTO library_consoles(id,root_id,platform,folder_name,folder_path,fingerprint_hash)
                 SELECT 1,1,'Ps1','psx','psx',''
                 WHERE NOT EXISTS(SELECT 1 FROM library_consoles WHERE id=1);",
            )
            .unwrap();
        self.conn
            .execute(
                "INSERT INTO library_entries(console_id,entry_key,display_name,game_entry_json,sha1,crc32,data_size)
                 VALUES(1,'file:'||?1||'.chd',?1,'{}',?2,?3,?4)",
                rusqlite::params![key, sha1, crc32, size],
            )
            .unwrap();
    }

    fn media(&mut self, id: &str, disc_number: u32) {
        self.conn
            .execute(
                "INSERT INTO works(id,canonical_name) SELECT 'w','Game'
                 WHERE NOT EXISTS(SELECT 1 FROM works WHERE id='w')",
                [],
            )
            .unwrap();
        self.conn
            .execute(
                "INSERT INTO releases(id,work_id,platform_id,region,title)
                 SELECT 'catrel','w','psx','usa','Game'
                 WHERE NOT EXISTS(SELECT 1 FROM releases WHERE id='catrel')",
                [],
            )
            .unwrap();
        self.conn
            .execute(
                "INSERT INTO media(id,release_id,dat_source,dat_name,disc_number)
                 VALUES(?1,'catrel','redump','Game (USA)',?2)",
                (id, disc_number),
            )
            .unwrap();
    }

    /// Bind a release to the catalog release so expected-disc counts derive.
    fn bind_release(&mut self, release: &str) {
        self.conn
            .execute(
                "UPDATE archive_releases SET catalog_release_id='catrel' WHERE id=?1",
                [release],
            )
            .unwrap();
    }

    fn artwork(&mut self, release: &str) {
        let file = self.id("art");
        self.conn
            .execute(
                "INSERT INTO archive_release_files(id,archive_release_id,category,asset_type,relative_path,file_size,sha256,presence_state,captured_at,manifest_path,manifest_sha256)
                 VALUES(?1,?2,'artwork','box-front','artwork/box.png',10,'s','present','2026-01-01','f.toml','h')",
                (&file, release),
            )
            .unwrap();
    }

    /// A second, distinct archived asset — a new source for the projection.
    fn artwork_named(&mut self, release: &str, asset_type: &str) {
        let file = self.id("art");
        self.conn
            .execute(
                "INSERT INTO archive_release_files(id,archive_release_id,category,asset_type,relative_path,file_size,sha256,presence_state,captured_at,manifest_path,manifest_sha256)
                 VALUES(?1,?2,'artwork',?3,'artwork/'||?3||'.png',11,'s2','present','2026-01-01','f.toml','h')",
                (&file, release, asset_type),
            )
            .unwrap();
    }

    fn derive(&self) -> Vec<retro_junk_db::convergence::ProposedAction> {
        derive_convergence(
            &self.conn,
            &Scope::Profile("prof".to_owned()),
            &expected_assets(),
        )
        .unwrap()
    }
}

/// The artwork a release is expected to hold in these tests. Deliberately
/// just the cover: the fixture's release owns exactly that one asset, so a
/// Scrape action appearing means a genuine gap, not an unmet default.
fn expected_assets() -> retro_junk_frontend::AssetSelection {
    retro_junk_frontend::AssetSelection {
        types: vec![retro_junk_frontend::AssetType::Cover],
    }
}

fn kinds_for<'a>(
    actions: &'a [retro_junk_db::convergence::ProposedAction],
    target_id: &str,
) -> Vec<&'a ActionKind> {
    actions
        .iter()
        .filter(|action| action.target.id() == target_id)
        .map(|action| &action.kind)
        .collect()
}

#[test]
fn unverified_single_file_dump_owes_integrity_and_catalog_verification() {
    let mut fixture = Fixture::new();
    fixture.media("m1", 0);
    let (release, copy) = fixture.release("Game");
    fixture.bind_release(&release);
    let (_carrier, dump) =
        fixture.carrier_with_dump(&copy, Some("m1"), 0, "iso", "unknown", "not_attempted", 1);
    let actions = fixture.derive();
    assert_eq!(
        kinds_for(&actions, &dump),
        vec![&ActionKind::VerifyIntegrity, &ActionKind::VerifyCatalog]
    );
    // No policy is set, so the build gap (single unverified carrier) is
    // reported blocked rather than silently dropped.
    let build = actions
        .iter()
        .find(|action| action.kind == ActionKind::BuildPlayable)
        .expect("build gap derived");
    assert_eq!(build.blocked, Some(BlockedReason::NoPolicy));
}

#[test]
fn unbound_redumper_master_owes_an_audit_not_a_file_verification() {
    let mut fixture = Fixture::new();
    let (_release, copy) = fixture.release("Raw Game");
    let (_carrier, dump) = fixture.carrier_with_dump(
        &copy,
        None,
        0,
        "redumper_raw",
        "verified",
        "not_attempted",
        4,
    );
    let actions = fixture.derive();
    assert_eq!(kinds_for(&actions, &dump), vec![&ActionKind::AuditRedumper]);
}

/// Reproducing a raw disc costs a full copy and split of its raw dump, so a
/// disc that already came back matching no single catalog medium must not be
/// proposed again. Failing to match is exactly what leaves a dump looking
/// unidentified, so without this the same disc is re-reproduced on every run,
/// forever.
#[test]
fn a_disc_that_matched_nothing_is_not_proposed_again() {
    let mut fixture = Fixture::new();
    let (_release, copy) = fixture.release("Unknown Disc");
    let (_carrier, dump) = fixture.carrier_with_dump(
        &copy,
        None,
        0,
        "redumper_raw",
        "verified",
        retro_junk_db::archive::CATALOG_UNRESOLVED,
        4,
    );
    assert!(kinds_for(&fixture.derive(), &dump).is_empty());
}

/// Same rule for single-file masters: hashing an ISO against the catalog again
/// cannot produce a different answer until the file or the catalog changes.
#[test]
fn a_file_master_that_matched_nothing_is_not_proposed_again() {
    let mut fixture = Fixture::new();
    let (_release, copy) = fixture.release("Unknown ISO");
    let (_carrier, dump) = fixture.carrier_with_dump(
        &copy,
        None,
        0,
        "iso",
        "verified",
        retro_junk_db::archive::CATALOG_UNRESOLVED,
        1,
    );
    assert!(kinds_for(&fixture.derive(), &dump).is_empty());
}

/// Evidence says these bytes matched, but the carrier carries no medium id —
/// an inconsistency a fresh identification would repair, so it still gets
/// proposed.
#[test]
fn a_verified_dump_whose_carrier_lost_its_binding_is_proposed() {
    let mut fixture = Fixture::new();
    let (_release, copy) = fixture.release("Rebindable");
    let (_carrier, dump) = fixture.carrier_with_dump(
        &copy,
        None,
        0,
        "redumper_raw",
        "verified",
        retro_junk_db::archive::CATALOG_VERIFIED,
        4,
    );
    assert_eq!(
        kinds_for(&fixture.derive(), &dump),
        vec![&ActionKind::AuditRedumper]
    );
}

#[test]
fn verified_complete_release_with_policy_derives_an_unblocked_build() {
    let mut fixture = Fixture::new();
    fixture.media("m1", 0);
    let (release, copy) = fixture.release("Game");
    fixture.bind_release(&release);
    let (carrier, dump) =
        fixture.carrier_with_dump(&copy, Some("m1"), 0, "iso", "verified", "not_attempted", 1);
    fixture.catalog_verify(&dump);
    fixture.policy(&carrier, "chd");
    let actions = fixture.derive();
    let build = actions
        .iter()
        .find(|action| action.kind == ActionKind::BuildPlayable)
        .expect("build derived");
    assert_eq!(build.blocked, None);
    assert_eq!(build.target, WorkTarget::Release(release.clone()));
    let gap = build.build.as_ref().expect("gap payload");
    assert!(gap.needs_playable);
    assert!(gap.buildable);
    assert_eq!(gap.expected_disc_count, 1);
    // Nothing owes verification any more.
    assert!(kinds_for(&actions, &dump).is_empty());
}

#[test]
fn incomplete_multi_disc_release_is_blocked_with_honest_counts() {
    let mut fixture = Fixture::new();
    fixture.media("m1", 1);
    fixture.media("m2", 2);
    let (release, copy) = fixture.release("Two Discs");
    fixture.bind_release(&release);
    let (carrier, dump) =
        fixture.carrier_with_dump(&copy, Some("m1"), 1, "iso", "verified", "not_attempted", 1);
    fixture.catalog_verify(&dump);
    fixture.policy(&carrier, "chd");
    let actions = fixture.derive();
    let build = actions
        .iter()
        .find(|action| action.kind == ActionKind::BuildPlayable)
        .expect("build derived");
    assert_eq!(
        build.blocked,
        Some(BlockedReason::IncompleteArchive { have: 1, need: 2 })
    );
}

#[test]
fn complete_split_multidisc_playable_owes_layout_normalization() {
    let mut fixture = Fixture::new();
    fixture.media("m1", 1);
    fixture.media("m2", 2);
    let (release, copy) = fixture.release("Game");
    fixture.bind_release(&release);
    let (carrier1, dump1) =
        fixture.carrier_with_dump(&copy, Some("m1"), 1, "cue_bin", "verified", "verified", 1);
    let (carrier2, dump2) =
        fixture.carrier_with_dump(&copy, Some("m2"), 2, "cue_bin", "verified", "verified", 1);
    fixture.catalog_verify(&dump1);
    fixture.catalog_verify(&dump2);
    fixture.policy(&carrier1, "chd");
    fixture.policy(&carrier2, "chd");
    fixture.playable_in(&carrier1, "chd", "psx");
    fixture.playable_in(&carrier2, "chd", "psx/Game.m3u");

    let actions = fixture.derive();

    assert_eq!(
        actions
            .iter()
            .filter(|action| action.kind == ActionKind::NormalizePlayableSet)
            .count(),
        1,
        "{actions:?}"
    );
}

#[test]
fn satisfied_release_owes_only_projections() {
    let mut fixture = Fixture::new();
    fixture.media("m1", 0);
    let (release, copy) = fixture.release("Done Game");
    fixture.bind_release(&release);
    let (carrier, dump) =
        fixture.carrier_with_dump(&copy, Some("m1"), 0, "iso", "verified", "not_attempted", 1);
    fixture.catalog_verify(&dump);
    fixture.policy(&carrier, "chd");
    fixture.playable_present(&carrier, "chd");
    fixture.artwork(&release);
    let actions = fixture.derive();
    assert_eq!(
        kinds_for(&actions, &release),
        vec![&ActionKind::ProjectAssets]
    );
    // The gamelist is per folder, not per game, so it is derived against the
    // folder the playable actually sits in rather than against this release.
    let gamelists = actions
        .iter()
        .filter(|action| action.kind == ActionKind::SyncGamelist)
        .collect::<Vec<_>>();
    assert_eq!(gamelists.len(), 1, "one gamelist action per folder");
    assert_eq!(gamelists[0].target.kind(), "console");
    assert!(
        !actions
            .iter()
            .any(|action| action.kind == ActionKind::BuildPlayable),
        "satisfied release must not re-derive a build"
    );
}

/// A projection that has already been made is not pending work. Without this,
/// `status` on a fully converged library reported hundreds of outstanding
/// projections forever and every explicit run redid all of them.
#[test]
fn a_recorded_projection_stops_being_derived_until_its_source_changes() {
    use retro_junk_db::projection_state::{ProjectionOf, forget_projections, record_projection};

    let mut fixture = Fixture::new();
    fixture.media("m1", 0);
    let (release, copy) = fixture.release("Done Game");
    fixture.bind_release(&release);
    let (carrier, dump) =
        fixture.carrier_with_dump(&copy, Some("m1"), 0, "iso", "verified", "not_attempted", 1);
    fixture.catalog_verify(&dump);
    fixture.policy(&carrier, "chd");
    fixture.playable_in(&carrier, "chd", "psx");
    fixture.artwork(&release);

    let projections = |fixture: &Fixture| {
        fixture
            .derive()
            .into_iter()
            .filter(|action| {
                matches!(
                    action.kind,
                    ActionKind::ProjectAssets | ActionKind::SyncGamelist
                )
            })
            .count()
    };
    assert_eq!(projections(&fixture), 2, "nothing projected yet");

    record_projection(&fixture.conn, ProjectionOf::assets(&release)).unwrap();
    record_projection(&fixture.conn, ProjectionOf::gamelist("prof", "psx")).unwrap();
    assert_eq!(
        projections(&fixture),
        0,
        "already-current projections are not pending work"
    );

    // New artwork is a new source, so the projection is owed again — and the
    // gamelist too, since its entries name the artwork files.
    fixture.artwork_named(&release, "screenshot");
    assert_eq!(
        projections(&fixture),
        2,
        "fresh artwork must reach the frontend"
    );

    record_projection(&fixture.conn, ProjectionOf::assets(&release)).unwrap();
    record_projection(&fixture.conn, ProjectionOf::gamelist("prof", "psx")).unwrap();
    assert_eq!(projections(&fixture), 0);

    // The escape hatch for a destination somebody deleted by hand, which no
    // source-side fingerprint can see.
    forget_projections(&fixture.conn, &Scope::Profile("prof".to_owned())).unwrap();
    assert_eq!(projections(&fixture), 2);
}

/// The whole point of moving the gamelist to the folder: a folder holding many
/// games is one action that writes one file, not one action per game rewriting
/// the same file over and over.
#[test]
fn one_gamelist_action_covers_every_game_in_a_folder() {
    let mut fixture = Fixture::new();
    let mut releases = Vec::new();
    for (index, title) in ["First Game", "Second Game", "Third Game"]
        .iter()
        .enumerate()
    {
        let media = format!("m{index}");
        fixture.media(&media, 0);
        let (release, copy) = fixture.release(title);
        fixture.bind_release(&release);
        let (carrier, dump) = fixture.carrier_with_dump(
            &copy,
            Some(&media),
            0,
            "iso",
            "verified",
            "not_attempted",
            1,
        );
        fixture.catalog_verify(&dump);
        fixture.policy(&carrier, "chd");
        fixture.playable_in(&carrier, "chd", "psx");
        releases.push(release);
    }

    let gamelists = fixture
        .derive()
        .into_iter()
        .filter(|action| action.kind == ActionKind::SyncGamelist)
        .collect::<Vec<_>>();
    assert_eq!(
        gamelists.len(),
        1,
        "three games in one folder must be one gamelist action, not three"
    );
    assert_eq!(gamelists[0].playable_platform_id, "psx");
    assert_eq!(
        retro_junk_db::convergence::releases_for_target(&fixture.conn, &gamelists[0].target).len(),
        releases.len(),
        "the action has to know every release whose entry it writes"
    );
}

/// A playable that moved is not a playable that is missing. Deriving only a
/// build for it rebuilt the file beside the copy the library already held; the
/// adoption action has to come first, and the worker's stage order depends on
/// `ActionKind`'s declaration order to run it that way.
#[test]
fn a_moved_playable_owes_adoption_before_a_rebuild() {
    let mut fixture = Fixture::new();
    fixture.media("m1", 0);
    let (release, copy) = fixture.release("Moved Game");
    fixture.bind_release(&release);
    let (carrier, dump) =
        fixture.carrier_with_dump(&copy, Some("m1"), 0, "iso", "verified", "not_attempted", 1);
    fixture.catalog_verify(&dump);
    fixture.policy(&carrier, "chd");
    fixture.playable_missing(&carrier, "chd");

    let actions = fixture.derive();
    let kinds = kinds_for(&actions, &release);
    assert!(
        kinds.contains(&&ActionKind::AdoptPlayable),
        "a missing playable owes adoption, got {kinds:?}"
    );
    let adopt = actions
        .iter()
        .position(|action| action.kind == ActionKind::AdoptPlayable)
        .expect("adoption derived");
    let build = actions
        .iter()
        .position(|action| action.kind == ActionKind::BuildPlayable);
    if let Some(build) = build {
        assert!(adopt < build, "adoption must be ordered before the rebuild");
    }
}

/// Presence states other than `missing` are not adoption work: a `stale`
/// output belongs to superseded evidence and a `modified` one is an integrity
/// question. Neither is answered by finding bytes elsewhere.
#[test]
fn a_present_playable_owes_no_adoption() {
    let mut fixture = Fixture::new();
    fixture.media("m1", 0);
    let (release, copy) = fixture.release("Done Game");
    fixture.bind_release(&release);
    let (carrier, dump) =
        fixture.carrier_with_dump(&copy, Some("m1"), 0, "iso", "verified", "not_attempted", 1);
    fixture.catalog_verify(&dump);
    fixture.policy(&carrier, "chd");
    fixture.playable_present(&carrier, "chd");
    assert!(
        !fixture
            .derive()
            .iter()
            .any(|action| action.kind == ActionKind::AdoptPlayable)
    );
}

#[test]
fn scope_release_filters_to_one_release_including_its_dumps() {
    let mut fixture = Fixture::new();
    let (release_a, copy_a) = fixture.release("Alpha");
    let (_carrier_a, dump_a) =
        fixture.carrier_with_dump(&copy_a, None, 0, "iso", "unknown", "not_attempted", 2);
    let (_release_b, copy_b) = fixture.release("Beta");
    let (_carrier_b, _dump_b) =
        fixture.carrier_with_dump(&copy_b, None, 0, "iso", "unknown", "not_attempted", 2);
    let actions = derive_convergence(
        &fixture.conn,
        &Scope::Release {
            archive_release_id: release_a.clone(),
        },
        &expected_assets(),
    )
    .unwrap();
    assert!(!actions.is_empty());
    // Everything in scope belongs to release A: its dump's verification and
    // its (policy-blocked) build gap.
    assert!(
        actions
            .iter()
            .all(|action| action.target.id() == dump_a || action.target.id() == release_a)
    );
    assert!(
        actions.iter().any(
            |action| action.target.id() == dump_a && action.kind == ActionKind::VerifyIntegrity
        )
    );
}

#[test]
fn summary_counts_pending_blocked_done_and_running() {
    let mut fixture = Fixture::new();
    fixture.media("m1", 0);
    let (release, copy) = fixture.release("Game");
    fixture.bind_release(&release);
    let (carrier, dump) =
        fixture.carrier_with_dump(&copy, Some("m1"), 0, "iso", "verified", "not_attempted", 1);
    fixture.catalog_verify(&dump);
    fixture.policy(&carrier, "chd");
    // A live claim on the build and a recorded error on another action.
    assert!(
        retro_junk_db::work::try_claim(&fixture.conn, "build", "release", &release, "daemon")
            .unwrap()
    );
    let summary = summarize_convergence(
        &fixture.conn,
        &Scope::Profile("prof".to_owned()),
        &expected_assets(),
    )
    .unwrap();
    let build = summary.per_kind[&ActionKind::BuildPlayable];
    assert_eq!(build.pending, 1);
    assert_eq!(build.running, 1);
    let integrity = summary.per_kind[&ActionKind::VerifyIntegrity];
    assert_eq!(integrity.done, 1);
    assert_eq!(integrity.pending, 0);
    let catalog = summary.per_kind[&ActionKind::VerifyCatalog];
    assert_eq!(catalog.done, 1);
}

/// A verification failure is recorded against a dump, but every UI that
/// shows failures shows one row per release. Grouping has to resolve the
/// dump back through its carrier and physical copy, or the release's badge
/// silently reports "no errors" while `work_errors` holds one.
#[test]
fn dump_errors_group_under_the_release_that_owns_them() {
    let mut fixture = Fixture::new();
    fixture.media("m1", 0);
    let (release, copy) = fixture.release("Game");
    fixture.bind_release(&release);
    let (_, dump) =
        fixture.carrier_with_dump(&copy, Some("m1"), 1, "file_set", "pending", "pending", 1);

    retro_junk_db::work::release_claim(
        &mut fixture.conn,
        ActionKind::VerifyIntegrity.as_str(),
        "dump",
        &dump,
        "test",
        &retro_junk_db::work::ClaimOutcome::Failed {
            error: "sha256 mismatch on track 2".to_owned(),
        },
    )
    .unwrap();

    let actions = fixture.derive();
    let grouped = retro_junk_db::convergence::errors_by_release(&fixture.conn, &actions).unwrap();
    let errors = grouped.get(&release).expect("error grouped under release");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].0, ActionKind::VerifyIntegrity);
    assert_eq!(errors[0].1.message, "sha256 mismatch on track 2");
}

/// An incoming package that failed pre-processing belongs to no release
/// yet; it must not be attributed to one.
#[test]
fn path_targeted_errors_belong_to_no_release() {
    let mut fixture = Fixture::new();
    retro_junk_db::work::release_claim(
        &mut fixture.conn,
        ActionKind::VerifyIntegrity.as_str(),
        "path",
        "/incoming/mystery.bin",
        "test",
        &retro_junk_db::work::ClaimOutcome::Failed {
            error: "unreadable".to_owned(),
        },
    )
    .unwrap();

    assert!(
        retro_junk_db::convergence::errors_by_release(&fixture.conn, &fixture.derive())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_failed_build_stops_being_open_once_the_projection_is_satisfied() {
    let mut fixture = Fixture::new();
    fixture.media("m1", 0);
    let (release, copy) = fixture.release("Game");
    fixture.bind_release(&release);
    let (carrier, dump) =
        fixture.carrier_with_dump(&copy, Some("m1"), 0, "iso", "verified", "verified", 1);
    fixture.catalog_verify(&dump);
    fixture.policy(&carrier, "chd");

    retro_junk_db::work::release_claim(
        &mut fixture.conn,
        ActionKind::BuildPlayable.as_str(),
        "release",
        &release,
        "test",
        &retro_junk_db::work::ClaimOutcome::Failed {
            error: "old build attempt failed".to_owned(),
        },
    )
    .unwrap();
    let pending = fixture.derive();
    assert!(
        retro_junk_db::convergence::errors_by_release(&fixture.conn, &pending)
            .unwrap()
            .contains_key(&release)
    );

    fixture.playable_present(&carrier, "chd");
    let satisfied = fixture.derive();
    assert!(!satisfied.iter().any(|action| {
        action.kind == ActionKind::BuildPlayable && action.target.id() == release
    }));
    assert!(
        !retro_junk_db::convergence::errors_by_release(&fixture.conn, &satisfied)
            .unwrap()
            .contains_key(&release),
        "attempt history must not masquerade as an open failure after the end state is reached"
    );
    assert!(
        retro_junk_db::work::try_claim(&fixture.conn, "build", "release", &release, "stale-worker")
            .unwrap()
    );
    let summary = summarize_convergence(
        &fixture.conn,
        &Scope::Profile("prof".to_owned()),
        &expected_assets(),
    )
    .unwrap();
    assert_eq!(summary.per_kind[&ActionKind::BuildPlayable].running, 0);
}

// ── Scrape derivation ──────────────────────────────────────────────────────

/// The whole point of the kind: a release with nothing archived owes a
/// scrape, and it does so without waiting for a playable build — an
/// archive-only release is scrapeable from its catalog identity, and the
/// collection view would otherwise stay blank for everything not yet built.
#[test]
fn a_release_with_no_artwork_owes_a_scrape_even_with_no_playable() {
    let mut fixture = Fixture::new();
    let (release, copy) = fixture.release("Alpha");
    fixture.media("m1", 0);
    let (_carrier, _dump) =
        fixture.carrier_with_dump(&copy, Some("m1"), 0, "iso", "verified", "verified", 1);

    let actions = fixture.derive();

    let scrape = actions
        .iter()
        .find(|action| action.kind == ActionKind::Scrape)
        .expect("a release with no artwork owes a scrape");
    assert_eq!(scrape.target, WorkTarget::Release(release));
    assert_eq!(scrape.blocked, None);
}

/// Holding the expected set is the end state: no action, and it counts as
/// done rather than quietly disappearing from the summary.
#[test]
fn a_release_holding_every_expected_type_owes_nothing() {
    let mut fixture = Fixture::new();
    let (release, copy) = fixture.release("Alpha");
    fixture.media("m1", 0);
    let (_carrier, _dump) =
        fixture.carrier_with_dump(&copy, Some("m1"), 0, "iso", "verified", "verified", 1);
    // The fixture's expected set is exactly the cover, which `artwork`
    // inserts as `box-front`.
    fixture.artwork(&release);

    let actions = fixture.derive();

    assert!(
        !actions
            .iter()
            .any(|action| action.kind == ActionKind::Scrape),
        "a fully-covered release should owe no scrape"
    );
    let summary = summarize_convergence(
        &fixture.conn,
        &Scope::Profile("prof".to_owned()),
        &expected_assets(),
    )
    .unwrap();
    assert_eq!(summary.per_kind[&ActionKind::Scrape].done, 1);
    assert_eq!(summary.per_kind[&ActionKind::Scrape].pending, 0);
}

/// A release with nothing to look up is reported as blocked, never silently
/// dropped: "we cannot identify this" is a fact the user should see.
#[test]
fn a_release_with_no_catalog_identity_is_blocked_not_hidden() {
    let mut fixture = Fixture::new();
    let (_release, copy) = fixture.release("Alpha");
    // No `media` row, so the carrier binds to no catalog medium.
    let (_carrier, _dump) =
        fixture.carrier_with_dump(&copy, None, 0, "iso", "verified", "verified", 1);

    let actions = fixture.derive();

    let scrape = actions
        .iter()
        .find(|action| action.kind == ActionKind::Scrape)
        .expect("the release still owes artwork");
    assert_eq!(scrape.blocked, Some(BlockedReason::NoScrapeIdentity));
}

/// Expecting nothing must not derive work for everything. An empty selection
/// is the user saying "I don't want artwork managed", not "fetch it all".
#[test]
fn expecting_no_asset_types_derives_no_scrapes() {
    let mut fixture = Fixture::new();
    let (_release, copy) = fixture.release("Alpha");
    fixture.media("m1", 0);
    let (_carrier, _dump) =
        fixture.carrier_with_dump(&copy, Some("m1"), 0, "iso", "verified", "verified", 1);

    let actions = derive_convergence(
        &fixture.conn,
        &Scope::Profile("prof".to_owned()),
        &retro_junk_frontend::AssetSelection { types: Vec::new() },
    )
    .unwrap();

    assert!(
        !actions
            .iter()
            .any(|action| action.kind == ActionKind::Scrape)
    );
}

/// Partial coverage is still a gap: this is the "artwork evidence is
/// presence-only" fix — one screenshot must not read as a complete set.
#[test]
fn partial_coverage_still_owes_the_missing_types() {
    let mut fixture = Fixture::new();
    let (release, copy) = fixture.release("Alpha");
    fixture.media("m1", 0);
    let (_carrier, _dump) =
        fixture.carrier_with_dump(&copy, Some("m1"), 0, "iso", "verified", "verified", 1);
    fixture.artwork(&release);

    let gaps = retro_junk_db::convergence::scrape_gaps(
        &fixture.conn,
        "prof",
        &retro_junk_frontend::AssetSelection {
            types: vec![
                retro_junk_frontend::AssetType::Cover,
                retro_junk_frontend::AssetType::Screenshot,
            ],
        },
    )
    .unwrap();

    let gap = gaps.first().expect("one release");
    assert_eq!(gap.have, vec![retro_junk_frontend::AssetType::Cover]);
    assert_eq!(
        gap.missing,
        vec![retro_junk_frontend::AssetType::Screenshot]
    );
    assert_eq!(gap.expected(), 2);
}

/// A collection assembled before the archive existed holds playable files
/// nobody built here: a CHD beside a preservation master of the same disc,
/// with no build evidence connecting them. The carrier has no playable
/// representation at all, so the "recorded output is missing" rule never fires
/// — but the catalog medium the carrier verified against identifies the file
/// just as well, and that is adoptable work rather than a build gap.
#[test]
fn an_unbuilt_playable_matching_the_carriers_catalog_digests_owes_adoption() {
    let mut fixture = Fixture::new();
    fixture.media("m1", 0);
    let (release, copy) = fixture.release("Pre-existing Game");
    fixture.bind_release(&release);
    let (carrier, dump) = fixture.carrier_with_dump(
        &copy,
        Some("m1"),
        0,
        "redumper_raw",
        "verified",
        "verified",
        4,
    );
    fixture.catalog_verify(&dump);
    fixture.policy(&carrier, "chd");
    // No playable representation exists for this carrier at all.
    fixture.unbound_library_file("m1");

    let actions = fixture.derive();
    let kinds = kinds_for(&actions, &release);
    assert!(
        kinds.contains(&&ActionKind::AdoptPlayable),
        "an unbuilt playable carrying the carrier's digests owes adoption, got {kinds:?}"
    );
}

/// The same shape, but the file on disk is a different game. Nothing links it
/// to this carrier, so there is nothing to adopt and the release simply owes a
/// build.
#[test]
fn an_unrelated_library_file_does_not_owe_adoption() {
    let mut fixture = Fixture::new();
    fixture.media("m1", 0);
    let (release, copy) = fixture.release("Unrelated Game");
    fixture.bind_release(&release);
    let (carrier, dump) = fixture.carrier_with_dump(
        &copy,
        Some("m1"),
        0,
        "redumper_raw",
        "verified",
        "verified",
        4,
    );
    fixture.catalog_verify(&dump);
    fixture.policy(&carrier, "chd");
    fixture.unbound_library_file_with_digest("deadbeef", "ff00", 999);

    assert!(
        !fixture
            .derive()
            .iter()
            .any(|action| action.kind == ActionKind::AdoptPlayable)
    );
}

/// The canonical spelling and the string the tables are keyed on must be the
/// same word. They are produced by two different `match` arms, so nothing but
/// this stops one from being edited without the other — and the way that would
/// surface is a `--only` value the help screen offers being rejected.
#[test]
fn every_action_kind_leads_with_the_name_it_is_stored_under() {
    for kind in ActionKind::all() {
        assert_eq!(
            kind.spellings().first().copied(),
            Some(kind.as_str()),
            "{kind:?} spells itself differently in as_str() and spellings()"
        );
    }
}

/// Every spelling the help screen offers, and every alias behind it, has to
/// parse back to the kind that claimed it.
#[test]
fn every_spelling_parses_back_to_its_own_kind() {
    for kind in ActionKind::all() {
        for spelling in kind.spellings() {
            assert_eq!(
                spelling.parse::<ActionKind>().as_ref(),
                Ok(kind),
                "{spelling} did not parse back to {kind:?}"
            );
        }
    }
    assert!("bogus".parse::<ActionKind>().is_err());
}

/// No two kinds may claim the same word, or one of them becomes unreachable
/// from the command line depending on list order.
#[test]
fn no_two_action_kinds_claim_the_same_spelling() {
    let mut seen = std::collections::BTreeSet::new();
    for kind in ActionKind::all() {
        for spelling in kind.spellings() {
            assert!(seen.insert(*spelling), "{spelling} is claimed twice");
        }
    }
}
