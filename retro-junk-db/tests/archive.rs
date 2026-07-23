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
            "media-flat",
            None,
            "archive_adoption",
        )
        .unwrap(),
        1
    );
    let binding: (String, Option<String>) = conn
        .query_row(
            "SELECT catalog_media_id,representation_id FROM library_entry_media_bindings",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(binding, ("media-flat".to_owned(), None));
    actual.md5 = "different".to_owned();
    assert!(
        match_catalog_file(&conn, "nes", &actual)
            .unwrap()
            .is_empty()
    );
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
    retro_junk_archive::ingest_new_carrier_dump(
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
    assert_eq!(summary.integrity_verified_count, 0);
    assert_eq!(summary.expected_disc_count, 1);
    assert_eq!(summary.verified_disc_count, 0);
    assert!(!summary.archive_complete);
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
    assert!(page.rows[0].archived);
    assert!(!page.rows[0].archive_complete);
    assert_eq!(page.rows[0].playable_format, "nes");
    assert_eq!(page.rows[0].preferred_format.as_deref(), Some("rom"));
    assert_eq!(page.availability_counts.archived_and_playable, 0);
    assert_eq!(page.availability_counts.incomplete_archive_and_playable, 1);
    assert_eq!(page.availability_counts.archived_not_playable, 0);
    assert_eq!(page.archived_playable_gaps.len(), 1);
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
    assert_eq!(gaps.archived_playable_gaps[0].title, "Game");
    assert_eq!(
        gaps.archived_playable_gaps[0].preferred_format.as_deref(),
        Some("rom")
    );
    assert_eq!(gaps.archived_playable_gaps[0].expected_disc_count, 1);
    assert_eq!(
        page.rows[0].archive_release_id.as_deref(),
        Some(summary.archive_release_id.as_str())
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
