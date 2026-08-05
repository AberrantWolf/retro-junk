//! The shared library-entry domain model.
//!
//! One scanned game — its identification, hashes, catalog match, and
//! per-disc breakdown — plus the logic that turns raw analysis and catalog
//! candidates into a status, and the conversions between this in-memory
//! shape and its database row. Scan, hash, and asset operations all work on
//! these types; frontends render them but never decide them.
//!
//! Moved here from the GUI so that operations in [`crate::ops`] and any
//! other frontend share exactly one implementation. Presentation concerns
//! (for example, the badge color for an [`EntryStatus`]) stay in the
//! frontends.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use retro_junk_catalog::CatalogTag;
use retro_junk_dat::{FileHashes, MatchMethod};
use retro_junk_db::LibraryEntryRow;
use retro_junk_frontend::AssetType;
use retro_junk_lib::rename::BrokenReference;
use retro_junk_lib::scanner::GameEntry;
use retro_junk_lib::{AnalysisError, Region, RomIdentification};

// ── Entry model ─────────────────────────────────────────────────────────────

/// Per-disc identification data for multi-disc entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscIdentification {
    pub path: PathBuf,
    pub identification: RomIdentification,
    #[serde(default)]
    pub hashes: Option<FileHashes>,
    #[serde(default)]
    pub dat_match: Option<DatMatchInfo>,
    #[serde(default)]
    pub ambiguous_candidates: Vec<String>,
    /// Whether a disc container was verified as a complete DAT track set.
    #[serde(default)]
    pub disc_verification: DiscVerification,
}

/// One successfully completed file/disc checksum from a batched entry job.
#[derive(Clone)]
pub struct EntryHashResult {
    pub disc_path: Option<PathBuf>,
    pub hashes: FileHashes,
    pub catalog_matches: Vec<retro_junk_db::CatalogMediaMatch>,
    pub disc_verification: DiscVerification,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiscVerification {
    /// Ordinary flat-file hashing, or legacy data created before disc-set
    /// verification existed.
    #[default]
    NotApplicable,
    /// Every logical track matched one catalog media entry.
    Complete,
    /// The disc was identified, but one or more tracks were absent or wrong.
    Incomplete,
    /// The descriptor did not define a safe, coherent logical track layout.
    InvalidLayout,
}

impl DiscVerification {
    fn permits_verified_status(self) -> bool {
        matches!(self, Self::NotApplicable | Self::Complete)
    }
}

#[derive(Clone)]
pub struct LibraryEntry {
    /// Durable database identity; absent only for not-yet-reconciled scan rows.
    pub id: Option<retro_junk_db::LibraryEntryId>,
    pub revision: u64,
    pub source_revision: u64,
    pub game_entry: GameEntry,
    pub identification: Option<RomIdentification>,
    pub hashes: Option<FileHashes>,
    /// Complete-disc verification for a standalone disc container. Multi-disc
    /// entries store this on each [`DiscIdentification`].
    pub disc_verification: DiscVerification,
    pub dat_match: Option<DatMatchInfo>,
    pub status: EntryStatus,
    /// When status is Ambiguous, holds the candidate game names from the DAT.
    pub ambiguous_candidates: Vec<String>,
    /// Discovered media files on disk. `None` = not yet scanned, `Some(empty)` = scanned but none found.
    pub asset_paths: Option<HashMap<AssetType, PathBuf>>,
    /// User-set region override. When set, takes precedence over detected regions.
    pub region_override: Option<Region>,
    /// Box/cover title from catalog DB (e.g., the title printed on the game box).
    /// Empty = absent.
    pub cover_title: String,
    /// Screen title from catalog DB (e.g., the title shown on the title screen).
    /// Empty = absent.
    pub screen_title: String,
    /// Per-disc identification data for multi-disc entries. `None` for single-file entries.
    pub disc_identifications: Option<Vec<DiscIdentification>>,
    /// Broken CUE/M3U references. `None` = not yet checked, `Some(empty)` = checked and clean.
    pub broken_references: Option<Vec<BrokenReference>>,
    /// CUE sheet compatibility issues. `None` = not yet checked, `Some(empty)` = checked and clean.
    pub cue_compat_issues: Option<Vec<CueCompatIssue>>,
    /// User-applied tag (homebrew or modded).
    pub tag: Option<CatalogTag>,
}

/// A CUE sheet compatibility issue detected during scan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CueCompatIssue {
    pub file_name: String,
    pub summary: String,
    pub can_auto_fix: bool,
}

impl LibraryEntry {
    /// Returns the effective status, accounting for user-applied tags.
    ///
    /// When a tag is set, the entry always shows as `Tagged` regardless
    /// of DAT matching status.
    pub fn effective_status(&self) -> EntryStatus {
        match self.tag {
            Some(tag) => EntryStatus::Tagged(tag),
            None => self.status,
        }
    }

    /// Returns the effective region list: the override if set, otherwise the detected regions.
    pub fn effective_regions(&self) -> Vec<Region> {
        if let Some(r) = self.region_override {
            vec![r]
        } else if let Some(ref id) = self.identification {
            id.regions.clone()
        } else {
            Vec::new()
        }
    }

    /// Whether this entry has detected CUE sheet compatibility issues.
    pub fn has_cue_compat_issues(&self) -> bool {
        self.cue_compat_issues
            .as_ref()
            .is_some_and(|issues| !issues.is_empty())
    }
}

/// Deserialize a JSON `null` (or missing field) as the type's default.
///
/// Cached `DiscIdentification` JSON written before the empty-string
/// convention serializes absent fields as `null`; this keeps those caches
/// loadable. (Mirrors the private helper in `retro-junk-core`.)
fn null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + serde::Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatMatchInfo {
    pub game_name: String,
    /// Individual ROM filename from the DAT (e.g., "Game Name (USA).chd").
    #[serde(default)]
    pub rom_name: String,
    pub method: MatchMethod,
    /// Region string from the DAT entry (e.g., "USA", "Japan"). Empty = unknown.
    #[serde(default, deserialize_with = "null_default")]
    pub region: String,
    /// True when the DAT match's region differs from the file's detected region.
    #[serde(default)]
    pub cross_region: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryStatus {
    /// Not yet analyzed / DAT not loaded
    Unknown,
    /// Analyzed but no serial and no hash match
    Unrecognized,
    /// Serial found but no DAT confirmation (or ambiguous)
    Ambiguous,
    /// A unique DAT entry was identified, but it is not completely verified
    /// (for example, serial-only or a disc with missing/mismatched tracks).
    LikelyMatched,
    /// Definitively matched to a DAT fingerprint by hash.
    Matched,
    /// User-tagged as homebrew or modded
    Tagged(CatalogTag),
}

impl EntryStatus {
    /// Where a library entry sits on the one severity scale.
    ///
    /// The same scale archive releases use, so the Library view cannot paint a
    /// file and the release it belongs to in contradictory colours.
    ///
    /// Two mappings are deliberate and were previously wrong:
    ///
    /// - `LikelyMatched` is [`Severity::Incomplete`], not a shade of good. A
    ///   disc with missing or mismatched tracks used to draw the same blue as
    ///   a healthy row, which is precisely the claim it cannot support.
    /// - `Tagged` is [`Severity::Asserted`]. Homebrew and mods are not defects
    ///   and never become verified, because no catalog will ever list them —
    ///   they are a person's assertion, which is what blue now means.
    #[must_use]
    pub const fn severity(self) -> crate::completion::Severity {
        use crate::completion::Severity;
        match self {
            Self::Matched => Severity::Verified,
            Self::Tagged(_) => Severity::Asserted,
            Self::LikelyMatched => Severity::Incomplete,
            Self::Unrecognized | Self::Ambiguous => Severity::Broken,
            Self::Unknown => Severity::Unmeasured,
        }
    }

    /// Human-readable tooltip explaining this status.
    pub fn tooltip(self) -> &'static str {
        match self {
            EntryStatus::Unknown => "Not yet analyzed",
            EntryStatus::Unrecognized => "Not recognized \u{2013} no serial or hash match found",
            EntryStatus::Ambiguous => "Possible match \u{2013} hash verification needed to confirm",
            EntryStatus::LikelyMatched => {
                "Likely match \u{2013} identity is known, but the complete content is not hash-verified"
            }
            EntryStatus::Matched => "Verified match in database",
            EntryStatus::Tagged(CatalogTag::Homebrew) => "Homebrew game",
            EntryStatus::Tagged(CatalogTag::Modded) => "Modded ROM",
        }
    }
}

// ── Catalog match resolution ────────────────────────────────────────────────

/// Describe the format of an M3U folder, e.g. "M3U folder (3x CHD)" or "M3U folder (2x CHD, 1x CUE)".
fn describe_m3u_format(files: &[PathBuf]) -> String {
    let mut ext_counts: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    for f in files {
        let ext = f
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("bin")
            .to_uppercase();
        *ext_counts.entry(ext).or_default() += 1;
    }
    let parts: Vec<String> = ext_counts
        .iter()
        .map(|(ext, count)| format!("{count}x {ext}"))
        .collect();
    format!("M3U folder ({})", parts.join(", "))
}

/// Check whether detected regions match the DAT entry's region string.
/// Returns `true` if any detected region name appears in the DAT region string.
pub fn regions_match_dat(detected: &[Region], dat_region: &str) -> bool {
    retro_junk_lib::catalog_match::regions_match_catalog(detected, dat_region)
}

enum CatalogUiResolution {
    Match {
        info: DatMatchInfo,
        cover_title: String,
        screen_title: String,
    },
    Ambiguous(Vec<String>),
    NotFound,
}

fn resolve_catalog_candidates(
    matches: &[retro_junk_db::CatalogMediaMatch],
    identification: Option<&RomIdentification>,
    hashes: Option<&FileHashes>,
    disc_verification: DiscVerification,
) -> CatalogUiResolution {
    let resolution =
        retro_junk_lib::catalog_match::resolve_catalog_match(matches, identification, hashes);
    match resolution {
        retro_junk_lib::catalog_match::CatalogMatchResolution::Match {
            candidate,
            mut method,
        } => {
            if disc_verification == DiscVerification::Complete
                && matches!(method, MatchMethod::Serial)
            {
                // Complete per-track verification is definitive even when the
                // catalog's representative media hash is not the data track.
                method = MatchMethod::Crc32;
            }
            let detected_regions = identification.map_or(&[][..], |id| id.regions.as_slice());
            CatalogUiResolution::Match {
                info: DatMatchInfo {
                    game_name: candidate.media.dat_name.clone(),
                    rom_name: candidate.media.rom_name.clone(),
                    method,
                    region: candidate.region.clone(),
                    cross_region: !candidate.region.is_empty()
                        && !regions_match_dat(detected_regions, &candidate.region),
                },
                cover_title: candidate.cover_title.clone(),
                screen_title: candidate.screen_title.clone(),
            }
        }
        retro_junk_lib::catalog_match::CatalogMatchResolution::Ambiguous { candidates } => {
            CatalogUiResolution::Ambiguous(candidates)
        }
        retro_junk_lib::catalog_match::CatalogMatchResolution::NotFound
            if disc_verification == DiscVerification::Complete && matches.len() == 1 =>
        {
            let candidate = &matches[0];
            let detected_regions = identification.map_or(&[][..], |id| id.regions.as_slice());
            CatalogUiResolution::Match {
                info: DatMatchInfo {
                    game_name: candidate.media.dat_name.clone(),
                    rom_name: candidate.media.rom_name.clone(),
                    method: MatchMethod::Crc32,
                    region: candidate.region.clone(),
                    cross_region: !candidate.region.is_empty()
                        && !regions_match_dat(detected_regions, &candidate.region),
                },
                cover_title: candidate.cover_title.clone(),
                screen_title: candidate.screen_title.clone(),
            }
        }
        retro_junk_lib::catalog_match::CatalogMatchResolution::NotFound => {
            CatalogUiResolution::NotFound
        }
    }
}

/// Resolve catalog candidates for one entry and write the outcome onto it:
/// status, DAT match, ambiguity list, and any catalog titles.
pub fn apply_catalog_resolution(
    entry: &mut LibraryEntry,
    matches: &[retro_junk_db::CatalogMediaMatch],
) {
    match resolve_catalog_candidates(
        matches,
        entry.identification.as_ref(),
        entry.hashes.as_ref(),
        entry.disc_verification,
    ) {
        CatalogUiResolution::Match {
            info,
            cover_title,
            screen_title,
        } => {
            entry.status = match info.method {
                MatchMethod::Serial => EntryStatus::LikelyMatched,
                // Recorded archive evidence already names a complete,
                // catalog-verified dump; there is no local track set to judge.
                MatchMethod::ArchiveEvidence => EntryStatus::Matched,
                MatchMethod::Crc32 | MatchMethod::Sha1
                    if entry.disc_verification.permits_verified_status() =>
                {
                    EntryStatus::Matched
                }
                MatchMethod::Crc32 | MatchMethod::Sha1 => EntryStatus::LikelyMatched,
            };
            entry.dat_match = Some(info);
            entry.ambiguous_candidates.clear();
            if !cover_title.is_empty() {
                entry.cover_title = cover_title;
            }
            if !screen_title.is_empty() {
                entry.screen_title = screen_title;
            }
        }
        CatalogUiResolution::Ambiguous(candidates) => {
            entry.status = EntryStatus::Ambiguous;
            entry.dat_match = None;
            entry.ambiguous_candidates = candidates;
        }
        CatalogUiResolution::NotFound => {
            entry.status = if entry.identification.is_some() {
                EntryStatus::Unrecognized
            } else {
                EntryStatus::Unknown
            };
            entry.dat_match = None;
            entry.ambiguous_candidates.clear();
        }
    }
}

fn apply_multi_disc_resolution(entry: &mut LibraryEntry) {
    let Some(discs) = entry.disc_identifications.as_ref() else {
        return;
    };
    let mut ambiguous_candidates: Vec<String> = discs
        .iter()
        .flat_map(|disc| disc.ambiguous_candidates.iter().cloned())
        .collect();
    ambiguous_candidates.sort();
    ambiguous_candidates.dedup();

    let matched: Vec<_> = discs
        .iter()
        .filter_map(|disc| disc.dat_match.as_ref())
        .collect();
    if matched.is_empty() {
        entry.dat_match = retro_junk_core::disc::candidates_are_same_game(&ambiguous_candidates)
            .map(|game_name| DatMatchInfo {
                game_name,
                rom_name: String::new(),
                method: MatchMethod::Serial,
                region: String::new(),
                cross_region: false,
            });
        if entry.dat_match.is_some() {
            entry.status = EntryStatus::LikelyMatched;
            entry.ambiguous_candidates.clear();
        } else if ambiguous_candidates.is_empty() {
            entry.status = EntryStatus::Unrecognized;
            entry.ambiguous_candidates.clear();
        } else {
            entry.status = EntryStatus::Ambiguous;
            entry.ambiguous_candidates = ambiguous_candidates;
        }
        return;
    }

    if !ambiguous_candidates.is_empty()
        && retro_junk_core::disc::candidates_are_same_game(&ambiguous_candidates).is_none()
    {
        entry.dat_match = None;
        entry.status = EntryStatus::Ambiguous;
        entry.ambiguous_candidates = ambiguous_candidates;
        return;
    }

    let names: Vec<_> = matched
        .iter()
        .map(|matched| matched.game_name.as_str())
        .collect();
    let first = matched[0];
    let all_hash_verified = !discs.is_empty()
        && discs.iter().all(|disc| {
            disc.disc_verification.permits_verified_status()
                && disc
                    .dat_match
                    .as_ref()
                    .is_some_and(|matched| !matches!(matched.method, MatchMethod::Serial))
        });
    entry.dat_match = Some(DatMatchInfo {
        game_name: retro_junk_core::disc::derive_base_game_name(&names),
        rom_name: first.rom_name.clone(),
        method: if all_hash_verified {
            first.method.clone()
        } else {
            MatchMethod::Serial
        },
        region: first.region.clone(),
        cross_region: matched.iter().any(|matched| matched.cross_region),
    });
    entry.status = if all_hash_verified {
        EntryStatus::Matched
    } else {
        EntryStatus::LikelyMatched
    };
    entry.ambiguous_candidates.clear();
}

/// Apply every successful checksum for one library entry as a unit. This is
/// shared by the durable snapshot and the optional live UI projection, so
/// navigation cannot change matching behavior.
pub fn apply_entry_hash_results(entry: &mut LibraryEntry, results: &[EntryHashResult]) {
    if entry.disc_identifications.is_none() {
        if let Some(result) = results.iter().find(|result| result.disc_path.is_none()) {
            entry.hashes = Some(result.hashes.clone());
            entry.disc_verification = result.disc_verification;
            apply_catalog_resolution(entry, &result.catalog_matches);
        }
        return;
    }

    if let Some(discs) = entry.disc_identifications.as_mut() {
        for result in results {
            let Some(disc_path) = result.disc_path.as_ref() else {
                continue;
            };
            let Some(disc) = discs.iter_mut().find(|disc| &disc.path == disc_path) else {
                continue;
            };
            disc.hashes = Some(result.hashes.clone());
            disc.disc_verification = result.disc_verification;
            match resolve_catalog_candidates(
                &result.catalog_matches,
                Some(&disc.identification),
                disc.hashes.as_ref(),
                disc.disc_verification,
            ) {
                CatalogUiResolution::Match {
                    info,
                    cover_title,
                    screen_title,
                } => {
                    disc.dat_match = Some(info);
                    disc.ambiguous_candidates.clear();
                    if entry.cover_title.is_empty() && !cover_title.is_empty() {
                        entry.cover_title = cover_title;
                    }
                    if entry.screen_title.is_empty() && !screen_title.is_empty() {
                        entry.screen_title = screen_title;
                    }
                }
                CatalogUiResolution::Ambiguous(candidates) => {
                    disc.dat_match = None;
                    disc.ambiguous_candidates = candidates;
                }
                CatalogUiResolution::NotFound => {
                    disc.dat_match = None;
                    disc.ambiguous_candidates.clear();
                }
            }
        }
    }
    apply_multi_disc_resolution(entry);
}

/// Apply one file's analysis result (and its catalog candidates) to an entry.
pub fn apply_single_analysis_result(
    entry: &mut LibraryEntry,
    result: Result<RomIdentification, AnalysisError>,
    catalog_matches: &[retro_junk_db::CatalogMediaMatch],
) {
    if let Ok(identification) = result {
        entry.identification = Some(identification);
        entry.disc_identifications = None;
        apply_catalog_resolution(entry, catalog_matches);
    } else {
        entry.identification = None;
        entry.disc_identifications = None;
        apply_catalog_resolution(entry, catalog_matches);
        if entry.status == EntryStatus::Unknown {
            entry.status = EntryStatus::Unrecognized;
        }
    }
}

/// Apply per-disc analysis results (and their catalog candidates) to a
/// multi-disc entry, then derive the whole-set status.
pub fn apply_multi_disc_analysis_results(
    entry: &mut LibraryEntry,
    disc_results: &[(PathBuf, Result<RomIdentification, AnalysisError>)],
    catalog_matches: &[Vec<retro_junk_db::CatalogMediaMatch>],
) {
    let old_disc_data: HashMap<
        PathBuf,
        (
            Option<FileHashes>,
            Option<DatMatchInfo>,
            Vec<String>,
            DiscVerification,
        ),
    > = entry
        .disc_identifications
        .as_ref()
        .map(|discs| {
            discs
                .iter()
                .map(|disc| {
                    (
                        disc.path.clone(),
                        (
                            disc.hashes.clone(),
                            disc.dat_match.clone(),
                            disc.ambiguous_candidates.clone(),
                            disc.disc_verification,
                        ),
                    )
                })
                .collect()
        })
        .unwrap_or_default();

    let mut disc_ids = Vec::new();
    for ((path, result), candidates) in disc_results.iter().zip(catalog_matches) {
        match result {
            Ok(identification) => {
                let (cached_hashes, cached_dat_match, cached_candidates, cached_disc_verification) =
                    old_disc_data.get(path).cloned().unwrap_or_default();
                let mut disc = DiscIdentification {
                    path: path.clone(),
                    identification: identification.clone(),
                    hashes: cached_hashes,
                    dat_match: cached_dat_match,
                    ambiguous_candidates: cached_candidates,
                    disc_verification: cached_disc_verification,
                };
                match resolve_catalog_candidates(
                    candidates,
                    Some(&disc.identification),
                    disc.hashes.as_ref(),
                    disc.disc_verification,
                ) {
                    CatalogUiResolution::Match { info, .. } => {
                        disc.dat_match = Some(info);
                        disc.ambiguous_candidates.clear();
                    }
                    CatalogUiResolution::Ambiguous(candidates) => {
                        disc.dat_match = None;
                        disc.ambiguous_candidates = candidates;
                    }
                    CatalogUiResolution::NotFound => {
                        disc.dat_match = None;
                        disc.ambiguous_candidates.clear();
                    }
                }
                disc_ids.push(disc);
            }
            Err(error) => log::warn!("Disc analysis failed for {}: {error}", path.display()),
        }
    }

    let mut regions = Vec::new();
    for disc in &disc_ids {
        for region in &disc.identification.regions {
            if !regions.contains(region) {
                regions.push(*region);
            }
        }
    }
    let disc_files: Vec<_> = disc_results.iter().map(|(path, _)| path.clone()).collect();
    let mut game_id = RomIdentification::new();
    game_id.regions = regions;
    game_id
        .extra
        .insert("format".to_owned(), describe_m3u_format(&disc_files));
    game_id
        .extra
        .insert("disc_count".to_owned(), disc_results.len().to_string());
    entry.identification = Some(game_id);
    entry.disc_identifications = Some(disc_ids);
    entry.status = EntryStatus::Unrecognized;
    entry.dat_match = None;
    entry.ambiguous_candidates.clear();
    apply_multi_disc_resolution(entry);
}

// ── Row ↔ Domain Conversion ─────────────────────────────────────────────────

/// An entry could not be converted to or from its database row.
#[derive(Debug)]
pub enum CacheError {
    Db(retro_junk_db::LibraryError),
    Json(serde_json::Error),
}

impl From<retro_junk_db::LibraryError> for CacheError {
    fn from(e: retro_junk_db::LibraryError) -> Self {
        CacheError::Db(e)
    }
}

impl From<serde_json::Error> for CacheError {
    fn from(e: serde_json::Error) -> Self {
        CacheError::Json(e)
    }
}

impl std::fmt::Display for CacheError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CacheError::Db(e) => write!(f, "database error: {e}"),
            CacheError::Json(e) => write!(f, "serialization error: {e}"),
        }
    }
}

/// Convert an entry to a scan row keyed by its source identity within
/// `folder_path` (the console folder the entry was discovered in).
pub fn scanned_entry_for_folder(
    folder_path: &std::path::Path,
    entry: &LibraryEntry,
) -> Result<retro_junk_db::ScannedLibraryEntry, CacheError> {
    let row = entry_to_row(entry)?;
    Ok(retro_junk_db::ScannedLibraryEntry {
        entry_key: retro_junk_db::source_key_from_game_entry_json(
            &row.game_entry_json,
            folder_path,
        )?,
        source_fingerprint: retro_junk_db::source_fingerprint_from_game_entry_json(
            &row.game_entry_json,
            folder_path,
        )?,
        row,
    })
}

/// Serialize an entry into its database row representation.
pub fn entry_to_row(entry: &LibraryEntry) -> Result<LibraryEntryRow, serde_json::Error> {
    let display_name = entry.game_entry.display_name().to_string();
    let game_entry_json = serde_json::to_string(&entry.game_entry)?;

    let (status_str, tag_str) = status_to_str(entry.effective_status());

    let (crc32, sha1, md5, data_size) = match &entry.hashes {
        Some(h) => (
            h.crc32.clone(),
            h.sha1.clone().unwrap_or_default(),
            h.md5.clone().unwrap_or_default(),
            h.data_size as i64,
        ),
        None => (String::new(), String::new(), String::new(), 0),
    };
    let hash_warnings_json = entry
        .hashes
        .as_ref()
        .filter(|hashes| !hashes.warnings.is_empty())
        .map(|hashes| serde_json::to_string(&hashes.warnings))
        .transpose()?;

    let (dat_game_name, dat_rom_name, dat_match_method) = match &entry.dat_match {
        Some(dm) => (
            dm.game_name.clone(),
            dm.rom_name.clone(),
            match_method_to_str(&dm.method).to_string(),
        ),
        None => (String::new(), String::new(), String::new()),
    };

    let region_override = entry
        .region_override
        .map(|r| r.name().to_string())
        .unwrap_or_default();

    let identification_json = entry
        .identification
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;

    let disc_identifications_json = entry
        .disc_identifications
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;

    let broken_references_json = entry
        .broken_references
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;

    let cue_compat_issues_json = entry
        .cue_compat_issues
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;

    let ambiguous_candidates_json = if entry.ambiguous_candidates.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&entry.ambiguous_candidates)?)
    };

    Ok(LibraryEntryRow {
        display_name,
        game_entry_json,
        status: status_str.to_string(),
        tag: tag_str.unwrap_or("").to_string(),
        crc32,
        sha1,
        md5,
        data_size,
        hash_warnings_json,
        disc_verification: disc_verification_to_str(entry.disc_verification).to_string(),
        dat_game_name,
        dat_rom_name,
        dat_match_method,
        region_override,
        cover_title: entry.cover_title.clone(),
        screen_title: entry.screen_title.clone(),
        identification_json,
        disc_identifications_json,
        broken_references_json,
        ambiguous_candidates_json,
        cue_compat_issues_json,
    })
}

/// Build the intent-specific database payload for derived analysis fields.
/// Source identity and user-owned fields are deliberately excluded.
pub fn entry_analysis_update(
    entry: &LibraryEntry,
) -> Result<retro_junk_db::EntryAnalysisUpdate, serde_json::Error> {
    let row = entry_to_row(entry)?;
    Ok(retro_junk_db::EntryAnalysisUpdate {
        status: row.status,
        crc32: row.crc32,
        sha1: row.sha1,
        md5: row.md5,
        data_size: row.data_size,
        hash_warnings_json: row.hash_warnings_json,
        disc_verification: row.disc_verification,
        dat_game_name: row.dat_game_name,
        dat_rom_name: row.dat_rom_name,
        dat_match_method: row.dat_match_method,
        cover_title: row.cover_title,
        screen_title: row.screen_title,
        identification_json: row.identification_json,
        disc_identifications_json: row.disc_identifications_json,
        broken_references_json: row.broken_references_json,
        ambiguous_candidates_json: row.ambiguous_candidates_json,
        cue_compat_issues_json: row.cue_compat_issues_json,
    })
}

/// Build the narrow payload written by an explicit hash operation. This must
/// not include diagnostics or identification fields which may have changed
/// independently while a large disc was hashing.
pub fn entry_hash_update(
    entry: &LibraryEntry,
) -> Result<retro_junk_db::EntryHashUpdate, serde_json::Error> {
    let row = entry_to_row(entry)?;
    Ok(retro_junk_db::EntryHashUpdate {
        status: row.status,
        crc32: row.crc32,
        sha1: row.sha1,
        md5: row.md5,
        data_size: row.data_size,
        hash_warnings_json: row.hash_warnings_json,
        disc_verification: row.disc_verification,
        dat_game_name: row.dat_game_name,
        dat_rom_name: row.dat_rom_name,
        dat_match_method: row.dat_match_method,
        cover_title: row.cover_title,
        screen_title: row.screen_title,
        disc_identifications_json: row.disc_identifications_json,
        ambiguous_candidates_json: row.ambiguous_candidates_json,
    })
}

/// Deserialize a database row back into an in-memory entry.
pub fn row_to_entry(row: LibraryEntryRow) -> Option<LibraryEntry> {
    let game_entry = serde_json::from_str(&row.game_entry_json).ok()?;

    let status = str_to_status(&row.status);
    let tag = str_to_tag(&row.tag);
    let hash_warnings = row
        .hash_warnings_json
        .as_deref()
        .and_then(|json| serde_json::from_str(json).ok())
        .unwrap_or_default();
    let disc_verification = str_to_disc_verification(&row.disc_verification);

    // Empty string in the row means "not set" — map back to the Option fields.
    let hashes = if row.crc32.is_empty() {
        None
    } else {
        Some(retro_junk_dat::FileHashes {
            crc32: row.crc32,
            sha1: (!row.sha1.is_empty()).then_some(row.sha1),
            md5: (!row.md5.is_empty()).then_some(row.md5),
            data_size: row.data_size as u64,
            warnings: hash_warnings,
        })
    };

    let dat_match = if row.dat_game_name.is_empty() {
        None
    } else {
        Some(DatMatchInfo {
            game_name: row.dat_game_name,
            rom_name: row.dat_rom_name,
            method: str_to_match_method(&row.dat_match_method),
            region: String::new(),
            cross_region: false,
        })
    };

    let region_override = Region::ALL
        .iter()
        .find(|r| r.name() == row.region_override)
        .copied();

    let identification = row
        .identification_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

    let disc_identifications = row
        .disc_identifications_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

    let broken_references = row
        .broken_references_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

    let cue_compat_issues = row
        .cue_compat_issues_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

    let ambiguous_candidates: Vec<String> = row
        .ambiguous_candidates_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();

    Some(LibraryEntry {
        id: None,
        revision: 0,
        source_revision: 0,
        game_entry,
        identification,
        hashes,
        disc_verification,
        dat_match,
        status,
        ambiguous_candidates,
        asset_paths: None, // re-discovered lazily
        region_override,
        cover_title: row.cover_title,
        screen_title: row.screen_title,
        disc_identifications,
        broken_references,
        cue_compat_issues,
        tag,
    })
}

/// [`row_to_entry`] plus the durable identity carried on the detail record.
pub fn detail_to_entry(detail: retro_junk_db::LibraryEntryDetail) -> Option<LibraryEntry> {
    let mut entry = row_to_entry(detail.row)?;
    entry.id = Some(detail.id);
    entry.revision = detail.revision;
    entry.source_revision = detail.source_revision;
    Some(entry)
}

fn status_to_str(status: EntryStatus) -> (&'static str, Option<&'static str>) {
    match status {
        EntryStatus::Unknown => ("unknown", None),
        EntryStatus::Unrecognized => ("unrecognized", None),
        EntryStatus::Ambiguous => ("ambiguous", None),
        EntryStatus::LikelyMatched => ("likely", None),
        EntryStatus::Matched => ("matched", None),
        EntryStatus::Tagged(CatalogTag::Homebrew) => ("tagged", Some("homebrew")),
        EntryStatus::Tagged(CatalogTag::Modded) => ("tagged", Some("modded")),
    }
}

fn str_to_status(s: &str) -> EntryStatus {
    match s {
        "unrecognized" => EntryStatus::Unrecognized,
        "ambiguous" => EntryStatus::Ambiguous,
        "likely" => EntryStatus::LikelyMatched,
        "matched" => EntryStatus::Matched,
        // "unknown", "tagged" (tag column provides the real tag), and anything else
        _ => EntryStatus::Unknown,
    }
}

fn str_to_tag(s: &str) -> Option<CatalogTag> {
    match s {
        "homebrew" => Some(CatalogTag::Homebrew),
        "modded" => Some(CatalogTag::Modded),
        _ => None,
    }
}

fn match_method_to_str(m: &MatchMethod) -> &'static str {
    match m {
        MatchMethod::Serial => "serial",
        MatchMethod::Crc32 => "crc32",
        MatchMethod::Sha1 => "sha1",
        MatchMethod::ArchiveEvidence => "archive_evidence",
    }
}

fn str_to_match_method(s: &str) -> MatchMethod {
    match s {
        "serial" => MatchMethod::Serial,
        "sha1" => MatchMethod::Sha1,
        "archive_evidence" => MatchMethod::ArchiveEvidence,
        // "crc32" and anything else default to CRC32
        _ => MatchMethod::Crc32,
    }
}

fn disc_verification_to_str(verification: DiscVerification) -> &'static str {
    match verification {
        DiscVerification::NotApplicable => "not_applicable",
        DiscVerification::Complete => "complete",
        DiscVerification::Incomplete => "incomplete",
        DiscVerification::InvalidLayout => "invalid_layout",
    }
}

fn str_to_disc_verification(value: &str) -> DiscVerification {
    match value {
        "complete" => DiscVerification::Complete,
        "incomplete" => DiscVerification::Incomplete,
        "invalid_layout" => DiscVerification::InvalidLayout,
        _ => DiscVerification::NotApplicable,
    }
}
