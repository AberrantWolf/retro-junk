use retro_junk_catalog::types::*;
use retro_junk_core::Platform;
use retro_junk_dat::{DatFile, DatGame, DatRom};
use retro_junk_db::*;
use retro_junk_import::*;

fn setup_db() -> rusqlite::Connection {
    let conn = open_memory().unwrap();
    let platform = CatalogPlatform {
        id: "nes".to_string(),
        display_name: "Nintendo Entertainment System".to_string(),
        short_name: "NES".to_string(),
        manufacturer: "Nintendo".to_string(),
        generation: 3,
        media_type: MediaType::Cartridge,
        release_year: 1985,
        description: String::new(),
        core_platform: "Nes".to_string(),
        regions: vec![],
        relationships: vec![],
    };
    upsert_platform(&conn, &platform).unwrap();
    conn
}

fn sample_dat() -> DatFile {
    DatFile {
        name: "Nintendo - Nintendo Entertainment System".to_string(),
        description: "Nintendo - NES".to_string(),
        version: "2024-01-15".to_string(),
        games: vec![
            DatGame {
                name: "Super Mario Bros. (USA)".to_string(),
                region: None,
                serial: None,
                version: None,
                category: None,
                roms: vec![DatRom {
                    name: "Super Mario Bros. (USA).nes".to_string(),
                    size: 40976,
                    crc: "d445f698".to_string(),
                    sha1: Some("ea343f4e445a9050d4b4fbac2c77d0693b1d0922".to_string()),
                    md5: None,
                    serial: None,
                }],
            },
            DatGame {
                name: "The Legend of Zelda (USA)".to_string(),
                region: None,
                serial: None,
                version: None,
                category: None,
                roms: vec![DatRom {
                    name: "Legend of Zelda, The (USA).nes".to_string(),
                    size: 131_088,
                    crc: "a12d74c1".to_string(),
                    sha1: Some("7fcbc2007a277e05f97054153cc850eb47589bcd".to_string()),
                    md5: None,
                    serial: None,
                }],
            },
            DatGame {
                name: "The Legend of Zelda (USA) (Rev A)".to_string(),
                region: None,
                serial: None,
                version: None,
                category: None,
                roms: vec![DatRom {
                    name: "Legend of Zelda, The (USA) (Rev A).nes".to_string(),
                    size: 131_088,
                    crc: "cebd2a31".to_string(),
                    sha1: Some("4addc7c8bc3ab5ba5421c4f1f6e5bba4fbafc4de".to_string()),
                    md5: None,
                    serial: None,
                }],
            },
            DatGame {
                name: "Bad Game (USA) [b]".to_string(),
                region: None,
                serial: None,
                version: None,
                category: None,
                roms: vec![DatRom {
                    name: "Bad Game (USA) [b].nes".to_string(),
                    size: 16384,
                    crc: "00000000".to_string(),
                    sha1: None,
                    md5: None,
                    serial: None,
                }],
            },
        ],
    }
}

#[test]
fn import_creates_works_releases_media() {
    let conn = setup_db();
    let dat = sample_dat();

    let stats = import_dat(&conn, &dat, Platform::Nes, "no-intro", &SilentProgress).unwrap();

    // 3 unique works (SMB, Zelda, Bad Game skipped)
    assert_eq!(stats.works_created, 2);
    assert_eq!(stats.media_created, 3); // SMB + Zelda + Zelda Rev A
    assert_eq!(stats.skipped_bad, 1);
}

#[test]
fn import_creates_correct_releases() {
    let conn = setup_db();
    let dat = sample_dat();
    import_dat(&conn, &dat, Platform::Nes, "no-intro", &SilentProgress).unwrap();

    let releases = releases_for_platform(&conn, "nes").unwrap();
    assert_eq!(releases.len(), 3); // SMB + Zelda + Zelda Rev A (revisions are separate releases)

    let titles: Vec<&str> = releases.iter().map(|r| r.title.as_str()).collect();
    assert!(titles.contains(&"Super Mario Bros."));
    assert!(titles.contains(&"The Legend of Zelda"));

    // Zelda Rev A should have revision set on its release
    let zelda_reva = releases
        .iter()
        .find(|r| r.title == "The Legend of Zelda" && r.revision == "Rev A")
        .expect("should have Zelda Rev A release");
    assert_eq!(zelda_reva.revision, "Rev A");
}

#[test]
fn import_media_has_correct_hashes() {
    let conn = setup_db();
    let dat = sample_dat();
    import_dat(&conn, &dat, Platform::Nes, "no-intro", &SilentProgress).unwrap();

    let media = find_media_by_crc32(&conn, "d445f698").unwrap();
    assert_eq!(media.len(), 1);
    assert_eq!(media[0].sha1, "ea343f4e445a9050d4b4fbac2c77d0693b1d0922");
    assert_eq!(media[0].file_size, 40976);
    assert_eq!(media[0].dat_source, "no-intro");
    assert_eq!(
        media[0].dat_system,
        "Nintendo - Nintendo Entertainment System"
    );
    assert_eq!(media[0].rom_name, "Super Mario Bros. (USA).nes");
}

#[test]
fn import_revision_creates_separate_media() {
    let conn = setup_db();
    let dat = sample_dat();
    import_dat(&conn, &dat, Platform::Nes, "no-intro", &SilentProgress).unwrap();

    // Both Zelda entries should exist as media
    let zelda_orig = find_media_by_crc32(&conn, "a12d74c1").unwrap();
    let zelda_reva = find_media_by_crc32(&conn, "cebd2a31").unwrap();
    assert_eq!(zelda_orig.len(), 1);
    assert_eq!(zelda_reva.len(), 1);

    // Rev A media should have revision set
    assert_eq!(zelda_reva[0].revision, "Rev A");
    assert!(zelda_orig[0].revision.is_empty());

    // Revisions now create separate releases
    assert_ne!(zelda_orig[0].release_id, zelda_reva[0].release_id);
}

#[test]
fn import_preserves_dat_native_version_when_name_has_no_revision() {
    let conn = setup_db();
    let mut dat = sample_dat();
    dat.games.truncate(1);
    dat.games[0].version = Some("1.006".to_string());

    import_dat(&conn, &dat, Platform::Nes, "redump", &SilentProgress).unwrap();

    let releases = releases_for_platform(&conn, "nes").unwrap();
    assert_eq!(releases.len(), 1);
    assert!(releases[0].revision.is_empty());
    let media = media_for_release(&conn, &releases[0].id).unwrap();
    assert_eq!(media[0].revision, "1.006");
}

#[test]
fn reimport_backfills_dat_native_version_without_duplicate_release() {
    let conn = setup_db();
    let mut dat = sample_dat();
    dat.games.truncate(1);
    import_dat(&conn, &dat, Platform::Nes, "redump", &SilentProgress).unwrap();

    dat.games[0].version = Some("1.006".to_string());
    let stats = import_dat(&conn, &dat, Platform::Nes, "redump", &SilentProgress).unwrap();

    assert_eq!(stats.media_updated, 1);
    let releases = releases_for_platform(&conn, "nes").unwrap();
    assert_eq!(releases.len(), 1);
    let media = media_for_release(&conn, &releases[0].id).unwrap();
    assert_eq!(media.len(), 1);
    assert_eq!(media[0].revision, "1.006");
}

#[test]
fn reimport_is_idempotent() {
    let conn = setup_db();
    let dat = sample_dat();

    let stats1 = import_dat(&conn, &dat, Platform::Nes, "no-intro", &SilentProgress).unwrap();
    assert_eq!(stats1.media_created, 3);

    let stats2 = import_dat(&conn, &dat, Platform::Nes, "no-intro", &SilentProgress).unwrap();
    assert_eq!(stats2.media_created, 0);
    assert_eq!(stats2.media_unchanged, 3);
    // 3 games processed (bad dump skipped), each finds existing work
    // (SMB=1, Zelda=1, Zelda Rev A=1 → 3 existing-work hits)
    assert_eq!(stats2.works_existing, 3);
}

#[test]
fn reimport_backfills_primary_rom_name() {
    let conn = setup_db();
    let dat = sample_dat();
    import_dat(&conn, &dat, Platform::Nes, "no-intro", &SilentProgress).unwrap();
    conn.execute("UPDATE media SET rom_name=''", []).unwrap();

    let stats = import_dat(&conn, &dat, Platform::Nes, "no-intro", &SilentProgress).unwrap();
    assert_eq!(stats.media_updated, 3);
    assert_eq!(
        find_media_by_crc32(&conn, "d445f698").unwrap()[0].rom_name,
        "Super Mario Bros. (USA).nes"
    );
}

#[test]
fn same_game_name_preserves_distinct_rom_representations() {
    let conn = setup_db();
    let dat = DatFile {
        name: "Nintendo - Nintendo 64".to_string(),
        description: "Nintendo - Nintendo 64".to_string(),
        version: "1".to_string(),
        games: vec![
            DatGame {
                name: "Super Mario 64 (USA)".to_string(),
                region: Some("USA".to_string()),
                serial: Some("NSME".to_string()),
                version: None,
                category: None,
                roms: vec![DatRom {
                    name: "Super Mario 64 (USA).z64".to_string(),
                    size: 8_388_608,
                    crc: "3ce60709".to_string(),
                    sha1: Some("9bef1128717f958171a4afac3ed78ee2bb4e86ce".to_string()),
                    md5: None,
                    serial: Some("NSME".to_string()),
                }],
            },
            DatGame {
                name: "Super Mario 64 (USA)".to_string(),
                region: Some("USA".to_string()),
                serial: Some("NSME".to_string()),
                version: None,
                category: None,
                roms: vec![DatRom {
                    name: "Super Mario 64 (USA).v64".to_string(),
                    size: 8_388_608,
                    crc: "42c43204".to_string(),
                    sha1: Some("1002dd7b56aa0a59a9103f1fb3d57d6b161f8da7".to_string()),
                    md5: None,
                    serial: Some("NSME".to_string()),
                }],
            },
        ],
    };

    let stats = import_dat(&conn, &dat, Platform::Nes, "no-intro", &SilentProgress).unwrap();
    assert_eq!(stats.media_created, 2);
    assert_eq!(find_media_by_crc32(&conn, "3ce60709").unwrap().len(), 1);
    assert_eq!(find_media_by_crc32(&conn, "42c43204").unwrap().len(), 1);

    let second = import_dat(&conn, &dat, Platform::Nes, "no-intro", &SilentProgress).unwrap();
    assert_eq!(second.media_created, 0);
    assert_eq!(second.media_unchanged, 2);
}

#[test]
fn bad_dumps_skipped() {
    let conn = setup_db();
    let dat = sample_dat();
    let stats = import_dat(&conn, &dat, Platform::Nes, "no-intro", &SilentProgress).unwrap();

    assert_eq!(stats.skipped_bad, 1);
    assert_eq!(stats.total_games, 4);
}

#[test]
fn log_import_records_stats() {
    let conn = setup_db();
    let dat = sample_dat();
    let stats = import_dat(&conn, &dat, Platform::Nes, "no-intro", &SilentProgress).unwrap();

    let log_id = log_import(&conn, "no-intro", "Nintendo - NES", "2024-01-15", &stats).unwrap();
    assert!(log_id > retro_junk_catalog::ImportLogId(0));

    let logs = list_import_logs(&conn, None).unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].source_name, "Nintendo - NES");
    assert_eq!(logs[0].records_created, 3);
    let snapshot: (String, String, String) = conn
        .query_row(
            "SELECT source,system,version FROM catalog_source_snapshots",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        snapshot,
        (
            "no-intro".to_owned(),
            "Nintendo - NES".to_owned(),
            "2024-01-15".to_owned()
        )
    );
}

#[test]
fn multi_region_game() {
    let conn = setup_db();
    let dat = DatFile {
        name: "Test".to_string(),
        description: "Test".to_string(),
        version: "1".to_string(),
        games: vec![DatGame {
            name: "Tetris (USA, Europe)".to_string(),
            region: None,
            serial: None,
            version: None,
            category: None,
            roms: vec![DatRom {
                name: "Tetris (USA, Europe).nes".to_string(),
                size: 32768,
                crc: "aabbccdd".to_string(),
                sha1: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
                md5: None,
                serial: None,
            }],
        }],
    };

    let stats = import_dat(&conn, &dat, Platform::Nes, "no-intro", &SilentProgress).unwrap();
    assert_eq!(stats.works_created, 1);
    assert_eq!(stats.releases_created, 1);

    // Should be filed under "usa" (first region)
    let releases = releases_for_platform(&conn, "nes").unwrap();
    assert_eq!(releases[0].region, "usa");
}

#[test]
fn prototype_flag_sets_media_status() {
    let conn = setup_db();
    let dat = DatFile {
        name: "Test".to_string(),
        description: "Test".to_string(),
        version: "1".to_string(),
        games: vec![DatGame {
            name: "Unreleased Game (USA) (Proto)".to_string(),
            region: None,
            serial: None,
            version: None,
            category: None,
            roms: vec![DatRom {
                name: "Unreleased Game (USA) (Proto).nes".to_string(),
                size: 16384,
                crc: "11223344".to_string(),
                sha1: Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()),
                md5: None,
                serial: None,
            }],
        }],
    };

    import_dat(&conn, &dat, Platform::Nes, "no-intro", &SilentProgress).unwrap();
    let media = find_media_by_crc32(&conn, "11223344").unwrap();
    assert_eq!(media.len(), 1);
    assert_eq!(media[0].status, MediaStatus::Prototype);
}

#[test]
fn disc_number_extracted() {
    let conn = setup_db();
    let ps1 = CatalogPlatform {
        id: "ps1".to_string(),
        display_name: "Sony PlayStation".to_string(),
        short_name: "PS1".to_string(),
        manufacturer: "Sony".to_string(),
        generation: 5,
        media_type: MediaType::Disc,
        release_year: 1994,
        description: String::new(),
        core_platform: "Ps1".to_string(),
        regions: vec![],
        relationships: vec![],
    };
    upsert_platform(&conn, &ps1).unwrap();

    let dat = DatFile {
        name: "Test".to_string(),
        description: "Test".to_string(),
        version: "1".to_string(),
        games: vec![
            DatGame {
                name: "Final Fantasy VII (USA) (Disc 1)".to_string(),
                region: None,
                serial: None,
                version: None,
                category: None,
                roms: vec![DatRom {
                    name: "Final Fantasy VII (USA) (Disc 1).bin".to_string(),
                    size: 700_000_000,
                    crc: "aabb0001".to_string(),
                    sha1: Some("cccccccccccccccccccccccccccccccccccccccc".to_string()),
                    md5: None,
                    serial: Some("SCUS-94163".to_string()),
                }],
            },
            DatGame {
                name: "Final Fantasy VII (USA) (Disc 2)".to_string(),
                region: None,
                serial: None,
                version: None,
                category: None,
                roms: vec![DatRom {
                    name: "Final Fantasy VII (USA) (Disc 2).bin".to_string(),
                    size: 700_000_000,
                    crc: "aabb0002".to_string(),
                    sha1: Some("dddddddddddddddddddddddddddddddddddddddd".to_string()),
                    md5: None,
                    serial: Some("SCUS-94164".to_string()),
                }],
            },
        ],
    };

    import_dat(&conn, &dat, Platform::Ps1, "redump", &SilentProgress).unwrap();

    let disc1 = find_media_by_crc32(&conn, "aabb0001").unwrap();
    let disc2 = find_media_by_crc32(&conn, "aabb0002").unwrap();
    assert_eq!(disc1[0].disc_number, 1);
    assert_eq!(disc2[0].disc_number, 2);
    assert_eq!(disc1[0].media_serial, "SCUS-94163");
    assert_eq!(disc2[0].media_serial, "SCUS-94164");

    // Both discs should share the same release
    assert_eq!(disc1[0].release_id, disc2[0].release_id);
}

#[test]
fn saturn_masterings_share_an_edition_but_named_editions_do_not() {
    let conn = setup_db();
    upsert_platform(
        &conn,
        &CatalogPlatform {
            id: "saturn".to_owned(),
            display_name: "Sega Saturn".to_owned(),
            short_name: "Saturn".to_owned(),
            manufacturer: "Sega".to_owned(),
            generation: 5,
            media_type: MediaType::Disc,
            release_year: 1994,
            description: String::new(),
            core_platform: "Saturn".to_owned(),
            regions: vec![],
            relationships: vec![],
        },
    )
    .unwrap();
    let game = |name: &str, crc: &str, sha1: &str| DatGame {
        name: name.to_owned(),
        region: None,
        serial: None,
        version: None,
        category: Some("Games".to_owned()),
        roms: vec![DatRom {
            name: format!("{name}.bin"),
            size: 700_000_000,
            crc: crc.to_owned(),
            sha1: Some(sha1.to_owned()),
            md5: None,
            serial: None,
        }],
    };
    let dat = DatFile {
        name: "Sega - Saturn".to_owned(),
        description: "Sega - Saturn".to_owned(),
        version: "1".to_owned(),
        games: vec![
            game(
                "Sakura Taisen (Japan) (Disc 1) (Sakura) (7M)",
                "10000001",
                "1000000000000000000000000000000000000001",
            ),
            game(
                "Sakura Taisen (Japan) (Disc 2) (Sumire) (8M)",
                "10000002",
                "1000000000000000000000000000000000000002",
            ),
            game(
                "Sakura Taisen (Japan) (Disc 1) (Satakore) (9M)",
                "10000003",
                "1000000000000000000000000000000000000003",
            ),
            game(
                "Sakura Taisen (Japan) (Disc 2) (Satakore) (10M)",
                "10000004",
                "1000000000000000000000000000000000000004",
            ),
        ],
    };

    import_dat(&conn, &dat, Platform::Saturn, "redump", &SilentProgress).unwrap();

    let releases: Vec<(String, String, i64)> = conn
        .prepare(
            "SELECT r.variant,group_concat(m.disc_designator),count(*)
             FROM releases r JOIN media m ON m.release_id=r.id
             WHERE r.platform_id='saturn'
             GROUP BY r.id ORDER BY r.variant",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(releases.len(), 2, "{releases:?}");
    assert_eq!(releases[0], (String::new(), "1,2".to_owned(), 2));
    assert_eq!(releases[1], ("Satakore".to_owned(), "1,2".to_owned(), 2));

    // A catalog upgraded from the legacy integer-only column can otherwise
    // look unchanged and return before restoring the exact DAT designator.
    conn.execute(
        "UPDATE media SET disc_designator='' WHERE crc32='10000001'",
        [],
    )
    .unwrap();
    let stats = import_dat(&conn, &dat, Platform::Saturn, "redump", &SilentProgress).unwrap();
    assert_eq!(stats.media_updated, 1);
    let repaired: String = conn
        .query_row(
            "SELECT disc_designator FROM media WHERE crc32='10000001'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(repaired, "1");
}

#[test]
fn disc_role_inference_is_platform_generic_and_edition_scoped() {
    let conn = setup_db();
    upsert_platform(
        &conn,
        &CatalogPlatform {
            id: "ps1".to_owned(),
            display_name: "Sony PlayStation".to_owned(),
            short_name: "PS1".to_owned(),
            manufacturer: "Sony".to_owned(),
            generation: 5,
            media_type: MediaType::Disc,
            release_year: 1994,
            description: String::new(),
            core_platform: "Ps1".to_owned(),
            regions: vec![],
            relationships: vec![],
        },
    )
    .unwrap();
    let game = |name: &str, crc: &str, sha1: &str| DatGame {
        name: name.to_owned(),
        region: None,
        serial: None,
        version: None,
        category: Some("Games".to_owned()),
        roms: vec![DatRom {
            name: format!("{name}.bin"),
            size: 600_000_000,
            crc: crc.to_owned(),
            sha1: Some(sha1.to_owned()),
            md5: None,
            serial: None,
        }],
    };
    let dat = DatFile {
        name: "Sony - PlayStation".to_owned(),
        description: "Sony - PlayStation".to_owned(),
        version: "1".to_owned(),
        games: vec![
            game(
                "Scenario Game (Japan) (Disc 1) (Leon)",
                "20000001",
                "2000000000000000000000000000000000000001",
            ),
            game(
                "Scenario Game (Japan) (Disc 2) (Claire)",
                "20000002",
                "2000000000000000000000000000000000000002",
            ),
            game(
                "Scenario Game (USA) (Disc 1) (Leon)",
                "20000003",
                "2000000000000000000000000000000000000003",
            ),
            game(
                "Scenario Game (USA) (Disc 2) (Leon)",
                "20000004",
                "2000000000000000000000000000000000000004",
            ),
        ],
    };

    import_dat(&conn, &dat, Platform::Ps1, "REDUMP", &SilentProgress).unwrap();

    let releases: Vec<(String, String, i64)> = conn
        .prepare(
            "SELECT r.region,r.variant,count(*)
             FROM releases r JOIN media m ON m.release_id=r.id
             WHERE r.platform_id='ps1'
             GROUP BY r.id ORDER BY r.region",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        releases,
        vec![
            ("japan".to_owned(), String::new(), 2),
            ("usa".to_owned(), "Leon".to_owned(), 2),
        ]
    );
}

#[test]
fn ambiguous_or_incomplete_disc_roles_do_not_merge_releases() {
    let conn = setup_db();
    upsert_platform(
        &conn,
        &CatalogPlatform {
            id: "ps1".to_owned(),
            display_name: "Sony PlayStation".to_owned(),
            short_name: "PS1".to_owned(),
            manufacturer: "Sony".to_owned(),
            generation: 5,
            media_type: MediaType::Disc,
            release_year: 1994,
            description: String::new(),
            core_platform: "Ps1".to_owned(),
            regions: vec![],
            relationships: vec![],
        },
    )
    .unwrap();
    let game = |name: &str, suffix: u8| DatGame {
        name: name.to_owned(),
        region: None,
        serial: None,
        version: None,
        category: Some("Games".to_owned()),
        roms: vec![DatRom {
            name: format!("{name}.bin"),
            size: 600_000_000,
            crc: format!("3000000{suffix}"),
            sha1: Some(format!("300000000000000000000000000000000000000{suffix}")),
            md5: None,
            serial: None,
        }],
    };
    let dat = DatFile {
        name: "Sony - PlayStation".to_owned(),
        description: "Sony - PlayStation".to_owned(),
        version: "1".to_owned(),
        games: vec![
            game("Ambiguous Game (Japan) (Disc 1) (Limited) (Leon)", 1),
            game("Ambiguous Game (Japan) (Disc 2) (Claire)", 2),
            game("Incomplete Game (Japan) (Disc 1) (Install)", 3),
            game("Incomplete Game (Japan) (Disc 3) (Play)", 4),
        ],
    };

    import_dat(&conn, &dat, Platform::Ps1, "redump", &SilentProgress).unwrap();

    for title in ["Ambiguous Game", "Incomplete Game"] {
        let releases: i64 = conn
            .query_row(
                "SELECT count(DISTINCT r.id)
                 FROM releases r JOIN media m ON m.release_id=r.id
                 WHERE r.platform_id='ps1' AND r.title=?1",
                [title],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(releases, 2, "{title} was merged despite ambiguous evidence");
    }
}

/// A corrected DAT name must keep the entry it already had, not grow a twin.
///
/// The title is part of the work, release and media ids, so a renamed game
/// used to mint a whole new triple beside the old one with identical hashes —
/// 871 of them appeared the day an XML-entity bug was fixed and the names
/// changed from `1 &amp; 2` to `1 & 2`. Content-based re-binding then found
/// two candidates for one disc and refused to identify it at all, which is how
/// the duplicate was noticed: a release simply went unidentified.
#[test]
fn a_renamed_game_keeps_its_entry_instead_of_gaining_a_twin() {
    let conn = setup_db();
    let rom = DatRom {
        name: "Tom & Jerry (USA).nes".to_string(),
        size: 40976,
        crc: "d445f698".to_string(),
        sha1: Some("ea343f4e445a9050d4b4fbac2c77d0693b1d0922".to_string()),
        md5: None,
        serial: None,
    };
    let dat_with = |title: &str, rom_name: &str| DatFile {
        name: "Nintendo - Nintendo Entertainment System".to_string(),
        description: String::new(),
        version: "1".to_string(),
        games: vec![DatGame {
            name: title.to_string(),
            region: None,
            serial: None,
            version: None,
            category: None,
            roms: vec![DatRom {
                name: rom_name.to_string(),
                ..rom.clone()
            }],
        }],
    };

    // The name as the entity bug left it.
    import_dat(
        &conn,
        &dat_with("Tom &amp; Jerry (USA)", "Tom &amp; Jerry (USA).nes"),
        Platform::Nes,
        "no-intro",
        &SilentProgress,
    )
    .unwrap();
    let first: String = conn
        .query_row("SELECT id FROM media", [], |row| row.get(0))
        .unwrap();

    // The same game, re-imported once the parser stopped carrying the entity.
    import_dat(
        &conn,
        &dat_with("Tom & Jerry (USA)", "Tom & Jerry (USA).nes"),
        Platform::Nes,
        "no-intro",
        &SilentProgress,
    )
    .unwrap();

    let media_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM media", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        media_rows, 1,
        "the corrected name created a second catalog entry for the same bytes"
    );
    let (id, dat_name): (String, String) = conn
        .query_row("SELECT id, dat_name FROM media", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!(
        id, first,
        "the row's id changed, orphaning anything bound to it"
    );
    assert_eq!(
        dat_name, "Tom & Jerry (USA)",
        "the row kept the stale name instead of taking the correction"
    );
}

/// The rename must be a *relabel* all the way up: the work and release that
/// held the medium keep their ids and take the new wording, or every archive
/// manifest and library binding pointing at them is orphaned in the same way
/// the media row used to be.
#[test]
fn a_renamed_game_relabels_its_work_and_release_rather_than_replacing_them() {
    let conn = setup_db();
    let dat_with = |title: &str| DatFile {
        name: "Nintendo - Nintendo Entertainment System".to_string(),
        description: String::new(),
        version: "1".to_string(),
        games: vec![DatGame {
            name: title.to_string(),
            region: None,
            serial: None,
            version: None,
            category: None,
            roms: vec![DatRom {
                name: format!("{title}.nes"),
                size: 40976,
                crc: "d445f698".to_string(),
                sha1: Some("ea343f4e445a9050d4b4fbac2c77d0693b1d0922".to_string()),
                md5: None,
                serial: None,
            }],
        }],
    };

    import_dat(
        &conn,
        &dat_with("Tom &amp; Jerry (USA)"),
        Platform::Nes,
        "no-intro",
        &SilentProgress,
    )
    .unwrap();
    let before: (String, String) = conn
        .query_row("SELECT r.id,r.work_id FROM releases r", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();

    import_dat(
        &conn,
        &dat_with("Tom & Jerry (USA)"),
        Platform::Nes,
        "no-intro",
        &SilentProgress,
    )
    .unwrap();

    let works: i64 = conn
        .query_row("SELECT COUNT(*) FROM works", [], |row| row.get(0))
        .unwrap();
    let releases: i64 = conn
        .query_row("SELECT COUNT(*) FROM releases", [], |row| row.get(0))
        .unwrap();
    assert_eq!(works, 1, "the corrected name minted a second work");
    assert_eq!(releases, 1, "the corrected name minted a second release");

    let after: (String, String) = conn
        .query_row("SELECT r.id,r.work_id FROM releases r", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!(
        after, before,
        "the ids moved when only the label should have"
    );

    let (work_name, release_title): (String, String) = conn
        .query_row(
            "SELECT w.canonical_name,r.title FROM works w JOIN releases r ON r.work_id=w.id",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(work_name, "Tom & Jerry");
    assert_eq!(release_title, "Tom & Jerry");
}

/// Titles with no ASCII in them at all used to slug to the empty string, so
/// every such game on a platform collided into one work id. This was live:
/// `パロディウスだ!` and `がんばれゴエモン` were one row.
#[test]
fn two_titles_with_no_ascii_are_two_works() {
    let conn = setup_db();
    let dat = DatFile {
        name: "Nintendo - Nintendo Entertainment System".to_string(),
        description: String::new(),
        version: "1".to_string(),
        games: vec![
            DatGame {
                name: "パロディウスだ! (Japan)".to_string(),
                region: None,
                serial: None,
                version: None,
                category: None,
                roms: vec![DatRom {
                    name: "パロディウスだ! (Japan).nes".to_string(),
                    size: 262_144,
                    crc: "11111111".to_string(),
                    sha1: Some("1111111111111111111111111111111111111111".to_string()),
                    md5: None,
                    serial: None,
                }],
            },
            DatGame {
                name: "がんばれゴエモン (Japan)".to_string(),
                region: None,
                serial: None,
                version: None,
                category: None,
                roms: vec![DatRom {
                    name: "がんばれゴエモン (Japan).nes".to_string(),
                    size: 393_216,
                    crc: "22222222".to_string(),
                    sha1: Some("2222222222222222222222222222222222222222".to_string()),
                    md5: None,
                    serial: None,
                }],
            },
        ],
    };

    let stats = import_dat(&conn, &dat, Platform::Nes, "no-intro", &SilentProgress).unwrap();
    assert_eq!(stats.media_created, 2);
    assert_eq!(
        stats.works_created, 2,
        "two Japanese titles became one work"
    );

    let works: i64 = conn
        .query_row("SELECT COUNT(*) FROM works", [], |row| row.get(0))
        .unwrap();
    assert_eq!(works, 2);
    let media: i64 = conn
        .query_row("SELECT COUNT(*) FROM media", [], |row| row.get(0))
        .unwrap();
    assert_eq!(media, 2);
}

/// Two entries whose bytes are identical are one medium, whatever they are
/// called. The second overwrites the first rather than sitting beside it,
/// because they share a primary key.
#[test]
fn identical_bytes_under_two_names_are_one_medium() {
    let conn = setup_db();
    let entry = |title: &str| DatGame {
        name: title.to_string(),
        region: None,
        serial: None,
        version: None,
        category: None,
        roms: vec![DatRom {
            name: format!("{title}.nes"),
            size: 40976,
            crc: "d445f698".to_string(),
            sha1: Some("ea343f4e445a9050d4b4fbac2c77d0693b1d0922".to_string()),
            md5: None,
            serial: None,
        }],
    };
    let dat = DatFile {
        name: "Nintendo - Nintendo Entertainment System".to_string(),
        description: String::new(),
        version: "1".to_string(),
        games: vec![entry("Game (USA)"), entry("Game, The (USA)")],
    };

    import_dat(&conn, &dat, Platform::Nes, "no-intro", &SilentProgress).unwrap();
    let media: i64 = conn
        .query_row("SELECT COUNT(*) FROM media", [], |row| row.get(0))
        .unwrap();
    assert_eq!(media, 1);
}

/// A DAT entry with no SHA-1, or one describing an empty file, cannot be told
/// apart from any other such entry. Importing it would hand out an id meaning
/// "some empty thing", so it is left out and counted where someone can see it.
#[test]
fn entries_that_nothing_can_identify_are_skipped_and_counted() {
    let conn = setup_db();
    let dat = DatFile {
        name: "Nintendo - Nintendo Entertainment System".to_string(),
        description: String::new(),
        version: "1".to_string(),
        games: vec![
            DatGame {
                name: "No Digest (USA)".to_string(),
                region: None,
                serial: None,
                version: None,
                category: None,
                roms: vec![DatRom {
                    name: "No Digest (USA).nes".to_string(),
                    size: 40976,
                    crc: "d445f698".to_string(),
                    sha1: None,
                    md5: None,
                    serial: None,
                }],
            },
            DatGame {
                name: "Empty File (USA)".to_string(),
                region: None,
                serial: None,
                version: None,
                category: None,
                roms: vec![DatRom {
                    name: "Empty File (USA).nes".to_string(),
                    size: 0,
                    crc: "00000000".to_string(),
                    sha1: Some("da39a3ee5e6b4b0d3255bfef95601890afd80709".to_string()),
                    md5: None,
                    serial: None,
                }],
            },
        ],
    };

    let stats = import_dat(&conn, &dat, Platform::Nes, "no-intro", &SilentProgress).unwrap();
    assert_eq!(stats.skipped_unidentifiable, 2);
    assert_eq!(stats.media_created, 0);
    let media: i64 = conn
        .query_row("SELECT COUNT(*) FROM media", [], |row| row.get(0))
        .unwrap();
    assert_eq!(media, 0);
}
