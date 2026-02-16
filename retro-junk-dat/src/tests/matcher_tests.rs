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
