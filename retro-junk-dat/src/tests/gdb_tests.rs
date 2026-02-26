use super::*;

#[test]
fn test_split_title_with_native() {
    let (rom, native) = split_title("8 Eyes@8 Eyes エイト アイズ");
    assert_eq!(rom, "8 Eyes");
    assert_eq!(native, Some("8 Eyes エイト アイズ"));
}

#[test]
fn test_split_title_no_native() {
    let (rom, native) = split_title("Super Mario Bros.");
    assert_eq!(rom, "Super Mario Bros.");
    assert_eq!(native, None);
}

#[test]
fn test_split_title_empty_native() {
    let (rom, native) = split_title("Some Game@");
    assert_eq!(rom, "Some Game");
    assert_eq!(native, None);
}

#[test]
fn test_parse_tags_full() {
    let tags = parse_tags("#players:2:coop #genre:action>platformer #lang:ja #input:zapper");
    assert_eq!(tags.players, Some("2:coop".to_string()));
    assert_eq!(tags.genres.len(), 1);
    assert_eq!(tags.genres[0], vec!["action", "platformer"]);
    assert_eq!(tags.languages, vec!["ja"]);
    assert_eq!(tags.inputs, vec!["zapper"]);
}

#[test]
fn test_parse_tags_multiple_genres() {
    let tags = parse_tags("#genre:action>platformer #genre:adventure");
    assert_eq!(tags.genres.len(), 2);
    assert_eq!(tags.genres[0], vec!["action", "platformer"]);
    assert_eq!(tags.genres[1], vec!["adventure"]);
}

#[test]
fn test_parse_tags_multi_lang() {
    let tags = parse_tags("#lang:ja,en");
    assert_eq!(tags.languages, vec!["ja", "en"]);
}

#[test]
fn test_parse_tags_empty() {
    let tags = parse_tags("");
    assert!(tags.genres.is_empty());
    assert!(tags.players.is_none());
    assert!(tags.languages.is_empty());
}

#[test]
fn test_parse_csv() {
    let csv = "\
Screen title @ Exact,Cover title @ Exact,ID,Region,Release date,Developer,Publisher,Tags,MD5,SHA1,SHA256,SHA512
4 Nin Uchi Mahjong@4人打ち麻雀,4 Nin Uchi Mahjong@4人打ち麻雀,4ninuchimahjong,Japan,1984-11-02,Hudson,Nintendo,#players:1 #genre:board>mahjong #lang:ja,44f219c48d7b62798d814efacf164865,abc123def456,sha256hash,sha512hash
Super Mario Bros.,Super Mario Bros.,supermariobros,USA,1985-09-13,Nintendo,Nintendo,#players:2:vs #genre:action>platformer #lang:en,md5hash,def789abc012,sha256hash2,sha512hash2";

    let games = parse_gdb_csv(csv).unwrap();
    assert_eq!(games.len(), 2);

    let first = &games[0];
    assert_eq!(first.screen_title, "4 Nin Uchi Mahjong@4人打ち麻雀");
    assert_eq!(first.developer, "Hudson");
    assert_eq!(first.publisher, "Nintendo");
    assert_eq!(first.sha1, "abc123def456");
    assert_eq!(first.region, "Japan");
    assert_eq!(first.release_date, "1984-11-02");
    assert_eq!(first.tags.players, Some("1".to_string()));
    assert_eq!(first.tags.genres[0], vec!["board", "mahjong"]);
    assert_eq!(first.tags.languages, vec!["ja"]);

    let second = &games[1];
    let (rom, native) = split_title(&second.screen_title);
    assert_eq!(rom, "Super Mario Bros.");
    assert_eq!(native, None);
}

#[test]
fn test_parse_csv_skips_empty_sha1() {
    let csv = "\
Screen title @ Exact,Cover title @ Exact,ID,Region,Release date,Developer,Publisher,Tags,MD5,SHA1,SHA256,SHA512
Game With Hash,Game With Hash,gamehash,USA,2000-01-01,Dev,Pub,#genre:action,md5,sha1val,sha256,sha512
Game No Hash,Game No Hash,nohash,USA,2000-01-01,Dev,Pub,#genre:action,md5,,sha256,sha512";

    let games = parse_gdb_csv(csv).unwrap();
    assert_eq!(games.len(), 1);
    assert_eq!(games[0].id, "gamehash");
}
