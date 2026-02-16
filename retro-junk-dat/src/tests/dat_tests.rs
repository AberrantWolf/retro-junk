use super::*;

// -- XML tests --

const SAMPLE_XML_DAT: &str = r#"<?xml version="1.0"?>
<!DOCTYPE datafile SYSTEM "http://www.logiqx.com/Dats/datafile.dtd">
<datafile>
    <header>
        <name>Nintendo - Super Nintendo Entertainment System</name>
        <description>Nintendo - Super Nintendo Entertainment System (20240101-000000)</description>
        <version>20240101-000000</version>
    </header>
    <game name="Super Mario World (USA)">
        <rom name="Super Mario World (USA).sfc" size="524288" crc="b19ed489" sha1="6b47bb75d16514b6a476aa0c73a683a2a4c18765"/>
    </game>
    <game name="The Legend of Zelda - A Link to the Past (USA)">
        <rom name="The Legend of Zelda - A Link to the Past (USA).sfc" size="1048576" crc="777aac2f" sha1="59b4b1730a3e2ae4b30efc9c1e0d31986b6c4b44"/>
    </game>
</datafile>"#;

#[test]
fn test_parse_xml_dat() {
    let dat = parse_dat(SAMPLE_XML_DAT.as_bytes()).unwrap();
    assert_eq!(dat.name, "Nintendo - Super Nintendo Entertainment System");
    assert_eq!(dat.version, "20240101-000000");
    assert_eq!(dat.games.len(), 2);

    let smw = &dat.games[0];
    assert_eq!(smw.name, "Super Mario World (USA)");
    assert_eq!(smw.roms[0].name, "Super Mario World (USA).sfc");
    assert_eq!(smw.roms[0].size, 524288);
    assert_eq!(smw.roms[0].crc, "b19ed489");
    assert_eq!(
        smw.roms[0].sha1.as_deref(),
        Some("6b47bb75d16514b6a476aa0c73a683a2a4c18765")
    );
}

#[test]
fn test_parse_empty_xml() {
    let xml = r#"<?xml version="1.0"?><datafile></datafile>"#;
    let result = parse_dat(xml.as_bytes());
    assert!(result.is_err());
}

#[test]
fn test_parse_xml_with_serial() {
    let xml = r#"<?xml version="1.0"?>
<datafile>
    <header><name>Test</name><version>1</version></header>
    <game name="Test Game">
        <rom name="Test Game.bin" size="1024" crc="deadbeef" serial="SLUS-00001"/>
    </game>
</datafile>"#;
    let dat = parse_dat(xml.as_bytes()).unwrap();
    assert_eq!(dat.games[0].roms[0].serial.as_deref(), Some("SLUS-00001"));
}

// -- ClrMamePro tests --

const SAMPLE_CLR_DAT: &str = r#"clrmamepro (
	name "Nintendo - Nintendo Entertainment System"
	description "Nintendo - Nintendo Entertainment System"
	version 20141025-064058
)

game (
	name "'89 Dennou Kyuusei Uranai (Japan)"
	description "'89 Dennou Kyuusei Uranai (Japan)"
	rom ( name "'89 Dennou Kyuusei Uranai (Japan).nes" size 262144 crc BA58ED29 md5 4187A797E33BC96A96993220DA6F09F7 sha1 56FE858D1035DCE4B68520F457A0858BAE7BB16D )
)

game (
	name "10-Yard Fight (USA, Europe)"
	description "10-Yard Fight (USA, Europe)"
	rom ( name "10-Yard Fight (USA, Europe).nes" size 40960 crc 3D564757 md5 BD2C15391B0641D43A35E83F5FCE073A sha1 016818BF6BAAF779F4F5C1658880B81D23EA40CA )
)
"#;

#[test]
fn test_parse_clrmamepro_dat() {
    let dat = parse_dat(SAMPLE_CLR_DAT.as_bytes()).unwrap();
    assert_eq!(dat.name, "Nintendo - Nintendo Entertainment System");
    assert_eq!(dat.version, "20141025-064058");
    assert_eq!(dat.games.len(), 2);

    let game0 = &dat.games[0];
    assert_eq!(game0.name, "'89 Dennou Kyuusei Uranai (Japan)");
    assert_eq!(game0.roms.len(), 1);
    assert_eq!(game0.roms[0].name, "'89 Dennou Kyuusei Uranai (Japan).nes");
    assert_eq!(game0.roms[0].size, 262144);
    assert_eq!(game0.roms[0].crc, "ba58ed29");
    assert_eq!(
        game0.roms[0].md5.as_deref(),
        Some("4187a797e33bc96a96993220da6f09f7")
    );
    assert_eq!(
        game0.roms[0].sha1.as_deref(),
        Some("56fe858d1035dce4b68520f457a0858bae7bb16d")
    );

    let game1 = &dat.games[1];
    assert_eq!(game1.name, "10-Yard Fight (USA, Europe)");
    assert_eq!(game1.roms[0].size, 40960);
}

#[test]
fn test_parse_empty_clrmamepro() {
    let result = parse_dat("clrmamepro (\n)\n".as_bytes());
    assert!(result.is_err());
}

#[test]
fn test_tokenize_quoted_rom() {
    let tokens = tokenize_rom_line(r#"name "Game (USA, Europe).sfc" size 524288 crc ABCD1234"#);
    assert_eq!(
        tokens,
        vec![
            "name",
            "Game (USA, Europe).sfc",
            "size",
            "524288",
            "crc",
            "ABCD1234",
        ]
    );
}

#[test]
fn test_auto_detect_xml() {
    // Should auto-detect XML from leading '<'
    let dat = parse_dat(SAMPLE_XML_DAT.as_bytes()).unwrap();
    assert!(dat.games.len() > 0);
}

#[test]
fn test_parse_clrmamepro_libretro_enhanced() {
    let dat_str = r#"clrmamepro (
	name "Nintendo - Nintendo 64"
	description "Nintendo - Nintendo 64"
	version 20240101-000000
)

game (
	name "GoldenEye 007 (USA)"
	region "USA"
	serial "NGEE"
	releaseyear "1997"
	releasemonth "8"
	releaseday "25"
	rom ( name "GoldenEye 007 (USA).z64" size 12582912 crc DBC23B14 md5 AB1234CD56EF7890AB1234CD56EF7890 sha1 0123456789ABCDEF0123456789ABCDEF01234567 serial "NGEE" )
)

game (
	name "Homebrew Game (World)"
	rom ( name "Homebrew Game (World).z64" size 1048576 crc 11223344 md5 AABBCCDD11223344AABBCCDD11223344 sha1 AABBCCDD11223344AABBCCDD11223344AABBCCDD )
)
"#;
    let dat = parse_dat(dat_str.as_bytes()).unwrap();
    assert_eq!(dat.name, "Nintendo - Nintendo 64");

    // Game with serial and region
    let ge = &dat.games[0];
    assert_eq!(ge.name, "GoldenEye 007 (USA)");
    assert_eq!(ge.region.as_deref(), Some("USA"));
    assert_eq!(ge.roms[0].serial.as_deref(), Some("NGEE"));
    assert_eq!(ge.roms[0].crc, "dbc23b14");

    // Game without serial — should have no serial propagated
    let hb = &dat.games[1];
    assert_eq!(hb.name, "Homebrew Game (World)");
    assert_eq!(hb.region, None);
    assert_eq!(hb.roms[0].serial, None);
}

#[test]
fn test_clrmamepro_game_serial_propagation() {
    // Game-level serial should propagate to ROMs that lack rom-level serial
    let dat_str = r#"clrmamepro (
	name "Test"
	version 1
)

game (
	name "Test Game (USA)"
	serial "ABCD"
	rom ( name "Test Game (USA).bin" size 1024 crc DEADBEEF md5 00112233445566778899AABBCCDDEEFF sha1 00112233445566778899AABBCCDDEEFF00112233 )
)
"#;
    let dat = parse_dat(dat_str.as_bytes()).unwrap();
    let game = &dat.games[0];
    // ROM didn't have serial, so game-level serial should be propagated
    assert_eq!(game.roms[0].serial.as_deref(), Some("ABCD"));
}

#[test]
fn test_auto_detect_clrmamepro() {
    // Should auto-detect ClrMamePro from leading 'c'
    let dat = parse_dat(SAMPLE_CLR_DAT.as_bytes()).unwrap();
    assert!(dat.games.len() > 0);
}
