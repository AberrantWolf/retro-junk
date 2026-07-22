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
fn archive_projection_is_rebuildable_from_portable_manifests() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("archive");
    let root_manifest = retro_junk_archive::ArchiveRootManifest::new("Collection");
    retro_junk_archive::initialize_archive(&root, &root_manifest).unwrap();
    let source = temp.path().join("game.nes");
    std::fs::write(&source, b"game").unwrap();
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
            catalog_binding: retro_junk_archive::CatalogBinding::default(),
            source_package: retro_junk_archive::SourcePackageRecord::default(),
            physical_copy_id: None,
        },
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    let snapshot = retro_junk_archive::scan_archive(&root).unwrap();
    let mut conn = open_memory().unwrap();
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
