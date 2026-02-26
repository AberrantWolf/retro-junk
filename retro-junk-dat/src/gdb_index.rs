//! GDB hash index for fast SHA1/MD5 lookups.
//!
//! Builds an in-memory index from parsed GDB games, keyed by SHA1 (primary)
//! and MD5 (fallback). Used to match ROM file hashes to GDB metadata.

use std::collections::HashMap;

use crate::gdb::GdbGame;

/// An index of GDB games, keyed by hash for fast lookups.
pub struct GdbIndex {
    by_sha1: HashMap<String, usize>,
    by_md5: HashMap<String, usize>,
    games: Vec<GdbGame>,
}

impl GdbIndex {
    /// Build an index from a list of GDB games.
    ///
    /// Duplicate hashes are resolved by keeping the first entry (later
    /// duplicates are silently ignored).
    pub fn from_games(games: Vec<GdbGame>) -> Self {
        let mut by_sha1 = HashMap::with_capacity(games.len());
        let mut by_md5 = HashMap::with_capacity(games.len());

        for (i, game) in games.iter().enumerate() {
            if !game.sha1.is_empty() {
                by_sha1.entry(game.sha1.clone()).or_insert(i);
            }
            if !game.md5.is_empty() {
                by_md5.entry(game.md5.clone()).or_insert(i);
            }
        }

        Self {
            by_sha1,
            by_md5,
            games,
        }
    }

    /// Look up a game by SHA1 hash (primary method).
    pub fn lookup_sha1(&self, sha1: &str) -> Option<&GdbGame> {
        let sha1_lower = sha1.to_lowercase();
        self.by_sha1.get(&sha1_lower).map(|&i| &self.games[i])
    }

    /// Look up a game by MD5 hash (fallback method).
    pub fn lookup_md5(&self, md5: &str) -> Option<&GdbGame> {
        let md5_lower = md5.to_lowercase();
        self.by_md5.get(&md5_lower).map(|&i| &self.games[i])
    }

    /// Returns the total number of indexed games.
    pub fn len(&self) -> usize {
        self.games.len()
    }

    /// Returns true if the index is empty.
    pub fn is_empty(&self) -> bool {
        self.games.is_empty()
    }

    /// Returns the number of SHA1 entries in the index.
    pub fn sha1_count(&self) -> usize {
        self.by_sha1.len()
    }

    /// Returns the number of MD5 entries in the index.
    pub fn md5_count(&self) -> usize {
        self.by_md5.len()
    }
}

#[cfg(test)]
mod tests {
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
}
