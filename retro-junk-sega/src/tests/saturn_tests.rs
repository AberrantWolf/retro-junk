use super::*;
use retro_junk_core::{AnalysisOptions, Platform, Region, RomAnalyzer};
use retro_junk_disc::test_helpers::make_pvd_sector;
use std::io::Cursor;

/// Build a minimal 2048-byte Saturn IP.BIN sector.
fn make_saturn_ip_bin(serial: &str, region: &str, game_name: &str) -> [u8; 2048] {
    let mut data = [0u8; 2048];

    // Hardware ID: "SEGA SEGASATURN " (16 bytes)
    data[0x000..0x010].copy_from_slice(SATURN_MAGIC);

    // Maker ID (16 bytes)
    let maker = b"SEGA ENTERPRISES";
    data[0x010..0x010 + maker.len()].copy_from_slice(maker);
    for i in maker.len()..16 {
        data[0x010 + i] = b' ';
    }

    // Product Number / serial (10 bytes)
    let serial_bytes = serial.as_bytes();
    let len = serial_bytes.len().min(10);
    data[0x020..0x020 + len].copy_from_slice(&serial_bytes[..len]);
    for i in len..10 {
        data[0x020 + i] = b' ';
    }

    // Version (6 bytes)
    data[0x02A..0x030].copy_from_slice(b"V1.000");

    // Release Date (8 bytes)
    data[0x030..0x038].copy_from_slice(b"19961122");

    // Device Info (8 bytes)
    data[0x038..0x040].copy_from_slice(b"J       ");

    // Compatible Area / region (10 bytes)
    let region_bytes = region.as_bytes();
    let rlen = region_bytes.len().min(10);
    data[0x040..0x040 + rlen].copy_from_slice(&region_bytes[..rlen]);
    for i in rlen..10 {
        data[0x040 + i] = b' ';
    }

    // Game Name (112 bytes)
    let name_bytes = game_name.as_bytes();
    let nlen = name_bytes.len().min(112);
    data[0x060..0x060 + nlen].copy_from_slice(&name_bytes[..nlen]);
    for i in nlen..112 {
        data[0x060 + i] = b' ';
    }

    data
}

/// Build a Saturn ISO with IP.BIN at sector 0 and PVD at sector 16.
fn make_saturn_iso(serial: &str, region: &str, game_name: &str) -> Vec<u8> {
    let ip_bin = make_saturn_ip_bin(serial, region, game_name);
    let pvd = make_pvd_sector("SEGA SEGASATURN");

    let mut data = Vec::new();
    // Sector 0: IP.BIN
    data.extend_from_slice(&ip_bin);
    // Sectors 1-15: empty
    data.extend_from_slice(&vec![0u8; 15 * 2048]);
    // Sector 16: PVD
    data.extend_from_slice(&pvd);
    data
}

// -- IP.BIN parsing tests --

#[test]
fn test_parse_ip_bin_valid() {
    let data = make_saturn_ip_bin("MK-81009", "JUE", "NiGHTS into Dreams...");
    let header = parse_ip_bin(&data).unwrap();
    assert_eq!(header.serial, "MK-81009");
    assert_eq!(header.region_codes, "JUE");
    assert_eq!(header.game_name, "NiGHTS into Dreams...");
    assert_eq!(header.maker_id, "SEGA ENTERPRISES");
    assert_eq!(header.version, "V1.000");
    assert_eq!(header.release_date, "19961122");
}

#[test]
fn test_identification_promotes_header_version_for_matching() {
    let header = parse_ip_bin(&make_saturn_ip_bin("MK-81009", "U", "NiGHTS")).unwrap();
    let identification = SaturnAnalyzer::build_identification(&header);
    assert_eq!(identification.version, "V1.000");
}

#[test]
fn test_parse_ip_bin_wrong_magic() {
    let mut data = [0u8; 2048];
    data[0..16].copy_from_slice(b"NOT SEGA SATURN ");
    assert!(parse_ip_bin(&data).is_none());
}

#[test]
fn test_parse_ip_bin_too_short() {
    let data = [0u8; 100]; // Too short for header
    assert!(parse_ip_bin(&data).is_none());
}

// -- Region code tests --

#[test]
fn test_saturn_region_codes_japan() {
    let regions = saturn_region_codes("J");
    assert_eq!(regions, vec![Region::Japan]);
}

#[test]
fn test_saturn_region_codes_usa() {
    let regions = saturn_region_codes("U");
    assert_eq!(regions, vec![Region::Usa]);
}

#[test]
fn test_saturn_region_codes_europe() {
    let regions = saturn_region_codes("E");
    assert_eq!(regions, vec![Region::Europe]);
}

#[test]
fn test_saturn_region_codes_multi() {
    let regions = saturn_region_codes("JUE");
    assert_eq!(regions, vec![Region::Japan, Region::Usa, Region::Europe]);
}

#[test]
fn test_saturn_region_codes_korea() {
    let regions = saturn_region_codes("K");
    assert_eq!(regions, vec![Region::Korea]);
}

#[test]
fn test_saturn_region_codes_brazil() {
    let regions = saturn_region_codes("B");
    assert_eq!(regions, vec![Region::Brazil]);
}

#[test]
fn test_saturn_region_codes_latin_america() {
    let regions = saturn_region_codes("L");
    assert_eq!(regions, vec![Region::LatinAmerica]);
}

#[test]
fn test_saturn_region_codes_asia() {
    let regions = saturn_region_codes("T");
    assert_eq!(regions, vec![Region::Asia]);
}

#[test]
fn test_saturn_region_codes_empty() {
    let regions = saturn_region_codes("");
    assert!(regions.is_empty());
}

// -- can_handle tests --

#[test]
fn test_can_handle_saturn_iso() {
    let data = make_saturn_iso("MK-81009", "JUE", "NiGHTS");
    let mut cursor = Cursor::new(data);
    let analyzer = SaturnAnalyzer;
    assert!(analyzer.can_handle(&mut cursor));
}

#[test]
fn test_can_handle_non_saturn_iso() {
    // An ISO with PLAYSTATION system ID should not match Saturn
    let data = retro_junk_disc::test_helpers::make_iso("PLAYSTATION");
    let mut cursor = Cursor::new(data);
    let analyzer = SaturnAnalyzer;
    assert!(!analyzer.can_handle(&mut cursor));
}

// -- analyze tests --

#[test]
fn test_analyze_saturn_iso() {
    let data = make_saturn_iso("MK-81009", "JUE", "NiGHTS into Dreams...");
    let mut cursor = Cursor::new(data);
    let analyzer = SaturnAnalyzer;
    let options = AnalysisOptions::default();
    let id = analyzer.analyze(&mut cursor, &options).unwrap();

    assert_eq!(id.serial_number, "MK-81009");
    assert_eq!(id.internal_name, "NiGHTS into Dreams...");
    assert_eq!(id.regions, vec![Region::Japan, Region::Usa, Region::Europe]);
    assert_eq!(
        id.extra.get("format").map(std::string::String::as_str),
        Some("ISO 9660")
    );
    assert_eq!(
        id.extra.get("maker_id").map(std::string::String::as_str),
        Some("SEGA ENTERPRISES")
    );
    assert_eq!(
        id.extra.get("version").map(std::string::String::as_str),
        Some("V1.000")
    );
    assert_eq!(
        id.extra
            .get("release_date")
            .map(std::string::String::as_str),
        Some("19961122")
    );
}

#[test]
fn test_analyze_saturn_iso_japan_only() {
    let data = make_saturn_iso("GS-9001", "J", "Virtua Fighter Remix");
    let mut cursor = Cursor::new(data);
    let analyzer = SaturnAnalyzer;
    let options = AnalysisOptions::default();
    let id = analyzer.analyze(&mut cursor, &options).unwrap();

    assert_eq!(id.serial_number, "GS-9001");
    assert_eq!(id.regions, vec![Region::Japan]);
}

#[test]
fn test_analyze_non_saturn_iso_rejected() {
    // PLAYSTATION ISO should fail
    let data = retro_junk_disc::test_helpers::make_iso("PLAYSTATION");
    let mut cursor = Cursor::new(data);
    let analyzer = SaturnAnalyzer;
    let options = AnalysisOptions::default();
    assert!(analyzer.analyze(&mut cursor, &options).is_err());
}

// -- Metadata tests --

#[test]
fn test_platform() {
    let analyzer = SaturnAnalyzer;
    assert_eq!(analyzer.platform(), Platform::Saturn);
}

#[test]
fn test_file_extensions() {
    let analyzer = SaturnAnalyzer;
    assert!(analyzer.file_extensions().contains(&"bin"));
    assert!(analyzer.file_extensions().contains(&"cue"));
    assert!(analyzer.file_extensions().contains(&"iso"));
    assert!(analyzer.file_extensions().contains(&"chd"));
}

#[test]
fn test_dat_names() {
    let analyzer = SaturnAnalyzer;
    assert_eq!(analyzer.dat_names(), &["Sega - Saturn"]);
}

#[test]
fn test_dat_source() {
    let analyzer = SaturnAnalyzer;
    assert_eq!(analyzer.dat_source(), retro_junk_core::DatSource::Redump);
}

#[test]
fn test_expects_serial() {
    let analyzer = SaturnAnalyzer;
    assert!(analyzer.expects_serial());
}

#[test]
fn test_extract_dat_game_code() {
    let analyzer = SaturnAnalyzer;
    assert_eq!(
        analyzer.extract_dat_game_code("MK-81009"),
        Some("MK-81009".to_string())
    );
}

#[test]
fn test_serial_extraction_various() {
    // Various Saturn serial formats
    let data1 = make_saturn_iso("MK-81009", "U", "Test Game");
    let mut cursor = Cursor::new(data1);
    let analyzer = SaturnAnalyzer;
    let options = AnalysisOptions::default();
    let id = analyzer.analyze(&mut cursor, &options).unwrap();
    assert_eq!(id.serial_number, "MK-81009");

    let data2 = make_saturn_iso("T-1001G", "J", "Third Party");
    let mut cursor = Cursor::new(data2);
    let id = analyzer.analyze(&mut cursor, &options).unwrap();
    assert_eq!(id.serial_number, "T-1001G");

    let data3 = make_saturn_iso("GS-9001", "J", "Sega Game");
    let mut cursor = Cursor::new(data3);
    let id = analyzer.analyze(&mut cursor, &options).unwrap();
    assert_eq!(id.serial_number, "GS-9001");
}

// -- CHD extension table --

#[test]
fn test_chd_media_for_extension_cue() {
    let analyzer = SaturnAnalyzer;
    assert_eq!(
        analyzer.chd_media_for_extension("cue"),
        Some(retro_junk_core::ChdMedia::Cd)
    );
    assert_eq!(analyzer.chd_media_for_extension("iso"), None);
}

#[test]
fn test_chd_extensions_declares_a_source_role() {
    // console_supports_chd-equivalent logic: Saturn is disc-based, so at
    // least one declared extension must be a CHD Source.
    let analyzer = SaturnAnalyzer;
    assert!(
        analyzer
            .chd_extensions()
            .iter()
            .any(|(_, role)| matches!(role, retro_junk_core::ChdExtensionRole::Source(_)))
    );
}
