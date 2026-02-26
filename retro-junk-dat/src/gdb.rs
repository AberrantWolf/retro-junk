//! GameDataBase (GDB) CSV parser.
//!
//! Parses CSV files from PigSaint's GameDataBase repository, which provides
//! rich metadata (Japanese titles, developer/publisher, genre, player count)
//! indexed by SHA1 hash.
//!
//! Repository: <https://github.com/PigSaint/GameDataBase>
//! License: CC BY 4.0 — Attribution to PigSaint required.

use std::io::Read;
use std::path::Path;

use crate::error::DatError;

/// A single game entry parsed from a GDB CSV file.
#[derive(Debug, Clone)]
pub struct GdbGame {
    /// Screen title (may contain `@` separator: `"English@日本語"`)
    pub screen_title: String,
    /// Cover title (may contain `@` separator)
    pub cover_title: String,
    /// GDB identifier slug (e.g., `"4ninuchimahjong"`)
    pub id: String,
    /// Region string (e.g., `"Japan"`, `"USA"`, `"Europe"`)
    pub region: String,
    /// Release date in `YYYY-MM-DD` format
    pub release_date: String,
    /// Developer name
    pub developer: String,
    /// Publisher name
    pub publisher: String,
    /// Parsed tag data
    pub tags: GdbTags,
    /// MD5 hash (lowercase hex)
    pub md5: String,
    /// SHA1 hash (lowercase hex)
    pub sha1: String,
    /// SHA256 hash (lowercase hex)
    pub sha256: String,
    /// SHA512 hash (lowercase hex)
    pub sha512: String,
}

/// Parsed tag data from a GDB entry's tag string.
///
/// Tags use the format `#key:value` separated by spaces. Hierarchical genres
/// use `>` (e.g., `#genre:action>platformer`). Multiple genres appear as
/// separate `#genre:` tags.
#[derive(Debug, Clone, Default)]
pub struct GdbTags {
    /// Genre paths (e.g., `["action", "platformer"]` for `action>platformer`)
    pub genres: Vec<Vec<String>>,
    /// Player info (e.g., `"2:coop"`, `"1"`)
    pub players: Option<String>,
    /// Language codes (e.g., `["ja"]`, `["ja", "en"]`)
    pub languages: Vec<String>,
    /// Input types (e.g., `["zapper"]`)
    pub inputs: Vec<String>,
    /// All tags as raw (key, value) pairs for fields we don't specifically parse
    pub raw: Vec<(String, String)>,
}

/// A collection of games parsed from a single GDB CSV file.
#[derive(Debug, Clone)]
pub struct GdbFile {
    /// Original filename (without path)
    pub filename: String,
    /// Parsed game entries
    pub games: Vec<GdbGame>,
}

/// Split a title string on the `@` separator.
///
/// Returns `(romanized, Option<native>)`. If there's no `@`, the entire
/// string is returned as the romanized title with `None` for native.
pub fn split_title(s: &str) -> (&str, Option<&str>) {
    match s.find('@') {
        Some(pos) => {
            let romanized = &s[..pos];
            let native = &s[pos + 1..];
            if native.is_empty() {
                (romanized, None)
            } else {
                (romanized, Some(native))
            }
        }
        None => (s, None),
    }
}

/// Parse a GDB tag string into structured data.
///
/// Tag format: `#players:2:coop #genre:action>platformer #lang:ja #input:zapper`
/// Tags are space-separated, each starting with `#`.
pub fn parse_tags(tag_str: &str) -> GdbTags {
    let mut tags = GdbTags::default();

    for token in tag_str.split_whitespace() {
        let token = token.strip_prefix('#').unwrap_or(token);
        if token.is_empty() {
            continue;
        }

        // Split on first ':' to get key and value
        let (key, value) = match token.find(':') {
            Some(pos) => (&token[..pos], &token[pos + 1..]),
            None => (token, ""),
        };

        tags.raw.push((key.to_string(), value.to_string()));

        match key {
            "genre" => {
                let path: Vec<String> = value.split('>').map(|s| s.to_string()).collect();
                tags.genres.push(path);
            }
            "players" => {
                tags.players = Some(value.to_string());
            }
            "lang" => {
                for lang in value.split(',') {
                    if !lang.is_empty() {
                        tags.languages.push(lang.to_string());
                    }
                }
            }
            "input" => {
                if !value.is_empty() {
                    tags.inputs.push(value.to_string());
                }
            }
            _ => {}
        }
    }

    tags
}

/// Parse a GDB CSV file from a file path.
pub fn parse_gdb_file(path: &Path) -> Result<GdbFile, DatError> {
    let mut file = std::fs::File::open(path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;

    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();

    let games = parse_gdb_csv(&contents)?;
    Ok(GdbFile { filename, games })
}

/// Parse GDB CSV content from a string.
pub fn parse_gdb_csv(content: &str) -> Result<Vec<GdbGame>, DatError> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(content.as_bytes());

    let mut games = Vec::new();

    for result in reader.records() {
        let record = match result {
            Ok(r) => r,
            Err(e) => {
                log::warn!("Skipping malformed GDB CSV row: {e}");
                continue;
            }
        };

        // CSV columns:
        // 0: Screen title @ Exact
        // 1: Cover title @ Exact
        // 2: ID
        // 3: Region
        // 4: Release date
        // 5: Developer
        // 6: Publisher
        // 7: Tags
        // 8: MD5
        // 9: SHA1
        // 10: SHA256
        // 11: SHA512
        let get = |i: usize| record.get(i).unwrap_or("").to_string();

        let sha1 = record.get(9).unwrap_or("").to_lowercase();
        let md5 = record.get(8).unwrap_or("").to_lowercase();

        // Skip entries with no SHA1 — they can't be matched
        if sha1.is_empty() {
            continue;
        }

        let tag_str = record.get(7).unwrap_or("");

        games.push(GdbGame {
            screen_title: get(0),
            cover_title: get(1),
            id: get(2),
            region: get(3),
            release_date: get(4),
            developer: get(5),
            publisher: get(6),
            tags: parse_tags(tag_str),
            md5,
            sha1,
            sha256: record.get(10).unwrap_or("").to_lowercase(),
            sha512: record.get(11).unwrap_or("").to_lowercase(),
        });
    }

    Ok(games)
}

#[cfg(test)]
mod tests {
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
}
