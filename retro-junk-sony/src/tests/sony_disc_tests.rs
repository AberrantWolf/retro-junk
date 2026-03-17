use super::*;
use crate::disc_test_helpers::make_iso_with_system_cnf;
use retro_junk_core::Region;
use retro_junk_disc::format::DiscFormat;
use retro_junk_disc::iso9660::{find_file_in_root, read_pvd};
use std::io::Cursor;

// sony_disc tests use "BOOT" key by default for SYSTEM.CNF
fn make_boot_iso_with_system_cnf(serial: &str) -> Vec<u8> {
    make_iso_with_system_cnf(serial, "BOOT")
}

// -- SYSTEM.CNF parsing tests --

#[test]
fn test_parse_system_cnf_standard() {
    let cnf = "BOOT = cdrom:\\SLUS_012.34;1\r\nVMODE = NTSC\r\n";
    let result = parse_system_cnf(cnf).unwrap();
    assert_eq!(result.boot_path, "cdrom:\\SLUS_012.34;1");
    assert_eq!(result.boot_key, BootKey::Boot);
    assert_eq!(result.vmode.as_deref(), Some("NTSC"));
}

#[test]
fn test_parse_system_cnf_boot2() {
    let cnf = "BOOT2 = cdrom0:\\SLPS_123.45;1\r\n";
    let result = parse_system_cnf(cnf).unwrap();
    assert_eq!(result.boot_path, "cdrom0:\\SLPS_123.45;1");
    assert_eq!(result.boot_key, BootKey::Boot2);
    assert_eq!(result.vmode, None);
}

#[test]
fn test_parse_system_cnf_boot2_preferred_over_boot() {
    let cnf = "BOOT = cdrom:\\OLD.EXE;1\r\nBOOT2 = cdrom0:\\SLUS_999.99;1\r\n";
    let result = parse_system_cnf(cnf).unwrap();
    assert_eq!(result.boot_path, "cdrom0:\\SLUS_999.99;1");
    assert_eq!(result.boot_key, BootKey::Boot2);
}

#[test]
fn test_parse_system_cnf_missing_boot() {
    let cnf = "VMODE = PAL\r\n";
    assert!(parse_system_cnf(cnf).is_err());
}

// -- Serial extraction tests --

#[test]
fn test_extract_serial_slus() {
    assert_eq!(
        extract_serial("cdrom:\\SLUS_012.34;1"),
        Some("SLUS-01234".to_string())
    );
}

#[test]
fn test_extract_serial_sles() {
    assert_eq!(
        extract_serial("cdrom:\\SLES_567.89;1"),
        Some("SLES-56789".to_string())
    );
}

#[test]
fn test_extract_serial_scps() {
    assert_eq!(
        extract_serial("cdrom:\\SCPS_100.01;1"),
        Some("SCPS-10001".to_string())
    );
}

#[test]
fn test_extract_serial_double_backslash() {
    assert_eq!(
        extract_serial("cdrom:\\\\SLUS_012.34;1"),
        Some("SLUS-01234".to_string())
    );
}

#[test]
fn test_extract_serial_no_version() {
    assert_eq!(
        extract_serial("cdrom:\\SLPS_000.01"),
        Some("SLPS-00001".to_string())
    );
}

#[test]
fn test_extract_serial_no_backslash() {
    assert_eq!(
        extract_serial("cdrom:SLUS_006.91;1"),
        Some("SLUS-00691".to_string())
    );
}

#[test]
fn test_extract_serial_ps2_cdrom0() {
    assert_eq!(
        extract_serial("cdrom0:\\SLUS_200.62;1"),
        Some("SLUS-20062".to_string())
    );
}

#[test]
fn test_extract_serial_invalid() {
    assert_eq!(extract_serial("cdrom:\\BOOT.EXE;1"), None);
}

// -- Region mapping tests --

#[test]
fn test_serial_to_region() {
    assert_eq!(serial_to_region("SLUS-01234"), Some(Region::Usa));
    assert_eq!(serial_to_region("SCUS-94900"), Some(Region::Usa));
    assert_eq!(serial_to_region("SLES-01234"), Some(Region::Europe));
    assert_eq!(serial_to_region("SCES-01234"), Some(Region::Europe));
    assert_eq!(serial_to_region("SLPS-01234"), Some(Region::Japan));
    assert_eq!(serial_to_region("SCPS-01234"), Some(Region::Japan));
    assert_eq!(serial_to_region("SLPM-01234"), Some(Region::Japan));
    assert_eq!(serial_to_region("SLKA-01234"), Some(Region::Korea));
    assert_eq!(serial_to_region("XXXX-01234"), None);
}

// -- Full ISO with SYSTEM.CNF tests --

#[test]
fn test_find_system_cnf_in_iso() {
    let data = make_boot_iso_with_system_cnf("SLUS_012.34");
    let mut cursor = Cursor::new(data);
    let pvd = read_pvd(&mut cursor, DiscFormat::Iso2048).unwrap();
    let content = find_file_in_root(&mut cursor, DiscFormat::Iso2048, &pvd, "SYSTEM.CNF").unwrap();
    let text = String::from_utf8_lossy(&content);
    assert!(text.contains("SLUS_012.34"));
}

#[test]
fn test_full_iso_serial_extraction() {
    let data = make_boot_iso_with_system_cnf("SLUS_012.34");
    let mut cursor = Cursor::new(data);
    let pvd = read_pvd(&mut cursor, DiscFormat::Iso2048).unwrap();
    let content = find_file_in_root(&mut cursor, DiscFormat::Iso2048, &pvd, "SYSTEM.CNF").unwrap();
    let text = String::from_utf8_lossy(&content);
    let cnf = parse_system_cnf(&text).unwrap();
    let serial = extract_serial(&cnf.boot_path).unwrap();
    assert_eq!(serial, "SLUS-01234");
}
