//! Reviewing playable files the archive cannot account for.
//!
//! The adoption sweep walks the playable library and, for every file no build
//! evidence claims, tries to prove what it is. Most files prove themselves and
//! are adopted without asking. The rest land here, and they land here for one
//! of two very different reasons, which is why a single "Apply" button was
//! never enough:
//!
//! * **The tool found several answers and will not choose between them.** A
//!   file byte-identical to three archived masters, or matching four catalog
//!   media, is a question with candidates attached. Choosing one is a real
//!   command, and this module executes it.
//! * **The tool found no answer at all.** A readme, a save state, a format the
//!   archive cannot accept yet. There is nothing to apply, and the honest
//!   resolutions are to leave it alone or to say "never ask me about these
//!   again" — which is a durable ignore rule, not a closed row.
//!
//! Re-validation is not optional here. A suggestion records what was true when
//! the sweep ran, and the file may have been renamed, replaced, or deleted
//! since. Every resolution re-hashes the file and re-checks the claim before
//! writing anything, so an accepted review is never weaker evidence than an
//! automatic adoption.

use std::sync::atomic::AtomicBool;

use serde::{Deserialize, Serialize};

use crate::executor::{ExecContext, WorkError};

/// A playable file the sweep could not account for.
pub const ADOPT_SUGGESTION_KIND: &str = "adopt_playable";

/// What a review of one unaccounted playable file carries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AdoptionSuggestionPayload {
    /// Where the file sits under the playable root.
    pub relative_path: String,
    /// Why it could not be adopted: `unmatched`, `catalog_only`,
    /// `ambiguous_archive_master`, `ambiguous_catalog`.
    pub status: String,
    /// The same thing in words, for reading.
    pub detail: String,
    /// The answers the sweep found but would not choose between. Empty when
    /// there was nothing to choose from, which is what makes a row
    /// unresolvable by command rather than merely undecided.
    #[serde(default)]
    pub candidates: Vec<AdoptionCandidate>,
}

/// One thing the file might be, and enough to act on that.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdoptionCandidate {
    pub kind: AdoptionCandidateKind,
    /// The archived dump id, or the catalog medium id.
    pub id: String,
    /// What to show the person choosing.
    pub label: String,
    #[serde(default)]
    pub archive_release_id: String,
    #[serde(default)]
    pub carrier_id: String,
    #[serde(default)]
    pub platform_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdoptionCandidateKind {
    /// A preservation master in this archive with these exact bytes. Choosing
    /// it records the file as that master's derivative.
    ArchiveMaster,
    /// A catalogued medium these bytes match. Choosing it records the identity
    /// only — nothing in the archive holds the file.
    CatalogMedium,
}

impl AdoptionCandidateKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ArchiveMaster => "archive master",
            Self::CatalogMedium => "catalog medium",
        }
    }
}

/// File one unaccounted playable file for review.
///
/// One open row per path: re-running the sweep refreshes the row rather than
/// piling a second one on top of it.
pub fn open_adoption_suggestion(
    conn: &mut retro_junk_db::Connection,
    payload: &AdoptionSuggestionPayload,
    confidence: f64,
    provenance: &str,
) -> Result<(), WorkError> {
    let payload_json =
        serde_json::to_string(payload).map_err(|error| WorkError::Message(error.to_string()))?;
    retro_junk_db::work::open_suggestion(
        conn,
        &retro_junk_db::work::NewSuggestion {
            kind: ADOPT_SUGGESTION_KIND,
            target_kind: "path",
            target_id: &payload.relative_path,
            payload_json: &payload_json,
            confidence,
            provenance,
        },
    )?;
    Ok(())
}

/// Read a review's payload, tolerating the shape written before candidates
/// existed — those rows are still open on live collections and must stay
/// readable rather than turning into blank cards.
#[must_use]
pub fn read_payload(suggestion: &retro_junk_db::work::Suggestion) -> AdoptionSuggestionPayload {
    serde_json::from_str(&suggestion.payload_json).unwrap_or_else(|_| AdoptionSuggestionPayload {
        relative_path: suggestion.target_id.clone(),
        ..AdoptionSuggestionPayload::default()
    })
}

/// Resolve an adoption review by choosing one of its candidates.
///
/// `choice` names a candidate id. It may be omitted only when there is exactly
/// one candidate, since then there is nothing to choose.
pub fn apply_adoption(
    ctx: &ExecContext,
    suggestion: &retro_junk_db::work::Suggestion,
    choice: Option<&str>,
    cancelled: &AtomicBool,
) -> Result<String, WorkError> {
    let payload = read_payload(suggestion);
    let candidate = pick(&payload, choice)?;
    let path = ctx.profile.playable_root.join(&payload.relative_path);
    if !path.exists() {
        return Err(WorkError::Message(format!(
            "{} is no longer there; re-run adoption to see what is",
            payload.relative_path
        )));
    }

    let summary = match candidate.kind {
        AdoptionCandidateKind::ArchiveMaster => {
            adopt_as_master_derivative(ctx, &payload, candidate, &path, cancelled)?
        }
        AdoptionCandidateKind::CatalogMedium => {
            bind_to_catalog_medium(ctx, &payload, candidate, &path)?
        }
    };
    let mut conn = retro_junk_db::open_database(&ctx.db_path).map_err(WorkError::msg)?;
    retro_junk_db::work::resolve_suggestion(&mut conn, suggestion.id, "applied")?;
    Ok(summary)
}

/// Which candidate the caller meant.
fn pick<'a>(
    payload: &'a AdoptionSuggestionPayload,
    choice: Option<&str>,
) -> Result<&'a AdoptionCandidate, WorkError> {
    match (choice, payload.candidates.as_slice()) {
        (_, []) => Err(WorkError::Message(format!(
            "nothing to apply for {}: the sweep found no candidate for it, so this review \
             records a decision rather than an action",
            payload.relative_path
        ))),
        (None, [only]) => Ok(only),
        (None, many) => Err(WorkError::Message(format!(
            "{} matches {} candidates; choose one",
            payload.relative_path,
            many.len()
        ))),
        (Some(wanted), candidates) => candidates
            .iter()
            .find(|candidate| candidate.id == wanted)
            .ok_or_else(|| {
                WorkError::Message(format!("'{wanted}' is not one of this review's candidates"))
            }),
    }
}

/// Record the file as the chosen archived master's derivative.
///
/// The claim being made is "these are the same bytes", so it is re-proved here
/// rather than trusted from the suggestion: the archive is rescanned, the file
/// re-hashed, and the digests compared against the master's single-file
/// manifest. Anything else has moved on since the sweep, and the right answer
/// is to say so rather than to write evidence that was true last week.
fn adopt_as_master_derivative(
    ctx: &ExecContext,
    payload: &AdoptionSuggestionPayload,
    candidate: &AdoptionCandidate,
    path: &std::path::Path,
    cancelled: &AtomicBool,
) -> Result<String, WorkError> {
    // This writes evidence into the archive, and every archive mutation
    // happens under the whole-archive lock — an apply clicked while the
    // daemon is mid-build must wait its turn, not write concurrently. The
    // scan below happens under the same lock so what gets re-proved is what
    // gets written against.
    let Some(_lock) =
        retro_junk_archive::ArchiveLock::acquire_wait(&ctx.profile.archive_root, cancelled)
            .map_err(WorkError::msg)?
    else {
        return Err(WorkError::Cancelled);
    };
    let snapshot =
        retro_junk_archive::scan_archive(&ctx.profile.archive_root).map_err(WorkError::msg)?;
    let found = snapshot
        .releases
        .iter()
        .flat_map(|release| {
            release.physical_copies.iter().flat_map(move |copy| {
                copy.carriers.iter().flat_map(move |carrier| {
                    carrier
                        .dumps
                        .iter()
                        .map(move |dump| (release, carrier, dump))
                })
            })
        })
        .find(|(_, _, dump)| dump.manifest.dump_id.to_string() == candidate.id);
    let Some((release, carrier, dump)) = found else {
        return Err(WorkError::Message(format!(
            "the archive no longer holds dump {}; re-run adoption",
            candidate.id
        )));
    };

    let digests = retro_junk_archive::hash_file_digests(path, cancelled).map_err(WorkError::msg)?;
    let identical = dump.manifest.files.len() == 1
        && dump.manifest.files[0].size == digests.size
        && dump.manifest.files[0].sha256 == digests.sha256;
    if !identical {
        return Err(WorkError::Message(format!(
            "{} is no longer byte-identical to that master; re-run adoption to see what it is now",
            payload.relative_path
        )));
    }

    let conn = retro_junk_db::open_database(&ctx.db_path).map_err(WorkError::msg)?;
    retro_junk_lib::archive_ops::adopt_identical_playable(
        &retro_junk_lib::archive_ops::IdenticalAdoption {
            dump,
            carrier,
            platform_id: &release.manifest.platform_id,
            relative_path: &payload.relative_path,
            digests: &digests,
        },
        &conn,
    )
    .map_err(|error| WorkError::Message(error.to_string()))?;
    Ok(format!(
        "Adopted {} as a derivative of {}",
        payload.relative_path, candidate.label
    ))
}

/// Record the file's catalog identity, with no claim that the archive holds
/// it. That is the whole content of this resolution: the library row learns
/// what the file is, and the archive stays silent about a master it does not
/// have.
fn bind_to_catalog_medium(
    ctx: &ExecContext,
    payload: &AdoptionSuggestionPayload,
    candidate: &AdoptionCandidate,
    path: &std::path::Path,
) -> Result<String, WorkError> {
    let platform = if candidate.platform_id.is_empty() {
        platform_of(&payload.relative_path)
    } else {
        catalog_platform(&candidate.platform_id)
    };
    let digests = catalog_digests(ctx, &platform, path)?;
    let conn = retro_junk_db::open_database(&ctx.db_path).map_err(WorkError::msg)?;
    let matches = retro_junk_db::match_catalog_file(&conn, &platform, &digests)?;
    if !matches
        .iter()
        .any(|matched| matched.media_id == candidate.id)
    {
        return Err(WorkError::Message(format!(
            "{} no longer matches that catalog medium; re-run adoption",
            payload.relative_path
        )));
    }
    retro_junk_db::bind_library_entries_by_hash(
        &conn,
        &platform,
        &digests,
        &retro_junk_db::LibraryEntryBinding {
            catalog_media_id: &candidate.id,
            match_method: "catalog_adoption",
            ..Default::default()
        },
    )?;
    Ok(format!(
        "Bound {} to {}",
        payload.relative_path, candidate.label
    ))
}

/// The digests the catalog compares against.
///
/// A catalogued digest describes a ROM's logical payload, which for a headered
/// cartridge is not the file's bytes — so the platform's analyzer normalizes
/// it first. A file that analyzer cannot read is not a failure: the raw digests
/// stand in, and if they match nothing the caller says so. This is the same
/// rule the adoption sweep applies, and it has to stay the same rule, or a
/// reviewed decision could reach a different verdict than the sweep that
/// proposed it.
pub fn catalog_digests(
    ctx: &ExecContext,
    platform_id: &str,
    path: &std::path::Path,
) -> Result<retro_junk_archive::FileDigests, WorkError> {
    let raw = retro_junk_archive::hash_file_digests(path, &AtomicBool::new(false))
        .map_err(WorkError::msg)?;
    let normalized = ctx
        .analyzers
        .get_by_short_name(platform_id)
        .and_then(|console| {
            let mut input = std::fs::File::open(path).ok()?;
            let hashes = retro_junk_lib::hasher::compute_all_hashes(
                &mut input,
                console.analyzer.as_ref(),
                Some(path),
            )
            .map_err(|error| {
                log::debug!("{}: {error}; comparing raw digests instead", path.display());
            })
            .ok()?;
            Some(retro_junk_archive::FileDigests {
                size: hashes.data_size,
                crc32: hashes.crc32,
                md5: hashes.md5.unwrap_or_default(),
                sha1: hashes.sha1.unwrap_or_default(),
                sha256: raw.sha256.clone(),
            })
        });
    Ok(normalized.unwrap_or(raw))
}

/// The platform a playable path sits under — its first directory component,
/// which is how the playable library is laid out.
#[must_use]
pub fn platform_of(relative_path: &str) -> String {
    let directory = std::path::Path::new(relative_path)
        .components()
        .next()
        .and_then(|component| component.as_os_str().to_str())
        .unwrap_or_default();
    catalog_platform(directory)
}

/// The catalog stores `Platform::short_name()` as `releases.platform_id`,
/// while playable directories use frontend spellings (`psx`, `gc`, `sfc`) and
/// archive releases regional ones (`snesna`, `megadrivejp`). Catalog queries
/// compare with exact SQL equality, so anything headed there must fold onto
/// the catalog's spelling first. An unrecognized name passes through, so a
/// stranger still files for review under the name the user can see.
#[must_use]
pub fn catalog_platform(name: &str) -> String {
    retro_junk_core::catalog_platform_id(name)
}

/// Stop filing reviews for every file a pattern covers, and close the ones
/// already open.
///
/// Two halves, and both are needed: the durable rule stops the *next* sweep
/// from re-filing these, and closing the open rows clears the ones already
/// filed. A rule without the second half would leave the backlog untouched
/// until the next sweep; closing rows without the first is the treadmill this
/// exists to end.
pub fn ignore_playables(
    ctx: &ExecContext,
    pattern: &str,
    note: &str,
) -> Result<IgnoreOutcome, WorkError> {
    let rule = retro_junk_archive::IgnoreRule::new(pattern, note);
    if rule.is_empty() {
        return Err(WorkError::Message(
            "an ignore rule needs a pattern; an empty one would cover the whole library".to_owned(),
        ));
    }
    retro_junk_archive::write_rule(&ctx.profile.collection_root(), &rule)
        .map_err(|error| WorkError::Message(error.to_string()))?;

    let mut conn = retro_junk_db::open_database(&ctx.db_path).map_err(WorkError::msg)?;
    let matcher = retro_junk_io::glob::Pattern::new(&rule.pattern);
    let open = retro_junk_db::work::list_open_suggestions(&conn, Some(ADOPT_SUGGESTION_KIND))?;
    let covered: Vec<i64> = open
        .iter()
        .filter(|suggestion| matcher.matches(&suggestion.target_id))
        .map(|suggestion| suggestion.id)
        .collect();
    let dismissed = retro_junk_db::work::resolve_suggestions(&mut conn, &covered, "ignored")?;
    Ok(IgnoreOutcome {
        rule,
        dismissed: dismissed.len(),
    })
}

/// What an ignore rule did when it was recorded.
#[derive(Debug, Clone)]
pub struct IgnoreOutcome {
    pub rule: retro_junk_archive::IgnoreRule,
    /// Open reviews the new rule covered, now closed.
    pub dismissed: usize,
}

/// Revoke an ignore rule. The next sweep files those files again.
pub fn unignore_playables(ctx: &ExecContext, pattern: &str) -> Result<bool, WorkError> {
    let rule = retro_junk_archive::IgnoreRule::new(pattern, "");
    retro_junk_archive::remove_rule(&ctx.profile.collection_root(), &rule)
        .map_err(|error| WorkError::Message(error.to_string()))
}

/// Every ignore rule in force for this collection.
pub fn ignore_rules(ctx: &ExecContext) -> Result<Vec<retro_junk_archive::IgnoreRule>, WorkError> {
    retro_junk_archive::load_rules(&ctx.profile.collection_root())
        .map_err(|error| WorkError::Message(error.to_string()))
}
