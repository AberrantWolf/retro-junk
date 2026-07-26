use retro_junk_db::{analyze_catalog_duplicates, deduplicate_catalog, open_memory};

fn seed_exact_duplicates(conn: &retro_junk_db::Connection) {
    conn.execute("INSERT INTO platforms(id,display_name,short_name,manufacturer,generation,media_type,release_year,description,core_platform) VALUES('ps1','PlayStation','PS1','Sony',5,'disc',1994,'','Ps1')", []).unwrap();
    conn.execute(
        "INSERT INTO works(id,canonical_name) VALUES('work','Game')",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO releases(id,work_id,platform_id,region,revision,title) VALUES('release','work','ps1','japan','','Game')", []).unwrap();
    for id in ["a", "b"] {
        conn.execute(
            "INSERT INTO media(id,release_id,media_serial,disc_number,rom_name,dat_source)
             VALUES(?1,'release','SLPS-02300',1,'track.bin','redump')",
            [id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO media_serial_keys(media_id,serial_key) VALUES(?1,'SLPS02300')",
            [id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO media_tracks(media_id,track_number,track_name,file_size,crc32,sha1,md5)
             VALUES(?1,1,'track.bin',4,'abcd','1111','2222')",
            [id],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO collection(media_id,user_id) VALUES('b','default')",
        [],
    )
    .unwrap();
}

#[test]
fn exact_cleanup_is_dry_run_then_repoints_references_and_is_idempotent() {
    let conn = open_memory().unwrap();
    seed_exact_duplicates(&conn);

    let report = analyze_catalog_duplicates(&conn, Some("ps1")).unwrap();
    assert_eq!(report.exact_groups.len(), 1);
    assert!(!report.applied);
    assert_eq!(
        conn.query_row("SELECT count(*) FROM media", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        2
    );

    let applied = deduplicate_catalog(&conn, Some("ps1")).unwrap();
    assert!(applied.applied);
    assert_eq!(applied.exact_groups[0].canonical_media_id, "b");
    assert_eq!(
        conn.query_row("SELECT media_id FROM collection", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        "b"
    );
    assert!(
        analyze_catalog_duplicates(&conn, None)
            .unwrap()
            .exact_groups
            .is_empty()
    );
}

#[test]
fn differing_complete_tracks_are_only_suspected() {
    let conn = open_memory().unwrap();
    seed_exact_duplicates(&conn);
    conn.execute(
        "UPDATE media_tracks SET sha1='different' WHERE media_id='a'",
        [],
    )
    .unwrap();
    let report = analyze_catalog_duplicates(&conn, None).unwrap();
    assert!(report.exact_groups.is_empty());
    assert_eq!(report.suspected_groups, 1);
}
