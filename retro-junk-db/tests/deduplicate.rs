//! Catalog entries that claim to be the same edition while their bytes differ.
//!
//! The byte-identical case is gone by construction: a media id is folded from
//! the medium's digests, so two rows with the same content share one primary
//! key. What is left is the case no key can settle.

use retro_junk_db::{analyze_catalog_duplicates, open_memory};

/// Two media under one release, same disc number and serial — so they claim to
/// be the same edition — each with its own track digests.
fn seed_two_claiming_the_same_edition(conn: &retro_junk_db::Connection) {
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
}

#[test]
fn rows_that_agree_about_their_bytes_are_not_a_conflict() {
    let conn = open_memory().unwrap();
    seed_two_claiming_the_same_edition(&conn);
    // Two rows this alike could only exist in a catalog built by the old
    // title-derived key. Content-derived ids would have collapsed them into
    // one, so there is nothing here for a person to decide.
    let report = analyze_catalog_duplicates(&conn, Some("ps1")).unwrap();
    assert!(report.suspected_groups.is_empty());
}

#[test]
fn rows_that_claim_the_same_edition_but_hash_differently_are_reported() {
    let conn = open_memory().unwrap();
    seed_two_claiming_the_same_edition(&conn);
    conn.execute(
        "UPDATE media_tracks SET sha1='different' WHERE media_id='a'",
        [],
    )
    .unwrap();

    let report = analyze_catalog_duplicates(&conn, None).unwrap();
    assert_eq!(report.suspected_groups.len(), 1);
    assert_eq!(report.suspected_groups[0].media_ids, vec!["a", "b"]);
    assert_eq!(report.suspected_groups[0].platform_id, "ps1");
}

#[test]
fn a_platform_filter_narrows_the_report() {
    let conn = open_memory().unwrap();
    seed_two_claiming_the_same_edition(&conn);
    conn.execute(
        "UPDATE media_tracks SET sha1='different' WHERE media_id='a'",
        [],
    )
    .unwrap();

    assert_eq!(
        analyze_catalog_duplicates(&conn, Some("ps1"))
            .unwrap()
            .suspected_groups
            .len(),
        1
    );
    assert!(
        analyze_catalog_duplicates(&conn, Some("snes"))
            .unwrap()
            .suspected_groups
            .is_empty()
    );
}

#[test]
fn media_of_different_discs_are_not_confused_with_each_other() {
    let conn = open_memory().unwrap();
    seed_two_claiming_the_same_edition(&conn);
    conn.execute("UPDATE media SET disc_number=2 WHERE id='a'", [])
        .unwrap();
    conn.execute(
        "UPDATE media_tracks SET sha1='different' WHERE media_id='a'",
        [],
    )
    .unwrap();

    // Disc 1 and disc 2 of one release are meant to differ.
    let report = analyze_catalog_duplicates(&conn, None).unwrap();
    assert!(report.suspected_groups.is_empty());
}
