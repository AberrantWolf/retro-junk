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

/// Result of a serial lookup, distinguishing unique match from ambiguous.
#[derive(Debug, Clone)]
pub enum SerialLookupResult {
    /// Unique match found
    Match(MatchResult),
    /// Multiple games share this serial — caller should fall back to hash
    Ambiguous {
        /// Names of the candidate games
        candidates: Vec<String>,
    },
    /// No match found at all
    NotFound,
}

/// An indexed view of a DAT file for fast lookups.
pub struct DatIndex {
    /// File size → list of (game_index, rom_index)
    by_size: HashMap<u64, Vec<(usize, usize)>>,
    /// CRC32 (lowercase hex) → (game_index, rom_index)
    by_crc32: HashMap<String, (usize, usize)>,
    /// SHA1 (lowercase hex) → (game_index, rom_index)
    by_sha1: HashMap<String, (usize, usize)>,
    /// Serial (uppercase, stripped of spaces/hyphens) → list of (game_index, rom_index)
    by_serial: HashMap<String, Vec<(usize, usize)>>,
    /// Backing store of games
    pub games: Vec<DatGame>,
}

/// Normalize a serial number for matching.
/// Uppercases, strips spaces and hyphens. Redump DATs inconsistently use
/// spaces (e.g., "SLPS 00700") vs dashes (e.g., "SLPS-00700"), so we
/// strip both for reliable matching.
fn normalize_serial(serial: &str) -> String {
    serial
        .to_uppercase()
        .replace(' ', "")
        .replace('-', "")
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
        let mut by_serial: HashMap<String, Vec<(usize, usize)>> = HashMap::new();

        for (gi, game) in dat.games.iter().enumerate() {
            for (ri, rom) in game.roms.iter().enumerate() {
                by_size.entry(rom.size).or_default().push((gi, ri));
                by_crc32.insert(rom.crc.clone(), (gi, ri));

                if let Some(ref sha1) = rom.sha1 {
                    by_sha1.insert(sha1.clone(), (gi, ri));
                }

                if let Some(ref serial) = rom.serial {
                    // Redump DATs may have comma-separated serials
                    // (e.g., "SLUS-01041, SLUS-01041GH, SLUS-01041GH-F").
                    // Index each one individually for lookup.
                    for part in serial.split(',') {
                        let trimmed = part.trim();
                        if !trimmed.is_empty() {
                            by_serial
                                .entry(normalize_serial(trimmed))
                                .or_default()
                                .push((gi, ri));
                        }
                    }
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
    /// - For multi-disc games, tries disc suffixes (`-0` through `-9`) to match
    ///   LibRetro Redump DAT entries that use suffixed serials
    ///
    /// Returns `SerialLookupResult::Ambiguous` when multiple games share the
    /// same serial (e.g., alternate versions, Greatest Hits re-releases, or
    /// romhacks). The caller should fall back to hash matching in that case.
    ///
    /// The `game_code` parameter is the platform-specific extracted code
    /// (e.g., `NSME` from `NUS-NSME-USA`), provided by the analyzer's
    /// `extract_dat_game_code()` method.
    pub fn match_by_serial(
        &self,
        serial: &str,
        game_code: Option<&str>,
    ) -> SerialLookupResult {
        let norm = normalize_serial(serial);

        // Try exact match first
        if let Some(entries) = self.by_serial.get(&norm) {
            let result = self.resolve_serial_entries(entries, &norm);
            if !matches!(result, SerialLookupResult::NotFound) {
                return result;
            }
        }

        // Try with the pre-extracted game code
        if let Some(code) = game_code {
            let norm_code = normalize_serial(code);
            if let Some(entries) = self.by_serial.get(&norm_code) {
                let result = self.resolve_serial_entries(entries, &norm_code);
                if !matches!(result, SerialLookupResult::NotFound) {
                    return result;
                }
            }
        }

        // No exact match — try with disc suffixes as a last resort.
        // Handles cases where the disc's boot serial doesn't appear bare
        // in the DAT but does appear with a suffix.
        for suffix in b'0'..=b'9' {
            let suffixed = format!("{norm}{}", suffix as char);
            if let Some(entries) = self.by_serial.get(&suffixed) {
                let result = self.resolve_serial_entries(entries, &suffixed);
                if !matches!(result, SerialLookupResult::NotFound) {
                    return result;
                }
            }
        }

        SerialLookupResult::NotFound
    }

    /// Resolve a Vec of serial entries to a single match or ambiguity.
    ///
    /// - 1 entry → unique match
    /// - Multiple entries but a `-0` suffix resolves uniquely → use that
    ///   (preserves multi-disc behavior where bare serial is shared)
    /// - Multiple entries with no suffix resolution → Ambiguous
    fn resolve_serial_entries(
        &self,
        entries: &[(usize, usize)],
        norm: &str,
    ) -> SerialLookupResult {
        if entries.len() == 1 {
            let (gi, ri) = entries[0];
            // Check if a "-0" suffixed entry exists — if so, the bare serial
            // is from a multi-disc set and we should use the specific entry.
            let suffixed = format!("{norm}0");
            if let Some(suffixed_entries) = self.by_serial.get(&suffixed) {
                if suffixed_entries.len() == 1 {
                    let (sgi, sri) = suffixed_entries[0];
                    return SerialLookupResult::Match(MatchResult {
                        game_index: sgi,
                        rom_index: sri,
                        method: MatchMethod::Serial,
                    });
                }
            }
            return SerialLookupResult::Match(MatchResult {
                game_index: gi,
                rom_index: ri,
                method: MatchMethod::Serial,
            });
        }

        // Multiple entries — try "-0" suffix to disambiguate multi-disc sets
        let suffixed = format!("{norm}0");
        if let Some(suffixed_entries) = self.by_serial.get(&suffixed) {
            if suffixed_entries.len() == 1 {
                let (sgi, sri) = suffixed_entries[0];
                return SerialLookupResult::Match(MatchResult {
                    game_index: sgi,
                    rom_index: sri,
                    method: MatchMethod::Serial,
                });
            }
        }

        // Genuinely ambiguous — deduplicate game names for the warning
        let mut candidate_names: Vec<String> = entries
            .iter()
            .map(|&(gi, _)| self.games[gi].name.clone())
            .collect();
        candidate_names.sort();
        candidate_names.dedup();

        SerialLookupResult::Ambiguous {
            candidates: candidate_names,
        }
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
