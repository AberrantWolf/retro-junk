use std::collections::HashMap;

use crate::dat::{DatFile, DatGame};

/// Hash results for a file.
#[derive(Debug, Clone)]
pub struct FileHashes {
    pub crc32: String,
    pub sha1: Option<String>,
    pub md5: Option<String>,
    /// Size of the data that was hashed (after header stripping)
    pub data_size: u64,
}

/// How a match was determined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchMethod {
    /// Matched by serial number from the ROM header
    Serial,
    /// Matched by CRC32 hash (definitive)
    Crc32,
    /// Matched by SHA1 hash (definitive)
    Sha1,
}

/// Result of matching a file against the DAT index.
#[derive(Debug, Clone)]
pub struct MatchResult {
    /// Index into the DatIndex's games Vec
    pub game_index: usize,
    /// Index of the matching ROM within the game
    pub rom_index: usize,
    /// How the match was determined
    pub method: MatchMethod,
}

/// An indexed view of a DAT file for fast lookups.
pub struct DatIndex {
    /// File size → list of (game_index, rom_index)
    by_size: HashMap<u64, Vec<(usize, usize)>>,
    /// CRC32 (lowercase hex) → (game_index, rom_index)
    by_crc32: HashMap<String, (usize, usize)>,
    /// SHA1 (lowercase hex) → (game_index, rom_index)
    by_sha1: HashMap<String, (usize, usize)>,
    /// Serial (uppercase, stripped of spaces/hyphens) → (game_index, rom_index)
    by_serial: HashMap<String, (usize, usize)>,
    /// Backing store of games
    pub games: Vec<DatGame>,
}

/// Normalize a serial number for matching.
/// Uppercases, strips spaces. Keeps hyphens since they're structurally
/// significant in serials (e.g., "SLUS-00123" vs "SNS-ZL-USA").
fn normalize_serial(serial: &str) -> String {
    serial.to_uppercase().replace(' ', "")
}

impl DatIndex {
    /// Build an index by merging multiple parsed DAT files into one.
    ///
    /// All games from every DAT are combined into a single index, so
    /// downstream consumers (rename, match_by_serial, match_by_hash)
    /// see one unified set of entries.
    pub fn from_dats(dats: Vec<DatFile>) -> Self {
        let all_games: Vec<DatGame> = dats.into_iter().flat_map(|d| d.games).collect();
        Self::from_dat(DatFile {
            name: String::new(),
            description: String::new(),
            version: String::new(),
            games: all_games,
        })
    }

    /// Build an index from a parsed DAT file.
    pub fn from_dat(dat: DatFile) -> Self {
        let mut by_size: HashMap<u64, Vec<(usize, usize)>> = HashMap::new();
        let mut by_crc32 = HashMap::new();
        let mut by_sha1 = HashMap::new();
        let mut by_serial = HashMap::new();

        for (gi, game) in dat.games.iter().enumerate() {
            for (ri, rom) in game.roms.iter().enumerate() {
                by_size.entry(rom.size).or_default().push((gi, ri));
                by_crc32.insert(rom.crc.clone(), (gi, ri));

                if let Some(ref sha1) = rom.sha1 {
                    by_sha1.insert(sha1.clone(), (gi, ri));
                }

                if let Some(ref serial) = rom.serial {
                    by_serial.insert(normalize_serial(serial), (gi, ri));
                }
            }
        }

        Self {
            by_size,
            by_crc32,
            by_sha1,
            by_serial,
            games: dat.games,
        }
    }

    /// Match by hash (CRC32, optionally SHA1).
    pub fn match_by_hash(&self, size: u64, hashes: &FileHashes) -> Option<MatchResult> {
        // Try CRC32 first
        if let Some(&(gi, ri)) = self.by_crc32.get(&hashes.crc32) {
            // Verify size matches
            if self.games[gi].roms[ri].size == size {
                return Some(MatchResult {
                    game_index: gi,
                    rom_index: ri,
                    method: MatchMethod::Crc32,
                });
            }
        }

        // Try SHA1 if available
        if let Some(ref sha1) = hashes.sha1 {
            if let Some(&(gi, ri)) = self.by_sha1.get(sha1) {
                return Some(MatchResult {
                    game_index: gi,
                    rom_index: ri,
                    method: MatchMethod::Sha1,
                });
            }
        }

        None
    }

    /// Match by serial number extracted from the ROM header.
    ///
    /// Handles the format gap between analyzers and DATs:
    /// - Analyzer may produce `NUS-NSME-USA`, DAT may have `NSME`
    /// - Tries exact match first, then tries with the pre-extracted game code
    ///
    /// The `game_code` parameter is the platform-specific extracted code
    /// (e.g., `NSME` from `NUS-NSME-USA`), provided by the analyzer's
    /// `extract_dat_game_code()` method.
    pub fn match_by_serial(&self, serial: &str, game_code: Option<&str>) -> Option<MatchResult> {
        let norm = normalize_serial(serial);

        // Try exact match first
        if let Some(&(gi, ri)) = self.by_serial.get(&norm) {
            return Some(MatchResult {
                game_index: gi,
                rom_index: ri,
                method: MatchMethod::Serial,
            });
        }

        // Try with the pre-extracted game code
        if let Some(code) = game_code {
            let norm_code = normalize_serial(code);
            if let Some(&(gi, ri)) = self.by_serial.get(&norm_code) {
                return Some(MatchResult {
                    game_index: gi,
                    rom_index: ri,
                    method: MatchMethod::Serial,
                });
            }
        }

        None
    }

    /// Number of games in the index.
    pub fn game_count(&self) -> usize {
        self.games.len()
    }

    /// Get entries matching a given file size (for pre-filtering before hashing).
    pub fn candidates_by_size(&self, size: u64) -> Option<&[(usize, usize)]> {
        self.by_size.get(&size).map(|v| v.as_slice())
    }
}

#[cfg(test)]
#[path = "tests/matcher_tests.rs"]
mod tests;
