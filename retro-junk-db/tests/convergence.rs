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
        let representation = self.id("repp");
        self.conn
            .execute(
                "INSERT INTO representations(id,carrier_id,role,format,location_role,relative_path,presence_state,input_manifest_sha256)
                 VALUES(?1,?2,'playable',?3,'playable',?1||'/game.chd','present','msha')",
                (&representation, carrier, format),
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
                "INSERT INTO archive_release_files(id,archive_release_id,category,asset_type,relative_path,file_size,sha256,captured_at,manifest_path,manifest_sha256)
                 VALUES(?1,?2,'artwork','box-front','artwork/box.png',10,'s','2026-01-01','f.toml','h')",
                (&file, release),
            )
            .unwrap();
    }

    fn derive(&self) -> Vec<retro_junk_db::convergence::ProposedAction> {
        derive_convergence(&self.conn, &Scope::Profile("prof".to_owned())).unwrap()
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
        vec![&ActionKind::ProjectAssets, &ActionKind::SyncGamelist]
    );
    assert!(
        !actions
            .iter()
            .any(|action| action.kind == ActionKind::BuildPlayable),
        "satisfied release must not re-derive a build"
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
    let summary = summarize_convergence(&fixture.conn, &Scope::Profile("prof".to_owned())).unwrap();
    let build = summary.per_kind[&ActionKind::BuildPlayable];
    assert_eq!(build.pending, 1);
    assert_eq!(build.running, 1);
    let integrity = summary.per_kind[&ActionKind::VerifyIntegrity];
    assert_eq!(integrity.done, 1);
    assert_eq!(integrity.pending, 0);
    let catalog = summary.per_kind[&ActionKind::VerifyCatalog];
    assert_eq!(catalog.done, 1);
}
