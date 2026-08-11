//! First-class cue/bin disc sets.
//!
//! A disc set is a `.cue` sheet plus every track file its FILE directives
//! reference. Redump DATs describe such a game as one `DatGame` whose ROM
//! entries are the individual tracks plus (usually) the `.cue` itself, each
//! with its own size/CRC32/SHA1.
//!
//! This module builds the set from the cue's parsed FILE lines (never from
//! filename-stem guessing), verifies every track against the DAT game's ROM
//! list by hash, and produces a rename plan that moves the cue and all
//! tracks together, with the cue's FILE lines rewritten to the new names.
//! Sets that cannot be fully verified are never partially renamed.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use retro_junk_core::{HashAlgorithms, MultiHasher, RomAnalyzer};
use retro_junk_dat::dat::DatGame;
use retro_junk_dat::matcher::{DatIndex, FileHashes, MatchMethod};

use crate::hasher;
use crate::rename::parse_cue_file_directive;

/// The files that make up a disc set, resolved from the cue's FILE lines.
#[derive(Debug, Clone)]
pub struct DiscSetFiles {
    pub cue: PathBuf,
    /// Track files referenced by the cue, in FILE-line order (deduplicated).
    pub tracks: Vec<PathBuf>,
    /// Referenced filenames that do not exist next to the cue.
    pub missing: Vec<String>,
}

/// A planned rename for one track file within a verified disc set.
#[derive(Debug, Clone)]
pub struct TrackRename {
    pub source: PathBuf,
    /// The DAT ROM name this track hash-matched (e.g., "Game (USA) (Track 2).bin").
    pub target_filename: String,
}

/// A fully verified, renameable disc set.
#[derive(Debug, Clone)]
pub struct DiscSetPlan {
    pub cue: PathBuf,
    /// Canonical cue filename from the DAT (or derived from the game name).
    pub cue_target_filename: String,
    /// The DAT game name.
    pub game_name: String,
    /// The definitive content method. A serial may narrow the work needed,
    /// but a plan exists only after every track hash agrees.
    pub matched_by: MatchMethod,
    /// Per-track renames, every one hash-verified against the DAT.
    pub tracks: Vec<TrackRename>,
    /// Rewritten cue content (FILE lines updated), if any rename changes it.
    pub new_cue_content: Option<String>,
    /// Whether the final cue content hash-matches the DAT's `.cue` ROM entry.
    pub cue_verified: CueVerification,
}

/// Result of verifying a set's cue content against the DAT's `.cue` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CueVerification {
    /// The final cue content hash-matches the DAT's `.cue` ROM entry.
    Verified,
    /// The DAT has a `.cue` entry but the content does not match it.
    Mismatch,
    /// The DAT has no cue entry for this game.
    NoCueEntry,
}

impl DiscSetPlan {
    /// Directory containing the set.
    #[must_use]
    pub fn dir(&self) -> &Path {
        self.cue.parent().unwrap_or(Path::new("."))
    }

    /// Target path for the cue file.
    #[must_use]
    pub fn cue_target(&self) -> PathBuf {
        self.dir().join(&self.cue_target_filename)
    }

    /// All (source, target) file renames in this set, cue included.
    /// No-op renames (source already correctly named) are excluded.
    #[must_use]
    pub fn file_renames(&self) -> Vec<(PathBuf, PathBuf)> {
        let dir = self.dir();
        let mut renames = Vec::new();
        for track in &self.tracks {
            let target = dir.join(&track.target_filename);
            if track.source != target {
                renames.push((track.source.clone(), target));
            }
        }
        let cue_target = self.cue_target();
        if self.cue != cue_target {
            renames.push((self.cue.clone(), cue_target));
        }
        renames
    }

    /// Whether the set is already fully named and referenced correctly.
    #[must_use]
    pub fn is_noop(&self) -> bool {
        self.file_renames().is_empty() && self.new_cue_content.is_none()
    }

    /// Old-stem → new-stem map for media/gamelist renaming.
    #[must_use]
    pub fn stem_map(&self) -> HashMap<String, String> {
        let mut map = HashMap::new();
        for (source, target) in self.file_renames() {
            let old = source
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let new = target
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if old != new {
                map.insert(old, new);
            }
        }
        map
    }

    /// Build the filesystem transaction for this set: all file renames plus
    /// the cue content rewrite (written at the cue's final path).
    #[must_use]
    pub fn build_transaction(&self) -> crate::fs_txn::FsTransaction {
        let mut txn = crate::fs_txn::FsTransaction::new();
        for (source, target) in self.file_renames() {
            txn.rename(source, target);
        }
        if let Some(ref content) = self.new_cue_content {
            txn.write_file(self.cue_target(), content.clone());
        }
        txn
    }
}

/// Outcome of planning a disc set.
#[derive(Debug, Clone)]
pub enum DiscSetOutcome {
    /// Fully verified; safe to rename as a unit.
    Planned(DiscSetPlan),
    /// Fully verified and already correctly named.
    AlreadyCorrect {
        game_name: String,
        cue_verified: CueVerification,
    },
    /// A game was identified, but the set could not be fully verified
    /// (missing/extra/mismatched tracks). Renaming would be unsafe.
    NotVerified {
        /// Identified game name; empty when unidentified
        game_name: String,
        issues: Vec<String>,
    },
    /// The cue references files that do not exist.
    Broken { missing: Vec<String> },
    /// No DAT game could be identified for this set.
    Unmatched { issues: Vec<String> },
}

/// Parse a cue file's FILE directives and resolve the referenced files.
pub fn expand_disc_set(cue_path: &Path) -> std::io::Result<DiscSetFiles> {
    let content = fs::read_to_string(cue_path)?;
    let dir = cue_path.parent().unwrap_or(Path::new("."));

    let mut tracks = Vec::new();
    let mut missing = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for line in content.lines() {
        let Some((filename, _)) = parse_cue_file_directive(line.trim()) else {
            continue;
        };
        if !seen.insert(filename.clone()) {
            continue;
        }
        let path = dir.join(&filename);
        if path.is_file() {
            tracks.push(path);
        } else {
            missing.push(filename);
        }
    }

    Ok(DiscSetFiles {
        cue: cue_path.to_path_buf(),
        tracks,
        missing,
    })
}

/// Progress callback for per-file hashing: `(file, bytes_done, bytes_total)`.
pub type SetHashProgress<'a> = &'a dyn Fn(&Path, u64, u64);

/// Identify and verify a disc set against the DAT index.
///
/// Identification order:
/// 1. Serial extracted from the cue's data track (via the analyzer)
/// 2. Track-hash intersection: the unique DAT game that every track file
///    hash-matches
///
/// Whatever identifies the game, the set is only `Planned` when every track
/// file hash-matches a distinct ROM entry of that game and no track ROM
/// entry of the game is left unmatched. Anything less is `NotVerified`.
pub fn plan_disc_set(
    cue_path: &Path,
    analyzer: &dyn RomAnalyzer,
    index: &DatIndex,
    progress: SetHashProgress<'_>,
) -> DiscSetOutcome {
    let files = match expand_disc_set(cue_path) {
        Ok(f) => f,
        Err(e) => {
            return DiscSetOutcome::Unmatched {
                issues: vec![format!("Failed to read cue: {e}")],
            };
        }
    };
    plan_disc_set_from_files(&files, analyzer, index, progress)
}

/// Like [`plan_disc_set`], but takes already-expanded set files (callers that
/// need the member list regardless of outcome expand once and reuse it).
pub fn plan_disc_set_from_files(
    files: &DiscSetFiles,
    analyzer: &dyn RomAnalyzer,
    index: &DatIndex,
    progress: SetHashProgress<'_>,
) -> DiscSetOutcome {
    let cue_path = files.cue.as_path();
    if !files.missing.is_empty() {
        return DiscSetOutcome::Broken {
            missing: files.missing.clone(),
        };
    }
    if files.tracks.is_empty() {
        return DiscSetOutcome::Unmatched {
            issues: vec!["Cue sheet references no files".to_string()],
        };
    }

    // Hash every track file (plain full-file hashes, as Redump stores them).
    let mut track_hashes: Vec<FileHashes> = Vec::with_capacity(files.tracks.len());
    for track in &files.tracks {
        let mut file = match fs::File::open(track) {
            Ok(f) => f,
            Err(e) => {
                return DiscSetOutcome::Unmatched {
                    issues: vec![format!("Failed to open {}: {e}", track.display())],
                };
            }
        };
        let per_file = |done: u64, total: u64| progress(track, done, total);
        match hasher::compute_plain_crc32_sha1(&mut file, Some(&per_file)) {
            Ok(h) => track_hashes.push(h),
            Err(e) => {
                return DiscSetOutcome::Unmatched {
                    issues: vec![format!("Failed to hash {}: {e}", track.display())],
                };
            }
        }
    }

    // Identification 1: serial from the cue (analyzer reads the data track).
    let serial_outcome = crate::rename::serial_lookup(cue_path, analyzer, index);
    let serial_game = serial_outcome.result.as_ref().map(|r| r.game_index);

    if let Some(gi) = serial_game {
        match assign_tracks(&index.games[gi], &files.tracks, &track_hashes) {
            Ok(assignment) => {
                return build_plan(files, &index.games[gi], &assignment, MatchMethod::Crc32);
            }
            Err(serial_issues) => {
                // Serial game doesn't verify — a hash-identified game is
                // definitive if one exists (e.g., serial collisions across
                // revisions with different track data).
                if let Some((gi, assignment)) =
                    unique_full_hash_match(index, &files.tracks, &track_hashes)
                {
                    return build_plan(files, &index.games[gi], &assignment, MatchMethod::Crc32);
                }
                return DiscSetOutcome::NotVerified {
                    game_name: index.games[gi].name.clone(),
                    issues: serial_issues,
                };
            }
        }
    }

    // Identification 2: track-hash intersection.
    if let Some((gi, assignment)) = unique_full_hash_match(index, &files.tracks, &track_hashes) {
        return build_plan(files, &index.games[gi], &assignment, MatchMethod::Crc32);
    }

    let mut issues = describe_track_matches(index, &files.tracks, &track_hashes);
    if !serial_outcome.full_serial.is_empty() {
        issues.insert(
            0,
            format!("Serial \"{}\" not found in DAT", serial_outcome.full_serial),
        );
    } else if !serial_outcome.ambiguous_candidates.is_empty() {
        issues.insert(
            0,
            format!(
                "Serial is ambiguous across: {}",
                serial_outcome.ambiguous_candidates.join(", ")
            ),
        );
    }
    DiscSetOutcome::Unmatched { issues }
}

/// Try to assign every track file to a distinct non-cue ROM entry of `game`
/// by hash. Returns `rom_index` per track (parallel to `tracks`), or a list
/// of human-readable verification issues.
fn assign_tracks(
    game: &DatGame,
    tracks: &[PathBuf],
    hashes: &[FileHashes],
) -> Result<Vec<usize>, Vec<String>> {
    let mut issues = Vec::new();
    let mut assigned: Vec<Option<usize>> = vec![None; tracks.len()];
    let mut used: HashSet<usize> = HashSet::new();

    for (ti, (track, h)) in tracks.iter().zip(hashes).enumerate() {
        let track_name = track
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        let candidates: Vec<usize> = game
            .roms
            .iter()
            .enumerate()
            .filter(|(ri, rom)| {
                !used.contains(ri)
                    && !rom.name.to_lowercase().ends_with(".cue")
                    && rom.size == h.data_size
                    && rom.crc.eq_ignore_ascii_case(&h.crc32)
                    && match (&rom.sha1, &h.sha1) {
                        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                        _ => true,
                    }
            })
            .map(|(ri, _)| ri)
            .collect();

        // Prefer the ROM whose name already matches the file (stability when
        // duplicate-content tracks exist); otherwise first in DAT order.
        let chosen = candidates
            .iter()
            .find(|&&ri| game.roms[ri].name == track_name)
            .or_else(|| candidates.first());

        match chosen {
            Some(&ri) => {
                used.insert(ri);
                assigned[ti] = Some(ri);
            }
            None => {
                issues.push(format!(
                    "\"{track_name}\" ({} bytes, CRC32 {}) matches no track of \"{}\"",
                    h.data_size, h.crc32, game.name
                ));
            }
        }
    }

    // Every non-cue ROM of the game must be accounted for.
    for (ri, rom) in game.roms.iter().enumerate() {
        if !rom.name.to_lowercase().ends_with(".cue") && !used.contains(&ri) {
            issues.push(format!(
                "Dump has no file matching DAT track \"{}\" ({} bytes, CRC32 {})",
                rom.name, rom.size, rom.crc
            ));
        }
    }

    if issues.is_empty() {
        Ok(assigned
            .into_iter()
            .map(|a| a.expect("all assigned"))
            .collect())
    } else {
        Err(issues)
    }
}

/// Find the unique DAT game that every track fully assigns against.
fn unique_full_hash_match(
    index: &DatIndex,
    tracks: &[PathBuf],
    hashes: &[FileHashes],
) -> Option<(usize, Vec<usize>)> {
    // Intersect the candidate game sets of every track.
    let mut candidates: Option<HashSet<usize>> = None;
    for h in hashes {
        let games: HashSet<usize> = index
            .match_by_hash(h.data_size, h)
            .into_iter()
            .map(|m| m.game_index)
            .collect();
        candidates = Some(match candidates {
            Some(prev) => prev.intersection(&games).copied().collect(),
            None => games,
        });
        if candidates
            .as_ref()
            .is_some_and(std::collections::HashSet::is_empty)
        {
            return None;
        }
    }

    let full_matches: Vec<(usize, Vec<usize>)> = candidates?
        .into_iter()
        .filter_map(|gi| {
            assign_tracks(&index.games[gi], tracks, hashes)
                .ok()
                .map(|a| (gi, a))
        })
        .collect();

    match full_matches.as_slice() {
        [only] => Some(only.clone()),
        _ => None, // zero or ambiguous — caller reports
    }
}

/// Identify a disc only when its complete logical track hash set assigns to
/// exactly one catalog medium.
#[must_use]
pub fn match_complete_track_hashes(
    index: &DatIndex,
    tracks: &[crate::disc_hash::DiscTrackHashes],
) -> Option<String> {
    let paths = tracks
        .iter()
        .map(|track| PathBuf::from(format!("Track {}", track.track_number)))
        .collect::<Vec<_>>();
    let hashes = tracks
        .iter()
        .map(|track| track.hashes.clone())
        .collect::<Vec<_>>();
    unique_full_hash_match(index, &paths, &hashes).map(|(game, _)| index.games[game].name.clone())
}

/// Describe per-track DAT matches for diagnostics when no game verifies.
fn describe_track_matches(
    index: &DatIndex,
    tracks: &[PathBuf],
    hashes: &[FileHashes],
) -> Vec<String> {
    tracks
        .iter()
        .zip(hashes)
        .map(|(track, h)| {
            let name = track
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let games: Vec<String> = index
                .match_by_hash(h.data_size, h)
                .into_iter()
                .map(|m| index.games[m.game_index].name.clone())
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();
            if games.is_empty() {
                format!("\"{name}\" (CRC32 {}) matches no DAT entry", h.crc32)
            } else {
                format!("\"{name}\" matches: {}", games.join(", "))
            }
        })
        .collect()
}

/// Build the final plan for a verified assignment.
fn build_plan(
    files: &DiscSetFiles,
    game: &DatGame,
    assignment: &[usize],
    matched_by: MatchMethod,
) -> DiscSetOutcome {
    let cue_rom = game
        .roms
        .iter()
        .find(|r| r.name.to_lowercase().ends_with(".cue"));
    let cue_target_filename =
        cue_rom.map_or_else(|| format!("{}.cue", game.name), |r| r.name.clone());

    let tracks: Vec<TrackRename> = files
        .tracks
        .iter()
        .zip(assignment)
        .map(|(source, &ri)| TrackRename {
            source: source.clone(),
            target_filename: game.roms[ri].name.clone(),
        })
        .collect();

    // Rewrite FILE lines for tracks whose filename changes.
    let rename_map: HashMap<String, String> = tracks
        .iter()
        .filter_map(|t| {
            let old = t.source.file_name()?.to_str()?.to_string();
            (old != t.target_filename).then(|| (old, t.target_filename.clone()))
        })
        .collect();

    let original_content = match fs::read_to_string(&files.cue) {
        Ok(c) => c,
        Err(e) => {
            return DiscSetOutcome::NotVerified {
                game_name: game.name.clone(),
                issues: vec![format!("Failed to read cue for rewrite: {e}")],
            };
        }
    };
    let new_cue_content = rewrite_cue_references(&original_content, &rename_map);

    // Verify the final cue content against the DAT's cue entry, if present.
    let final_content = new_cue_content.as_deref().unwrap_or(&original_content);
    let cue_verified = match cue_rom {
        Some(rom) => {
            if rom.size == final_content.len() as u64
                && rom
                    .crc
                    .eq_ignore_ascii_case(&crc32_of(final_content.as_bytes()))
            {
                CueVerification::Verified
            } else {
                CueVerification::Mismatch
            }
        }
        None => CueVerification::NoCueEntry,
    };

    let plan = DiscSetPlan {
        cue: files.cue.clone(),
        cue_target_filename,
        game_name: game.name.clone(),
        matched_by,
        tracks,
        new_cue_content,
        cue_verified,
    };

    if plan.is_noop() {
        DiscSetOutcome::AlreadyCorrect {
            game_name: plan.game_name,
            cue_verified: plan.cue_verified,
        }
    } else {
        DiscSetOutcome::Planned(plan)
    }
}

/// Rewrite cue FILE/DATAFILE/AUDIOFILE directives per the rename map.
/// Returns `None` when nothing changes.
fn rewrite_cue_references(content: &str, rename_map: &HashMap<String, String>) -> Option<String> {
    if rename_map.is_empty() {
        return None;
    }

    let mut changed = false;
    let mut lines: Vec<String> = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some((filename, file_type)) = parse_cue_file_directive(trimmed)
            && let Some(new_name) = rename_map.get(&filename)
        {
            let indent = &line[..line.len() - trimmed.len()];
            lines.push(format!("{indent}FILE \"{new_name}\" {file_type}"));
            changed = true;
        } else {
            lines.push(line.to_string());
        }
    }

    if !changed {
        return None;
    }

    let mut out = lines.join("\n");
    if content.ends_with('\n') {
        out.push('\n');
    }
    Some(out)
}

/// CRC32 (lowercase hex) of a byte slice.
fn crc32_of(data: &[u8]) -> String {
    let mut hasher = MultiHasher::new(HashAlgorithms::Crc32, data.len() as u64, None);
    hasher.update(data);
    hasher.finalize().crc32
}

#[cfg(test)]
#[path = "tests/disc_set_tests.rs"]
mod tests;
