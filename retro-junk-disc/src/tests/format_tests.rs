use crate::format::*;
use crate::sector::CHD_MAGIC;
use crate::test_helpers::{make_iso, make_raw_bin};
use std::io::Cursor;

#[test]
fn test_detect_iso_format() {
    let data = make_iso("PLAYSTATION");
    let mut cursor = Cursor::new(data);
    assert_eq!(detect_disc_format(&mut cursor).unwrap(), DiscFormat::Iso2048);
}

#[test]
fn test_detect_raw_bin_format() {
    let data = make_raw_bin("PLAYSTATION");
    let mut cursor = Cursor::new(data);
    assert_eq!(
        detect_disc_format(&mut cursor).unwrap(),
        DiscFormat::RawSector2352
    );
}

#[test]
fn test_detect_chd_magic() {
    let mut data = vec![0u8; 64];
    data[..8].copy_from_slice(CHD_MAGIC);
    let mut cursor = Cursor::new(data);
    assert_eq!(detect_disc_format(&mut cursor).unwrap(), DiscFormat::Chd);
}

#[test]
fn test_detect_cue_text() {
    let cue = b"FILE \"game.bin\" BINARY\r\n  TRACK 01 MODE2/2352\r\n    INDEX 01 00:00:00\r\n";
    let mut cursor = Cursor::new(cue.to_vec());
    assert_eq!(detect_disc_format(&mut cursor).unwrap(), DiscFormat::Cue);
}

#[test]
fn test_detect_invalid_data() {
    let data = vec![
        0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    let mut cursor = Cursor::new(data);
    assert!(detect_disc_format(&mut cursor).is_err());
}

#[test]
fn test_detect_non_playstation_iso() {
    let data = make_iso("SEGA SEGASATURN");
    let mut cursor = Cursor::new(data);
    assert_eq!(detect_disc_format(&mut cursor).unwrap(), DiscFormat::Iso2048);
}

#[test]
fn test_disc_format_name() {
    assert_eq!(DiscFormat::Iso2048.name(), "ISO 9660");
    assert_eq!(DiscFormat::RawSector2352.name(), "Raw BIN (2352)");
    assert_eq!(DiscFormat::Cue.name(), "CUE Sheet");
    assert_eq!(DiscFormat::Chd.name(), "CHD");
}

#[test]
fn test_disc_format_extension() {
    assert_eq!(DiscFormat::Iso2048.extension(), "iso");
    assert_eq!(DiscFormat::RawSector2352.extension(), "bin");
    assert_eq!(DiscFormat::Cue.extension(), "cue");
    assert_eq!(DiscFormat::Chd.extension(), "chd");
}
