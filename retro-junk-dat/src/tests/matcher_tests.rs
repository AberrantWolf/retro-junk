use super::*;
use crate::dat::{DatFile, DatGame, DatRom};

fn make_test_dat() -> DatFile {
    DatFile {
        name: "Test".into(),
        description: "Test".into(),
        version: "1".into(),
        games: vec![
            DatGame {
                name: "Super Mario World (USA)".into(),
                region: None,
                roms: vec![DatRom {
                    name: "Super Mario World (USA).sfc".into(),
                    size: 524288,
                    crc: "b19ed489".into(),
                    sha1: Some("6b47bb75d16514b6a476aa0c73a683a2a4c18765".into()),
                    md5: None,
                    serial: None,
                }],
            },
            DatGame {
                name: "Super Mario 64 (USA)".into(),
                region: None,
                roms: vec![DatRom {
                    name: "Super Mario 64 (USA).z64".into(),
                    size: 8388608,
                    crc: "635a2bff".into(),
                    sha1: None,
                    md5: None,
                    // LibRetro DATs use short 4-char game codes
                    serial: Some("NSME".into()),
                }],
            },
            DatGame {
                name: "Super Mario 64 (Japan)".into(),
                region: None,
                roms: vec![DatRom {
                    name: "Super Mario 64 (Japan).z64".into(),
                    size: 8388608,
                    crc: "4eab3152".into(),
                    sha1: None,
                    md5: None,
                    serial: Some("NSMJ".into()),
                }],
            },
            DatGame {
                name: "The Legend of Zelda - A Link to the Past (USA)".into(),
                region: None,
                roms: vec![DatRom {
                    name: "The Legend of Zelda - A Link to the Past (USA).sfc".into(),
                    size: 1048576,
                    crc: "777aac2f".into(),
                    sha1: None,
                    md5: None,
                    serial: Some("SNS-ZL-USA".into()),
                }],
            },
        ],
    }
}

#[test]
fn test_match_by_crc32() {
    let index = DatIndex::from_dat(make_test_dat());
    let hashes = FileHashes {
        crc32: "b19ed489".into(),
        sha1: None,
        md5: None,
        data_size: 524288,
    };
    let result = index.match_by_hash(524288, &hashes).unwrap();
    assert_eq!(result.game_index, 0);
    assert_eq!(result.method, MatchMethod::Crc32);
}

#[test]
fn test_match_by_serial_exact() {
    let index = DatIndex::from_dat(make_test_dat());
    // Exact match: DAT has "SNS-ZL-USA", query "SNS-ZL-USA"
    let result = index.match_by_serial("SNS-ZL-USA", None).unwrap();
    assert_eq!(result.game_index, 3);
    assert_eq!(result.method, MatchMethod::Serial);
}

#[test]
fn test_match_by_serial_short_code() {
    let index = DatIndex::from_dat(make_test_dat());
    // DAT has short code "NSME", query with short code "NSME"
    let result = index.match_by_serial("NSME", None).unwrap();
    assert_eq!(result.game_index, 1);
    assert_eq!(result.method, MatchMethod::Serial);
}

#[test]
fn test_match_by_serial_long_to_short() {
    // Analyzer produces NUS-NSME-USA, DAT has NSME — should still match
    // via pre-extracted game code
    let index = DatIndex::from_dat(make_test_dat());
    let result = index.match_by_serial("NUS-NSME-USA", Some("NSME")).unwrap();
    assert_eq!(result.game_index, 1);
    assert_eq!(index.games[result.game_index].name, "Super Mario 64 (USA)");
}

#[test]
fn test_serial_distinguishes_regions() {
    let index = DatIndex::from_dat(make_test_dat());

    // Analyzer produces NUS-NSME-USA, extracts NSME → matches DAT's NSME
    let usa = index.match_by_serial("NUS-NSME-USA", Some("NSME")).unwrap();
    assert_eq!(usa.game_index, 1);
    assert_eq!(index.games[usa.game_index].name, "Super Mario 64 (USA)");

    // Analyzer produces NUS-NSMJ-JPN, extracts NSMJ → matches DAT's NSMJ
    let jpn = index.match_by_serial("NUS-NSMJ-JPN", Some("NSMJ")).unwrap();
    assert_eq!(jpn.game_index, 2);
    assert_eq!(index.games[jpn.game_index].name, "Super Mario 64 (Japan)");
}

#[test]
fn test_hash_distinguishes_regions() {
    let index = DatIndex::from_dat(make_test_dat());

    let usa_hashes = FileHashes {
        crc32: "635a2bff".into(),
        sha1: None,
        md5: None,
        data_size: 8388608,
    };
    let usa = index.match_by_hash(8388608, &usa_hashes).unwrap();
    assert_eq!(index.games[usa.game_index].name, "Super Mario 64 (USA)");

    let jpn_hashes = FileHashes {
        crc32: "4eab3152".into(),
        sha1: None,
        md5: None,
        data_size: 8388608,
    };
    let jpn = index.match_by_hash(8388608, &jpn_hashes).unwrap();
    assert_eq!(index.games[jpn.game_index].name, "Super Mario 64 (Japan)");
}

#[test]
fn test_no_match() {
    let index = DatIndex::from_dat(make_test_dat());
    let hashes = FileHashes {
        crc32: "00000000".into(),
        sha1: None,
        md5: None,
        data_size: 999,
    };
    assert!(index.match_by_hash(999, &hashes).is_none());
    assert!(index.match_by_serial("UNKNOWN", None).is_none());
}

#[test]
fn test_from_dats_merge() {
    let dat1 = DatFile {
        name: "DAT A".into(),
        description: "".into(),
        version: "1".into(),
        games: vec![DatGame {
            name: "Game A (USA)".into(),
            region: None,
            roms: vec![DatRom {
                name: "Game A (USA).bin".into(),
                size: 1024,
                crc: "aaaa0001".into(),
                sha1: None,
                md5: None,
                serial: Some("SLUS-99999".into()),
            }],
        }],
    };
    let dat2 = DatFile {
        name: "DAT B".into(),
        description: "".into(),
        version: "2".into(),
        games: vec![DatGame {
            name: "Game B (USA)".into(),
            region: None,
            roms: vec![DatRom {
                name: "Game B (USA).bin".into(),
                size: 2048,
                crc: "bbbb0002".into(),
                sha1: None,
                md5: None,
                serial: Some("SLUS-88888".into()),
            }],
        }],
    };

    let index = DatIndex::from_dats(vec![dat1, dat2]);
    assert_eq!(index.game_count(), 2);

    // Can find game from first DAT
    let result_a = index.match_by_serial("SLUS-99999", None).unwrap();
    assert_eq!(index.games[result_a.game_index].name, "Game A (USA)");

    // Can find game from second DAT
    let result_b = index.match_by_serial("SLUS-88888", None).unwrap();
    assert_eq!(index.games[result_b.game_index].name, "Game B (USA)");

    // Hash lookup works across merged DATs
    let hashes = FileHashes {
        crc32: "bbbb0002".into(),
        sha1: None,
        md5: None,
        data_size: 2048,
    };
    let hash_result = index.match_by_hash(2048, &hashes).unwrap();
    assert_eq!(index.games[hash_result.game_index].name, "Game B (USA)");
}

#[test]
fn test_crc32_requires_matching_size() {
    let index = DatIndex::from_dat(make_test_dat());
    // Right CRC but wrong size — should not match
    let hashes = FileHashes {
        crc32: "b19ed489".into(),
        sha1: None,
        md5: None,
        data_size: 524288,
    };
    assert!(index.match_by_hash(999, &hashes).is_none());
}

#[test]
fn test_comma_separated_serials() {
    // Redump DATs have comma-separated serials like "SLUS-01041, SLUS-01041GH"
    let dat = DatFile {
        name: "Test".into(),
        description: "".into(),
        version: "1".into(),
        games: vec![DatGame {
            name: "Chrono Cross (USA) (Disc 1)".into(),
            region: None,
            roms: vec![DatRom {
                name: "Chrono Cross (USA) (Disc 1).bin".into(),
                size: 736651104,
                crc: "a07898cc".into(),
                sha1: None,
                md5: None,
                serial: Some("SLUS-01041, SLUS-01041GH, SLUS-01041GH-F".into()),
            }],
        }],
    };
    let index = DatIndex::from_dat(dat);

    // Each individual serial should be findable
    assert!(index.match_by_serial("SLUS-01041", None).is_some());
    assert!(index.match_by_serial("SLUS-01041GH", None).is_some());
    assert!(index.match_by_serial("SLUS-01041GH-F", None).is_some());
}

#[test]
fn test_serial_space_dash_normalization() {
    // Redump DATs sometimes use spaces instead of dashes: "SLPS 00700"
    // ROM analysis produces dashes: "SLPS-00700"
    let dat = DatFile {
        name: "Test".into(),
        description: "".into(),
        version: "1".into(),
        games: vec![DatGame {
            name: "Some Game (Japan)".into(),
            region: None,
            roms: vec![DatRom {
                name: "Some Game (Japan).bin".into(),
                size: 1024,
                crc: "deadbeef".into(),
                sha1: None,
                md5: None,
                serial: Some("SLPS 00700".into()),
            }],
        }],
    };
    let index = DatIndex::from_dat(dat);

    // Query with dash should match DAT with space
    let result = index.match_by_serial("SLPS-00700", None);
    assert!(result.is_some());
    assert_eq!(
        index.games[result.unwrap().game_index].name,
        "Some Game (Japan)"
    );
}

#[test]
fn test_multi_disc_suffix_prefers_suffixed_over_bare() {
    // LibRetro Redump DATs have both bare and suffixed entries for multi-disc
    // games. When a disc's boot serial matches the bare entry, the "-0"
    // suffixed entry should be preferred since the bare serial is ambiguous.
    let dat = DatFile {
        name: "Test".into(),
        description: "".into(),
        version: "1".into(),
        games: vec![
            // Bare entries (shared serial — last one overwrites in the HashMap)
            DatGame {
                name: "FF7 (USA) (Disc 1)".into(),
                region: None,
                roms: vec![DatRom {
                    name: "FF7 (USA) (Disc 1).bin".into(),
                    size: 747435024,
                    crc: "1459cbef".into(),
                    sha1: None,
                    md5: None,
                    serial: Some("SCUS-94163".into()),
                }],
            },
            DatGame {
                name: "FF7 (USA) (Disc 1) [suffixed]".into(),
                region: None,
                roms: vec![DatRom {
                    name: "FF7 (USA) (Disc 1).bin".into(),
                    size: 747435024,
                    crc: "1459cbef".into(),
                    sha1: None,
                    md5: None,
                    serial: Some("SCUS-94163-0".into()),
                }],
            },
            DatGame {
                name: "FF7 (USA) (Disc 2)".into(),
                region: None,
                roms: vec![DatRom {
                    name: "FF7 (USA) (Disc 2).bin".into(),
                    size: 732657408,
                    crc: "a997a8cc".into(),
                    sha1: None,
                    md5: None,
                    serial: Some("SCUS-94163".into()),
                }],
            },
            DatGame {
                name: "FF7 (USA) (Disc 2) [suffixed]".into(),
                region: None,
                roms: vec![DatRom {
                    name: "FF7 (USA) (Disc 2).bin".into(),
                    size: 732657408,
                    crc: "a997a8cc".into(),
                    sha1: None,
                    md5: None,
                    serial: Some("SCUS-94163-1".into()),
                }],
            },
            DatGame {
                name: "FF7 (USA) (Disc 3)".into(),
                region: None,
                roms: vec![DatRom {
                    name: "FF7 (USA) (Disc 3).bin".into(),
                    size: 659561952,
                    crc: "1c27b277".into(),
                    sha1: None,
                    md5: None,
                    serial: Some("SCUS-94163".into()),
                }],
            },
            DatGame {
                name: "FF7 (USA) (Disc 3) [suffixed]".into(),
                region: None,
                roms: vec![DatRom {
                    name: "FF7 (USA) (Disc 3).bin".into(),
                    size: 659561952,
                    crc: "1c27b277".into(),
                    sha1: None,
                    md5: None,
                    serial: Some("SCUS-94163-2".into()),
                }],
            },
        ],
    };
    let index = DatIndex::from_dat(dat);

    // Disc 1's boot serial "SCUS-94163" should prefer the "-0" suffixed entry
    let disc1 = index.match_by_serial("SCUS-94163", None).unwrap();
    assert!(
        index.games[disc1.game_index].name.contains("Disc 1"),
        "Expected Disc 1 match, got: {}",
        index.games[disc1.game_index].name
    );

    // A serial that doesn't exist bare but does with suffix should still match
    // (suffix fallback when no exact match)
    // Note: SCUS-94164 (disc 2's actual boot serial) won't match anything here
    // because the DAT uses SCUS-94163-1, not SCUS-94164-anything. Hash fallback
    // handles that case.
}

#[test]
fn test_suffix_fallback_when_no_exact_match() {
    // When exact serial doesn't match, try with disc suffixes
    let dat = DatFile {
        name: "Test".into(),
        description: "".into(),
        version: "1".into(),
        games: vec![DatGame {
            name: "Some Game (USA) (Disc 1)".into(),
            region: None,
            roms: vec![DatRom {
                name: "Some Game (USA) (Disc 1).bin".into(),
                size: 700000000,
                crc: "deadbeef".into(),
                sha1: None,
                md5: None,
                // Only suffixed entry, no bare serial
                serial: Some("SLUS-99999-0".into()),
            }],
        }],
    };
    let index = DatIndex::from_dat(dat);

    // "SLUS-99999" doesn't exist bare, but "SLUS-99999-0" does
    let result = index.match_by_serial("SLUS-99999", None).unwrap();
    assert_eq!(
        index.games[result.game_index].name,
        "Some Game (USA) (Disc 1)"
    );
}

#[test]
fn test_normal_game_unaffected_by_suffix_logic() {
    // Single-disc games with no suffixed variants should still match normally
    let dat = DatFile {
        name: "Test".into(),
        description: "".into(),
        version: "1".into(),
        games: vec![DatGame {
            name: "Crash Bandicoot (USA)".into(),
            region: None,
            roms: vec![DatRom {
                name: "Crash Bandicoot (USA).bin".into(),
                size: 500000000,
                crc: "aabbccdd".into(),
                sha1: None,
                md5: None,
                serial: Some("SCUS-94900".into()),
            }],
        }],
    };
    let index = DatIndex::from_dat(dat);

    let result = index.match_by_serial("SCUS-94900", None).unwrap();
    assert_eq!(
        index.games[result.game_index].name,
        "Crash Bandicoot (USA)"
    );
}

