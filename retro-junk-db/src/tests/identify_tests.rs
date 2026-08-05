//! The ladder, rung by rung. See `IDENTIFICATION.md`.

use super::*;
use crate::schema::open_memory;

/// Insert one catalog medium, optionally as a set of tracks.
///
/// `tracks` is `(size, sha1)` per track in order; an empty list stores the
/// medium as a single file, which is how the importer stores a single-ROM game.
fn add_media(
    conn: &Connection,
    platform: &str,
    media_id: &str,
    title: &str,
    primary: (u64, &str),
    tracks: &[(u64, &str)],
) {
    let work_id = format!("{platform}:{title}");
    let release_id = format!("{work_id}:{platform}:usa");
    conn.execute(
        "INSERT OR IGNORE INTO works(id,canonical_name) VALUES(?1,?2)",
        rusqlite::params![work_id, title],
    )
    .unwrap();
    conn.execute(
        "INSERT OR IGNORE INTO platforms(id,display_name,short_name,manufacturer,media_type)
         VALUES(?1,?1,?1,'','cartridge')",
        [platform],
    )
    .unwrap();
    conn.execute(
        "INSERT OR IGNORE INTO releases(id,work_id,platform_id,region,title) VALUES(?1,?2,?3,'usa',?4)",
        rusqlite::params![release_id, work_id, platform, title],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO media(id,release_id,dat_name,rom_name,file_size,sha1,crc32)
         VALUES(?1,?2,?3,?3,?4,?5,'')",
        rusqlite::params![
            media_id,
            release_id,
            title,
            i64::try_from(primary.0).unwrap(),
            primary.1
        ],
    )
    .unwrap();
    for (index, (size, sha1)) in tracks.iter().enumerate() {
        conn.execute(
            "INSERT INTO media_tracks(media_id,track_number,track_name,file_size,sha1)
             VALUES(?1,?2,'',?3,?4)",
            rusqlite::params![
                media_id,
                i64::try_from(index + 1).unwrap(),
                i64::try_from(*size).unwrap(),
                sha1
            ],
        )
        .unwrap();
    }
}

fn track(number: u32, size: u64, sha1: &str) -> TrackDigest {
    TrackDigest {
        number,
        size,
        crc32: String::new(),
        md5: String::new(),
        sha1: sha1.to_owned(),
    }
}

fn evidence(platform: &str, tracks: Vec<TrackDigest>) -> Evidence<'_> {
    Evidence {
        platform_id: platform,
        tracks,
        ..Evidence::default()
    }
}

/// A cartridge is the one-track case, so it reaches the top rung for free.
#[test]
fn a_single_file_medium_that_matches_is_complete() {
    let conn = open_memory().unwrap();
    add_media(&conn, "nes", "m1", "Game", (100, "aaa"), &[]);

    let found = identify(&conn, &evidence("nes", vec![track(1, 100, "aaa")])).unwrap();
    assert_eq!(
        found,
        Identification::Complete {
            media_id: "m1".to_owned(),
            release_id: "nes:Game:nes:usa".to_owned(),
        }
    );
    assert!(found.is_actionable());
}

/// The bug this whole design exists for: two discs sharing their largest data
/// track. The primary hash alone must not pick one of them.
#[test]
fn one_shared_data_track_never_identifies_a_multi_track_disc() {
    let conn = open_memory().unwrap();
    add_media(
        &conn,
        "ps1",
        "shared-a",
        "Monster Lair",
        (500, "data"),
        &[(500, "data"), (900, "audio-a")],
    );
    add_media(
        &conn,
        "ps1",
        "shared-b",
        "Wonder Boy III",
        (500, "data"),
        &[(500, "data"), (900, "audio-b")],
    );

    // Only the shared data track hashed: two discs remain possible.
    let found = identify(&conn, &evidence("ps1", vec![track(1, 500, "data")])).unwrap();
    assert!(
        matches!(found, Identification::Ambiguous { ref candidates } if candidates.len() == 2),
        "one shared track picked a single disc: {found:?}"
    );
    assert!(
        !found.is_actionable(),
        "an ambiguous disc must not be acted on"
    );

    // The audio track is what tells them apart.
    let found = identify(
        &conn,
        &evidence("ps1", vec![track(1, 500, "data"), track(2, 900, "audio-b")]),
    )
    .unwrap();
    assert_eq!(found.media_id(), Some("shared-b"));
    assert!(matches!(found, Identification::Complete { .. }));
}

/// Hashing some of a disc's tracks narrows it to one entry without verifying
/// it. That is actionable, but it must say what is missing.
#[test]
fn a_partly_hashed_disc_is_unique_and_says_what_is_missing() {
    let conn = open_memory().unwrap();
    add_media(
        &conn,
        "ps1",
        "only",
        "Solo",
        (500, "d"),
        &[(500, "d"), (900, "a")],
    );

    let found = identify(
        &conn,
        &Evidence {
            platform_id: "ps1",
            tracks: vec![track(1, 500, "d"), track(2, 900, "a")],
            total_tracks: Some(3),
            ..Evidence::default()
        },
    )
    .unwrap();
    assert!(
        found.is_actionable(),
        "a single possible entry is a certain answer"
    );
    assert!(!matches!(found, Identification::Complete { .. }));
    assert_eq!(
        found.incompleteness(),
        Some(&Incompleteness::TracksUnhashed {
            hashed: 2,
            total: 3
        })
    );
    assert!(
        found
            .incompleteness()
            .unwrap()
            .explain()
            .contains("hash the rest")
    );
}

/// Hashes that match nothing, when a catalog for the platform exists, means
/// the bytes are wrong — not that the game is unknown. Different problem,
/// different fix, and it used to render identically.
#[test]
fn hashes_that_match_nothing_are_distinguished_from_having_no_catalog() {
    let conn = open_memory().unwrap();
    add_media(&conn, "nes", "m1", "Game", (100, "aaa"), &[]);

    let wrong = identify(&conn, &evidence("nes", vec![track(1, 100, "zzz")])).unwrap();
    assert_eq!(
        wrong,
        Identification::Unidentified {
            why: Incompleteness::HashesDisagree
        }
    );

    let uncatalogued = identify(&conn, &evidence("saturn", vec![track(1, 100, "zzz")])).unwrap();
    assert_eq!(
        uncatalogued,
        Identification::Unidentified {
            why: Incompleteness::NoCatalogForPlatform
        }
    );
}

/// A person may choose only from the entries that were actually possible.
#[test]
fn a_manual_choice_is_limited_to_the_candidates_and_never_reads_as_verified() {
    let conn = open_memory().unwrap();
    add_media(
        &conn,
        "ps1",
        "shared-a",
        "A",
        (500, "d"),
        &[(500, "d"), (900, "a1")],
    );
    add_media(
        &conn,
        "ps1",
        "shared-b",
        "B",
        (500, "d"),
        &[(500, "d"), (900, "a2")],
    );

    let chosen = identify(
        &conn,
        &Evidence {
            platform_id: "ps1",
            tracks: vec![track(1, 500, "d")],
            manual_media_id: Some("shared-b"),
            ..Evidence::default()
        },
    )
    .unwrap();
    assert_eq!(chosen.media_id(), Some("shared-b"));
    assert!(chosen.is_actionable());
    assert!(
        !matches!(chosen, Identification::Complete { .. }),
        "a hand-picked entry must never report as verified"
    );
    assert_eq!(
        chosen.candidates().len(),
        2,
        "the choice stays re-selectable"
    );

    // An entry that was never a candidate cannot be forced in.
    let invented = identify(
        &conn,
        &Evidence {
            platform_id: "ps1",
            tracks: vec![track(1, 500, "d")],
            manual_media_id: Some("something-else"),
            ..Evidence::default()
        },
    )
    .unwrap();
    assert!(
        matches!(invented, Identification::Ambiguous { .. }),
        "a choice outside the candidate list was accepted: {invented:?}"
    );
}

/// Homebrew and hacks are not defects. They have no catalog entry and never
/// will, so they must not read as a gap waiting to be closed.
#[test]
fn uncatalogued_content_is_not_treated_as_a_missing_match() {
    let conn = open_memory().unwrap();
    let found = identify(
        &conn,
        &Evidence {
            platform_id: "nes",
            tracks: vec![track(1, 100, "aaa")],
            not_catalogued: true,
            ..Evidence::default()
        },
    )
    .unwrap();
    assert_eq!(
        found,
        Identification::Unidentified {
            why: Incompleteness::NotCatalogued
        }
    );
}
