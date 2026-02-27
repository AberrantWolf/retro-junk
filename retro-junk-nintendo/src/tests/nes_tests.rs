use super::*;
use std::io::Cursor;

/// Build a minimal valid iNES 1.0 header.
fn make_ines_header(prg_banks: u8, chr_banks: u8, flags6: u8, flags7: u8) -> Vec<u8> {
    let mut h = vec![0u8; 16];
    h[0..4].copy_from_slice(&INES_MAGIC);
    h[4] = prg_banks;
    h[5] = chr_banks;
    h[6] = flags6;
    h[7] = flags7;
    h
}

#[test]
fn test_detect_ines() {
    let data = make_ines_header(2, 1, 0x00, 0x00);
    let mut cursor = Cursor::new(data);
    assert_eq!(detect_format(&mut cursor).unwrap(), NesFormat::INes);
}

#[test]
fn test_detect_nes2() {
    let data = make_ines_header(2, 1, 0x00, 0x08); // bits 2-3 = 0b10 = NES 2.0
    let mut cursor = Cursor::new(data);
    assert_eq!(detect_format(&mut cursor).unwrap(), NesFormat::Nes2);
}

#[test]
fn test_detect_unif() {
    let mut data = vec![0u8; 32];
    data[0..4].copy_from_slice(&UNIF_MAGIC);
    data[4] = 7; // revision
    let mut cursor = Cursor::new(data);
    assert_eq!(detect_format(&mut cursor).unwrap(), NesFormat::Unif);
}

#[test]
fn test_detect_fds_headered() {
    let mut data = vec![0u8; 16];
    data[0..4].copy_from_slice(&FDS_HEADER_MAGIC);
    data[4] = 2; // 2 sides
    let mut cursor = Cursor::new(data);
    assert_eq!(detect_format(&mut cursor).unwrap(), NesFormat::FdsHeadered);
}

#[test]
fn test_detect_invalid() {
    let data = vec![0xDE, 0xAD, 0xBE, 0xEF];
    let mut cursor = Cursor::new(data);
    assert!(detect_format(&mut cursor).is_err());
}

#[test]
fn test_parse_ines_basic() {
    // Mapper 0, 2x16KB PRG, 1x8KB CHR, horizontal mirroring
    let data = make_ines_header(2, 1, 0x00, 0x00);
    let mut cursor = Cursor::new(data);
    let analyzer = NesAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut cursor, &options).unwrap();
    assert_eq!(result.extra.get("mapper").unwrap(), "0");
    assert_eq!(result.extra.get("mapper_name").unwrap(), "NROM");
    assert_eq!(result.extra.get("prg_rom_size").unwrap(), "32 KB");
    assert_eq!(result.extra.get("chr_rom_size").unwrap(), "8 KB");
    assert_eq!(result.extra.get("mirroring").unwrap(), "Horizontal");
    assert_eq!(result.extra.get("format").unwrap(), "iNES");
    // Expected: 16 header + 32KB PRG + 8KB CHR = 40976
    assert_eq!(result.expected_size, Some(16 + 32768 + 8192));
    // File only has the 16-byte header
    assert_eq!(result.file_size, Some(16));
}

#[test]
fn test_parse_ines_mapper4_vertical_battery() {
    // Mapper 4 (MMC3): flags6 = 0b0100_0011 (mapper lo=4, battery=1, vertical=1)
    //                   flags7 = 0b0000_0000 (mapper hi=0)
    let data = make_ines_header(16, 16, 0x43, 0x00);
    let mut cursor = Cursor::new(data);
    let analyzer = NesAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut cursor, &options).unwrap();
    assert_eq!(result.extra.get("mapper").unwrap(), "4");
    assert_eq!(result.extra.get("mapper_name").unwrap(), "MMC3 (TxROM)");
    assert_eq!(result.extra.get("mirroring").unwrap(), "Vertical");
    assert_eq!(result.extra.get("battery").unwrap(), "Yes");
    assert_eq!(result.extra.get("prg_rom_size").unwrap(), "256 KB");
    assert_eq!(result.extra.get("chr_rom_size").unwrap(), "128 KB");
    // Expected: 16 + 256KB + 128KB = 393232
    assert_eq!(result.expected_size, Some(16 + 262144 + 131072));
}

#[test]
fn test_ines_size_with_trainer() {
    // Trainer bit set: flags6 bit 2
    let data = make_ines_header(1, 1, 0x04, 0x00);
    let mut cursor = Cursor::new(data);
    let analyzer = NesAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut cursor, &options).unwrap();
    assert_eq!(result.extra.get("trainer").unwrap(), "Yes");
    // Expected: 16 header + 512 trainer + 16KB PRG + 8KB CHR
    assert_eq!(result.expected_size, Some(16 + 512 + 16384 + 8192));
}

#[test]
fn test_ines_exact_size_match() {
    // Build a "complete" ROM: header + PRG data + CHR data
    let mut data = make_ines_header(1, 1, 0x00, 0x00);
    // 1 * 16KB PRG = 16384 bytes
    data.extend(vec![0u8; 16384]);
    // 1 * 8KB CHR = 8192 bytes
    data.extend(vec![0u8; 8192]);

    let mut cursor = Cursor::new(data);
    let analyzer = NesAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut cursor, &options).unwrap();
    assert_eq!(result.file_size, result.expected_size);
}

#[test]
fn test_parse_ines_chr_ram() {
    // CHR ROM = 0 means CHR RAM
    let data = make_ines_header(1, 0, 0x00, 0x00);
    let mut cursor = Cursor::new(data);
    let analyzer = NesAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut cursor, &options).unwrap();
    assert_eq!(result.extra.get("chr_rom_size").unwrap(), "CHR RAM");
}

#[test]
fn test_parse_nes2_header() {
    let mut data = make_ines_header(2, 1, 0x10, 0x08); // NES 2.0, mapper 1
    // Byte 8: mapper MSB 0, submapper 0
    data[8] = 0x00;
    // Byte 9: PRG/CHR MSB both 0
    data[9] = 0x00;
    // Byte 10: PRG RAM = shift 7 (8KB), no NVRAM
    data[10] = 0x07;
    // Byte 12: NTSC
    data[12] = 0x00;

    let mut cursor = Cursor::new(data);
    let analyzer = NesAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut cursor, &options).unwrap();
    assert_eq!(result.extra.get("format").unwrap(), "NES 2.0");
    assert_eq!(result.extra.get("mapper").unwrap(), "1");
    assert_eq!(result.extra.get("prg_ram_size").unwrap(), "8 KB");
}

#[test]
fn test_parse_unif() {
    let mut data = vec![0u8; 32];
    data[0..4].copy_from_slice(&UNIF_MAGIC);
    data[4] = 7;
    let mut cursor = Cursor::new(data);
    let analyzer = NesAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut cursor, &options).unwrap();
    assert_eq!(result.extra.get("format").unwrap(), "UNIF");
    assert_eq!(result.extra.get("unif_revision").unwrap(), "7");
}

#[test]
fn test_can_handle() {
    let analyzer = NesAnalyzer::new();

    let ines = make_ines_header(1, 1, 0, 0);
    assert!(analyzer.can_handle(&mut Cursor::new(ines)));

    let garbage = vec![0xFFu8; 16];
    assert!(!analyzer.can_handle(&mut Cursor::new(garbage)));
}
