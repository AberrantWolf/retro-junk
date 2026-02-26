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
#[path = "tests/gdb_index_tests.rs"]
mod tests;
