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
        generation: Some(3),
        media_type: MediaType::Cartridge,
        release_year: Some(1985),
        description: None,
        core_platform: Some("Nes".to_string()),
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
                roms: vec![DatRom {
                    name: "Legend of Zelda, The (USA).nes".to_string(),
                    size: 131088,
                    crc: "a12d74c1".to_string(),
                    sha1: Some("7fcbc2007a277e05f97054153cc850eb47589bcd".to_string()),
                    md5: None,
                    serial: None,
                }],
            },
            DatGame {
                name: "The Legend of Zelda (USA) (Rev A)".to_string(),
                region: None,
                roms: vec![DatRom {
                    name: "Legend of Zelda, The (USA) (Rev A).nes".to_string(),
                    size: 131088,
                    crc: "cebd2a31".to_string(),
                    sha1: Some("4addc7c8bc3ab5ba5421c4f1f6e5bba4fbafc4de".to_string()),
                    md5: None,
                    serial: None,
                }],
            },
            DatGame {
                name: "Bad Game (USA) [b]".to_string(),
                region: None,
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

    let stats = import_dat(&conn, &dat, Platform::Nes, "no-intro", None).unwrap();

    // 3 unique works (SMB, Zelda, Bad Game skipped)
    assert_eq!(stats.works_created, 2);
    assert_eq!(stats.media_created, 3); // SMB + Zelda + Zelda Rev A
    assert_eq!(stats.skipped_bad, 1);
}

#[test]
fn import_creates_correct_releases() {
    let conn = setup_db();
    let dat = sample_dat();
    import_dat(&conn, &dat, Platform::Nes, "no-intro", None).unwrap();

    let releases = releases_for_platform(&conn, "nes").unwrap();
    assert_eq!(releases.len(), 2); // SMB + Zelda (Rev A shares Zelda's release)

    let titles: Vec<&str> = releases.iter().map(|r| r.title.as_str()).collect();
    assert!(titles.contains(&"Super Mario Bros."));
    assert!(titles.contains(&"The Legend of Zelda"));
}

#[test]
fn import_media_has_correct_hashes() {
    let conn = setup_db();
    let dat = sample_dat();
    import_dat(&conn, &dat, Platform::Nes, "no-intro", None).unwrap();

    let media = find_media_by_crc32(&conn, "d445f698").unwrap();
    assert_eq!(media.len(), 1);
    assert_eq!(
        media[0].sha1.as_deref(),
        Some("ea343f4e445a9050d4b4fbac2c77d0693b1d0922")
    );
    assert_eq!(media[0].file_size, Some(40976));
    assert_eq!(media[0].dat_source.as_deref(), Some("no-intro"));
}

#[test]
fn import_revision_creates_separate_media() {
    let conn = setup_db();
    let dat = sample_dat();
    import_dat(&conn, &dat, Platform::Nes, "no-intro", None).unwrap();

    // Both Zelda entries should exist as media
    let zelda_orig = find_media_by_crc32(&conn, "a12d74c1").unwrap();
    let zelda_reva = find_media_by_crc32(&conn, "cebd2a31").unwrap();
    assert_eq!(zelda_orig.len(), 1);
    assert_eq!(zelda_reva.len(), 1);

    // Rev A should have revision set
    assert_eq!(zelda_reva[0].revision.as_deref(), Some("Rev A"));
    assert!(zelda_orig[0].revision.is_none());

    // Both should share the same release
    assert_eq!(zelda_orig[0].release_id, zelda_reva[0].release_id);
}

#[test]
fn reimport_is_idempotent() {
    let conn = setup_db();
    let dat = sample_dat();

    let stats1 = import_dat(&conn, &dat, Platform::Nes, "no-intro", None).unwrap();
    assert_eq!(stats1.media_created, 3);

    let stats2 = import_dat(&conn, &dat, Platform::Nes, "no-intro", None).unwrap();
    assert_eq!(stats2.media_created, 0);
    assert_eq!(stats2.media_unchanged, 3);
    // 3 games processed (bad dump skipped), each finds existing work
    // (SMB=1, Zelda=1, Zelda Rev A=1 → 3 existing-work hits)
    assert_eq!(stats2.works_existing, 3);
}

#[test]
fn bad_dumps_skipped() {
    let conn = setup_db();
    let dat = sample_dat();
    let stats = import_dat(&conn, &dat, Platform::Nes, "no-intro", None).unwrap();

    assert_eq!(stats.skipped_bad, 1);
    assert_eq!(stats.total_games, 4);
}

#[test]
fn log_import_records_stats() {
    let conn = setup_db();
    let dat = sample_dat();
    let stats = import_dat(&conn, &dat, Platform::Nes, "no-intro", None).unwrap();

    let log_id = log_import(
        &conn,
        "no-intro",
        "Nintendo - NES",
        Some("2024-01-15"),
        &stats,
    )
    .unwrap();
    assert!(log_id > 0);

    let logs = list_import_logs(&conn, None).unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].source_name, "Nintendo - NES");
    assert_eq!(logs[0].records_created, 3);
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
            roms: vec![DatRom {
                name: "Tetris (USA, Europe).nes".to_string(),
                size: 32768,
                crc: "aabbccdd".to_string(),
                sha1: None,
                md5: None,
                serial: None,
            }],
        }],
    };

    let stats = import_dat(&conn, &dat, Platform::Nes, "no-intro", None).unwrap();
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
            roms: vec![DatRom {
                name: "Unreleased Game (USA) (Proto).nes".to_string(),
                size: 16384,
                crc: "11223344".to_string(),
                sha1: None,
                md5: None,
                serial: None,
            }],
        }],
    };

    import_dat(&conn, &dat, Platform::Nes, "no-intro", None).unwrap();
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
        generation: Some(5),
        media_type: MediaType::Disc,
        release_year: Some(1994),
        description: None,
        core_platform: Some("Ps1".to_string()),
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
                roms: vec![DatRom {
                    name: "Final Fantasy VII (USA) (Disc 1).bin".to_string(),
                    size: 700000000,
                    crc: "aabb0001".to_string(),
                    sha1: None,
                    md5: None,
                    serial: Some("SCUS-94163".to_string()),
                }],
            },
            DatGame {
                name: "Final Fantasy VII (USA) (Disc 2)".to_string(),
                region: None,
                roms: vec![DatRom {
                    name: "Final Fantasy VII (USA) (Disc 2).bin".to_string(),
                    size: 700000000,
                    crc: "aabb0002".to_string(),
                    sha1: None,
                    md5: None,
                    serial: Some("SCUS-94164".to_string()),
                }],
            },
        ],
    };

    import_dat(&conn, &dat, Platform::Ps1, "redump", None).unwrap();

    let disc1 = find_media_by_crc32(&conn, "aabb0001").unwrap();
    let disc2 = find_media_by_crc32(&conn, "aabb0002").unwrap();
    assert_eq!(disc1[0].disc_number, Some(1));
    assert_eq!(disc2[0].disc_number, Some(2));
    assert_eq!(disc1[0].media_serial.as_deref(), Some("SCUS-94163"));
    assert_eq!(disc2[0].media_serial.as_deref(), Some("SCUS-94164"));

    // Both discs should share the same release
    assert_eq!(disc1[0].release_id, disc2[0].release_id);
}
