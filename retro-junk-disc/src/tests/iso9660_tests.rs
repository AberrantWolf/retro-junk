use crate::format::DiscFormat;
use crate::iso9660::*;
use crate::test_helpers::{make_iso, make_iso_with_file, make_raw_bin};
use std::io::Cursor;

#[test]
fn test_read_pvd_iso() {
    let data = make_iso("PLAYSTATION");
    let mut cursor = Cursor::new(data);
    let pvd = read_pvd(&mut cursor, DiscFormat::Iso2048).unwrap();
    assert_eq!(pvd.system_identifier, "PLAYSTATION");
    assert_eq!(pvd.volume_identifier, "TEST_VOLUME");
    assert_eq!(pvd.volume_space_size, 200);
}

#[test]
fn test_read_pvd_raw_bin() {
    let data = make_raw_bin("PLAYSTATION");
    let mut cursor = Cursor::new(data);
    let pvd = read_pvd(&mut cursor, DiscFormat::RawSector2352).unwrap();
    assert_eq!(pvd.system_identifier, "PLAYSTATION");
    assert_eq!(pvd.volume_identifier, "TEST_VOLUME");
}

#[test]
fn test_pvd_non_playstation() {
    let data = make_iso("SOME_OTHER_SYS");
    let mut cursor = Cursor::new(data);
    let pvd = read_pvd(&mut cursor, DiscFormat::Iso2048).unwrap();
    assert_eq!(pvd.system_identifier, "SOME_OTHER_SYS");
}

#[test]
fn test_pvd_saturn_system_id() {
    let data = make_iso("SEGA SEGASATURN");
    let mut cursor = Cursor::new(data);
    let pvd = read_pvd(&mut cursor, DiscFormat::Iso2048).unwrap();
    assert_eq!(pvd.system_identifier, "SEGA SEGASATURN");
}

#[test]
fn test_find_file_in_root() {
    let content = b"Hello, world!";
    let data = make_iso_with_file("PLAYSTATION", "TEST.TXT", content);
    let mut cursor = Cursor::new(data);
    let pvd = read_pvd(&mut cursor, DiscFormat::Iso2048).unwrap();
    let result = find_file_in_root(&mut cursor, DiscFormat::Iso2048, &pvd, "TEST.TXT").unwrap();
    assert_eq!(&result[..content.len()], content);
}

#[test]
fn test_file_not_found_in_root() {
    let data = make_iso_with_file("PLAYSTATION", "TEST.TXT", b"data");
    let mut cursor = Cursor::new(data);
    let pvd = read_pvd(&mut cursor, DiscFormat::Iso2048).unwrap();
    assert!(find_file_in_root(&mut cursor, DiscFormat::Iso2048, &pvd, "NONEXIST.TXT").is_err());
}

#[test]
fn test_parse_directory_record_short_buffer() {
    let data = vec![0x28u8; 20];
    assert!(parse_directory_record(&data).is_none());
}

#[test]
fn test_parse_directory_record_empty_buffer() {
    assert!(parse_directory_record(&[]).is_none());
}

#[test]
fn test_parse_directory_record_valid_dot_entry() {
    let mut data = vec![0u8; 34];
    data[0] = 34; // record_len
    data[32] = 1; // id_len
    data[33] = 0x00; // file identifier = "."
    let rec = parse_directory_record(&data);
    assert!(rec.is_some());
    assert_eq!(rec.unwrap().file_identifier, ".");
}
