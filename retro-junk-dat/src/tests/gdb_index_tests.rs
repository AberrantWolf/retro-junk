use super::*;
use crate::gdb::{GdbGame, GdbTags};

fn make_game(sha1: &str, md5: &str, title: &str) -> GdbGame {
    GdbGame {
        screen_title: title.to_string(),
        cover_title: title.to_string(),
        id: "test".to_string(),
        region: "USA".to_string(),
        release_date: "2000-01-01".to_string(),
        developer: "Dev".to_string(),
        publisher: "Pub".to_string(),
        tags: GdbTags::default(),
        md5: md5.to_string(),
        sha1: sha1.to_string(),
        sha256: String::new(),
        sha512: String::new(),
    }
}

#[test]
fn test_lookup_sha1() {
    let games = vec![
        make_game("abc123", "md5aaa", "Game A"),
        make_game("def456", "md5bbb", "Game B"),
    ];
    let index = GdbIndex::from_games(games);

    let result = index.lookup_sha1("abc123");
    assert!(result.is_some());
    assert_eq!(result.unwrap().screen_title, "Game A");

    let result = index.lookup_sha1("def456");
    assert!(result.is_some());
    assert_eq!(result.unwrap().screen_title, "Game B");

    assert!(index.lookup_sha1("nonexistent").is_none());
}

#[test]
fn test_lookup_sha1_case_insensitive() {
    let games = vec![make_game("abc123def", "", "Game")];
    let index = GdbIndex::from_games(games);

    assert!(index.lookup_sha1("ABC123DEF").is_some());
    assert!(index.lookup_sha1("Abc123Def").is_some());
}

#[test]
fn test_lookup_md5_fallback() {
    let games = vec![make_game("sha1val", "aabbccdd", "Game")];
    let index = GdbIndex::from_games(games);

    assert!(index.lookup_md5("aabbccdd").is_some());
    assert!(index.lookup_md5("AABBCCDD").is_some());
}

#[test]
fn test_duplicate_sha1_keeps_first() {
    let games = vec![
        make_game("same_sha1", "", "First"),
        make_game("same_sha1", "", "Second"),
    ];
    let index = GdbIndex::from_games(games);

    let result = index.lookup_sha1("same_sha1").unwrap();
    assert_eq!(result.screen_title, "First");
}

#[test]
fn test_empty_index() {
    let index = GdbIndex::from_games(vec![]);
    assert!(index.is_empty());
    assert_eq!(index.len(), 0);
    assert!(index.lookup_sha1("anything").is_none());
}

#[test]
fn test_counts() {
    let games = vec![make_game("sha1a", "md5a", "A"), make_game("sha1b", "", "B")];
    let index = GdbIndex::from_games(games);
    assert_eq!(index.len(), 2);
    assert_eq!(index.sha1_count(), 2);
    assert_eq!(index.md5_count(), 1);
}
