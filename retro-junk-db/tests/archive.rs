use retro_junk_archive::TrackDigest;
use retro_junk_db::{
    bind_library_entries_by_hash, match_catalog_file, match_complete_catalog_media, open_memory,
};
use std::sync::atomic::AtomicBool;

#[test]
fn complete_track_matching_rejects_partial_and_accepts_exact_sets() {
    let conn = open_memory().unwrap();
    conn.execute("INSERT INTO platforms(id,display_name,short_name,manufacturer,generation,media_type,release_year,description,core_platform) VALUES('psx','PlayStation','PSX','Sony',5,'cd',1994,'','Psx')", []).unwrap();
    conn.execute(
        "INSERT INTO works(id,canonical_name) VALUES('work','Game')",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO releases(id,work_id,platform_id,region,title) VALUES('release','work','psx','usa','Game')", []).unwrap();
    conn.execute(
        "INSERT INTO media(id,release_id,dat_source) VALUES('media','release','redump')",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO media_tracks(media_id,track_number,track_name,file_size,crc32,sha1,md5) VALUES('media',1,'Track 1',2352,'aa','1111','bb')", []).unwrap();
    conn.execute("INSERT INTO media_tracks(media_id,track_number,track_name,file_size,crc32,sha1,md5) VALUES('media',2,'Track 2',4704,'cc','2222','dd')", []).unwrap();

    let one = vec![TrackDigest {
        number: 1,
        size: 2352,
        crc32: "aa".to_owned(),
        md5: "bb".to_owned(),
        sha1: "1111".to_owned(),
    }];
    assert!(
        match_complete_catalog_media(&conn, "psx", &one)
            .unwrap()
            .is_empty()
    );
    let mut complete = one;
    complete.push(TrackDigest {
        number: 2,
        size: 4704,
        crc32: "cc".to_owned(),
        md5: "dd".to_owned(),
        sha1: "2222".to_owned(),
    });
    let matches = match_complete_catalog_media(&conn, "psx", &complete).unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].media_id, "media");
}

#[test]
fn complete_track_matching_accepts_primary_hash_only_for_single_track_media() {
    let conn = open_memory().unwrap();
    conn.execute("INSERT INTO platforms(id,display_name,short_name,manufacturer,generation,media_type,release_year,description,core_platform) VALUES('psx','PlayStation','PSX','Sony',5,'cd',1994,'','Psx')", []).unwrap();
    conn.execute(
        "INSERT INTO works(id,canonical_name) VALUES('work-single','Single Track Game')",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO releases(id,work_id,platform_id,region,title,revision) VALUES('release-original','work-single','psx','japan','Single Track Game',''),('release-revision','work-single','psx','japan','Single Track Game','Rev 1')", []).unwrap();
    conn.execute("INSERT INTO media(id,release_id,dat_source,file_size,crc32,sha1,md5) VALUES('media-original','release-original','redump',2352,'aa','1111','bb'),('media-revision','release-revision','redump',2352,'cc','2222','dd')", []).unwrap();
    // This medium deliberately repeats the revision's primary digest. Its
    // extra track means a one-track audit must not claim it as complete.
    conn.execute("INSERT INTO media(id,release_id,dat_source,file_size,crc32,sha1,md5) VALUES('media-multitrack','release-revision','redump',2352,'cc','2222','dd')", []).unwrap();
    conn.execute("INSERT INTO media_tracks(media_id,track_number,track_name,file_size,crc32,sha1,md5) VALUES('media-multitrack',1,'Track 1',2352,'cc','2222','dd'),('media-multitrack',2,'Track 2',4704,'ee','3333','ff')", []).unwrap();

    let actual = vec![TrackDigest {
        number: 1,
        size: 2352,
        crc32: "cc".to_owned(),
        md5: "dd".to_owned(),
        sha1: "2222".to_owned(),
    }];
    let matches = match_complete_catalog_media(&conn, "psx", &actual).unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].media_id, "media-revision");
    assert_eq!(matches[0].revision, "Rev 1");
}

#[test]
fn flat_file_matching_requires_size_and_every_available_catalog_digest() {
    let conn = open_memory().unwrap();
    conn.execute("INSERT INTO platforms(id,display_name,short_name,manufacturer,generation,media_type,release_year,description,core_platform) VALUES('nes','NES','NES','Nintendo',3,'cartridge',1983,'','Nes')", []).unwrap();
    conn.execute(
        "INSERT INTO works(id,canonical_name) VALUES('work-flat','Flat Game')",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO releases(id,work_id,platform_id,region,title) VALUES('release-flat','work-flat','nes','usa','Flat Game')", []).unwrap();
    conn.execute("INSERT INTO media(id,release_id,dat_source,file_size,crc32,sha1,md5) VALUES('media-flat','release-flat','no-intro',4,'11223344','aabb','ccdd')", []).unwrap();
    let mut actual = retro_junk_archive::FileDigests {
        size: 4,
        crc32: "11223344".to_owned(),
        md5: "ccdd".to_owned(),
        sha1: "aabb".to_owned(),
        sha256: String::new(),
    };
    assert_eq!(match_catalog_file(&conn, "nes", &actual).unwrap().len(), 1);
    conn.execute(
        "INSERT INTO library_roots(id,root_path) VALUES(1,'/playable')",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO library_consoles(id,root_id,platform,folder_name,folder_path,fingerprint_hash,scan_state) VALUES(1,1,'Nes','nes','/playable/nes','fp','ready')", []).unwrap();
    conn.execute("INSERT INTO library_entries(id,console_id,entry_key,display_name,game_entry_json,status,data_size,crc32,sha1,md5) VALUES(1,1,'file:game.nes','game.nes','{}','matched',4,'11223344','aabb','ccdd')", []).unwrap();
    assert_eq!(
        bind_library_entries_by_hash(
            &conn,
            "nes",
            &actual,
            &retro_junk_db::LibraryEntryBinding {
                catalog_media_id: "media-flat",
                match_method: "archive_adoption",
                ..Default::default()
            },
        )
        .unwrap(),
        1
    );
    let binding: (Option<String>, String, Option<String>) = conn
        .query_row(
            "SELECT carrier_id,catalog_media_id,representation_id FROM library_entry_media_bindings",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(binding, (None, "media-flat".to_owned(), None));
    actual.md5 = "different".to_owned();
    assert!(
        match_catalog_file(&conn, "nes", &actual)
            .unwrap()
            .is_empty()
    );
}

/// Adopting a playable file must not depend on this machine's catalog holding
/// the exact medium id the archive's manifest was written with. Those ids are
/// minted per DAT import, so an archive built elsewhere (or before a DAT
/// update) names media this catalog never created — and inserting one anyway
/// made `SQLite` reject the whole adoption run with a foreign-key failure.
#[test]
fn binding_to_a_carrier_survives_a_catalog_medium_this_database_never_imported() {
    let conn = open_memory().unwrap();
    seed_playable_library_row(&conn);
    seed_carrier(&conn, None);

    let bound = bind_library_entries_by_hash(
        &conn,
        "nes",
        &library_row_digests(),
        &retro_junk_db::LibraryEntryBinding {
            carrier_id: Some("carrier"),
            // What the manifest says, which this catalog does not have.
            catalog_media_id: "media-from-another-machine",
            match_method: "archive_adoption",
            ..Default::default()
        },
    )
    .unwrap();

    assert_eq!(bound, 1);
    let binding: (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT carrier_id,catalog_media_id FROM library_entry_media_bindings",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(binding, (Some("carrier".to_owned()), None));
}

/// The carrier row is what knows which medium this catalog resolved it to —
/// reindexing re-derives that from digests when the manifest's id is unusable —
/// so it wins over whatever the caller passes.
#[test]
fn binding_prefers_the_carriers_own_catalog_medium() {
    let conn = open_memory().unwrap();
    seed_playable_library_row(&conn);
    seed_carrier(&conn, Some("media-flat"));

    bind_library_entries_by_hash(
        &conn,
        "nes",
        &library_row_digests(),
        &retro_junk_db::LibraryEntryBinding {
            carrier_id: Some("carrier"),
            catalog_media_id: "media-from-another-machine",
            match_method: "archive_adoption",
            ..Default::default()
        },
    )
    .unwrap();

    let medium: Option<String> = conn
        .query_row(
            "SELECT catalog_media_id FROM library_entry_media_bindings",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(medium, Some("media-flat".to_owned()));
}

/// A carrier the projection has not ingested yet is nothing to bind to. Writing
/// the row anyway is a foreign-key failure; reindexing the archive is what
/// makes it bindable, so this pass simply records nothing.
#[test]
fn binding_to_an_unknown_carrier_writes_nothing_instead_of_failing() {
    let conn = open_memory().unwrap();
    seed_playable_library_row(&conn);

    let bound = bind_library_entries_by_hash(
        &conn,
        "nes",
        &library_row_digests(),
        &retro_junk_db::LibraryEntryBinding {
            carrier_id: Some("carrier-not-indexed-yet"),
            match_method: "archive_adoption",
            ..Default::default()
        },
    )
    .unwrap();

    assert_eq!(bound, 0);
    let rows: i64 = conn
        .query_row(
            "SELECT count(*) FROM library_entry_media_bindings",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rows, 0);
}

/// The digests of the scanned playable row [`seed_playable_library_row`] makes.
fn library_row_digests() -> retro_junk_archive::FileDigests {
    retro_junk_archive::FileDigests {
        size: 4,
        crc32: "11223344".to_owned(),
        md5: "ccdd".to_owned(),
        sha1: "aabb".to_owned(),
        sha256: String::new(),
    }
}

/// One scanned NES file in the playable-library projection, plus the catalog
/// medium `media-flat` that matches its digests.
fn seed_playable_library_row(conn: &retro_junk_db::Connection) {
    conn.execute("INSERT INTO platforms(id,display_name,short_name,manufacturer,generation,media_type,release_year,description,core_platform) VALUES('nes','NES','NES','Nintendo',3,'cartridge',1983,'','Nes')", []).unwrap();
    conn.execute(
        "INSERT INTO works(id,canonical_name) VALUES('work-flat','Flat Game')",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO releases(id,work_id,platform_id,region,title) VALUES('release-flat','work-flat','nes','usa','Flat Game')", []).unwrap();
    conn.execute("INSERT INTO media(id,release_id,dat_source,file_size,crc32,sha1,md5) VALUES('media-flat','release-flat','no-intro',4,'11223344','aabb','ccdd')", []).unwrap();
    conn.execute(
        "INSERT INTO library_roots(id,root_path) VALUES(1,'/playable')",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO library_consoles(id,root_id,platform,folder_name,folder_path,fingerprint_hash,scan_state) VALUES(1,1,'Nes','nes','/playable/nes','fp','ready')", []).unwrap();
    conn.execute("INSERT INTO library_entries(id,console_id,entry_key,display_name,game_entry_json,status,data_size,crc32,sha1,md5) VALUES(1,1,'file:game.nes','game.nes','{}','matched',4,'11223344','aabb','ccdd')", []).unwrap();
}

/// An archived carrier `carrier`, bound to `catalog_media_id` if this catalog
/// resolved one for it — as reindexing records it.
fn seed_carrier(conn: &retro_junk_db::Connection, catalog_media_id: Option<&str>) {
    conn.execute("INSERT INTO archive_profiles(id,display_name,manifest_path,manifest_sha256,archive_root) VALUES('profile','Collection','retro-junk-archive.toml','sha','/archive')", []).unwrap();
    conn.execute("INSERT INTO archive_releases(id,profile_id,platform_id,title,manifest_path,manifest_sha256) VALUES('archive-release','profile','nes','Flat Game','release.toml','sha')", []).unwrap();
    conn.execute("INSERT INTO physical_copies(id,archive_release_id,copy_number,manifest_path,manifest_sha256) VALUES('copy','archive-release',1,'copy.toml','sha')", []).unwrap();
    conn.execute(
        "INSERT INTO carriers(id,physical_copy_id,catalog_media_id,manifest_path,manifest_sha256)
         VALUES('carrier','copy',?1,'carrier.toml','sha')",
        [catalog_media_id],
    )
    .unwrap();
}

#[test]
#[allow(clippy::too_many_lines)]
fn archive_projection_is_rebuildable_from_portable_manifests() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("archive");
    let mut root_manifest = retro_junk_archive::ArchiveRootManifest::new("Collection");
    root_manifest
        .platform_defaults
        .push(retro_junk_archive::PlatformPlayableDefault {
            platform_id: "nes".to_owned(),
            policy: retro_junk_archive::DesiredPlayablePolicy {
                format: retro_junk_archive::RepresentationFormat::Rom,
                retain_canonical_intermediate: false,
                allow_unverified: false,
                options: std::collections::BTreeMap::default(),
            },
        });
    retro_junk_archive::initialize_archive(&root, &root_manifest).unwrap();
    let source = temp.path().join("game.nes");
    std::fs::write(&source, b"game").unwrap();
    let digests = retro_junk_archive::hash_file_digests(&source, &AtomicBool::new(false)).unwrap();
    let ingested = retro_junk_archive::ingest_new_carrier_dump(
        &root,
        &source,
        retro_junk_archive::NewCarrierDump {
            platform_id: "nes".to_owned(),
            title: "Game".to_owned(),
            region: "usa".to_owned(),
            revision: String::new(),
            variant: String::new(),
            owner_id: "default".to_owned(),
            physical_copy_label: String::new(),
            serial: String::new(),
            sequence_number: 0,
            carrier_label: String::new(),
            carrier_kind: retro_junk_archive::CarrierKind::Cartridge,
            format: retro_junk_archive::RepresentationFormat::Rom,
            catalog_binding: retro_junk_archive::CatalogBinding {
                catalog_release_id: "release-game".to_owned(),
                catalog_media_id: "media-game".to_owned(),
                source: "no-intro".to_owned(),
                dat_name: "Game".to_owned(),
                ..Default::default()
            },
            source_package: retro_junk_archive::SourcePackageRecord::default(),
            expected_files: Vec::new(),
            physical_copy_id: None,
        },
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    let cover = temp.path().join("cover.png");
    std::fs::write(&cover, b"png").unwrap();
    retro_junk_archive::add_release_file(
        &root,
        retro_junk_archive::NewReleaseFile {
            release_id: ingested.release.archive_release_id,
            source_file: &cover,
            category: retro_junk_archive::ReleaseFileCategory::Artwork,
            asset_type: "cover",
            source: "test",
            source_url: "https://example.invalid/cover.png",
            caption: "",
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    let snapshot = retro_junk_archive::scan_archive(&root).unwrap();
    let mut conn = open_memory().unwrap();
    conn.execute("INSERT INTO platforms(id,display_name,short_name,manufacturer,generation,media_type,release_year,description,core_platform) VALUES('nes','NES','NES','Nintendo',3,'cartridge',1983,'','Nes')", []).unwrap();
    conn.execute(
        "INSERT INTO works(id,canonical_name) VALUES('work-game','Game')",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO releases(id,work_id,platform_id,region,title) VALUES('release-game','work-game','nes','usa','Game')", []).unwrap();
    conn.execute(
        "INSERT INTO media(id,release_id,dat_source,dat_name,file_size,crc32,sha1,md5) VALUES('media-game','release-game','no-intro','Game',?1,?2,?3,?4)",
        rusqlite::params![digests.size, digests.crc32, digests.sha1, digests.md5],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO library_roots(id,root_path) VALUES(1,?1)",
        [temp.path().join("playable").to_string_lossy().as_ref()],
    )
    .unwrap();
    conn.execute("INSERT INTO library_consoles(id,root_id,platform,folder_name,folder_path,fingerprint_hash,scan_state) VALUES(1,1,'Nes','nes','/playable/nes','fp','ready')", []).unwrap();
    conn.execute(
        "INSERT INTO library_entries(id,console_id,entry_key,display_name,game_entry_json,status,data_size,crc32,sha1,md5) VALUES(1,1,'file:game.nes','game.nes','{}','matched',?1,?2,?3,?4)",
        rusqlite::params![digests.size, digests.crc32, digests.sha1, digests.md5],
    )
    .unwrap();
    retro_junk_db::reconcile_archive_snapshot(
        &mut conn,
        &snapshot,
        &temp.path().join("playable"),
        &temp.path().join("work"),
    )
    .unwrap();
    let summary = &retro_junk_db::list_archive_release_summaries(
        &conn,
        &root_manifest.profile_id.to_string(),
    )
    .unwrap()[0];
    assert_eq!(summary.preservation_count, 1);
    assert_eq!(summary.preservation_present_count, 1);
    // Ingest read the published bytes back and recorded that verification,
    // so the master arrives integrity-verified without a separate pass.
    assert_eq!(summary.integrity_verified_count, 1);
    // Disc expectations and verification are facts now, counted by the one
    // rule rather than by a second SQL definition on the summary row.
    let facts = retro_junk_db::facts::release_facts_by_id(
        &conn,
        &retro_junk_db::facts::FactsScope::profile(&root_manifest.profile_id.to_string()),
    )
    .unwrap();
    let release_facts = &facts[&summary.archive_release_id];
    assert_eq!(release_facts.expected_discs.unwrap().count, 1);
    assert_eq!(retro_junk_db::facts::verified_disc_count(release_facts), 0);
    let details =
        retro_junk_db::load_archive_collection_details(&conn, &summary.archive_release_id)
            .unwrap()
            .unwrap();
    assert_eq!(details.title, "Game");
    assert_eq!(details.catalog_source, "no-intro");
    assert_eq!(details.release_binding_state, "resolved");
    let binding: (String, Option<String>, String) = conn
        .query_row(
            "SELECT catalog_media_id,representation_id,match_method FROM library_entry_media_bindings WHERE library_entry_id=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        binding,
        (
            "media-game".to_owned(),
            None,
            "archive_projection".to_owned()
        )
    );
    let page = retro_junk_db::query_entry_list(
        &conn,
        &retro_junk_db::LibraryEntryListQuery {
            console_id: retro_junk_db::LibraryConsoleId(1),
            search: String::new(),
            filter: retro_junk_db::LibraryEntryFilter::All,
            sort: retro_junk_db::LibraryEntrySortField::DisplayName,
            direction: retro_junk_db::SortDirection::Ascending,
            offset: 0,
            limit: 10,
        },
    )
    .unwrap();
    assert!(page.rows.is_empty());
    assert_eq!(page.availability_counts.archived_and_playable, 0);
    assert_eq!(page.availability_counts.incomplete_archive_and_playable, 1);
    assert_eq!(page.availability_counts.archived_not_playable, 0);
    assert_eq!(page.archived_playable_gaps.len(), 1);
    assert_eq!(page.archived_releases.len(), 1);
    assert_eq!(
        page.archived_releases[0].summary.archive_release_id,
        summary.archive_release_id
    );
    assert_eq!(
        page.archived_releases[0].playable_library_entries,
        [retro_junk_db::ArchivedPlayableLibraryEntry {
            id: retro_junk_db::LibraryEntryId(1),
            display_name: "game.nes".to_owned(),
            playable_format: "nes".to_owned(),
        }]
    );
    assert!(page.archived_releases[0].action.is_some());
    assert_eq!(page.archived_releases[0].archived_assets.len(), 1);
    assert_eq!(
        page.archived_releases[0].archived_assets[0].asset_type,
        "cover"
    );
    assert!(
        std::path::Path::new(&page.archived_releases[0].archived_assets[0].absolute_path).is_file()
    );
    let scrape_identity = page.archived_releases[0].scrape_identity.as_ref().unwrap();
    assert_eq!(scrape_identity.filename, "Game");
    assert_eq!(scrape_identity.file_size, digests.size);
    assert_eq!(scrape_identity.sha1, digests.sha1);
    assert_eq!(
        page.archived_releases[0]
            .action
            .as_ref()
            .and_then(|action| action.preferred_format.as_deref()),
        Some("rom")
    );
    assert_eq!(page.logical_count, 1);
    assert!(!page.archived_playable_gaps[0].needs_playable);
    conn.execute("DELETE FROM library_entry_media_bindings", [])
        .unwrap();
    let gaps = retro_junk_db::query_entry_list(
        &conn,
        &retro_junk_db::LibraryEntryListQuery {
            console_id: retro_junk_db::LibraryConsoleId(1),
            search: String::new(),
            filter: retro_junk_db::LibraryEntryFilter::All,
            sort: retro_junk_db::LibraryEntrySortField::DisplayName,
            direction: retro_junk_db::SortDirection::Ascending,
            offset: 0,
            limit: 10,
        },
    )
    .unwrap();
    assert_eq!(gaps.availability_counts.archived_not_playable, 1);
    // The unbound playable file and archival release are now honestly two
    // logical rows instead of being silently conflated.
    assert_eq!(gaps.logical_count, 2);
    assert_eq!(gaps.archived_releases.len(), 1);
    assert_eq!(gaps.archived_playable_gaps[0].title, "Game");
    assert_eq!(
        gaps.archived_playable_gaps[0].preferred_format.as_deref(),
        Some("rom")
    );
    assert_eq!(gaps.archived_playable_gaps[0].expected_disc_count, 1);
    let logical_pages = [0, 1].map(|offset| {
        retro_junk_db::query_entry_list(
            &conn,
            &retro_junk_db::LibraryEntryListQuery {
                console_id: retro_junk_db::LibraryConsoleId(1),
                search: String::new(),
                filter: retro_junk_db::LibraryEntryFilter::All,
                sort: retro_junk_db::LibraryEntrySortField::DisplayName,
                direction: retro_junk_db::SortDirection::Ascending,
                offset,
                limit: 1,
            },
        )
        .unwrap()
    });
    assert!(
        logical_pages
            .iter()
            .all(|logical_page| logical_page.total_count == 2)
    );
    assert_eq!(
        logical_pages
            .iter()
            .map(|logical_page| logical_page.rows.len())
            .sum::<usize>(),
        1
    );
    assert_eq!(
        logical_pages
            .iter()
            .map(|logical_page| logical_page.archived_releases.len())
            .sum::<usize>(),
        1
    );
    let chd_policy = retro_junk_archive::DesiredPlayablePolicy {
        format: retro_junk_archive::RepresentationFormat::Chd,
        retain_canonical_intermediate: false,
        allow_unverified: false,
        options: std::collections::BTreeMap::new(),
    };
    retro_junk_db::update_projected_platform_policy(
        &mut conn,
        &root_manifest.profile_id.to_string(),
        "nes",
        Some(&chd_policy),
        "updated-root-digest",
    )
    .unwrap();
    let projected: String = conn
        .query_row(
            "SELECT format FROM playable_policies WHERE scope_type='carrier'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(projected, "chd");
    let digest: String = conn
        .query_row("SELECT manifest_sha256 FROM archive_profiles", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(digest, "updated-root-digest");

    retro_junk_db::update_projected_platform_policy(
        &mut conn,
        &root_manifest.profile_id.to_string(),
        "nes",
        None,
        "cleared-root-digest",
    )
    .unwrap();
    let inherited_count: u64 = conn
        .query_row(
            "SELECT COUNT(*) FROM playable_policies WHERE scope_type='carrier'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(inherited_count, 0);

    let mut override_snapshot = snapshot.clone();
    override_snapshot.releases[0].physical_copies[0].carriers[0]
        .manifest
        .playable_policy = Some(retro_junk_archive::DesiredPlayablePolicy {
        format: retro_junk_archive::RepresentationFormat::Rvz,
        retain_canonical_intermediate: false,
        allow_unverified: true,
        options: std::collections::BTreeMap::new(),
    });
    retro_junk_db::reconcile_archive_snapshot(
        &mut conn,
        &override_snapshot,
        &temp.path().join("playable"),
        &temp.path().join("work"),
    )
    .unwrap();
    let carrier_id = override_snapshot.releases[0].physical_copies[0].carriers[0]
        .manifest
        .carrier_id
        .to_string();
    let override_markers: u64 = conn
        .query_row(
            "SELECT COUNT(*) FROM playable_policies WHERE scope_type='carrier_override' AND scope_id=?1",
            [&carrier_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(override_markers, 1);
    retro_junk_db::update_projected_platform_policy(
        &mut conn,
        &root_manifest.profile_id.to_string(),
        "nes",
        Some(&chd_policy),
        "override-preserved",
    )
    .unwrap();
    let overridden: String = conn
        .query_row(
            "SELECT format FROM playable_policies WHERE scope_type='carrier' AND scope_id=?1",
            [&carrier_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(overridden, "rvz");

    conn.execute(
        "INSERT INTO library_entries(
             id,console_id,entry_key,display_name,game_entry_json,status,dat_game_name)
         VALUES(2,1,'set:game.m3u','game.m3u',
                '{\"MultiDisc\":{\"name\":\"game.m3u\",\"files\":[\"game.m3u/disc.chd\"]}}',
                'matched','Game')",
        [],
    )
    .unwrap();
    retro_junk_db::reconcile_archive_snapshot(
        &mut conn,
        &snapshot,
        &temp.path().join("playable"),
        &temp.path().join("work"),
    )
    .unwrap();
    let release_binding: String = conn
        .query_row(
            "SELECT match_method FROM library_entry_media_bindings WHERE library_entry_id=2",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(release_binding, "archive_release_projection");
    conn.execute(
        "UPDATE library_entries SET dat_game_name='Different Game' WHERE id=2",
        [],
    )
    .unwrap();
    retro_junk_db::reconcile_archive_snapshot(
        &mut conn,
        &snapshot,
        &temp.path().join("playable"),
        &temp.path().join("work"),
    )
    .unwrap();
    let stale_release_bindings: u64 = conn
        .query_row(
            "SELECT COUNT(*) FROM library_entry_media_bindings
             WHERE library_entry_id=2 AND match_method='archive_release_projection'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stale_release_bindings, 0);

    // A generated container is not expected to reproduce the raw catalog
    // hashes. Its exact evidence path is sufficient provenance to collapse the
    // scanned playable file into the archival release.
    let dump = &snapshot.releases[0].physical_copies[0].carriers[0].dumps[0];
    let playable_relative = "nes/Game (USA).chd";
    let playable_file = temp.path().join("playable").join(playable_relative);
    std::fs::create_dir_all(playable_file.parent().unwrap()).unwrap();
    std::fs::write(&playable_file, b"chd").unwrap();
    let evidence = retro_junk_archive::BuildEvidence {
        schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
        build_id: retro_junk_archive::BuildId::new(),
        parent_representation_id: dump.manifest.representation_id,
        child_representation_id: retro_junk_archive::RepresentationId::new(),
        performed_at: "2026-07-24T00:00:00Z".to_owned(),
        input_manifest_sha256: dump.manifest_sha256.clone(),
        recipe_version: 1,
        format: retro_junk_archive::RepresentationFormat::Chd,
        relative_output_path: playable_relative.to_owned(),
        output_sha256: String::new(),
        output_size: 3,
        catalog_verified: true,
        round_trip_verified: true,
        tool: None,
        omitted_features: Vec::new(),
        canonical_intermediate: None,
    };
    std::fs::create_dir_all(dump.directory.join("evidence")).unwrap();
    std::fs::write(
        dump.directory
            .join("evidence")
            .join(format!("build-{}.json", evidence.build_id)),
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();
    conn.execute(
        "INSERT INTO library_entries(
             id,console_id,entry_key,display_name,game_entry_json,status,data_size)
         VALUES(3,1,'file:Game (USA).chd','Game (USA).chd',
                '{\"SingleFile\":\"/playable/nes/Game (USA).chd\"}','unknown',3)",
        [],
    )
    .unwrap();
    let playable_snapshot = retro_junk_archive::scan_archive(&root).unwrap();
    retro_junk_db::reconcile_archive_snapshot(
        &mut conn,
        &playable_snapshot,
        &temp.path().join("playable"),
        &temp.path().join("work"),
    )
    .unwrap();
    let output_binding: (Option<String>, String) = conn
        .query_row(
            "SELECT representation_id,match_method
             FROM library_entry_media_bindings WHERE library_entry_id=3",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        output_binding,
        (
            Some(evidence.child_representation_id.to_string()),
            "archive_output_path".to_owned()
        )
    );
    let playable_page = retro_junk_db::query_entry_list(
        &conn,
        &retro_junk_db::LibraryEntryListQuery {
            console_id: retro_junk_db::LibraryConsoleId(1),
            search: String::new(),
            filter: retro_junk_db::LibraryEntryFilter::All,
            sort: retro_junk_db::LibraryEntrySortField::DisplayName,
            direction: retro_junk_db::SortDirection::Ascending,
            offset: 0,
            limit: 10,
        },
    )
    .unwrap();
    assert!(
        playable_page.archived_releases[0]
            .playable_library_entries
            .iter()
            .any(|entry| entry.id == retro_junk_db::LibraryEntryId(3))
    );

    conn.execute(
        "DELETE FROM playable_policies WHERE scope_type='carrier_override'",
        [],
    )
    .unwrap();
    conn.execute("UPDATE archive_releases SET platform_id='ps1'", [])
        .unwrap();
    retro_junk_db::update_projected_platform_policy(
        &mut conn,
        &root_manifest.profile_id.to_string(),
        "psx",
        Some(&chd_policy),
        "alias-policy",
    )
    .unwrap();
    let alias_projected: String = conn
        .query_row(
            "SELECT format FROM playable_policies WHERE scope_type='carrier'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(alias_projected, "chd");

    conn.execute("DELETE FROM archive_profiles", []).unwrap();
    retro_junk_db::reconcile_archive_snapshot(
        &mut conn,
        &snapshot,
        &temp.path().join("playable"),
        &temp.path().join("work"),
    )
    .unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM dump_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn multidisc_playable_format_comes_from_disc_images_not_playlist_name() {
    let conn = open_memory().unwrap();
    conn.execute(
        "INSERT INTO library_roots(id,root_path) VALUES(1,'/playable')",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO library_consoles(id,root_id,platform,folder_name,folder_path,fingerprint_hash,scan_state) VALUES(1,1,'Ps1','psx','/playable/psx','fp','ready')", []).unwrap();
    conn.execute(
        "INSERT INTO library_entries(
             id,console_id,entry_key,display_name,game_entry_json,status)
         VALUES(1,1,'set:game.m3u','game.m3u',
                '{\"MultiDisc\":{\"name\":\"game.m3u\",\"files\":[\"game.m3u/disc-1.chd\",\"game.m3u/disc-2.chd\"]}}',
                'matched')",
        [],
    )
    .unwrap();
    let page = retro_junk_db::query_entry_list(
        &conn,
        &retro_junk_db::LibraryEntryListQuery {
            console_id: retro_junk_db::LibraryConsoleId(1),
            search: String::new(),
            filter: retro_junk_db::LibraryEntryFilter::All,
            sort: retro_junk_db::LibraryEntrySortField::DisplayName,
            direction: retro_junk_db::SortDirection::Ascending,
            offset: 0,
            limit: 10,
        },
    )
    .unwrap();
    assert_eq!(page.rows[0].playable_format, "chd");
}

#[test]
fn saturn_archive_projection_keeps_japan_in_saturnjp() {
    let conn = open_memory().unwrap();
    conn.execute_batch(
        "INSERT INTO library_roots(id,root_path) VALUES(1,'/playable');
         INSERT INTO library_consoles(
             id,root_id,platform,folder_name,folder_path,fingerprint_hash,scan_state)
         VALUES(1,1,'saturn','saturn','/playable/saturn','fp','ready'),
               (2,1,'saturn','saturnjp','/playable/saturnjp','fp','ready');
         INSERT INTO archive_profiles(
             id,display_name,manifest_path,manifest_sha256,archive_root,playable_root)
         VALUES('profile','Saturn','manifest','hash','/archive','/playable');
         INSERT INTO archive_releases(
             id,profile_id,platform_id,title,region,manifest_path,manifest_sha256)
         VALUES('usa','profile','saturn','USA Game','USA','usa/release.toml','usa-hash'),
               ('japan','profile','saturnjp','Japan Game','Japan','japan/release.toml','jp-hash');
         INSERT INTO physical_copies(
             id,archive_release_id,copy_number,manifest_path,manifest_sha256)
         VALUES('usa-copy','usa',1,'usa/copy.toml','usa-copy-hash'),
               ('jp-copy','japan',1,'japan/copy.toml','jp-copy-hash');
         INSERT INTO carriers(
             id,physical_copy_id,kind,sequence_number,manifest_path,manifest_sha256)
         VALUES('usa-disc','usa-copy','optical_disc',1,'usa/carrier.toml','usa-carrier-hash'),
               ('jp-disc','jp-copy','optical_disc',1,'japan/carrier.toml','jp-carrier-hash');",
    )
    .unwrap();

    let query = |console_id| {
        retro_junk_db::query_entry_list(
            &conn,
            &retro_junk_db::LibraryEntryListQuery {
                console_id: retro_junk_db::LibraryConsoleId(console_id),
                search: String::new(),
                filter: retro_junk_db::LibraryEntryFilter::All,
                sort: retro_junk_db::LibraryEntrySortField::DisplayName,
                direction: retro_junk_db::SortDirection::Ascending,
                offset: 0,
                limit: 10,
            },
        )
        .unwrap()
    };

    let saturn = query(1);
    assert_eq!(saturn.archived_releases.len(), 1);
    assert_eq!(saturn.archived_releases[0].summary.title, "USA Game");
    let saturn_japan = query(2);
    assert_eq!(saturn_japan.archived_releases.len(), 1);
    assert_eq!(
        saturn_japan.archived_releases[0].summary.title,
        "Japan Game"
    );
}

/// Ingest one cartridge dump, record the catalog verification the archive
/// carries beside it, and record a playable build. `recorded_output_path` is
/// what the build evidence claims; `actual_relative_path` is where the file
/// really sits below the playable root, so callers can reproduce evidence
/// written before outputs were filed under a platform directory.
fn archive_verified_playable(
    root: &std::path::Path,
    playable_root: &std::path::Path,
    source_dir: &std::path::Path,
    title: &str,
    catalog_game: &str,
    recorded_output_path: &str,
    actual_relative_path: &str,
) {
    let source = source_dir.join(format!("{title}.nes"));
    std::fs::write(&source, title.as_bytes()).unwrap();
    retro_junk_archive::ingest_new_carrier_dump(
        root,
        &source,
        retro_junk_archive::NewCarrierDump {
            platform_id: "nes".to_owned(),
            title: title.to_owned(),
            region: "usa".to_owned(),
            revision: String::new(),
            variant: String::new(),
            owner_id: "default".to_owned(),
            physical_copy_label: String::new(),
            serial: String::new(),
            sequence_number: 0,
            carrier_label: String::new(),
            carrier_kind: retro_junk_archive::CarrierKind::Cartridge,
            format: retro_junk_archive::RepresentationFormat::Rom,
            // Deliberately unresolvable: this machine has no catalog rows.
            catalog_binding: retro_junk_archive::CatalogBinding {
                catalog_release_id: "missing-release".to_owned(),
                catalog_media_id: "missing-media".to_owned(),
                source: "no-intro".to_owned(),
                dat_name: catalog_game.to_owned(),
                ..Default::default()
            },
            source_package: retro_junk_archive::SourcePackageRecord::default(),
            expected_files: Vec::new(),
            physical_copy_id: None,
        },
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();

    let snapshot = retro_junk_archive::scan_archive(root).unwrap();
    let dump = snapshot
        .releases
        .iter()
        .find(|release| release.manifest.title == title)
        .map(|release| &release.physical_copies[0].carriers[0].dumps[0])
        .unwrap();
    let evidence_dir = dump.directory.join("evidence");
    std::fs::create_dir_all(&evidence_dir).unwrap();

    let verification = retro_junk_archive::VerificationEvidence {
        schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
        verification_id: retro_junk_archive::VerificationId::new(),
        representation_id: dump.manifest.representation_id,
        performed_at: "2026-07-25T00:00:00Z".to_owned(),
        input_manifest_sha256: dump.manifest_sha256.clone(),
        kind: retro_junk_archive::VerificationKind::Catalog,
        outcome: retro_junk_archive::VerificationOutcome::Verified,
        tool: None,
        catalog: Some(retro_junk_archive::CatalogEvidence {
            source: "no-intro".to_owned(),
            system: "nes".to_owned(),
            version: "2026.05.02".to_owned(),
            game: catalog_game.to_owned(),
            complete_track_set: true,
        }),
        tracks: Vec::new(),
        detail: String::new(),
    };
    std::fs::write(
        evidence_dir.join(format!(
            "verification-{}.json",
            verification.verification_id
        )),
        serde_json::to_vec_pretty(&verification).unwrap(),
    )
    .unwrap();

    let playable_file = playable_root.join(actual_relative_path);
    std::fs::create_dir_all(playable_file.parent().unwrap()).unwrap();
    std::fs::write(&playable_file, title.as_bytes()).unwrap();
    let build = retro_junk_archive::BuildEvidence {
        schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
        build_id: retro_junk_archive::BuildId::new(),
        parent_representation_id: dump.manifest.representation_id,
        child_representation_id: retro_junk_archive::RepresentationId::new(),
        performed_at: "2026-07-25T00:00:00Z".to_owned(),
        input_manifest_sha256: dump.manifest_sha256.clone(),
        recipe_version: 1,
        format: retro_junk_archive::RepresentationFormat::Rom,
        relative_output_path: recorded_output_path.to_owned(),
        // A cartridge mirror is the master's bytes under another name.
        output_sha256: dump.manifest.files[0].sha256.clone(),
        output_size: title.len() as u64,
        catalog_verified: true,
        round_trip_verified: false,
        tool: None,
        omitted_features: Vec::new(),
        canonical_intermediate: None,
    };
    std::fs::write(
        evidence_dir.join(format!("build-{}.json", build.build_id)),
        serde_json::to_vec_pretty(&build).unwrap(),
    )
    .unwrap();
}

/// The library on a machine that has never imported a DAT: the archive's own
/// evidence still names the files, including builds recorded before outputs
/// were filed under a platform directory.
#[test]
fn archive_evidence_names_library_rows_without_any_catalog() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("archive");
    let playable_root = temp.path().join("playable");
    retro_junk_archive::initialize_archive(
        &root,
        &retro_junk_archive::ArchiveRootManifest::new("Collection"),
    )
    .unwrap();
    archive_verified_playable(
        &root,
        &playable_root,
        temp.path(),
        "Filed Game",
        "Filed Game (USA)",
        "nes/Filed Game.nes",
        "nes/Filed Game.nes",
    );
    archive_verified_playable(
        &root,
        &playable_root,
        temp.path(),
        "Legacy Game",
        "Legacy Game (USA)",
        // Written before playable outputs carried a platform directory.
        "Legacy Game.nes",
        "nes/Legacy Game.nes",
    );

    let mut conn = open_memory().unwrap();
    conn.execute(
        "INSERT INTO library_roots(id,root_path) VALUES(1,?1)",
        [playable_root.to_string_lossy().as_ref()],
    )
    .unwrap();
    conn.execute("INSERT INTO library_consoles(id,root_id,platform,folder_name,folder_path,fingerprint_hash,scan_state) VALUES(1,1,'Nes','nes','/playable/nes','fp','ready')", []).unwrap();
    for (id, title) in [(1, "Filed Game"), (2, "Legacy Game")] {
        conn.execute(
            "INSERT INTO library_entries(id,console_id,entry_key,display_name,game_entry_json,status,data_size)
             VALUES(?1,1,?2,?3,'{}','unrecognized',?4)",
            rusqlite::params![
                id,
                format!("file:{title}.nes"),
                format!("{title}.nes"),
                title.len() as i64,
            ],
        )
        .unwrap();
    }

    let snapshot = retro_junk_archive::scan_archive(&root).unwrap();
    retro_junk_db::reconcile_archive_snapshot(
        &mut conn,
        &snapshot,
        &playable_root,
        &temp.path().join("work"),
    )
    .unwrap();

    let catalog_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM media", [], |row| row.get(0))
        .unwrap();
    assert_eq!(catalog_rows, 0, "the test must prove evidence stands alone");

    // The legacy build recorded a bare file name; the projection must point at
    // the file inside the platform directory, not at the directory itself.
    let legacy: (String, String) = conn
        .query_row(
            "SELECT relative_path,presence_state FROM representations
             WHERE role='playable' AND relative_path LIKE '%Legacy%'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        legacy,
        ("nes/Legacy Game.nes".to_owned(), "present".to_owned())
    );

    let mut named: Vec<(i64, String, String, String)> = conn
        .prepare("SELECT id,status,dat_game_name,dat_match_method FROM library_entries ORDER BY id")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    named.sort_by_key(|row| row.0);
    assert_eq!(
        named,
        vec![
            (
                1,
                "matched".to_owned(),
                "Filed Game (USA)".to_owned(),
                "archive_evidence".to_owned()
            ),
            (
                2,
                "matched".to_owned(),
                "Legacy Game (USA)".to_owned(),
                "archive_evidence".to_owned()
            ),
        ]
    );
}

/// A playable file the archive built belongs to the carrier that produced it,
/// even when no catalog medium can be resolved for that carrier — an unbound
/// archive, a platform whose DAT was never imported, or a carrier manifest
/// naming a catalog id a later import has re-slugged.
///
/// Keying this on the catalog medium listed the very same file twice: once
/// inside the archived release and again as an unarchived "playable only" row.
#[test]
#[allow(clippy::too_many_lines)]
fn an_archived_playable_is_one_row_when_its_carrier_has_no_catalog_medium() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("archive");
    let playable_root = temp.path().join("playable");
    retro_junk_archive::initialize_archive(
        &root,
        &retro_junk_archive::ArchiveRootManifest::new("Collection"),
    )
    .unwrap();
    archive_verified_playable(
        &root,
        &playable_root,
        temp.path(),
        "Bound Game",
        "Bound Game (USA)",
        "nes/Bound Game.nes",
        "nes/Bound Game.nes",
    );

    let mut conn = open_memory().unwrap();
    conn.execute(
        "INSERT INTO library_roots(id,root_path) VALUES(1,?1)",
        [playable_root.to_string_lossy().as_ref()],
    )
    .unwrap();
    conn.execute("INSERT INTO library_consoles(id,root_id,platform,folder_name,folder_path,fingerprint_hash,scan_state) VALUES(1,1,'Nes','nes','/playable/nes','fp','ready')", []).unwrap();
    conn.execute(
        "INSERT INTO library_entries(id,console_id,entry_key,display_name,game_entry_json,status,data_size)
         VALUES(1,1,'file:Bound Game.nes','Bound Game.nes','{}','unrecognized',10)",
        [],
    )
    .unwrap();
    // A second, genuinely unarchived file must stay its own row.
    conn.execute(
        "INSERT INTO library_entries(id,console_id,entry_key,display_name,game_entry_json,status,data_size)
         VALUES(2,1,'file:Loose Game.nes','Loose Game.nes','{}','unrecognized',7)",
        [],
    )
    .unwrap();

    let snapshot = retro_junk_archive::scan_archive(&root).unwrap();
    retro_junk_db::reconcile_archive_snapshot(
        &mut conn,
        &snapshot,
        &playable_root,
        &temp.path().join("work"),
    )
    .unwrap();

    let unresolved_carriers: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM carriers WHERE catalog_media_id IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        unresolved_carriers, 1,
        "the test must prove the carrier has no catalog medium"
    );
    let binding: (Option<String>, Option<String>, String) = conn
        .query_row(
            "SELECT carrier_id,catalog_media_id,match_method
             FROM library_entry_media_bindings WHERE library_entry_id=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(binding.1, None);
    assert_eq!(binding.2, "archive_output_path");
    assert!(binding.0.is_some(), "the playable must belong to a carrier");

    let page = retro_junk_db::query_entry_list(
        &conn,
        &retro_junk_db::LibraryEntryListQuery {
            console_id: retro_junk_db::LibraryConsoleId(1),
            search: String::new(),
            filter: retro_junk_db::LibraryEntryFilter::All,
            sort: retro_junk_db::LibraryEntrySortField::DisplayName,
            direction: retro_junk_db::SortDirection::Ascending,
            offset: 0,
            limit: 10,
        },
    )
    .unwrap();
    assert_eq!(page.archived_releases.len(), 1);
    assert_eq!(
        page.archived_releases[0]
            .playable_library_entries
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        vec![retro_junk_db::LibraryEntryId(1)],
        "the archived release owns the playable file it built"
    );
    assert_eq!(
        page.rows.iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![retro_junk_db::LibraryEntryId(2)],
        "only the unarchived file remains a playable-only row"
    );
    assert_eq!(page.availability_counts.playable_only, 1);
    assert_eq!(page.logical_count, 2);

    // The console's own count agrees with the listing: one archived release
    // plus one unarchived file, not one release plus two loose files.
    let console_entry_count =
        retro_junk_db::list_console_summaries(&conn, retro_junk_db::LibraryRootId(1))
            .unwrap()
            .into_iter()
            .find(|summary| summary.id == retro_junk_db::LibraryConsoleId(1))
            .map(|summary| summary.entry_count);
    assert_eq!(console_entry_count, Some(2));
}

/// A multi-disc library row is one entry standing for a directory of disc
/// images, so every archived disc built into that directory belongs to it.
/// Matching the playlist directory against a disc's file name never did.
#[test]
#[allow(clippy::too_many_lines)]
fn a_multi_disc_row_owns_every_archived_disc_in_its_playlist_directory() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("archive");
    let playable_root = temp.path().join("playable");
    retro_junk_archive::initialize_archive(
        &root,
        &retro_junk_archive::ArchiveRootManifest::new("Collection"),
    )
    .unwrap();

    let mut physical_copy_id = None;
    for disc in 1..=2_u32 {
        let source = temp.path().join(format!("disc-{disc}.iso"));
        std::fs::write(&source, format!("disc {disc}").as_bytes()).unwrap();
        let ingested = retro_junk_archive::ingest_new_carrier_dump(
            &root,
            &source,
            retro_junk_archive::NewCarrierDump {
                platform_id: "psx".to_owned(),
                title: "Set Game".to_owned(),
                region: "usa".to_owned(),
                revision: String::new(),
                variant: String::new(),
                owner_id: "default".to_owned(),
                physical_copy_label: String::new(),
                serial: String::new(),
                sequence_number: disc,
                carrier_label: String::new(),
                carrier_kind: retro_junk_archive::CarrierKind::OpticalDisc,
                format: retro_junk_archive::RepresentationFormat::Iso,
                catalog_binding: retro_junk_archive::CatalogBinding::default(),
                source_package: retro_junk_archive::SourcePackageRecord::default(),
                expected_files: Vec::new(),
                physical_copy_id,
            },
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        physical_copy_id = Some(ingested.physical_copy.physical_copy_id);

        // Each disc builds into the playlist directory the library scans as
        // one multi-disc entry.
        let relative = format!("psx/Set Game (USA).m3u/Set Game (USA) (Disc {disc}).chd");
        let output = playable_root.join(&relative);
        std::fs::create_dir_all(output.parent().unwrap()).unwrap();
        std::fs::write(&output, b"chd").unwrap();
        let build = retro_junk_archive::BuildEvidence {
            schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
            build_id: retro_junk_archive::BuildId::new(),
            parent_representation_id: ingested.dump.representation_id,
            child_representation_id: retro_junk_archive::RepresentationId::new(),
            performed_at: "2026-07-30T00:00:00Z".to_owned(),
            input_manifest_sha256: retro_junk_archive::scan_archive(&root)
                .unwrap()
                .releases
                .iter()
                .flat_map(|release| &release.physical_copies)
                .flat_map(|copy| &copy.carriers)
                .flat_map(|carrier| &carrier.dumps)
                .find(|dump| dump.manifest.dump_id == ingested.dump.dump_id)
                .map(|dump| dump.manifest_sha256.clone())
                .unwrap(),
            recipe_version: 1,
            format: retro_junk_archive::RepresentationFormat::Chd,
            relative_output_path: relative,
            output_sha256: String::new(),
            output_size: 3,
            catalog_verified: false,
            round_trip_verified: true,
            tool: None,
            omitted_features: Vec::new(),
            canonical_intermediate: None,
        };
        let evidence_dir = ingested.dump_directory.join("evidence");
        std::fs::create_dir_all(&evidence_dir).unwrap();
        std::fs::write(
            evidence_dir.join(format!("build-{}.json", build.build_id)),
            serde_json::to_vec_pretty(&build).unwrap(),
        )
        .unwrap();
    }

    let mut conn = open_memory().unwrap();
    conn.execute(
        "INSERT INTO library_roots(id,root_path) VALUES(1,?1)",
        [playable_root.to_string_lossy().as_ref()],
    )
    .unwrap();
    conn.execute("INSERT INTO library_consoles(id,root_id,platform,folder_name,folder_path,fingerprint_hash,scan_state) VALUES(1,1,'Ps1','psx','/playable/psx','fp','ready')", []).unwrap();
    conn.execute(
        "INSERT INTO library_entries(id,console_id,entry_key,display_name,game_entry_json,status)
         VALUES(1,1,'set:Set Game (USA).m3u','Set Game (USA).m3u',
                '{\"MultiDisc\":{\"name\":\"Set Game (USA).m3u\",\"files\":[\"Set Game (USA).m3u/Set Game (USA) (Disc 1).chd\",\"Set Game (USA).m3u/Set Game (USA) (Disc 2).chd\"]}}',
                'matched')",
        [],
    )
    .unwrap();

    let snapshot = retro_junk_archive::scan_archive(&root).unwrap();
    retro_junk_db::reconcile_archive_snapshot(
        &mut conn,
        &snapshot,
        &playable_root,
        &temp.path().join("work"),
    )
    .unwrap();

    let bound_carriers: u64 = conn
        .query_row(
            "SELECT COUNT(*) FROM library_entry_media_bindings
             WHERE library_entry_id=1 AND carrier_id IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(bound_carriers, 2, "both archived discs belong to the set");

    let page = retro_junk_db::query_entry_list(
        &conn,
        &retro_junk_db::LibraryEntryListQuery {
            console_id: retro_junk_db::LibraryConsoleId(1),
            search: String::new(),
            filter: retro_junk_db::LibraryEntryFilter::All,
            sort: retro_junk_db::LibraryEntrySortField::DisplayName,
            direction: retro_junk_db::SortDirection::Ascending,
            offset: 0,
            limit: 10,
        },
    )
    .unwrap();
    assert!(
        page.rows.is_empty(),
        "the set is not also an unarchived row"
    );
    assert_eq!(page.availability_counts.playable_only, 0);
    assert_eq!(page.logical_count, 1);
}

/// A catalog verdict is a live comparison of the bytes on disk; recorded
/// evidence must never overwrite it, and user tags stay untouched.
#[test]
fn archive_evidence_never_overwrites_a_catalog_verdict_or_a_tag() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("archive");
    let playable_root = temp.path().join("playable");
    retro_junk_archive::initialize_archive(
        &root,
        &retro_junk_archive::ArchiveRootManifest::new("Collection"),
    )
    .unwrap();
    archive_verified_playable(
        &root,
        &playable_root,
        temp.path(),
        "Named Game",
        "Named Game (USA)",
        "nes/Named Game.nes",
        "nes/Named Game.nes",
    );
    archive_verified_playable(
        &root,
        &playable_root,
        temp.path(),
        "Tagged Game",
        "Tagged Game (USA)",
        "nes/Tagged Game.nes",
        "nes/Tagged Game.nes",
    );

    let mut conn = open_memory().unwrap();
    conn.execute(
        "INSERT INTO library_roots(id,root_path) VALUES(1,?1)",
        [playable_root.to_string_lossy().as_ref()],
    )
    .unwrap();
    conn.execute("INSERT INTO library_consoles(id,root_id,platform,folder_name,folder_path,fingerprint_hash,scan_state) VALUES(1,1,'Nes','nes','/playable/nes','fp','ready')", []).unwrap();
    conn.execute(
        "INSERT INTO library_entries(id,console_id,entry_key,display_name,game_entry_json,status,dat_game_name,dat_match_method)
         VALUES(1,1,'file:Named Game.nes','Named Game.nes','{}','matched','Catalog Name','sha1')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO library_entries(id,console_id,entry_key,display_name,game_entry_json,status,tag)
         VALUES(2,1,'file:Tagged Game.nes','Tagged Game.nes','{}','unrecognized','homebrew')",
        [],
    )
    .unwrap();

    let snapshot = retro_junk_archive::scan_archive(&root).unwrap();
    retro_junk_db::reconcile_archive_snapshot(
        &mut conn,
        &snapshot,
        &playable_root,
        &temp.path().join("work"),
    )
    .unwrap();

    let catalog_row: (String, String, String) = conn
        .query_row(
            "SELECT status,dat_game_name,dat_match_method FROM library_entries WHERE id=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        catalog_row,
        (
            "matched".to_owned(),
            "Catalog Name".to_owned(),
            "sha1".to_owned()
        )
    );
    let tagged_row: (String, String, String) = conn
        .query_row(
            "SELECT status,tag,dat_game_name FROM library_entries WHERE id=2",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        tagged_row,
        (
            "unrecognized".to_owned(),
            "homebrew".to_owned(),
            String::new()
        )
    );
}

/// Evidence that no longer describes the dump's current bytes is history, not
/// a claim: a dump without a current catalog verification names nothing.
#[test]
fn stale_or_absent_catalog_evidence_names_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("archive");
    let playable_root = temp.path().join("playable");
    retro_junk_archive::initialize_archive(
        &root,
        &retro_junk_archive::ArchiveRootManifest::new("Collection"),
    )
    .unwrap();
    archive_verified_playable(
        &root,
        &playable_root,
        temp.path(),
        "Stale Game",
        "Stale Game (USA)",
        "nes/Stale Game.nes",
        "nes/Stale Game.nes",
    );
    // Rewrite the verification against a manifest hash the dump no longer has.
    let snapshot = retro_junk_archive::scan_archive(&root).unwrap();
    let dump = &snapshot.releases[0].physical_copies[0].carriers[0].dumps[0];
    // Specifically the catalog record: ingest also files an integrity one.
    let catalog_record = dump
        .verifications
        .iter()
        .find(|verification| {
            verification.evidence.kind == retro_junk_archive::VerificationKind::Catalog
        })
        .expect("catalog evidence was written above");
    let verification_path = catalog_record.path.clone();
    let mut evidence = catalog_record.evidence.clone();
    evidence.input_manifest_sha256 = "0".repeat(64);
    std::fs::write(
        &verification_path,
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();

    let mut conn = open_memory().unwrap();
    conn.execute(
        "INSERT INTO library_roots(id,root_path) VALUES(1,?1)",
        [playable_root.to_string_lossy().as_ref()],
    )
    .unwrap();
    conn.execute("INSERT INTO library_consoles(id,root_id,platform,folder_name,folder_path,fingerprint_hash,scan_state) VALUES(1,1,'Nes','nes','/playable/nes','fp','ready')", []).unwrap();
    conn.execute(
        "INSERT INTO library_entries(id,console_id,entry_key,display_name,game_entry_json,status)
         VALUES(1,1,'file:Stale Game.nes','Stale Game.nes','{}','unrecognized')",
        [],
    )
    .unwrap();

    let stale_snapshot = retro_junk_archive::scan_archive(&root).unwrap();
    retro_junk_db::reconcile_archive_snapshot(
        &mut conn,
        &stale_snapshot,
        &playable_root,
        &temp.path().join("work"),
    )
    .unwrap();

    let row: (String, String) = conn
        .query_row(
            "SELECT status,dat_game_name FROM library_entries WHERE id=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(row, ("unrecognized".to_owned(), String::new()));
}

/// Hashing writes a verdict computed from the catalog alone. On a machine with
/// no catalog that verdict is "unrecognized", which must not erase the name the
/// archive's evidence already established.
#[test]
fn hashing_without_a_catalog_does_not_erase_an_archive_evidence_name() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("archive");
    let playable_root = temp.path().join("playable");
    retro_junk_archive::initialize_archive(
        &root,
        &retro_junk_archive::ArchiveRootManifest::new("Collection"),
    )
    .unwrap();
    archive_verified_playable(
        &root,
        &playable_root,
        temp.path(),
        "Hashed Game",
        "Hashed Game (USA)",
        "nes/Hashed Game.nes",
        "nes/Hashed Game.nes",
    );

    let mut conn = open_memory().unwrap();
    conn.execute(
        "INSERT INTO library_roots(id,root_path) VALUES(1,?1)",
        [playable_root.to_string_lossy().as_ref()],
    )
    .unwrap();
    conn.execute("INSERT INTO library_consoles(id,root_id,platform,folder_name,folder_path,fingerprint_hash,scan_state) VALUES(1,1,'Nes','nes','/playable/nes','fp','ready')", []).unwrap();
    conn.execute(
        "INSERT INTO library_entries(id,console_id,entry_key,display_name,game_entry_json,status)
         VALUES(1,1,'file:Hashed Game.nes','Hashed Game.nes','{}','unrecognized')",
        [],
    )
    .unwrap();

    let snapshot = retro_junk_archive::scan_archive(&root).unwrap();
    retro_junk_db::reconcile_archive_snapshot(
        &mut conn,
        &snapshot,
        &playable_root,
        &temp.path().join("work"),
    )
    .unwrap();

    retro_junk_db::apply_entry_hash_update(
        &mut conn,
        retro_junk_db::LibraryEntryId(1),
        0,
        &retro_junk_db::EntryHashUpdate {
            status: "unrecognized".to_owned(),
            crc32: "deadbeef".to_owned(),
            sha1: "a".repeat(40),
            md5: "b".repeat(32),
            data_size: 11,
            hash_warnings_json: None,
            disc_verification: "not_applicable".to_owned(),
            dat_game_name: String::new(),
            dat_rom_name: String::new(),
            dat_match_method: String::new(),
            cover_title: String::new(),
            screen_title: String::new(),
            disc_identifications_json: None,
            ambiguous_candidates_json: None,
        },
    )
    .unwrap();

    let row: (String, String, String, String) = conn
        .query_row(
            "SELECT status,dat_game_name,dat_match_method,crc32 FROM library_entries WHERE id=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        row,
        (
            "matched".to_owned(),
            "Hashed Game (USA)".to_owned(),
            "archive_evidence".to_owned(),
            // The freshly computed hashes are still recorded.
            "deadbeef".to_owned()
        )
    );
}

/// The archive recorded CRC32/MD5/SHA-1 when it ingested the master. A library
/// row holding the same bytes should be filled from that record, not by reading
/// the file again over the network.
#[test]
fn library_hashes_come_from_the_archive_instead_of_a_second_read() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("archive");
    let playable_root = temp.path().join("playable");
    retro_junk_archive::initialize_archive(
        &root,
        &retro_junk_archive::ArchiveRootManifest::new("Collection"),
    )
    .unwrap();
    archive_verified_playable(
        &root,
        &playable_root,
        temp.path(),
        "Mirrored Game",
        "Mirrored Game (USA)",
        "nes/Mirrored Game.nes",
        "nes/Mirrored Game.nes",
    );
    let snapshot = retro_junk_archive::scan_archive(&root).unwrap();
    let archived = snapshot.releases[0].physical_copies[0].carriers[0].dumps[0]
        .manifest
        .files[0]
        .clone();
    assert!(
        !archived.crc32.is_empty() && !archived.sha1.is_empty() && !archived.md5.is_empty(),
        "ingest must record every catalog-relevant digest"
    );

    let mut conn = open_memory().unwrap();
    conn.execute("INSERT INTO platforms(id,display_name,short_name,manufacturer,generation,media_type,release_year,description,core_platform) VALUES('nes','NES','NES','Nintendo',3,'cartridge',1983,'','Nes')", []).unwrap();
    conn.execute(
        "INSERT INTO works(id,canonical_name) VALUES('work','Mirrored Game')",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO releases(id,work_id,platform_id,region,title) VALUES('release','work','nes','usa','Mirrored Game')", []).unwrap();
    conn.execute(
        "INSERT INTO media(id,release_id,dat_source,dat_name,rom_name,file_size,crc32,sha1,md5)
         VALUES('media','release','no-intro','Mirrored Game (USA)','Mirrored Game (USA).nes',?1,?2,?3,?4)",
        rusqlite::params![archived.size, archived.crc32, archived.sha1, archived.md5],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO library_roots(id,root_path) VALUES(1,?1)",
        [playable_root.to_string_lossy().as_ref()],
    )
    .unwrap();
    conn.execute("INSERT INTO library_consoles(id,root_id,platform,folder_name,folder_path,fingerprint_hash,scan_state) VALUES(1,1,'Nes','nes','/playable/nes','fp','ready')", []).unwrap();
    conn.execute(
        "INSERT INTO library_entries(id,console_id,entry_key,display_name,game_entry_json,status)
         VALUES(1,1,'file:Mirrored Game.nes','Mirrored Game.nes','{}','unrecognized')",
        [],
    )
    .unwrap();

    retro_junk_db::reconcile_archive_snapshot(
        &mut conn,
        &snapshot,
        &playable_root,
        &temp.path().join("work"),
    )
    .unwrap();

    let row: (String, String, String, i64, String, String, String) = conn
        .query_row(
            "SELECT crc32,sha1,md5,data_size,hash_source,status,dat_game_name
             FROM library_entries WHERE id=1",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        row,
        (
            archived.crc32.clone(),
            archived.sha1.clone(),
            archived.md5.clone(),
            i64::try_from(archived.size).unwrap(),
            "archive_evidence".to_owned(),
            "matched".to_owned(),
            "Mirrored Game (USA)".to_owned(),
        )
    );
}

/// Recorded digests are raw file hashes; the library hashes format-aware
/// payloads. Adoption is therefore only safe when the catalog confirms the
/// recorded digests describe a known dump — otherwise the row must stay empty
/// so a real hash pass still reads it.
#[test]
fn hashes_are_not_adopted_when_the_catalog_does_not_confirm_them() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("archive");
    let playable_root = temp.path().join("playable");
    retro_junk_archive::initialize_archive(
        &root,
        &retro_junk_archive::ArchiveRootManifest::new("Collection"),
    )
    .unwrap();
    archive_verified_playable(
        &root,
        &playable_root,
        temp.path(),
        "Headered Game",
        "Headered Game (USA)",
        "nes/Headered Game.nes",
        "nes/Headered Game.nes",
    );
    let snapshot = retro_junk_archive::scan_archive(&root).unwrap();

    let mut conn = open_memory().unwrap();
    conn.execute("INSERT INTO platforms(id,display_name,short_name,manufacturer,generation,media_type,release_year,description,core_platform) VALUES('nes','NES','NES','Nintendo',3,'cartridge',1983,'','Nes')", []).unwrap();
    conn.execute(
        "INSERT INTO works(id,canonical_name) VALUES('work','Headered Game')",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO releases(id,work_id,platform_id,region,title) VALUES('release','work','nes','usa','Headered Game')", []).unwrap();
    // The catalog knows only the header-stripped payload, so the archive's raw
    // digests match nothing.
    conn.execute(
        "INSERT INTO media(id,release_id,dat_source,dat_name,file_size,crc32,sha1)
         VALUES('media','release','no-intro','Headered Game (USA)',999,'ffffffff',?1)",
        ["f".repeat(40)],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO library_roots(id,root_path) VALUES(1,?1)",
        [playable_root.to_string_lossy().as_ref()],
    )
    .unwrap();
    conn.execute("INSERT INTO library_consoles(id,root_id,platform,folder_name,folder_path,fingerprint_hash,scan_state) VALUES(1,1,'Nes','nes','/playable/nes','fp','ready')", []).unwrap();
    conn.execute(
        "INSERT INTO library_entries(id,console_id,entry_key,display_name,game_entry_json,status)
         VALUES(1,1,'file:Headered Game.nes','Headered Game.nes','{}','unrecognized')",
        [],
    )
    .unwrap();

    retro_junk_db::reconcile_archive_snapshot(
        &mut conn,
        &snapshot,
        &playable_root,
        &temp.path().join("work"),
    )
    .unwrap();

    let row: (String, String, String, String) = conn
        .query_row(
            "SELECT crc32,sha1,hash_source,dat_match_method FROM library_entries WHERE id=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        row,
        (
            String::new(),
            String::new(),
            String::new(),
            // Identity still comes from evidence; only the hash cache is left
            // for a real read.
            "archive_evidence".to_owned()
        )
    );
}

/// Append a second build record for a playable that was rebuilt in place —
/// a newer chdman or a changed recipe writes different bytes to the same path.
fn append_rebuild_evidence(
    root: &std::path::Path,
    title: &str,
    relative_output_path: &str,
    performed_at: &str,
) -> retro_junk_archive::BuildId {
    let snapshot = retro_junk_archive::scan_archive(root).unwrap();
    let dump = snapshot
        .releases
        .iter()
        .find(|release| release.manifest.title == title)
        .map(|release| &release.physical_copies[0].carriers[0].dumps[0])
        .unwrap();
    let build = retro_junk_archive::BuildEvidence {
        schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
        build_id: retro_junk_archive::BuildId::new(),
        parent_representation_id: dump.manifest.representation_id,
        child_representation_id: retro_junk_archive::RepresentationId::new(),
        performed_at: performed_at.to_owned(),
        input_manifest_sha256: dump.manifest_sha256.clone(),
        recipe_version: 1,
        format: retro_junk_archive::RepresentationFormat::Rom,
        relative_output_path: relative_output_path.to_owned(),
        // A rebuild that produced different bytes at the same path.
        output_sha256: "rebuilt".to_owned(),
        output_size: title.len() as u64,
        catalog_verified: true,
        round_trip_verified: false,
        tool: None,
        omitted_features: Vec::new(),
        canonical_intermediate: None,
    };
    std::fs::write(
        dump.directory
            .join("evidence")
            .join(format!("build-{}.json", build.build_id)),
        serde_json::to_vec_pretty(&build).unwrap(),
    )
    .unwrap();
    build.build_id
}

/// Adopting a moved playable appends evidence naming its *new* path. The old
/// record must stop projecting entirely: keying currency on the output path
/// instead of the build lineage left the old path behind as a permanently
/// `missing` representation, which kept deriving adoption work for a release
/// that was already whole and showed the game as both archived-only and
/// playable-only.
#[test]
fn adopting_a_moved_playable_retires_the_representation_at_its_old_path() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("archive");
    let playable_root = temp.path().join("playable");
    retro_junk_archive::initialize_archive(
        &root,
        &retro_junk_archive::ArchiveRootManifest::new("Collection"),
    )
    .unwrap();
    archive_verified_playable(
        &root,
        &playable_root,
        temp.path(),
        "Moved Game",
        "Moved Game (USA)",
        "nes/Moved Game.nes",
        "nes/Moved Game.nes",
    );
    // Renamed outside the archive, then re-adopted by content.
    std::fs::rename(
        playable_root.join("nes/Moved Game.nes"),
        playable_root.join("nes/Moved Game (USA).nes"),
    )
    .unwrap();
    let snapshot = retro_junk_archive::scan_archive(&root).unwrap();
    let orphans = retro_junk_archive::orphaned_playables(
        &snapshot,
        &playable_root,
        &retro_junk_db::playable_system_directory,
    );
    assert_eq!(orphans.len(), 1, "the recorded output path is empty now");
    retro_junk_archive::record_adoption(&orphans[0], "nes/Moved Game (USA).nes").unwrap();

    let snapshot = retro_junk_archive::scan_archive(&root).unwrap();
    let mut conn = open_memory().unwrap();
    retro_junk_db::reconcile_archive_snapshot(
        &mut conn,
        &snapshot,
        &playable_root,
        &temp.path().join("work"),
    )
    .expect("an adopted playable must not abort the projection");

    let rows: Vec<(String, String)> = conn
        .prepare(
            "SELECT relative_path,presence_state FROM representations
             WHERE location_role='playable' ORDER BY relative_path",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![("nes/Moved Game (USA).nes".to_owned(), "present".to_owned())],
        "the old path leaves no missing row behind"
    );
}

/// Build evidence is append-only, so rebuilding a playable derivative leaves
/// two records naming one output path. A representation row is the *current*
/// state of a file and the table admits one row per path, so projecting every
/// historical record aborted the entire reindex — one rebuilt game made the
/// whole archive unprojectable. The newest record wins and the rest stay in
/// the archive as history.
#[test]
fn a_rebuilt_playable_projects_its_newest_build_instead_of_failing_the_reindex() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("archive");
    let playable_root = temp.path().join("playable");
    retro_junk_archive::initialize_archive(
        &root,
        &retro_junk_archive::ArchiveRootManifest::new("Collection"),
    )
    .unwrap();
    archive_verified_playable(
        &root,
        &playable_root,
        temp.path(),
        "Rebuilt Game",
        "Rebuilt Game (USA)",
        "nes/Rebuilt Game.nes",
        "nes/Rebuilt Game.nes",
    );
    let newest = append_rebuild_evidence(
        &root,
        "Rebuilt Game",
        "nes/Rebuilt Game.nes",
        "2026-07-30T00:00:00Z",
    );

    let snapshot = retro_junk_archive::scan_archive(&root).unwrap();
    let mut conn = open_memory().unwrap();
    retro_junk_db::reconcile_archive_snapshot(
        &mut conn,
        &snapshot,
        &playable_root,
        &temp.path().join("work"),
    )
    .expect("a rebuilt playable must not abort the projection");

    let (rows, content): (u32, String) = conn
        .query_row(
            "SELECT COUNT(*), COALESCE(MAX(content_sha256),'') FROM representations
             WHERE location_role='playable' AND relative_path='nes/Rebuilt Game.nes'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(rows, 1, "one file has one current representation");
    assert_eq!(content, "rebuilt", "the newest build is the current one");

    // The superseded build is not projected, so its lineage row is absent too;
    // both records remain in the archive's append-only evidence directory.
    let derivations: u32 = conn
        .query_row(
            "SELECT COUNT(*) FROM derivations WHERE id=?1",
            [newest.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(derivations, 1);

    // Reindexing again is stable rather than accumulating.
    retro_junk_db::reconcile_archive_snapshot(
        &mut conn,
        &snapshot,
        &playable_root,
        &temp.path().join("work"),
    )
    .unwrap();
    let rows: u32 = conn
        .query_row(
            "SELECT COUNT(*) FROM representations WHERE location_role='playable'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rows, 1);
}

/// A media id encodes the DAT release it was minted against, so an archive
/// written on one machine binds carriers to ids a differently versioned import
/// on another machine never creates. Verified on the reference archive
/// 2026-07-31: 201 of 248 carriers read as `unresolved` after a full local
/// import, which in turn hid every catalog-derived binding behind them. The
/// digests the archive recorded do survive the trip, so the projection
/// re-resolves from those rather than trusting the id.
#[test]
fn a_carrier_whose_recorded_media_id_is_absent_re_resolves_from_its_track_digests() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("archive");
    let playable_root = temp.path().join("playable");
    retro_junk_archive::initialize_archive(
        &root,
        &retro_junk_archive::ArchiveRootManifest::new("Collection"),
    )
    .unwrap();
    archive_verified_playable(
        &root,
        &playable_root,
        temp.path(),
        "Rebound Game",
        "Rebound Game (USA)",
        "nes/Rebound Game.nes",
        "nes/Rebound Game.nes",
    );

    let mut conn = open_memory().unwrap();
    // This machine's catalog holds the game under a *different* media id than
    // the archive recorded — the cross-machine case.
    conn.execute_batch(
        "INSERT INTO platforms(id,display_name,short_name,manufacturer,generation,media_type,release_year,description,core_platform)
         VALUES('nes','NES','NES','Nintendo',3,'cartridge',1983,'','Nes');
         INSERT INTO works(id,canonical_name) VALUES('w','Rebound Game');
         INSERT INTO releases(id,work_id,platform_id,region,title)
         VALUES('rel','w','nes','usa','Rebound Game');
         INSERT INTO media(id,release_id,dat_source,dat_name,file_size,sha1,crc32)
         VALUES('rel:rebound-game-usa-nes','rel','no-intro','Rebound Game (USA)',
                12,'0f4d9c1e','11223344');",
    )
    .unwrap();
    // The archive's verification evidence carries the track digest that names
    // it, keyed on nothing this machine minted.
    set_catalog_track_digests(&root, "Rebound Game", 12, "0f4d9c1e");

    let snapshot = retro_junk_archive::scan_archive(&root).unwrap();
    retro_junk_db::reconcile_archive_snapshot(
        &mut conn,
        &snapshot,
        &playable_root,
        &temp.path().join("work"),
    )
    .unwrap();

    let (media, state): (Option<String>, String) = conn
        .query_row(
            "SELECT catalog_media_id,binding_state FROM carriers",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        media.as_deref(),
        Some("rel:rebound-game-usa-nes"),
        "the carrier re-resolves from the digests it recorded"
    );
    assert_eq!(
        state, "rederived",
        "and says so, rather than claiming the recorded id resolved"
    );
}

/// Rewrite the catalog verification evidence for `title`'s dump so it carries
/// one matched track digest.
fn set_catalog_track_digests(root: &std::path::Path, title: &str, size: u64, sha1: &str) {
    let snapshot = retro_junk_archive::scan_archive(root).unwrap();
    let dump = snapshot
        .releases
        .iter()
        .find(|release| release.manifest.title == title)
        .map(|release| &release.physical_copies[0].carriers[0].dumps[0])
        .unwrap();
    for verification in &dump.verifications {
        if verification.evidence.kind != retro_junk_archive::VerificationKind::Catalog {
            continue;
        }
        let mut evidence = verification.evidence.clone();
        evidence.tracks = vec![retro_junk_archive::TrackVerification {
            number: 1,
            size,
            expected_sha1: sha1.to_owned(),
            actual_sha1: sha1.to_owned(),
            matched: true,
        }];
        std::fs::write(
            &verification.path,
            serde_json::to_vec_pretty(&evidence).unwrap(),
        )
        .unwrap();
    }
}

/// A CHD is not a byte-identical mirror of its master, so the digest-equality
/// rule that lets a cartridge row adopt hashes can never fire for a disc — and
/// disc rows kept asking to be re-read even though the archive had already
/// established the answer. Round-trip verification decompressed the derivative
/// and compared it back against a master whose complete track set matched this
/// catalog medium, so re-reading the file cannot produce a different answer.
#[test]
fn a_round_trip_verified_disc_adopts_its_catalog_digests_without_a_second_read() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("archive");
    let playable_root = temp.path().join("playable");
    retro_junk_archive::initialize_archive(
        &root,
        &retro_junk_archive::ArchiveRootManifest::new("Collection"),
    )
    .unwrap();
    archive_verified_playable(
        &root,
        &playable_root,
        temp.path(),
        "Disc Game",
        "Disc Game (USA)",
        "nes/Disc Game.chd",
        "nes/Disc Game.chd",
    );
    // The compressed derivative's own bytes match no master file.
    set_build_flags(&root, "Disc Game", "compressed-bytes-unlike-any-master");

    let mut conn = open_memory().unwrap();
    conn.execute_batch(
        "INSERT INTO platforms(id,display_name,short_name,manufacturer,generation,media_type,release_year,description,core_platform)
         VALUES('nes','NES','NES','Nintendo',3,'cartridge',1983,'','Nes');
         INSERT INTO works(id,canonical_name) VALUES('w','Disc Game');
         INSERT INTO releases(id,work_id,platform_id,region,title)
         VALUES('rel','w','nes','usa','Disc Game');
         INSERT INTO media(id,release_id,dat_source,dat_name,rom_name,file_size,crc32,sha1,md5)
         VALUES('med','rel','redump','Disc Game (USA)','Disc Game (USA) (Track 1).bin',
                652028496,'42fc324d','a2aee128','9f8e7d6c');",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO library_roots(id,root_path) VALUES(1,?1)",
        [playable_root.to_string_lossy().as_ref()],
    )
    .unwrap();
    conn.execute("INSERT INTO library_consoles(id,root_id,platform,folder_name,folder_path,fingerprint_hash,scan_state) VALUES(1,1,'Nes','nes','/playable/nes','fp','ready')", []).unwrap();
    conn.execute(
        "INSERT INTO library_entries(id,console_id,entry_key,display_name,game_entry_json,status)
         VALUES(1,1,'file:Disc Game.chd','Disc Game.chd','{}','unrecognized')",
        [],
    )
    .unwrap();
    bind_carrier_to_media(&root, "Disc Game", "med");

    let snapshot = retro_junk_archive::scan_archive(&root).unwrap();
    retro_junk_db::reconcile_archive_snapshot(
        &mut conn,
        &snapshot,
        &playable_root,
        &temp.path().join("work"),
    )
    .unwrap();

    let (crc32, sha1, size, source): (String, String, i64, String) = conn
        .query_row(
            "SELECT crc32,sha1,data_size,hash_source FROM library_entries WHERE id=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(crc32, "42fc324d", "the disc row adopts without being read");
    assert_eq!(sha1, "a2aee128");
    assert_eq!(size, 652_028_496);
    assert_eq!(
        source, "archive_evidence",
        "and says the digests were adopted, not read here"
    );
}

/// Give `title`'s build evidence a distinct output digest and both verified
/// flags, the shape a compressed round-trip-verified derivative has.
fn set_build_flags(root: &std::path::Path, title: &str, output_sha256: &str) {
    let snapshot = retro_junk_archive::scan_archive(root).unwrap();
    let dump = snapshot
        .releases
        .iter()
        .find(|release| release.manifest.title == title)
        .map(|release| &release.physical_copies[0].carriers[0].dumps[0])
        .unwrap();
    for build in &dump.builds {
        let mut evidence = build.evidence.clone();
        output_sha256.clone_into(&mut evidence.output_sha256);
        evidence.round_trip_verified = true;
        evidence.catalog_verified = true;
        std::fs::write(&build.path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    }
}

/// Point `title`'s carrier at a catalog medium in its portable manifest.
fn bind_carrier_to_media(root: &std::path::Path, title: &str, media_id: &str) {
    let snapshot = retro_junk_archive::scan_archive(root).unwrap();
    let carrier = snapshot
        .releases
        .iter()
        .find(|release| release.manifest.title == title)
        .map(|release| &release.physical_copies[0].carriers[0])
        .unwrap();
    let mut manifest = carrier.manifest.clone();
    media_id.clone_into(&mut manifest.catalog_binding.catalog_media_id);
    retro_junk_archive::write_toml_atomic(&carrier.directory.join("carrier.toml"), &manifest)
        .unwrap();
}

/// The single-file verification path recorded `complete_track_set: true`
/// unconditionally. A medium the catalog stores as separate tracks can still be
/// matched there on its primary (largest track) digests — that identifies the
/// game while verifying one track of it, and calling it a complete set is
/// exactly what the flag exists to prevent. The match result now carries
/// whether the medium has tracks at all.
#[test]
fn a_single_file_match_against_a_multi_track_medium_is_not_a_complete_set() {
    let conn = open_memory().unwrap();
    conn.execute_batch(
        "INSERT INTO platforms(id,display_name,short_name,manufacturer,generation,media_type,release_year,description,core_platform)
         VALUES('psx','PlayStation','PSX','Sony',5,'cd',1994,'','Psx');
         INSERT INTO works(id,canonical_name) VALUES('w','Game');
         INSERT INTO releases(id,work_id,platform_id,region,title)
         VALUES('rel','w','psx','usa','Game');
         INSERT INTO media(id,release_id,dat_source,dat_name,file_size,crc32,sha1)
         VALUES('flat','rel','redump','Flat Game (USA)',12,'11111111','aaaa'),
               ('disc','rel','redump','Disc Game (USA)',34,'22222222','bbbb');
         INSERT INTO media_tracks(media_id,track_number,track_name,file_size,crc32,sha1,md5)
         VALUES('disc',1,'Disc Game (USA) (Track 1).bin',34,'22222222','bbbb','');",
    )
    .unwrap();

    let digests = |size: u64, crc32: &str, sha1: &str| retro_junk_archive::FileDigests {
        size,
        crc32: crc32.to_owned(),
        md5: String::new(),
        sha1: sha1.to_owned(),
        sha256: String::new(),
    };

    let flat =
        retro_junk_db::match_catalog_file(&conn, "psx", &digests(12, "11111111", "aaaa")).unwrap();
    assert_eq!(flat.len(), 1);
    assert!(
        !flat[0].medium_has_tracks,
        "a trackless medium matched by its only file is the complete set"
    );

    let disc =
        retro_junk_db::match_catalog_file(&conn, "psx", &digests(34, "22222222", "bbbb")).unwrap();
    assert_eq!(disc.len(), 1, "still identifies the game");
    assert!(
        disc[0].medium_has_tracks,
        "but one track of a multi-track medium is not a complete set"
    );
}

/// A mark carries the *inputs* catalog ids are minted from, never the ids, so
/// applying it on a machine that has never seen the decision rebuilds the same
/// rows — which is the whole point of keeping marks beside the collection
/// rather than in this device-local database.
#[test]
fn applying_a_homebrew_mark_rebuilds_its_catalog_rows_and_claims_its_file() {
    let temp = tempfile::tempdir().unwrap();
    let conn = open_memory().unwrap();
    conn.execute_batch(
        "INSERT INTO platforms(id,display_name,short_name,manufacturer,generation,media_type,release_year,description,core_platform)
         VALUES('gb','Game Boy','GB','Nintendo',4,'cartridge',1989,'','Gb');
         INSERT INTO library_roots(id,root_path) VALUES(1,'/roms');
         INSERT INTO library_consoles(id,root_id,platform,folder_name,folder_path,fingerprint_hash)
         VALUES(1,1,'Gb','gb','gb','');
         INSERT INTO library_entries(id,console_id,entry_key,display_name,game_entry_json,
                                     crc32,sha1,data_size,status)
         VALUES(1,1,'file:Finchy Quest.gb','Finchy Quest.gb','{}','deadbeef','aaaa1111',262144,'unrecognized');",
    )
    .unwrap();

    let mark = retro_junk_archive::CollectionMark {
        schema_version: 1,
        kind: retro_junk_archive::MarkKind::Homebrew,
        platform_id: "gb".to_owned(),
        region: "usa".to_owned(),
        name: "Finchy Quest".to_owned(),
        parent_work_id: String::new(),
        parent_dat_name: String::new(),
        content: retro_junk_archive::MarkedContent {
            size: 262_144,
            crc32: "deadbeef".to_owned(),
            sha1: "aaaa1111".to_owned(),
            md5: String::new(),
        },
        chosen_media_id: String::new(),
        chosen_dat_name: String::new(),
        note: String::new(),
    };
    retro_junk_archive::write_mark(temp.path(), &mark).unwrap();

    let report = retro_junk_db::apply_collection_marks(&conn, temp.path()).unwrap();
    assert_eq!(report.applied, 1);
    assert_eq!(report.deferred, 0);

    let (tag, media): (String, i64) = conn
        .query_row(
            "SELECT le.tag,(SELECT COUNT(*) FROM media m WHERE m.crc32='deadbeef')
             FROM library_entries le WHERE le.id=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(tag, "homebrew", "the file stops reading as a stranger");
    assert_eq!(
        media, 1,
        "and the catalog medium carries the digests the mark supplied"
    );

    // Idempotent: ids are derived, so a second pass rewrites the same rows.
    let again = retro_junk_db::apply_collection_marks(&conn, temp.path()).unwrap();
    assert_eq!(again.applied, 1);
    let works: i64 = conn
        .query_row("SELECT COUNT(*) FROM works", [], |row| row.get(0))
        .unwrap();
    assert_eq!(works, 1);
}

/// A mod's parent is resolved by DAT name, because media ids are minted per
/// DAT release and do not survive a re-import elsewhere. A machine whose
/// catalog lacks that DAT keeps the decision rather than manufacturing an
/// orphan work for it, and it resolves once the DAT arrives.
#[test]
fn a_mod_waits_for_the_parent_dat_instead_of_inventing_one() {
    let temp = tempfile::tempdir().unwrap();
    let conn = open_memory().unwrap();
    conn.execute_batch(
        "INSERT INTO platforms(id,display_name,short_name,manufacturer,generation,media_type,release_year,description,core_platform)
         VALUES('nes','NES','NES','Nintendo',3,'cartridge',1983,'','Nes');",
    )
    .unwrap();

    let mark = retro_junk_archive::CollectionMark {
        schema_version: 1,
        kind: retro_junk_archive::MarkKind::Modded,
        platform_id: "nes".to_owned(),
        region: "usa".to_owned(),
        name: "Castlevania II (Fan Enhancement)".to_owned(),
        parent_work_id: String::new(),
        parent_dat_name: "Castlevania II - Simon's Quest (USA)".to_owned(),
        content: retro_junk_archive::MarkedContent {
            size: 262_144,
            crc32: "beefcafe".to_owned(),
            sha1: "bbbb2222".to_owned(),
            md5: String::new(),
        },
        chosen_media_id: String::new(),
        chosen_dat_name: String::new(),
        note: String::new(),
    };
    retro_junk_archive::write_mark(temp.path(), &mark).unwrap();

    let before = retro_junk_db::apply_collection_marks(&conn, temp.path()).unwrap();
    assert_eq!(before.deferred, 1, "no parent yet");
    assert_eq!(before.applied, 0);
    let works: i64 = conn
        .query_row("SELECT COUNT(*) FROM works", [], |row| row.get(0))
        .unwrap();
    assert_eq!(works, 0, "and nothing invented in its place");

    // The DAT arrives.
    conn.execute_batch(
        "INSERT INTO works(id,canonical_name) VALUES('nes:castlevania-ii','Castlevania II');
         INSERT INTO releases(id,work_id,platform_id,region,title)
         VALUES('rel','nes:castlevania-ii','nes','usa','Castlevania II');
         INSERT INTO media(id,release_id,dat_source,dat_name)
         VALUES('med','rel','no-intro','Castlevania II - Simon''s Quest (USA)');",
    )
    .unwrap();

    let after = retro_junk_db::apply_collection_marks(&conn, temp.path()).unwrap();
    assert_eq!(after.applied, 1, "the same mark now resolves");
    let parent: String = conn
        .query_row(
            "SELECT r.work_id FROM media m JOIN releases r ON r.id=m.release_id
             WHERE m.crc32='beefcafe'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        parent, "nes:castlevania-ii",
        "the mod hangs off the work it was derived from"
    );
}

/// A DAT that retitles a game mints new catalog ids on the next import, and
/// every archive manifest bound to the old ids is orphaned. The carrier
/// recovers on its own by re-resolving from the digests the archive recorded;
/// the release above it must recover too, or a fully identified set of discs
/// still reads as unidentified with no way to say why.
#[test]
fn a_retitled_catalog_rebinds_the_release_from_its_carriers_content() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("archive");
    retro_junk_archive::initialize_archive(
        &root,
        &retro_junk_archive::ArchiveRootManifest::new("Collection"),
    )
    .unwrap();
    let source = temp.path().join("game.nes");
    std::fs::write(&source, b"cartridge bytes").unwrap();
    let digests = retro_junk_archive::hash_file_digests(&source, &AtomicBool::new(false)).unwrap();
    let ingested = retro_junk_archive::ingest_new_carrier_dump(
        &root,
        &source,
        retro_junk_archive::NewCarrierDump {
            platform_id: "nes".to_owned(),
            title: "Game".to_owned(),
            region: "usa".to_owned(),
            revision: String::new(),
            variant: String::new(),
            owner_id: "default".to_owned(),
            physical_copy_label: String::new(),
            serial: String::new(),
            sequence_number: 0,
            carrier_label: String::new(),
            carrier_kind: retro_junk_archive::CarrierKind::Cartridge,
            format: retro_junk_archive::RepresentationFormat::Rom,
            // Bound against catalog ids minted from the old title.
            catalog_binding: retro_junk_archive::CatalogBinding {
                catalog_work_id: "nes:game".to_owned(),
                catalog_release_id: "nes:game:nes:usa".to_owned(),
                catalog_media_id: "nes:game:nes:usa:1".to_owned(),
                source: "no-intro".to_owned(),
                dat_name: "Game".to_owned(),
                ..Default::default()
            },
            source_package: retro_junk_archive::SourcePackageRecord::default(),
            expected_files: Vec::new(),
            physical_copy_id: None,
        },
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    // The archive's own evidence records that these bytes matched the catalog.
    let snapshot = retro_junk_archive::scan_archive(&root).unwrap();
    let dump = &snapshot.releases[0].physical_copies[0].carriers[0].dumps[0];
    let verification_id = retro_junk_archive::VerificationId::new();
    let evidence_dir = dump.directory.join("evidence");
    std::fs::create_dir_all(&evidence_dir).unwrap();
    retro_junk_archive::write_json_new(
        &evidence_dir.join(format!("verification-{verification_id}.json")),
        &retro_junk_archive::VerificationEvidence {
            schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
            verification_id,
            representation_id: dump.manifest.representation_id,
            performed_at: "2026-01-01T00:00:00Z".to_owned(),
            input_manifest_sha256: dump.manifest_sha256.clone(),
            kind: retro_junk_archive::VerificationKind::Catalog,
            outcome: retro_junk_archive::VerificationOutcome::Verified,
            tool: None,
            catalog: Some(retro_junk_archive::CatalogEvidence {
                source: "no-intro".to_owned(),
                system: "nes".to_owned(),
                version: "1".to_owned(),
                game: "Game".to_owned(),
                complete_track_set: true,
            }),
            tracks: Vec::new(),
            detail: "matched".to_owned(),
        },
    )
    .unwrap();

    // The re-imported catalog holds the same bytes under new, retitled ids.
    let mut conn = open_memory().unwrap();
    conn.execute("INSERT INTO platforms(id,display_name,short_name,manufacturer,generation,media_type,release_year,description,core_platform) VALUES('nes','NES','NES','Nintendo',3,'cartridge',1983,'','Nes')", []).unwrap();
    conn.execute(
        "INSERT INTO works(id,canonical_name) VALUES('nes:game-the-adventure','Game: The Adventure')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO releases(id,work_id,platform_id,region,title)
         VALUES('nes:game-the-adventure:nes:usa','nes:game-the-adventure','nes','usa','Game: The Adventure')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO media(id,release_id,dat_source,dat_name,file_size,crc32,sha1,md5)
         VALUES('nes:game-the-adventure:nes:usa:1','nes:game-the-adventure:nes:usa','no-intro','Game: The Adventure',?1,?2,?3,?4)",
        rusqlite::params![digests.size, digests.crc32, digests.sha1, digests.md5],
    )
    .unwrap();

    let snapshot = retro_junk_archive::scan_archive(&root).unwrap();
    retro_junk_db::reconcile_archive_snapshot(
        &mut conn,
        &snapshot,
        &temp.path().join("playable"),
        &temp.path().join("work"),
    )
    .unwrap();

    let archive_release_id = ingested.release.archive_release_id.to_string();
    let (release_id, work_id, state): (Option<String>, Option<String>, String) = conn
        .query_row(
            "SELECT catalog_release_id,catalog_work_id,binding_state
             FROM archive_releases WHERE id=?1",
            [archive_release_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        release_id.as_deref(),
        Some("nes:game-the-adventure:nes:usa"),
        "the release did not recover the identity its carrier proved by content"
    );
    assert_eq!(work_id.as_deref(), Some("nes:game-the-adventure"));
    assert_eq!(state, "rederived");

    // And the claim the manifest still makes is preserved, so the difference
    // between "recovered" and "never identified" stays visible.
    let claimed: String = conn
        .query_row(
            "SELECT claimed_release_id FROM archive_releases WHERE id=?1",
            [archive_release_id.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(claimed, "nes:game:nes:usa");
}

/// An archive can already contain two live build records claiming one
/// representation, because a rename wrote the wrong format and started a
/// second lineage instead of superseding the first. Those records are on
/// disk and `evidence/` is append-only, so the projection has to cope with
/// them rather than failing — a unique-constraint error here rolls back the
/// whole reconcile, and every renamed playable reads as missing afterwards.
#[test]
fn two_live_builds_claiming_one_representation_project_the_newer() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("archive");
    let playable_root = temp.path().join("playable");
    retro_junk_archive::initialize_archive(
        &root,
        &retro_junk_archive::ArchiveRootManifest::new("Collection"),
    )
    .unwrap();
    let source = temp.path().join("master.bin");
    std::fs::write(&source, b"master bytes").unwrap();
    let ingested = retro_junk_archive::ingest_new_carrier_dump(
        &root,
        &source,
        retro_junk_archive::NewCarrierDump {
            platform_id: "psx".to_owned(),
            title: "Game".to_owned(),
            region: "usa".to_owned(),
            revision: String::new(),
            variant: String::new(),
            owner_id: "default".to_owned(),
            physical_copy_label: String::new(),
            serial: String::new(),
            sequence_number: 0,
            carrier_label: String::new(),
            carrier_kind: retro_junk_archive::CarrierKind::OpticalDisc,
            format: retro_junk_archive::RepresentationFormat::CueBin,
            catalog_binding: retro_junk_archive::CatalogBinding::default(),
            source_package: retro_junk_archive::SourcePackageRecord::default(),
            expected_files: Vec::new(),
            physical_copy_id: None,
        },
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();

    let system_dir = playable_root.join("psx");
    std::fs::create_dir_all(&system_dir).unwrap();
    std::fs::write(system_dir.join("Game (USA).chd"), b"playable").unwrap();
    let child = retro_junk_archive::RepresentationId::new();
    let base = retro_junk_archive::BuildEvidence {
        schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
        build_id: retro_junk_archive::BuildId::new(),
        parent_representation_id: ingested.dump.representation_id,
        child_representation_id: child,
        performed_at: "2026-01-01T00:00:00Z".to_owned(),
        input_manifest_sha256: String::new(),
        recipe_version: 1,
        format: retro_junk_archive::RepresentationFormat::Chd,
        relative_output_path: "psx/Game (USA) (Track 1).chd".to_owned(),
        output_sha256: String::new(),
        output_size: 8,
        catalog_verified: false,
        round_trip_verified: false,
        tool: None,
        omitted_features: Vec::new(),
        canonical_intermediate: None,
    };
    retro_junk_archive::write_build_evidence(&ingested.dump_directory, &base).unwrap();
    // The record the buggy rename wrote: same representation, later, but a
    // different format — so it reads as its own lineage rather than a
    // replacement.
    retro_junk_archive::write_build_evidence(
        &ingested.dump_directory,
        &retro_junk_archive::BuildEvidence {
            build_id: retro_junk_archive::BuildId::new(),
            performed_at: "2026-02-01T00:00:00Z".to_owned(),
            relative_output_path: "psx/Game (USA).chd".to_owned(),
            format: retro_junk_archive::RepresentationFormat::CueBin,
            ..base.clone()
        },
    )
    .unwrap();

    let snapshot = retro_junk_archive::scan_archive(&root).unwrap();
    let mut conn = open_memory().unwrap();
    retro_junk_db::reconcile_archive_snapshot(
        &mut conn,
        &snapshot,
        &playable_root,
        &temp.path().join("work"),
    )
    .expect("the projection must survive an archive it can read");

    // One row, naming where the file actually is.
    let (count, path): (i64, String) = conn
        .query_row(
            "SELECT COUNT(*),MAX(relative_path) FROM representations WHERE role='playable'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(path, "psx/Game (USA).chd");
}
