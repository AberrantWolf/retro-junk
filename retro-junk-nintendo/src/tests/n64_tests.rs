use super::*;
use std::io::Cursor;

use crate::n64_byteorder::MAGIC_Z64;

/// Build a synthetic z64 (big-endian) N64 ROM of MIN_CRC_SIZE bytes
/// with CIC-6102 boot code and valid CRC.
fn make_n64_rom() -> Vec<u8> {
    make_n64_rom_with_cic(CicVariant::Cic6102)
}

/// Build a synthetic z64 ROM with the given CIC variant.
/// We use a fake boot code that hashes to the expected CRC32 for each variant.
/// Since we can't fabricate 4032 bytes that hash to a specific CRC32 easily,
/// we store the CIC variant in a side channel and test CIC detection separately.
fn make_n64_rom_with_cic(cic: CicVariant) -> Vec<u8> {
    let size = MIN_CRC_SIZE as usize;
    let mut rom = vec![0u8; size];

    // z64 magic
    rom[0..4].copy_from_slice(&MAGIC_Z64);

    // Clock rate
    rom[0x04..0x08].copy_from_slice(&0x0000000Fu32.to_be_bytes());

    // Boot address
    rom[0x08..0x0C].copy_from_slice(&0x80000400u32.to_be_bytes());

    // Libultra version
    rom[0x0C..0x10].copy_from_slice(&0x0000144Bu32.to_be_bytes());

    // Title at 0x20: "SUPER MARIO 64      " (20 bytes, space-padded)
    let title = b"SUPER MARIO 64      ";
    rom[0x20..0x34].copy_from_slice(title);

    // Category code: 'N' (Game Pak)
    rom[0x3B] = b'N';

    // Game ID: "SM"
    rom[0x3C] = b'S';
    rom[0x3D] = b'M';

    // Destination code: 'E' (USA)
    rom[0x3E] = b'E';

    // ROM version: 0
    rom[0x3F] = 0;

    // Fill boot code region with a pattern (won't match any real CIC)
    for i in (BOOT_CODE_START as usize)..(BOOT_CODE_END as usize) {
        rom[i] = ((i * 13 + 7) & 0xFF) as u8;
    }

    // Fill CRC data region with some non-zero pattern for meaningful CRC
    for i in (CRC_START as usize)..(CRC_END as usize) {
        rom[i] = ((i * 7 + 3) & 0xFF) as u8;
    }

    // Compute and store correct CRC for the given CIC variant
    recompute_crc(&mut rom, cic);

    rom
}

/// Recompute CRC for a z64-format ROM buffer and write to header.
fn recompute_crc(rom: &mut Vec<u8>, cic: CicVariant) {
    let seed = cic.seed();
    let data = &rom[CRC_START as usize..CRC_END as usize];

    let mut t1: u32 = seed;
    let mut t2: u32 = seed;
    let mut t3: u32 = seed;
    let mut t4: u32 = seed;
    let mut t5: u32 = seed;
    let mut t6: u32 = seed;

    for (i, chunk) in data.chunks_exact(4).enumerate() {
        let d = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);

        let k1 = t6.wrapping_add(d);
        if k1 < t6 {
            t4 = t4.wrapping_add(1);
        }
        t6 = k1;

        t3 ^= d;

        let r = d.rotate_left(d & 0x1F);
        t5 = t5.wrapping_add(r);

        if d < t2 {
            t2 ^= r;
        } else {
            t2 ^= t6 ^ d;
        }

        if cic == CicVariant::Cic6105 {
            let byte_offset = (i * 4) & 0xFF;
            let boot_offset = BOOT_CODE_START as usize + 0x0710 + byte_offset;
            let b = u32::from_be_bytes([
                rom[boot_offset],
                rom[boot_offset + 1],
                rom[boot_offset + 2],
                rom[boot_offset + 3],
            ]);
            t1 = t1.wrapping_add(b ^ d);
        } else {
            t1 = t1.wrapping_add(d ^ t5);
        }
    }

    let (crc1, crc2) = match cic {
        CicVariant::Cic6103 => ((t6 ^ t4).wrapping_add(t3), (t5 ^ t2).wrapping_add(t1)),
        CicVariant::Cic6106 => (
            (t6.wrapping_mul(t4)).wrapping_add(t3),
            (t5.wrapping_mul(t2)).wrapping_add(t1),
        ),
        _ => (t6 ^ t4 ^ t3, t5 ^ t2 ^ t1),
    };

    rom[0x10..0x14].copy_from_slice(&crc1.to_be_bytes());
    rom[0x14..0x18].copy_from_slice(&crc2.to_be_bytes());
}

/// Convert a z64 ROM to v64 format (swap every pair of bytes).
fn to_v64(z64: &[u8]) -> Vec<u8> {
    let mut v64 = z64.to_vec();
    for i in (0..v64.len() - 1).step_by(2) {
        v64.swap(i, i + 1);
    }
    v64
}

/// Convert a z64 ROM to n64 format (reverse every 4-byte group).
fn to_n64_format(z64: &[u8]) -> Vec<u8> {
    let mut n64 = z64.to_vec();
    for i in (0..n64.len().saturating_sub(3)).step_by(4) {
        n64.swap(i, i + 3);
        n64.swap(i + 1, i + 2);
    }
    n64
}

// -- CRC32-IEEE unit test --

#[test]
fn test_crc32_ieee() {
    // Known test vector: CRC32 of "123456789" = 0xCBF43926
    assert_eq!(crc32fast::hash(b"123456789"), 0xCBF43926);
}

// -- can_handle tests --

#[test]
fn test_can_handle_z64() {
    let rom = make_n64_rom();
    let analyzer = N64Analyzer::new();
    assert!(analyzer.can_handle(&mut Cursor::new(rom)));
}

#[test]
fn test_can_handle_v64() {
    let rom = to_v64(&make_n64_rom());
    let analyzer = N64Analyzer::new();
    assert!(analyzer.can_handle(&mut Cursor::new(rom)));
}

#[test]
fn test_can_handle_n64() {
    let rom = to_n64_format(&make_n64_rom());
    let analyzer = N64Analyzer::new();
    assert!(analyzer.can_handle(&mut Cursor::new(rom)));
}

#[test]
fn test_can_handle_too_small() {
    let data = vec![0x80, 0x37, 0x12]; // Only 3 bytes
    let analyzer = N64Analyzer::new();
    assert!(!analyzer.can_handle(&mut Cursor::new(data)));
}

#[test]
fn test_can_handle_bad_magic() {
    let mut rom = make_n64_rom();
    rom[0] = 0xFF;
    let analyzer = N64Analyzer::new();
    assert!(!analyzer.can_handle(&mut Cursor::new(rom)));
}

// -- Basic analysis tests --

#[test]
fn test_basic_analysis() {
    let rom = make_n64_rom();
    let analyzer = N64Analyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(result.platform, Some(Platform::N64));
    assert_eq!(result.internal_name.as_deref(), Some("SUPER MARIO 64"));
    assert_eq!(result.serial_number.as_deref(), Some("NUS-NSME-USA"));
    assert_eq!(result.regions, vec![Region::Usa]);
    assert_eq!(result.version.as_deref(), Some("v1.0"));
    assert_eq!(result.file_size, Some(MIN_CRC_SIZE));
    assert_eq!(result.extra.get("format").unwrap(), "z64 (big-endian)");
}

// -- Region mapping tests --

#[test]
fn test_region_japan() {
    let mut rom = make_n64_rom();
    rom[0x3E] = b'J';
    recompute_crc(&mut rom, CicVariant::Unknown);
    let analyzer = N64Analyzer::new();
    let result = analyzer
        .analyze(&mut Cursor::new(rom), &AnalysisOptions::default())
        .unwrap();
    assert_eq!(result.regions, vec![Region::Japan]);
}

#[test]
fn test_region_europe_p() {
    let mut rom = make_n64_rom();
    rom[0x3E] = b'P';
    recompute_crc(&mut rom, CicVariant::Unknown);
    let analyzer = N64Analyzer::new();
    let result = analyzer
        .analyze(&mut Cursor::new(rom), &AnalysisOptions::default())
        .unwrap();
    assert_eq!(result.regions, vec![Region::Europe]);
}

#[test]
fn test_region_europe_d() {
    let mut rom = make_n64_rom();
    rom[0x3E] = b'D';
    recompute_crc(&mut rom, CicVariant::Unknown);
    let analyzer = N64Analyzer::new();
    let result = analyzer
        .analyze(&mut Cursor::new(rom), &AnalysisOptions::default())
        .unwrap();
    assert_eq!(result.regions, vec![Region::Europe]);
}

#[test]
fn test_region_australia() {
    let mut rom = make_n64_rom();
    rom[0x3E] = b'U';
    recompute_crc(&mut rom, CicVariant::Unknown);
    let analyzer = N64Analyzer::new();
    let result = analyzer
        .analyze(&mut Cursor::new(rom), &AnalysisOptions::default())
        .unwrap();
    assert_eq!(result.regions, vec![Region::Australia]);
}

#[test]
fn test_region_world() {
    let mut rom = make_n64_rom();
    rom[0x3E] = b'A';
    recompute_crc(&mut rom, CicVariant::Unknown);
    let analyzer = N64Analyzer::new();
    let result = analyzer
        .analyze(&mut Cursor::new(rom), &AnalysisOptions::default())
        .unwrap();
    assert_eq!(result.regions, vec![Region::World]);
}

#[test]
fn test_region_brazil() {
    let mut rom = make_n64_rom();
    rom[0x3E] = b'B';
    recompute_crc(&mut rom, CicVariant::Unknown);
    let analyzer = N64Analyzer::new();
    let result = analyzer
        .analyze(&mut Cursor::new(rom), &AnalysisOptions::default())
        .unwrap();
    assert_eq!(result.regions, vec![Region::Brazil]);
}

// -- CRC tests --

#[test]
fn test_crc_ok() {
    let rom = make_n64_rom();
    let analyzer = N64Analyzer::new();
    let result = analyzer
        .analyze(&mut Cursor::new(rom), &AnalysisOptions::default())
        .unwrap();
    assert_eq!(result.extra.get("checksum_status:N64 CRC").unwrap(), "OK");
}

#[test]
fn test_crc1_mismatch() {
    let mut rom = make_n64_rom();
    // Corrupt CRC1 in header
    rom[0x10] = rom[0x10].wrapping_add(1);
    let analyzer = N64Analyzer::new();
    let result = analyzer
        .analyze(&mut Cursor::new(rom), &AnalysisOptions::default())
        .unwrap();
    let status = result.extra.get("checksum_status:N64 CRC").unwrap();
    assert!(
        status.starts_with("CRC1 MISMATCH"),
        "Expected CRC1 MISMATCH, got: {}",
        status
    );
    assert!(
        !status.contains("CRC2 MISMATCH"),
        "Should not have CRC2 mismatch: {}",
        status
    );
}

#[test]
fn test_crc2_mismatch() {
    let mut rom = make_n64_rom();
    // Corrupt CRC2 in header
    rom[0x14] = rom[0x14].wrapping_add(1);
    let analyzer = N64Analyzer::new();
    let result = analyzer
        .analyze(&mut Cursor::new(rom), &AnalysisOptions::default())
        .unwrap();
    let status = result.extra.get("checksum_status:N64 CRC").unwrap();
    assert!(
        status.starts_with("CRC2 MISMATCH"),
        "Expected CRC2 MISMATCH, got: {}",
        status
    );
    assert!(
        !status.contains("CRC1 MISMATCH"),
        "Should not have CRC1 mismatch: {}",
        status
    );
}

#[test]
fn test_both_crc_mismatch() {
    let mut rom = make_n64_rom();
    rom[0x10] = rom[0x10].wrapping_add(1);
    rom[0x14] = rom[0x14].wrapping_add(1);
    let analyzer = N64Analyzer::new();
    let result = analyzer
        .analyze(&mut Cursor::new(rom), &AnalysisOptions::default())
        .unwrap();
    let status = result.extra.get("checksum_status:N64 CRC").unwrap();
    assert!(status.contains("CRC1 MISMATCH"), "Missing CRC1: {}", status);
    assert!(status.contains("CRC2 MISMATCH"), "Missing CRC2: {}", status);
}

#[test]
fn test_quick_mode_still_computes_crc() {
    let rom = make_n64_rom();
    let analyzer = N64Analyzer::new();
    let options = AnalysisOptions {
        quick: true,
        ..Default::default()
    };
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    let status = result.extra.get("checksum_status:N64 CRC").unwrap();
    assert!(
        !status.starts_with("SKIPPED"),
        "Expected CRC to be computed even in quick mode, got: {}",
        status
    );
}

#[test]
fn test_file_too_small_for_crc() {
    // Need at least BOOT_CODE_END (0x1000) for analysis now
    let mut rom = vec![0u8; BOOT_CODE_END as usize];
    rom[0..4].copy_from_slice(&MAGIC_Z64);
    rom[0x20..0x34].copy_from_slice(b"TINY ROM            ");
    rom[0x3B] = b'N';
    rom[0x3C] = b'T';
    rom[0x3D] = b'Y';
    rom[0x3E] = b'E';

    let analyzer = N64Analyzer::new();
    let result = analyzer
        .analyze(&mut Cursor::new(rom), &AnalysisOptions::default())
        .unwrap();
    let status = result.extra.get("checksum_status:N64 CRC").unwrap();
    assert!(
        status.starts_with("SKIPPED"),
        "Expected SKIPPED, got: {}",
        status
    );
}

// -- Format variant tests --

#[test]
fn test_v64_analysis() {
    let z64 = make_n64_rom();
    let v64 = to_v64(&z64);
    let analyzer = N64Analyzer::new();
    let result = analyzer
        .analyze(&mut Cursor::new(v64), &AnalysisOptions::default())
        .unwrap();

    assert_eq!(result.internal_name.as_deref(), Some("SUPER MARIO 64"));
    assert_eq!(result.serial_number.as_deref(), Some("NUS-NSME-USA"));
    assert_eq!(result.extra.get("format").unwrap(), "v64 (byte-swapped)");
    assert_eq!(result.extra.get("checksum_status:N64 CRC").unwrap(), "OK");
}

#[test]
fn test_n64_format_analysis() {
    let z64 = make_n64_rom();
    let n64 = to_n64_format(&z64);
    let analyzer = N64Analyzer::new();
    let result = analyzer
        .analyze(&mut Cursor::new(n64), &AnalysisOptions::default())
        .unwrap();

    assert_eq!(result.internal_name.as_deref(), Some("SUPER MARIO 64"));
    assert_eq!(result.serial_number.as_deref(), Some("NUS-NSME-USA"));
    assert_eq!(result.extra.get("format").unwrap(), "n64 (little-endian)");
    assert_eq!(result.extra.get("checksum_status:N64 CRC").unwrap(), "OK");
}

// -- CIC-variant CRC tests --
// These test that different seeds / final formulas produce different CRCs
// and that our compute matches our recompute for each variant.

#[test]
fn test_crc_ok_cic6103() {
    let rom = make_n64_rom_with_cic(CicVariant::Cic6103);
    // The ROM has Unknown CIC (fake boot code), so the analyzer will use
    // the Unknown/6102 seed. Instead, test the compute function directly.
    let mut cursor = Cursor::new(&rom);
    let (crc1, crc2) = compute_n64_crc(&mut cursor, RomFormat::Z64, CicVariant::Cic6103).unwrap();
    let header_crc1 = u32::from_be_bytes([rom[0x10], rom[0x11], rom[0x12], rom[0x13]]);
    let header_crc2 = u32::from_be_bytes([rom[0x14], rom[0x15], rom[0x16], rom[0x17]]);
    assert_eq!(crc1, header_crc1, "CRC1 mismatch for CIC-6103");
    assert_eq!(crc2, header_crc2, "CRC2 mismatch for CIC-6103");
}

#[test]
fn test_crc_ok_cic6105() {
    let rom = make_n64_rom_with_cic(CicVariant::Cic6105);
    let mut cursor = Cursor::new(&rom);
    let (crc1, crc2) = compute_n64_crc(&mut cursor, RomFormat::Z64, CicVariant::Cic6105).unwrap();
    let header_crc1 = u32::from_be_bytes([rom[0x10], rom[0x11], rom[0x12], rom[0x13]]);
    let header_crc2 = u32::from_be_bytes([rom[0x14], rom[0x15], rom[0x16], rom[0x17]]);
    assert_eq!(crc1, header_crc1, "CRC1 mismatch for CIC-6105");
    assert_eq!(crc2, header_crc2, "CRC2 mismatch for CIC-6105");
}

#[test]
fn test_crc_ok_cic6106() {
    let rom = make_n64_rom_with_cic(CicVariant::Cic6106);
    let mut cursor = Cursor::new(&rom);
    let (crc1, crc2) = compute_n64_crc(&mut cursor, RomFormat::Z64, CicVariant::Cic6106).unwrap();
    let header_crc1 = u32::from_be_bytes([rom[0x10], rom[0x11], rom[0x12], rom[0x13]]);
    let header_crc2 = u32::from_be_bytes([rom[0x14], rom[0x15], rom[0x16], rom[0x17]]);
    assert_eq!(crc1, header_crc1, "CRC1 mismatch for CIC-6106");
    assert_eq!(crc2, header_crc2, "CRC2 mismatch for CIC-6106");
}

#[test]
fn test_different_cic_seeds_produce_different_crcs() {
    let rom_6102 = make_n64_rom_with_cic(CicVariant::Cic6102);
    let rom_6103 = make_n64_rom_with_cic(CicVariant::Cic6103);
    let crc1_6102 = u32::from_be_bytes([
        rom_6102[0x10],
        rom_6102[0x11],
        rom_6102[0x12],
        rom_6102[0x13],
    ]);
    let crc1_6103 = u32::from_be_bytes([
        rom_6103[0x10],
        rom_6103[0x11],
        rom_6103[0x12],
        rom_6103[0x13],
    ]);
    assert_ne!(
        crc1_6102, crc1_6103,
        "Different CIC seeds should produce different CRCs"
    );
}

// -- Title trimming --

#[test]
fn test_title_trimming_spaces() {
    let mut rom = make_n64_rom();
    rom[0x20..0x34].copy_from_slice(b"HI                  ");
    recompute_crc(&mut rom, CicVariant::Unknown);
    let analyzer = N64Analyzer::new();
    let result = analyzer
        .analyze(&mut Cursor::new(rom), &AnalysisOptions::default())
        .unwrap();
    assert_eq!(result.internal_name.as_deref(), Some("HI"));
}

#[test]
fn test_title_trimming_nulls() {
    let mut rom = make_n64_rom();
    rom[0x20..0x34].copy_from_slice(b"HI\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0");
    recompute_crc(&mut rom, CicVariant::Unknown);
    let analyzer = N64Analyzer::new();
    let result = analyzer
        .analyze(&mut Cursor::new(rom), &AnalysisOptions::default())
        .unwrap();
    assert_eq!(result.internal_name.as_deref(), Some("HI"));
}

// -- Extra fields --

#[test]
fn test_version_field() {
    let mut rom = make_n64_rom();
    rom[0x3F] = 2;
    recompute_crc(&mut rom, CicVariant::Unknown);
    let analyzer = N64Analyzer::new();
    let result = analyzer
        .analyze(&mut Cursor::new(rom), &AnalysisOptions::default())
        .unwrap();
    assert_eq!(result.version.as_deref(), Some("v1.2"));
}

#[test]
fn test_boot_address_in_extra() {
    let rom = make_n64_rom();
    let analyzer = N64Analyzer::new();
    let result = analyzer
        .analyze(&mut Cursor::new(rom), &AnalysisOptions::default())
        .unwrap();
    assert_eq!(result.extra.get("boot_address").unwrap(), "0x80000400");
}

#[test]
fn test_clock_rate_in_extra() {
    let rom = make_n64_rom();
    let analyzer = N64Analyzer::new();
    let result = analyzer
        .analyze(&mut Cursor::new(rom), &AnalysisOptions::default())
        .unwrap();
    assert_eq!(result.extra.get("clock_rate").unwrap(), "0x0000000F");
}

#[test]
fn test_category_code_in_extra() {
    let rom = make_n64_rom();
    let analyzer = N64Analyzer::new();
    let result = analyzer
        .analyze(&mut Cursor::new(rom), &AnalysisOptions::default())
        .unwrap();
    assert_eq!(result.extra.get("category_code").unwrap(), "N");
}

#[test]
fn test_cic_in_extra() {
    let rom = make_n64_rom();
    let analyzer = N64Analyzer::new();
    let result = analyzer
        .analyze(&mut Cursor::new(rom), &AnalysisOptions::default())
        .unwrap();
    // Our fake boot code won't match any known CIC
    assert_eq!(result.extra.get("cic").unwrap(), "unknown");
}

// -- Error tests --

#[test]
fn test_invalid_format_error_message() {
    let mut data = vec![0u8; BOOT_CODE_END as usize];
    data[0] = 0xDE;
    data[1] = 0xAD;
    data[2] = 0xBE;
    data[3] = 0xEF;
    let analyzer = N64Analyzer::new();
    let err = analyzer
        .analyze(&mut Cursor::new(data), &AnalysisOptions::default())
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("DE, AD, BE, EF"),
        "Error should include actual bytes: {}",
        msg
    );
    assert!(
        msg.contains("z64=[80,37,12,40]"),
        "Error should include z64 magic: {}",
        msg
    );
    assert!(
        msg.contains("v64=[37,80,40,12]"),
        "Error should include v64 magic: {}",
        msg
    );
    assert!(
        msg.contains("n64=[40,12,37,80]"),
        "Error should include n64 magic: {}",
        msg
    );
}

#[test]
fn test_too_small_file() {
    let data = vec![0u8; 0x20]; // Not enough for header
    let analyzer = N64Analyzer::new();
    let result = analyzer.analyze(&mut Cursor::new(data), &AnalysisOptions::default());
    assert!(result.is_err());
}

// -- extract_scraper_serial (delegates to extract_dat_game_code) --

#[test]
fn test_extract_scraper_serial_delegates_to_dat() {
    let analyzer = N64Analyzer::new();
    assert_eq!(
        analyzer.extract_scraper_serial("NUS-NSME-USA"),
        Some("NSME".to_string()),
    );
}

// -- normalize_to_big_endian unit tests --

#[test]
fn test_normalize_z64_noop() {
    let original = vec![0x80, 0x37, 0x12, 0x40];
    let mut data = original.clone();
    normalize_to_big_endian(&mut data, RomFormat::Z64);
    assert_eq!(data, original);
}

#[test]
fn test_normalize_v64() {
    let mut data = vec![0x37, 0x80, 0x40, 0x12];
    normalize_to_big_endian(&mut data, RomFormat::V64);
    assert_eq!(data, vec![0x80, 0x37, 0x12, 0x40]);
}

#[test]
fn test_normalize_n64() {
    let mut data = vec![0x40, 0x12, 0x37, 0x80];
    normalize_to_big_endian(&mut data, RomFormat::N64);
    assert_eq!(data, vec![0x80, 0x37, 0x12, 0x40]);
}
