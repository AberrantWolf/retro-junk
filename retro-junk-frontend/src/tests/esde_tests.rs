use super::*;
use crate::ScrapedGame;
use std::collections::HashMap;

#[test]
fn test_format_esde_date() {
    assert_eq!(format_esde_date("1996-06-23"), "19960623T000000");
    assert_eq!(format_esde_date("19960623"), "19960623T000000");
}

#[test]
fn test_escape_xml() {
    assert_eq!(escape_xml("Tom & Jerry"), "Tom &amp; Jerry");
    assert_eq!(escape_xml("a < b"), "a &lt; b");
}

fn make_game(name: &str, cover_title: Option<&str>) -> ScrapedGame {
    ScrapedGame {
        rom_stem: "test".to_string(),
        rom_filename: "test.rom".to_string(),
        name: name.to_string(),
        description: None,
        developer: None,
        publisher: None,
        genre: None,
        players: None,
        rating: None,
        release_date: None,
        media: HashMap::new(),
        cover_title: cover_title.map(|s| s.to_string()),
    }
}

#[test]
fn test_cover_title_overrides_name_in_esde() {
    let games = vec![make_game("Scraper Name", Some("Box Title"))];
    let dir = tempfile::tempdir().unwrap();
    let rom_dir = dir.path().join("roms");
    let meta_dir = dir.path().join("meta");
    let media_dir = dir.path().join("media");
    std::fs::create_dir_all(&rom_dir).unwrap();

    let esde = EsDeFrontend::new();
    use crate::Frontend;
    esde.write_metadata(&games, &rom_dir, &meta_dir, &media_dir)
        .unwrap();

    let xml = std::fs::read_to_string(meta_dir.join("gamelist.xml")).unwrap();
    assert!(xml.contains("<name>Box Title</name>"));
    assert!(!xml.contains("<name>Scraper Name</name>"));
}

#[test]
fn test_name_used_when_cover_title_is_none() {
    let games = vec![make_game("Scraper Name", None)];
    let dir = tempfile::tempdir().unwrap();
    let rom_dir = dir.path().join("roms");
    let meta_dir = dir.path().join("meta");
    let media_dir = dir.path().join("media");
    std::fs::create_dir_all(&rom_dir).unwrap();

    let esde = EsDeFrontend::new();
    use crate::Frontend;
    esde.write_metadata(&games, &rom_dir, &meta_dir, &media_dir)
        .unwrap();

    let xml = std::fs::read_to_string(meta_dir.join("gamelist.xml")).unwrap();
    assert!(xml.contains("<name>Scraper Name</name>"));
}
