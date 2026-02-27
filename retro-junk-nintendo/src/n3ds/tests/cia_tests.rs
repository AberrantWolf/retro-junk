use super::*;
use std::io::Cursor;

use super::super::{MEDIA_UNIT, NCCH_MAGIC};

/// Build a minimal synthetic CIA file.
fn make_cia() -> Vec<u8> {
    // CIA header (0x2020 bytes)
    let header_size: u32 = 0x2020;
    let cert_chain_size: u32 = 0x0A00; // typical
    let ticket_size: u32 = 0x0350; // RSA-2048 ticket
    let tmd_size: u32 = 0x0208; // small TMD

    // NCCH content: 64 KB
    let ncch_size: u64 = 0x10000;

    let meta_size: u32 = 0;

    let mut cia = Vec::new();

    // -- CIA Header --
    let mut header = vec![0u8; header_size as usize];
    header[0x00..0x04].copy_from_slice(&header_size.to_le_bytes());
    header[0x04..0x06].copy_from_slice(&0u16.to_le_bytes()); // type
    header[0x06..0x08].copy_from_slice(&0u16.to_le_bytes()); // version
    header[0x08..0x0C].copy_from_slice(&cert_chain_size.to_le_bytes());
    header[0x0C..0x10].copy_from_slice(&ticket_size.to_le_bytes());
    header[0x10..0x14].copy_from_slice(&tmd_size.to_le_bytes());
    header[0x14..0x18].copy_from_slice(&meta_size.to_le_bytes());
    header[0x18..0x20].copy_from_slice(&ncch_size.to_le_bytes());
    // Content index: bit 0 set (content index 0 present)
    header[0x20] = 0x80; // big-endian bit 0 of content index
    cia.extend_from_slice(&header);
    // Align to 64
    cia.resize(align64(cia.len() as u64) as usize, 0);

    // -- Certificate chain (dummy) --
    let cert_end = cia.len() + cert_chain_size as usize;
    cia.resize(cert_end, 0xCC);
    cia.resize(align64(cia.len() as u64) as usize, 0);

    // -- Ticket --
    let mut ticket = vec![0u8; ticket_size as usize];
    // Signature type: RSA-2048 SHA-256 = 0x00010004 (big-endian)
    ticket[0x00..0x04].copy_from_slice(&0x00010004u32.to_be_bytes());
    // Title ID at ticket_data + 0x9C = 0x140 + 0x9C = 0x1DC
    let title_id: u64 = 0x00040000_00ABCDEF;
    ticket[0x1DC..0x1E4].copy_from_slice(&title_id.to_be_bytes());
    cia.extend_from_slice(&ticket);
    cia.resize(align64(cia.len() as u64) as usize, 0);

    // -- TMD --
    let mut tmd = vec![0u8; tmd_size as usize];
    // Signature type: RSA-2048 SHA-256
    tmd[0x00..0x04].copy_from_slice(&0x00010004u32.to_be_bytes());
    // Signature block size = 4 + 256 + 60 = 320 = 0x140
    // TMD header at 0x140:
    let tmd_hdr = 0x140;
    // Title ID at TMD header + 0x4C
    tmd[tmd_hdr + 0x4C..tmd_hdr + 0x54].copy_from_slice(&title_id.to_be_bytes());
    // Title version at TMD header + 0x9C
    let tv: u16 = 0x0410; // v1.1.0
    tmd[tmd_hdr + 0x9C..tmd_hdr + 0x9E].copy_from_slice(&tv.to_be_bytes());
    // Content count at TMD header + 0x9E
    tmd[tmd_hdr + 0x9E..tmd_hdr + 0xA0].copy_from_slice(&1u16.to_be_bytes());
    cia.extend_from_slice(&tmd);
    cia.resize(align64(cia.len() as u64) as usize, 0);

    // -- Content (NCCH) --
    let mut ncch = vec![0u8; ncch_size as usize];

    // NCCH magic
    ncch[0x100..0x104].copy_from_slice(&NCCH_MAGIC);
    ncch[0x104..0x108].copy_from_slice(&((ncch_size / MEDIA_UNIT) as u32).to_le_bytes());
    ncch[0x108..0x110].copy_from_slice(&title_id.to_le_bytes()); // partition ID
    ncch[0x110..0x112].copy_from_slice(b"31"); // maker code
    ncch[0x118..0x120].copy_from_slice(&title_id.to_le_bytes()); // program ID

    // Product code: "CTR-N-ABCJ" (10 bytes + 6 null padding = 16 bytes at 0x150)
    let product = b"CTR-N-ABCJ\0\0\0\0\0\0";
    ncch[0x150..0x160].copy_from_slice(product);

    // Flags: NoCrypto, executable
    ncch[0x188 + 4] = 1; // CTR platform
    ncch[0x188 + 5] = 3; // executable with RomFS
    ncch[0x188 + 7] = 0x04; // NoCrypto

    cia.extend_from_slice(&ncch);
    cia.resize(align64(cia.len() as u64) as usize, 0);

    cia
}

#[test]
fn test_cia_basic_analysis() {
    let cia = make_cia();
    let file_size = cia.len() as u64;
    let options = AnalysisOptions::default();
    let result = analyze_cia(&mut Cursor::new(cia), file_size, &options).unwrap();

    assert_eq!(result.platform, Some(Platform::N3ds));
    assert_eq!(result.serial_number.as_deref(), Some("CTR-N-ABCJ"));
    assert_eq!(result.maker_code.as_deref(), Some("Nintendo"));
    assert_eq!(result.extra.get("format").unwrap(), "CIA");
    assert_eq!(result.extra.get("origin").unwrap(), "Digital (eShop/CIA)");
}

#[test]
fn test_cia_title_id() {
    let cia = make_cia();
    let file_size = cia.len() as u64;
    let options = AnalysisOptions::default();
    let result = analyze_cia(&mut Cursor::new(cia), file_size, &options).unwrap();

    assert_eq!(result.extra.get("title_id").unwrap(), "0004000000ABCDEF");
    assert_eq!(result.extra.get("title_type").unwrap(), "Application");
}

#[test]
fn test_cia_title_version() {
    let cia = make_cia();
    let file_size = cia.len() as u64;
    let options = AnalysisOptions::default();
    let result = analyze_cia(&mut Cursor::new(cia), file_size, &options).unwrap();

    assert_eq!(result.version.as_deref(), Some("v1.1.0"));
}

#[test]
fn test_cia_regions() {
    let cia = make_cia();
    let file_size = cia.len() as u64;
    let options = AnalysisOptions::default();
    let result = analyze_cia(&mut Cursor::new(cia), file_size, &options).unwrap();

    assert_eq!(result.regions, vec![retro_junk_core::Region::Japan]);
}

#[test]
fn test_cia_content_count() {
    let cia = make_cia();
    let file_size = cia.len() as u64;
    let options = AnalysisOptions::default();
    let result = analyze_cia(&mut Cursor::new(cia), file_size, &options).unwrap();

    assert_eq!(result.extra.get("content_count").unwrap(), "1");
}
