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
                sha1: None,
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
                sha1: None,
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
                    sha1: None,
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
                    sha1: None,
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
