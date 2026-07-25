use super::*;
use crate::{Frontend, ScrapedGame};
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

fn make_game(name: &str, cover_title: &str) -> ScrapedGame {
    ScrapedGame {
        rom_stem: "test".to_string(),
        rom_filename: "test.rom".to_string(),
        name: name.to_string(),
        description: String::new(),
        developer: String::new(),
        publisher: String::new(),
        genre: String::new(),
        players: String::new(),
        rating: None,
        release_date: String::new(),
        assets: HashMap::new(),
        cover_title: cover_title.to_string(),
    }
}

#[test]
fn test_cover_title_overrides_name_in_esde() {
    let games = vec![make_game("Scraper Name", "Box Title")];
    let dir = tempfile::tempdir().unwrap();
    let rom_dir = dir.path().join("roms");
    let meta_dir = dir.path().join("meta");
    let media_dir = dir.path().join("media");
    std::fs::create_dir_all(&rom_dir).unwrap();

    let esde = EsDeFrontend;
    esde.write_metadata(&games, &rom_dir, &meta_dir, &media_dir)
        .unwrap();

    let xml = std::fs::read_to_string(meta_dir.join("gamelist.xml")).unwrap();
    assert!(xml.contains("<name>Box Title</name>"));
    assert!(!xml.contains("<name>Scraper Name</name>"));
}

#[test]
fn test_name_used_when_cover_title_is_empty() {
    let games = vec![make_game("Scraper Name", "")];
    let dir = tempfile::tempdir().unwrap();
    let rom_dir = dir.path().join("roms");
    let meta_dir = dir.path().join("meta");
    let media_dir = dir.path().join("media");
    std::fs::create_dir_all(&rom_dir).unwrap();

    let esde = EsDeFrontend;
    esde.write_metadata(&games, &rom_dir, &meta_dir, &media_dir)
        .unwrap();

    let xml = std::fs::read_to_string(meta_dir.join("gamelist.xml")).unwrap();
    assert!(xml.contains("<name>Scraper Name</name>"));
}

fn stem_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

#[test]
fn rewrite_gamelist_updates_path_and_asset_tags() {
    let content = "<?xml version=\"1.0\"?>\n<gameList>\n  <game>\n    <path>./dump.cue</path>\n    <name>Cool Game</name>\n    <image>./../roms-media/psx/miximages/dump.png</image>\n  </game>\n</gameList>\n";
    let map = stem_map(&[("dump", "Cool Game (USA)")]);
    let out = rewrite_gamelist_stems(content, &map).unwrap();
    assert!(out.contains("<path>./Cool Game (USA).cue</path>"));
    assert!(out.contains("<image>./../roms-media/psx/miximages/Cool Game (USA).png</image>"));
    // Game title is metadata, not a path — untouched.
    assert!(out.contains("<name>Cool Game</name>"));
}

#[test]
fn rewrite_gamelist_handles_escaped_names() {
    let content = "    <path>./Rock &amp; Roll.cue</path>\n";
    let map = stem_map(&[("Rock & Roll", "Rock & Roll Racing (USA)")]);
    let out = rewrite_gamelist_stems(content, &map).unwrap();
    assert_eq!(out, "    <path>./Rock &amp; Roll Racing (USA).cue</path>\n");
}

#[test]
fn rewrite_gamelist_handles_m3u_folder_entries() {
    let content = "    <path>./Old Game.m3u</path>\n";
    let map = stem_map(&[("Old Game", "New Game (USA)")]);
    let out = rewrite_gamelist_stems(content, &map).unwrap();
    assert_eq!(out, "    <path>./New Game (USA).m3u</path>\n");
}

#[test]
fn rewrite_gamelist_returns_none_when_unchanged() {
    let content = "    <path>./other.cue</path>\n";
    let map = stem_map(&[("dump", "Cool Game (USA)")]);
    assert!(rewrite_gamelist_stems(content, &map).is_none());
    assert!(rewrite_gamelist_stems(content, &HashMap::new()).is_none());
}

#[test]
fn rewrite_gamelist_does_not_match_partial_stems() {
    // "dump" must not match "dumpster.cue" (stem boundary is the dot).
    let content = "    <path>./dumpster.cue</path>\n";
    let map = stem_map(&[("dump", "Cool Game (USA)")]);
    assert!(rewrite_gamelist_stems(content, &map).is_none());
}

#[test]
fn upsert_adds_new_game_without_replacing_user_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let rom_dir = dir.path().join("roms");
    let metadata_dir = dir.path().join("metadata");
    let media_dir = dir.path().join("media");
    std::fs::create_dir_all(&metadata_dir).unwrap();
    std::fs::write(
        metadata_dir.join("gamelist.xml"),
        "<?xml version=\"1.0\"?>\n<gameList>\n  <game>\n    <path>./old.rom</path>\n    <name>Old Game</name>\n    <favorite>true</favorite>\n  </game>\n</gameList>\n",
    )
    .unwrap();
    let mut game = make_game("New Game", "");
    game.rom_filename = "new.rom".to_owned();

    assert!(upsert_game_metadata(&game, &rom_dir, &metadata_dir, &media_dir).unwrap());
    assert!(!upsert_game_metadata(&game, &rom_dir, &metadata_dir, &media_dir).unwrap());
    let xml = std::fs::read_to_string(metadata_dir.join("gamelist.xml")).unwrap();
    assert!(xml.contains("<favorite>true</favorite>"));
    assert_eq!(xml.matches("<path>./new.rom</path>").count(), 1);
}
