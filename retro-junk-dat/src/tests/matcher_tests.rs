use super::*;
use crate::dat::{DatFile, DatGame, DatRom};

/// Helper: unwrap a `SerialLookupResult::Match`, panicking on NotFound/Ambiguous.
fn expect_match(result: SerialLookupResult) -> MatchResult {
    match result {
        SerialLookupResult::Match(m) => m,
        SerialLookupResult::Ambiguous { candidates } => {
            panic!("Expected Match, got Ambiguous with: {candidates:?}")
        }
        SerialLookupResult::NotFound => panic!("Expected Match, got NotFound"),
    }
}

/// Helper: build a `DatGame` containing a single ROM with the given metadata.
fn single_rom_game(
    name: &str,
    rom_name: &str,
    size: u64,
    crc: &str,
    serial: Option<&str>,
) -> DatGame {
    DatGame {
        name: name.into(),
        region: None,
        serial: None,
        version: None,
        category: None,
        roms: vec![DatRom {
            name: rom_name.into(),
            size,
            crc: crc.into(),
            sha1: None,
            md5: None,
            serial: serial.map(Into::into),
        }],
    }
}

fn make_test_dat() -> DatFile {
    DatFile {
        name: "Test".into(),
        description: "Test".into(),
        version: "1".into(),
        games: vec![
            DatGame {
                name: "Super Mario World (USA)".into(),
                region: None,
                serial: None,
                version: None,
                category: None,
                roms: vec![DatRom {
                    name: "Super Mario World (USA).sfc".into(),
                    size: 524_288,
                    crc: "b19ed489".into(),
                    sha1: Some("6b47bb75d16514b6a476aa0c73a683a2a4c18765".into()),
                    md5: None,
                    serial: None,
                }],
            },
            DatGame {
                name: "Super Mario 64 (USA)".into(),
                region: None,
                serial: None,
                version: None,
                category: None,
                roms: vec![DatRom {
                    name: "Super Mario 64 (USA).z64".into(),
                    size: 8_388_608,
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
                serial: None,
                version: None,
                category: None,
                roms: vec![DatRom {
                    name: "Super Mario 64 (Japan).z64".into(),
                    size: 8_388_608,
                    crc: "4eab3152".into(),
                    sha1: None,
                    md5: None,
                    serial: Some("NSMJ".into()),
                }],
            },
            DatGame {
                name: "The Legend of Zelda - A Link to the Past (USA)".into(),
                region: None,
                serial: None,
                version: None,
                category: None,
                roms: vec![DatRom {
                    name: "The Legend of Zelda - A Link to the Past (USA).sfc".into(),
                    size: 1_048_576,
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
        data_size: 524_288,
        warnings: vec![],
    };
    let result = index
        .match_by_hash(524_288, &hashes)
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(result.game_index, 0);
    assert_eq!(result.method, MatchMethod::Crc32);
}

#[test]
fn test_match_by_serial_exact() {
    let index = DatIndex::from_dat(make_test_dat());
    // Exact match: DAT has "SNS-ZL-USA", query "SNS-ZL-USA"
    let result = expect_match(index.match_by_serial("SNS-ZL-USA", None));
    assert_eq!(result.game_index, 3);
    assert_eq!(result.method, MatchMethod::Serial);
}

#[test]
fn test_match_by_serial_short_code() {
    let index = DatIndex::from_dat(make_test_dat());
    // DAT has short code "NSME", query with short code "NSME"
    let result = expect_match(index.match_by_serial("NSME", None));
    assert_eq!(result.game_index, 1);
    assert_eq!(result.method, MatchMethod::Serial);
}

#[test]
fn test_match_by_serial_long_to_short() {
    // Analyzer produces NUS-NSME-USA, DAT has NSME — should still match
    // via pre-extracted game code
    let index = DatIndex::from_dat(make_test_dat());
    let result = expect_match(index.match_by_serial("NUS-NSME-USA", Some("NSME")));
    assert_eq!(result.game_index, 1);
    assert_eq!(index.games[result.game_index].name, "Super Mario 64 (USA)");
}

#[test]
fn test_serial_distinguishes_regions() {
    let index = DatIndex::from_dat(make_test_dat());

    // Analyzer produces NUS-NSME-USA, extracts NSME → matches DAT's NSME
    let usa = expect_match(index.match_by_serial("NUS-NSME-USA", Some("NSME")));
    assert_eq!(usa.game_index, 1);
    assert_eq!(index.games[usa.game_index].name, "Super Mario 64 (USA)");

    // Analyzer produces NUS-NSMJ-JPN, extracts NSMJ → matches DAT's NSMJ
    let jpn = expect_match(index.match_by_serial("NUS-NSMJ-JPN", Some("NSMJ")));
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
        data_size: 8_388_608,
        warnings: vec![],
    };
    let usa = index
        .match_by_hash(8_388_608, &usa_hashes)
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(index.games[usa.game_index].name, "Super Mario 64 (USA)");

    let jpn_hashes = FileHashes {
        crc32: "4eab3152".into(),
        sha1: None,
        md5: None,
        data_size: 8_388_608,
        warnings: vec![],
    };
    let jpn = index
        .match_by_hash(8_388_608, &jpn_hashes)
        .into_iter()
        .next()
        .unwrap();
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
        warnings: vec![],
    };
    assert!(index.match_by_hash(999, &hashes).is_empty());
    assert!(matches!(
        index.match_by_serial("UNKNOWN", None),
        SerialLookupResult::NotFound
    ));
}

#[test]
fn test_from_dats_merge() {
    let dat1 = DatFile {
        name: "DAT A".into(),
        description: String::new(),
        version: "1".into(),
        games: vec![DatGame {
            name: "Game A (USA)".into(),
            region: None,
            serial: None,
            version: None,
            category: None,
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
        description: String::new(),
        version: "2".into(),
        games: vec![DatGame {
            name: "Game B (USA)".into(),
            region: None,
            serial: None,
            version: None,
            category: None,
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
    let result_a = expect_match(index.match_by_serial("SLUS-99999", None));
    assert_eq!(index.games[result_a.game_index].name, "Game A (USA)");

    // Can find game from second DAT
    let result_b = expect_match(index.match_by_serial("SLUS-88888", None));
    assert_eq!(index.games[result_b.game_index].name, "Game B (USA)");

    // Hash lookup works across merged DATs
    let hashes = FileHashes {
        crc32: "bbbb0002".into(),
        sha1: None,
        md5: None,
        data_size: 2048,
        warnings: vec![],
    };
    let hash_result = index
        .match_by_hash(2048, &hashes)
        .into_iter()
        .next()
        .unwrap();
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
        data_size: 524_288,
        warnings: vec![],
    };
    assert!(index.match_by_hash(999, &hashes).is_empty());
}

#[test]
fn test_comma_separated_serials() {
    // Redump DATs have comma-separated serials like "SLUS-01041, SLUS-01041GH"
    let dat = DatFile {
        name: "Test".into(),
        description: String::new(),
        version: "1".into(),
        games: vec![DatGame {
            name: "Chrono Cross (USA) (Disc 1)".into(),
            region: None,
            serial: None,
            version: None,
            category: None,
            roms: vec![DatRom {
                name: "Chrono Cross (USA) (Disc 1).bin".into(),
                size: 736_651_104,
                crc: "a07898cc".into(),
                sha1: None,
                md5: None,
                serial: Some("SLUS-01041, SLUS-01041GH, SLUS-01041GH-F".into()),
            }],
        }],
    };
    let index = DatIndex::from_dat(dat);

    // Each individual serial should be findable
    assert!(matches!(
        index.match_by_serial("SLUS-01041", None),
        SerialLookupResult::Match(_)
    ));
    assert!(matches!(
        index.match_by_serial("SLUS-01041GH", None),
        SerialLookupResult::Match(_)
    ));
    assert!(matches!(
        index.match_by_serial("SLUS-01041GH-F", None),
        SerialLookupResult::Match(_)
    ));
}

#[test]
fn test_serial_space_dash_normalization() {
    // Redump DATs sometimes use spaces instead of dashes: "SLPS 00700"
    // ROM analysis produces dashes: "SLPS-00700"
    let dat = DatFile {
        name: "Test".into(),
        description: String::new(),
        version: "1".into(),
        games: vec![DatGame {
            name: "Some Game (Japan)".into(),
            region: None,
            serial: None,
            version: None,
            category: None,
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
    let result = expect_match(index.match_by_serial("SLPS-00700", None));
    assert_eq!(index.games[result.game_index].name, "Some Game (Japan)");
}

#[test]
fn test_multi_disc_suffix_prefers_suffixed_over_bare() {
    // LibRetro Redump DATs have both bare and suffixed entries for multi-disc
    // games. When a disc's boot serial matches the bare entry, the "-0"
    // suffixed entry should be preferred since the bare serial is ambiguous.
    let dat = DatFile {
        name: "Test".into(),
        description: String::new(),
        version: "1".into(),
        games: vec![
            // Bare entries (shared serial — multiple entries in Vec now)
            // interleaved with their "-N" suffixed counterparts.
            single_rom_game(
                "FF7 (USA) (Disc 1)",
                "FF7 (USA) (Disc 1).bin",
                747_435_024,
                "1459cbef",
                Some("SCUS-94163"),
            ),
            single_rom_game(
                "FF7 (USA) (Disc 1) [suffixed]",
                "FF7 (USA) (Disc 1).bin",
                747_435_024,
                "1459cbef",
                Some("SCUS-94163-0"),
            ),
            single_rom_game(
                "FF7 (USA) (Disc 2)",
                "FF7 (USA) (Disc 2).bin",
                732_657_408,
                "a997a8cc",
                Some("SCUS-94163"),
            ),
            single_rom_game(
                "FF7 (USA) (Disc 2) [suffixed]",
                "FF7 (USA) (Disc 2).bin",
                732_657_408,
                "a997a8cc",
                Some("SCUS-94163-1"),
            ),
            single_rom_game(
                "FF7 (USA) (Disc 3)",
                "FF7 (USA) (Disc 3).bin",
                659_561_952,
                "1c27b277",
                Some("SCUS-94163"),
            ),
            single_rom_game(
                "FF7 (USA) (Disc 3) [suffixed]",
                "FF7 (USA) (Disc 3).bin",
                659_561_952,
                "1c27b277",
                Some("SCUS-94163-2"),
            ),
        ],
    };
    let index = DatIndex::from_dat(dat);

    // Disc 1's boot serial "SCUS-94163" should prefer the "-0" suffixed entry
    let disc1 = expect_match(index.match_by_serial("SCUS-94163", None));
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
        description: String::new(),
        version: "1".into(),
        games: vec![DatGame {
            name: "Some Game (USA) (Disc 1)".into(),
            region: None,
            serial: None,
            version: None,
            category: None,
            roms: vec![DatRom {
                name: "Some Game (USA) (Disc 1).bin".into(),
                size: 700_000_000,
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
    let result = expect_match(index.match_by_serial("SLUS-99999", None));
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
        description: String::new(),
        version: "1".into(),
        games: vec![DatGame {
            name: "Crash Bandicoot (USA)".into(),
            region: None,
            serial: None,
            version: None,
            category: None,
            roms: vec![DatRom {
                name: "Crash Bandicoot (USA).bin".into(),
                size: 500_000_000,
                crc: "aabbccdd".into(),
                sha1: None,
                md5: None,
                serial: Some("SCUS-94900".into()),
            }],
        }],
    };
    let index = DatIndex::from_dat(dat);

    let result = expect_match(index.match_by_serial("SCUS-94900", None));
    assert_eq!(index.games[result.game_index].name, "Crash Bandicoot (USA)");
}

// --- Ambiguity tests ---

#[test]
fn test_ambiguous_serial_returns_ambiguous() {
    // Two different games share the same serial (e.g., original + romhack)
    let dat = DatFile {
        name: "Test".into(),
        description: String::new(),
        version: "1".into(),
        games: vec![
            DatGame {
                name: "Pokemon FireRed (USA)".into(),
                region: None,
                serial: None,
                version: None,
                category: None,
                roms: vec![DatRom {
                    name: "Pokemon FireRed (USA).gba".into(),
                    size: 16_777_216,
                    crc: "dd88761c".into(),
                    sha1: None,
                    md5: None,
                    serial: Some("BPRE".into()),
                }],
            },
            DatGame {
                name: "Pokemon FireRed (USA) (Rev 1)".into(),
                region: None,
                serial: None,
                version: None,
                category: None,
                roms: vec![DatRom {
                    name: "Pokemon FireRed (USA) (Rev 1).gba".into(),
                    size: 16_777_216,
                    crc: "aabbccdd".into(),
                    sha1: None,
                    md5: None,
                    serial: Some("BPRE".into()),
                }],
            },
        ],
    };
    let index = DatIndex::from_dat(dat);

    match index.match_by_serial("BPRE", None) {
        SerialLookupResult::Ambiguous { candidates } => {
            assert_eq!(candidates.len(), 2);
            assert!(candidates.contains(&"Pokemon FireRed (USA)".to_string()));
            assert!(candidates.contains(&"Pokemon FireRed (USA) (Rev 1)".to_string()));
        }
        other => panic!("Expected Ambiguous, got {other:?}"),
    }
}

#[test]
fn test_ambiguous_via_game_code() {
    // Two games share the same 4-char code, tested via the game_code path
    let dat = DatFile {
        name: "Test".into(),
        description: String::new(),
        version: "1".into(),
        games: vec![
            DatGame {
                name: "Game Original (USA)".into(),
                region: None,
                serial: None,
                version: None,
                category: None,
                roms: vec![DatRom {
                    name: "Game Original (USA).z64".into(),
                    size: 8_388_608,
                    crc: "11111111".into(),
                    sha1: None,
                    md5: None,
                    serial: Some("NXYZ".into()),
                }],
            },
            DatGame {
                name: "Game Original (USA) (Rev 1)".into(),
                region: None,
                serial: None,
                version: None,
                category: None,
                roms: vec![DatRom {
                    name: "Game Original (USA) (Rev 1).z64".into(),
                    size: 8_388_608,
                    crc: "22222222".into(),
                    sha1: None,
                    md5: None,
                    serial: Some("NXYZ".into()),
                }],
            },
        ],
    };
    let index = DatIndex::from_dat(dat);

    // Full serial doesn't exist, but game_code "NXYZ" matches two entries
    match index.match_by_serial("NUS-NXYZ-USA", Some("NXYZ")) {
        SerialLookupResult::Ambiguous { candidates } => {
            assert_eq!(candidates.len(), 2);
            assert!(candidates.contains(&"Game Original (USA)".to_string()));
            assert!(candidates.contains(&"Game Original (USA) (Rev 1)".to_string()));
        }
        other => panic!("Expected Ambiguous, got {other:?}"),
    }
}

#[test]
fn test_multi_disc_shared_bare_serial_resolves_via_suffix() {
    // Multi-disc games where the bare serial is shared but "-0" suffix exists
    // should NOT be ambiguous — the suffix resolves it.
    let dat = DatFile {
        name: "Test".into(),
        description: String::new(),
        version: "1".into(),
        games: vec![
            DatGame {
                name: "Multi Disc Game (USA) (Disc 1)".into(),
                region: None,
                serial: None,
                version: None,
                category: None,
                roms: vec![DatRom {
                    name: "Multi Disc Game (USA) (Disc 1).bin".into(),
                    size: 700_000_000,
                    crc: "aaaa0001".into(),
                    sha1: None,
                    md5: None,
                    serial: Some("SLUS-12345".into()),
                }],
            },
            DatGame {
                name: "Multi Disc Game (USA) (Disc 2)".into(),
                region: None,
                serial: None,
                version: None,
                category: None,
                roms: vec![DatRom {
                    name: "Multi Disc Game (USA) (Disc 2).bin".into(),
                    size: 700_000_000,
                    crc: "aaaa0002".into(),
                    sha1: None,
                    md5: None,
                    serial: Some("SLUS-12345".into()),
                }],
            },
            DatGame {
                name: "Multi Disc Game (USA) (Disc 1) [suffixed]".into(),
                region: None,
                serial: None,
                version: None,
                category: None,
                roms: vec![DatRom {
                    name: "Multi Disc Game (USA) (Disc 1).bin".into(),
                    size: 700_000_000,
                    crc: "aaaa0001".into(),
                    sha1: None,
                    md5: None,
                    serial: Some("SLUS-12345-0".into()),
                }],
            },
        ],
    };
    let index = DatIndex::from_dat(dat);

    // Bare serial "SLUS-12345" is shared by two games, but "-0" suffix
    // uniquely identifies Disc 1 — should resolve, not be ambiguous
    let result = expect_match(index.match_by_serial("SLUS-12345", None));
    assert!(
        index.games[result.game_index].name.contains("Disc 1"),
        "Expected Disc 1 match via suffix, got: {}",
        index.games[result.game_index].name
    );
}

#[test]
fn test_same_name_entries_resolve_as_match() {
    // Multiple DAT entries with the same serial AND the same game name
    // (e.g., same game listed in multiple DATs after merge) should resolve
    // as a unique match, not ambiguous.
    let dat = DatFile {
        name: "Test".into(),
        description: String::new(),
        version: "1".into(),
        games: vec![
            DatGame {
                name: "Metroid Fusion (USA)".into(),
                region: None,
                serial: None,
                version: None,
                category: None,
                roms: vec![DatRom {
                    name: "Metroid Fusion (USA).gba".into(),
                    size: 8_388_608,
                    crc: "11111111".into(),
                    sha1: None,
                    md5: None,
                    serial: Some("AMTE".into()),
                }],
            },
            DatGame {
                name: "Metroid Fusion (USA)".into(),
                region: None,
                serial: None,
                version: None,
                category: None,
                roms: vec![DatRom {
                    name: "Metroid Fusion (USA).gba".into(),
                    size: 8_388_608,
                    crc: "22222222".into(),
                    sha1: None,
                    md5: None,
                    serial: Some("AMTE".into()),
                }],
            },
        ],
    };
    let index = DatIndex::from_dat(dat);

    // Both entries have the same name — should match, not be ambiguous
    let result = expect_match(index.match_by_serial("AMTE", None));
    assert_eq!(index.games[result.game_index].name, "Metroid Fusion (USA)");
}

#[test]
fn test_match_short_game_code_to_long_dat_serial() {
    // Redump DATs use full product codes like "DL-DOL-GALE-0-USA"
    // Analyzer extracts short 4-char game code "GALE" from disc header
    let dat = DatFile {
        name: "Test".into(),
        description: String::new(),
        version: "1".into(),
        games: vec![DatGame {
            name: "The Legend of Zelda - The Wind Waker (USA)".into(),
            region: None,
            serial: None,
            version: None,
            category: None,
            roms: vec![DatRom {
                name: "The Legend of Zelda - The Wind Waker (USA).iso".into(),
                size: 1_459_978_240,
                crc: "d8e4d45a".into(),
                sha1: None,
                md5: None,
                serial: Some("DL-DOL-GALE-0-USA".into()),
            }],
        }],
    };
    let index = DatIndex::from_dat(dat);

    // Short game code should find the entry via sub-segment indexing
    let result = expect_match(index.match_by_serial("GALE", Some("GALE")));
    assert_eq!(
        index.games[result.game_index].name,
        "The Legend of Zelda - The Wind Waker (USA)"
    );
}

#[test]
fn test_hash_returns_all_regional_variants() {
    // When USA and Japan versions share the same data track hash,
    // match_by_hash should return both entries.
    let dat = DatFile {
        name: "Test".into(),
        description: String::new(),
        version: "1".into(),
        games: vec![
            DatGame {
                name: "NiGHTS into Dreams... (USA)".into(),
                region: Some("USA".into()),
                serial: None,
                version: None,
                category: None,
                roms: vec![DatRom {
                    name: "NiGHTS into Dreams... (USA).chd".into(),
                    size: 500_000_000,
                    crc: "aabb1122".into(),
                    sha1: None,
                    md5: None,
                    serial: None,
                }],
            },
            DatGame {
                name: "NiGHTS into Dreams... (Japan)".into(),
                region: Some("Japan".into()),
                serial: None,
                version: None,
                category: None,
                roms: vec![DatRom {
                    name: "NiGHTS into Dreams... (Japan).chd".into(),
                    size: 500_000_000,
                    crc: "aabb1122".into(), // Same hash as USA
                    sha1: None,
                    md5: None,
                    serial: None,
                }],
            },
        ],
    };
    let index = DatIndex::from_dat(dat);

    let hashes = FileHashes {
        crc32: "aabb1122".into(),
        sha1: None,
        md5: None,
        data_size: 500_000_000,
        warnings: vec![],
    };
    let matches = index.match_by_hash(500_000_000, &hashes);
    assert_eq!(matches.len(), 2, "Should return both USA and Japan entries");

    let names: Vec<&str> = matches
        .iter()
        .map(|m| index.games[m.game_index].name.as_str())
        .collect();
    assert!(names.contains(&"NiGHTS into Dreams... (USA)"));
    assert!(names.contains(&"NiGHTS into Dreams... (Japan)"));
}
