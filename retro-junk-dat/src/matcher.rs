use std::collections::HashMap;

use crate::dat::{DatFile, DatGame};

/// Hash results for a file.
#[derive(Debug, Clone)]
pub struct FileHashes {
    pub crc32: String,
    pub sha1: Option<String>,
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
    pub fn match_by_hash(
        &self,
        size: u64,
        hashes: &FileHashes,
    ) -> Option<MatchResult> {
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
mod tests {
    use super::*;
    use crate::dat::{DatFile, DatGame, DatRom};

    fn make_test_dat() -> DatFile {
        DatFile {
            name: "Test".into(),
            description: "Test".into(),
            version: "1".into(),
            games: vec![
                DatGame {
                    name: "Super Mario World (USA)".into(),
                    region: None,
                    roms: vec![DatRom {
                        name: "Super Mario World (USA).sfc".into(),
                        size: 524288,
                        crc: "b19ed489".into(),
                        sha1: Some("6b47bb75d16514b6a476aa0c73a683a2a4c18765".into()),
                        md5: None,
                        serial: None,
                    }],
                },
                DatGame {
                    name: "Super Mario 64 (USA)".into(),
                    region: None,
                    roms: vec![DatRom {
                        name: "Super Mario 64 (USA).z64".into(),
                        size: 8388608,
                        crc: "635a2bff".into(),
                        sha1: None,
                        md5: None,
                        // LibRetro DATs use short 4-char game codes
                        serial: Some("NSME".into()),
                    }],
                },
                DatGame {
                    name: "Super Mario 64 (Japan)".into(),
                    region: None,
                    roms: vec![DatRom {
                        name: "Super Mario 64 (Japan).z64".into(),
                        size: 8388608,
                        crc: "4eab3152".into(),
                        sha1: None,
                        md5: None,
                        serial: Some("NSMJ".into()),
                    }],
                },
                DatGame {
                    name: "The Legend of Zelda - A Link to the Past (USA)".into(),
                    region: None,
                    roms: vec![DatRom {
                        name: "The Legend of Zelda - A Link to the Past (USA).sfc".into(),
                        size: 1048576,
                        crc: "777aac2f".into(),
                        sha1: None,
                        md5: None,
                        serial: Some("SNS-ZL-USA".into()),
                    }],
                },
            ],
        }
    }

    #[test]
    fn test_match_by_crc32() {
        let index = DatIndex::from_dat(make_test_dat());
        let hashes = FileHashes {
            crc32: "b19ed489".into(),
            sha1: None,
            data_size: 524288,
        };
        let result = index.match_by_hash(524288, &hashes).unwrap();
        assert_eq!(result.game_index, 0);
        assert_eq!(result.method, MatchMethod::Crc32);
    }

    #[test]
    fn test_match_by_serial_exact() {
        let index = DatIndex::from_dat(make_test_dat());
        // Exact match: DAT has "SNS-ZL-USA", query "SNS-ZL-USA"
        let result = index.match_by_serial("SNS-ZL-USA", None).unwrap();
        assert_eq!(result.game_index, 3);
        assert_eq!(result.method, MatchMethod::Serial);
    }

    #[test]
    fn test_match_by_serial_short_code() {
        let index = DatIndex::from_dat(make_test_dat());
        // DAT has short code "NSME", query with short code "NSME"
        let result = index.match_by_serial("NSME", None).unwrap();
        assert_eq!(result.game_index, 1);
        assert_eq!(result.method, MatchMethod::Serial);
    }

    #[test]
    fn test_match_by_serial_long_to_short() {
        // Analyzer produces NUS-NSME-USA, DAT has NSME — should still match
        // via pre-extracted game code
        let index = DatIndex::from_dat(make_test_dat());
        let result = index.match_by_serial("NUS-NSME-USA", Some("NSME")).unwrap();
        assert_eq!(result.game_index, 1);
        assert_eq!(index.games[result.game_index].name, "Super Mario 64 (USA)");
    }

    #[test]
    fn test_serial_distinguishes_regions() {
        let index = DatIndex::from_dat(make_test_dat());

        // Analyzer produces NUS-NSME-USA, extracts NSME → matches DAT's NSME
        let usa = index.match_by_serial("NUS-NSME-USA", Some("NSME")).unwrap();
        assert_eq!(usa.game_index, 1);
        assert_eq!(index.games[usa.game_index].name, "Super Mario 64 (USA)");

        // Analyzer produces NUS-NSMJ-JPN, extracts NSMJ → matches DAT's NSMJ
        let jpn = index.match_by_serial("NUS-NSMJ-JPN", Some("NSMJ")).unwrap();
        assert_eq!(jpn.game_index, 2);
        assert_eq!(index.games[jpn.game_index].name, "Super Mario 64 (Japan)");
    }

    #[test]
    fn test_hash_distinguishes_regions() {
        let index = DatIndex::from_dat(make_test_dat());

        let usa_hashes = FileHashes {
            crc32: "635a2bff".into(),
            sha1: None,
            data_size: 8388608,
        };
        let usa = index.match_by_hash(8388608, &usa_hashes).unwrap();
        assert_eq!(index.games[usa.game_index].name, "Super Mario 64 (USA)");

        let jpn_hashes = FileHashes {
            crc32: "4eab3152".into(),
            sha1: None,
            data_size: 8388608,
        };
        let jpn = index.match_by_hash(8388608, &jpn_hashes).unwrap();
        assert_eq!(index.games[jpn.game_index].name, "Super Mario 64 (Japan)");
    }

    #[test]
    fn test_no_match() {
        let index = DatIndex::from_dat(make_test_dat());
        let hashes = FileHashes {
            crc32: "00000000".into(),
            sha1: None,
            data_size: 999,
        };
        assert!(index.match_by_hash(999, &hashes).is_none());
        assert!(index.match_by_serial("UNKNOWN", None).is_none());
    }

    #[test]
    fn test_crc32_requires_matching_size() {
        let index = DatIndex::from_dat(make_test_dat());
        // Right CRC but wrong size — should not match
        let hashes = FileHashes {
            crc32: "b19ed489".into(),
            sha1: None,
            data_size: 524288,
        };
        assert!(index.match_by_hash(999, &hashes).is_none());
    }
}
