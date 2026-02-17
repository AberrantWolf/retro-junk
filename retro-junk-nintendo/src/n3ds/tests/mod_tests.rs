use super::*;
use std::io::Cursor;

use retro_junk_core::{AnalysisOptions, Region, RomAnalyzer};

/// Minimal CCI for can_handle / format detection tests.
fn make_cci_minimal() -> Vec<u8> {
    let partition0_offset: u64 = 0x4000;
    let ncch_content_size_mu: u32 = 0x100;
    let total_size = partition0_offset + ncch_content_size_mu as u64 * MEDIA_UNIT;
    let mut rom = vec![0u8; total_size as usize];

    rom[0x00] = 0xAB;
    rom[0x100..0x104].copy_from_slice(&NCSD_MAGIC);
    let image_size_mu = (total_size / MEDIA_UNIT) as u32;
    rom[0x104..0x108].copy_from_slice(&image_size_mu.to_le_bytes());
    rom[0x108..0x110].copy_from_slice(&0x0004000000ABCDEF_u64.to_le_bytes());
    let p0_offset_mu = (partition0_offset / MEDIA_UNIT) as u32;
    rom[0x120..0x124].copy_from_slice(&p0_offset_mu.to_le_bytes());
    rom[0x124..0x128].copy_from_slice(&ncch_content_size_mu.to_le_bytes());
    rom[0x188 + 4] = 1;
    rom[0x188 + 5] = 1;
    rom[0x200..0x204].copy_from_slice(&0xFFFFFFFF_u32.to_le_bytes());
    rom[0x300..0x304].copy_from_slice(&(total_size as u32).to_le_bytes());
    rom[0x1000] = 0x42;

    let p0 = partition0_offset as usize;
    rom[p0 + 0x100..p0 + 0x104].copy_from_slice(&NCCH_MAGIC);
    rom[p0 + 0x104..p0 + 0x108].copy_from_slice(&ncch_content_size_mu.to_le_bytes());
    rom[p0 + 0x108..p0 + 0x110].copy_from_slice(&0x0004000000ABCDEF_u64.to_le_bytes());
    rom[p0 + 0x110..p0 + 0x112].copy_from_slice(b"31");
    rom[p0 + 0x118..p0 + 0x120].copy_from_slice(&0x0004000000ABCDEF_u64.to_le_bytes());
    rom[p0 + 0x150..p0 + 0x160].copy_from_slice(b"CTR-P-ABCE\0\0\0\0\0\0");
    rom[p0 + 0x180..p0 + 0x184].copy_from_slice(&0x400u32.to_le_bytes());
    rom[p0 + 0x188 + 4] = 0x01;
    rom[p0 + 0x188 + 5] = 0x03;
    rom[p0 + 0x188 + 7] = 0x04;

    rom
}

/// Minimal CIA for can_handle / format detection tests.
fn make_cia_minimal() -> Vec<u8> {
    let header_size: u32 = 0x2020;
    let cert_chain_size: u32 = 0x0A00;
    let ticket_size: u32 = 0x0350;
    let tmd_size: u32 = 0x0208;
    let ncch_size: u64 = 0x10000;

    let mut cia = Vec::new();
    let mut header = vec![0u8; header_size as usize];
    header[0x00..0x04].copy_from_slice(&header_size.to_le_bytes());
    header[0x08..0x0C].copy_from_slice(&cert_chain_size.to_le_bytes());
    header[0x0C..0x10].copy_from_slice(&ticket_size.to_le_bytes());
    header[0x10..0x14].copy_from_slice(&tmd_size.to_le_bytes());
    header[0x18..0x20].copy_from_slice(&ncch_size.to_le_bytes());
    header[0x20] = 0x80;
    cia.extend_from_slice(&header);
    cia.resize(common::align64(cia.len() as u64) as usize, 0);

    let cert_end = cia.len() + cert_chain_size as usize;
    cia.resize(cert_end, 0xCC);
    cia.resize(common::align64(cia.len() as u64) as usize, 0);

    let mut ticket = vec![0u8; ticket_size as usize];
    ticket[0x00..0x04].copy_from_slice(&0x00010004u32.to_be_bytes());
    let title_id: u64 = 0x00040000_00ABCDEF;
    ticket[0x1DC..0x1E4].copy_from_slice(&title_id.to_be_bytes());
    cia.extend_from_slice(&ticket);
    cia.resize(common::align64(cia.len() as u64) as usize, 0);

    let mut tmd = vec![0u8; tmd_size as usize];
    tmd[0x00..0x04].copy_from_slice(&0x00010004u32.to_be_bytes());
    let tmd_hdr = 0x140;
    tmd[tmd_hdr + 0x4C..tmd_hdr + 0x54].copy_from_slice(&title_id.to_be_bytes());
    tmd[tmd_hdr + 0x9C..tmd_hdr + 0x9E].copy_from_slice(&0x0410u16.to_be_bytes());
    tmd[tmd_hdr + 0x9E..tmd_hdr + 0xA0].copy_from_slice(&1u16.to_be_bytes());
    cia.extend_from_slice(&tmd);
    cia.resize(common::align64(cia.len() as u64) as usize, 0);

    let mut ncch = vec![0u8; ncch_size as usize];
    ncch[0x100..0x104].copy_from_slice(&NCCH_MAGIC);
    ncch[0x104..0x108].copy_from_slice(&((ncch_size / MEDIA_UNIT) as u32).to_le_bytes());
    ncch[0x110..0x112].copy_from_slice(b"31");
    ncch[0x150..0x160].copy_from_slice(b"CTR-N-ABCJ\0\0\0\0\0\0");
    ncch[0x188 + 4] = 1;
    ncch[0x188 + 5] = 3;
    ncch[0x188 + 7] = 0x04;
    cia.extend_from_slice(&ncch);
    cia.resize(common::align64(cia.len() as u64) as usize, 0);

    cia
}

#[test]
fn test_can_handle_cci() {
    let rom = make_cci_minimal();
    let analyzer = N3dsAnalyzer::new();
    assert!(analyzer.can_handle(&mut Cursor::new(rom)));
}

#[test]
fn test_can_handle_cia() {
    let cia = make_cia_minimal();
    let analyzer = N3dsAnalyzer::new();
    assert!(analyzer.can_handle(&mut Cursor::new(cia)));
}

#[test]
fn test_can_handle_invalid() {
    let data = vec![0u8; 0x10000];
    let analyzer = N3dsAnalyzer::new();
    assert!(!analyzer.can_handle(&mut Cursor::new(data)));
}

#[test]
fn test_can_handle_too_small() {
    let data = vec![0u8; 0x100];
    let analyzer = N3dsAnalyzer::new();
    assert!(!analyzer.can_handle(&mut Cursor::new(data)));
}

#[test]
fn test_detect_format_cci() {
    let rom = make_cci_minimal();
    let format = detect_format(&mut Cursor::new(rom)).unwrap();
    assert_eq!(format, Some(N3dsFormat::Cci));
}

#[test]
fn test_detect_format_cia() {
    let cia = make_cia_minimal();
    let format = detect_format(&mut Cursor::new(cia)).unwrap();
    assert_eq!(format, Some(N3dsFormat::Cia));
}

#[test]
fn test_detect_format_unknown() {
    let data = vec![0u8; 0x10000];
    let format = detect_format(&mut Cursor::new(data)).unwrap();
    assert_eq!(format, None);
}

#[test]
fn test_full_cci_via_analyzer() {
    let rom = make_cci_minimal();
    let analyzer = N3dsAnalyzer::new();
    let options = AnalysisOptions { quick: true, ..Default::default() };
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(result.platform.as_deref(), Some("Nintendo 3DS"));
    assert_eq!(result.serial_number.as_deref(), Some("CTR-P-ABCE"));
    assert_eq!(result.regions, vec![Region::Usa]);
}

#[test]
fn test_full_cia_via_analyzer() {
    let cia = make_cia_minimal();
    let analyzer = N3dsAnalyzer::new();
    let options = AnalysisOptions { quick: true, ..Default::default() };
    let result = analyzer.analyze(&mut Cursor::new(cia), &options).unwrap();

    assert_eq!(result.platform.as_deref(), Some("Nintendo 3DS"));
    assert_eq!(result.serial_number.as_deref(), Some("CTR-N-ABCJ"));
    assert_eq!(result.regions, vec![Region::Japan]);
}
