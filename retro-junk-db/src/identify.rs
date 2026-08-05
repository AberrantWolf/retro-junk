//! The one answer to "what game is this?".
//!
//! Every surface that used to ask its own version of this question — the DAT
//! matcher, the library scan, the archive's carrier resolution, the catalog
//! importer — funnels through [`identify`]. The ladder and the rules it
//! enforces are written down in `IDENTIFICATION.md` at the repository root;
//! this module is expected to match that document.
//!
//! Two rules hold at every rung, and they are the reason this is one function
//! rather than four:
//!
//! - **Ambiguity is never resolved by guessing.** More than one candidate
//!   yields [`Identification::Ambiguous`], never the first row of a sorted
//!   query.
//! - **Partial evidence never promotes.** One matching track out of five is
//!   evidence for a candidate list, never for an identity. A disc's identity
//!   is its complete ordered track set: 1029 catalog media rows share their
//!   primary track's hash with another row on the same platform, so the
//!   primary hash alone cannot be an identity.

#[cfg(test)]
#[path = "tests/identify_tests.rs"]
mod tests;

use rusqlite::Connection;

use crate::OperationError;
use crate::archive::{CompleteCatalogMediaMatch, match_complete_catalog_media};
use retro_junk_archive::TrackDigest;

/// One candidate the evidence cannot rule out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub media_id: String,
    pub release_id: String,
    pub work_id: String,
    /// The catalog's own name for it, for a chooser that must not invent one.
    pub game: String,
    pub region: String,
    pub revision: String,
    pub variant: String,
}

impl From<CompleteCatalogMediaMatch> for Candidate {
    fn from(matched: CompleteCatalogMediaMatch) -> Self {
        Self {
            media_id: matched.media_id,
            release_id: matched.release_id,
            work_id: matched.work_id,
            game: matched.game,
            region: matched.region,
            revision: matched.revision,
            variant: matched.variant,
        }
    }
}

/// Why an identification is less than fully verified.
///
/// Carried with the outcome so every surface explains the same gap the same
/// way, and each reason names its fix — the contract `UnknownReason::explain`
/// already holds for fractions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Incompleteness {
    /// The medium has tracks we have not hashed.
    TracksUnhashed { hashed: usize, total: usize },
    /// We hold the primary track's digests and no per-track digests, so one
    /// track of a multi-track disc vouched for the whole thing.
    PrimaryHashOnly,
    /// No DAT has been imported for this platform, so nothing can be verified.
    NoCatalogForPlatform,
    /// We can name a candidate, but a digest we hold contradicts it. Worse
    /// than not knowing: the file will probably not play correctly.
    HashesDisagree,
    /// A person selected this entry from its candidate list.
    ManuallyChosen,
    /// Homebrew, a ROM hack or a mod: no DAT will ever list it, so a complete
    /// match is unreachable and its absence is not a defect.
    NotCatalogued,
}

impl Incompleteness {
    /// One sentence: what is missing, and what closes it.
    #[must_use]
    pub fn explain(&self) -> String {
        match self {
            Self::TracksUnhashed { hashed, total } => {
                format!("{hashed} of {total} tracks hashed — hash the rest to verify this disc")
            }
            Self::PrimaryHashOnly => {
                "Only the largest track was hashed, which cannot tell this disc from another \
                 sharing that track — hash every track to verify it"
                    .to_owned()
            }
            Self::NoCatalogForPlatform => {
                "No DAT imported for this platform — import one to verify anything here".to_owned()
            }
            Self::HashesDisagree => {
                "A hash disagrees with the catalog entry this claims to be — the file is probably \
                 damaged or modified and may not play correctly"
                    .to_owned()
            }
            Self::ManuallyChosen => {
                "Identified by hand rather than verified — re-select it if this is the wrong entry"
                    .to_owned()
            }
            Self::NotCatalogued => {
                "Homebrew, a hack or a mod: no catalog lists it, so there is nothing to verify \
                 against"
                    .to_owned()
            }
        }
    }
}

/// The rungs of the ladder, in order. See `IDENTIFICATION.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Identification {
    /// Every track's digest and size agreed, in order.
    Complete {
        media_id: String,
        release_id: String,
    },
    /// The evidence left exactly one possible entry, but not every hash is
    /// verified.
    Unique {
        media_id: String,
        release_id: String,
        why: Incompleteness,
    },
    /// A person chose this entry from its candidate list.
    Manual {
        media_id: String,
        release_id: String,
        /// The candidates they chose from, so the choice stays re-selectable.
        candidates: Vec<Candidate>,
    },
    /// Several entries remain possible. Carries them so a chooser can offer
    /// exactly these and nothing else.
    Ambiguous { candidates: Vec<Candidate> },
    /// Nothing narrows it down.
    Unidentified { why: Incompleteness },
}

impl Identification {
    /// The catalog media this resolves to, when it resolves to one.
    #[must_use]
    pub fn media_id(&self) -> Option<&str> {
        match self {
            Self::Complete { media_id, .. }
            | Self::Unique { media_id, .. }
            | Self::Manual { media_id, .. } => Some(media_id),
            Self::Ambiguous { .. } | Self::Unidentified { .. } => None,
        }
    }

    /// The catalog release this resolves to, when it resolves to one.
    #[must_use]
    pub fn release_id(&self) -> Option<&str> {
        match self {
            Self::Complete { release_id, .. }
            | Self::Unique { release_id, .. }
            | Self::Manual { release_id, .. } => Some(release_id),
            Self::Ambiguous { .. } | Self::Unidentified { .. } => None,
        }
    }

    /// Whether the tool may rename this, build a playable from it, or scrape
    /// for it.
    ///
    /// The bar is an unambiguous identity, not a verified one: a single
    /// possible entry is a certain answer on cheaper evidence. Ambiguous and
    /// unidentified content is an error marker — it exists to be shown and
    /// fixed, never acted on.
    #[must_use]
    pub const fn is_actionable(&self) -> bool {
        matches!(
            self,
            Self::Complete { .. } | Self::Unique { .. } | Self::Manual { .. }
        )
    }

    /// Why this is not a verified identity, when it is not one.
    #[must_use]
    pub fn incompleteness(&self) -> Option<&Incompleteness> {
        match self {
            Self::Unique { why, .. } | Self::Unidentified { why, .. } => Some(why),
            Self::Manual { .. } => Some(&Incompleteness::ManuallyChosen),
            // A verified identity has no gap, and an ambiguous one is not a
            // gap in the answer but an absence of one.
            Self::Complete { .. } | Self::Ambiguous { .. } => None,
        }
    }

    /// The entries a chooser may offer. Empty unless a person has something to
    /// decide between.
    #[must_use]
    pub fn candidates(&self) -> &[Candidate] {
        match self {
            Self::Ambiguous { candidates } | Self::Manual { candidates, .. } => candidates,
            _ => &[],
        }
    }
}

/// What we know about one medium, as the caller measured it.
#[derive(Debug, Clone, Default)]
pub struct Evidence<'a> {
    pub platform_id: &'a str,
    /// Every track we hashed, in track order. A cartridge, a DVD or a
    /// single-track disc is the one-element case and reaches `Complete` for
    /// free.
    pub tracks: Vec<TrackDigest>,
    /// How many tracks the medium actually has, when that is known and larger
    /// than what we hashed. `None` means what we hashed is all there is.
    pub total_tracks: Option<usize>,
    /// A choice a person already made for this content, if any.
    pub manual_media_id: Option<&'a str>,
    /// Content the user has declared uncatalogued — homebrew, a hack, a mod.
    pub not_catalogued: bool,
}

/// Identify one medium against the catalog.
///
/// The single entry point. Callers supply what they measured and get back
/// exactly one rung of the ladder.
pub fn identify(
    conn: &Connection,
    evidence: &Evidence<'_>,
) -> Result<Identification, OperationError> {
    if evidence.not_catalogued {
        return Ok(Identification::Unidentified {
            why: Incompleteness::NotCatalogued,
        });
    }
    if evidence.tracks.is_empty() {
        return Ok(Identification::Unidentified {
            why: Incompleteness::NoCatalogForPlatform,
        });
    }

    let hashed = evidence.tracks.len();
    let total = evidence.total_tracks.unwrap_or(hashed);

    // Rung 1: every track agreed. `match_complete_catalog_media` is
    // deliberately strict — it refuses to let one data track vouch for a
    // multi-track disc — so anything it returns is fully checked.
    let verified = match_complete_catalog_media(conn, evidence.platform_id, &evidence.tracks)?;
    let (candidates, fully_checked) = if verified.is_empty() {
        // Partial evidence is evidence for a *candidate list*, never for an
        // identity. Anything sharing a track we hold stays possible.
        (partial_candidates(conn, evidence)?, false)
    } else {
        (
            verified
                .into_iter()
                .map(Candidate::from)
                .collect::<Vec<_>>(),
            true,
        )
    };

    // A person's choice outranks the automatic answer, but only among the
    // entries that were actually possible — a chooser may not invent one.
    if let Some(chosen) = evidence.manual_media_id
        && let Some(found) = candidates
            .iter()
            .find(|candidate| candidate.media_id == chosen)
    {
        return Ok(Identification::Manual {
            media_id: found.media_id.clone(),
            release_id: found.release_id.clone(),
            candidates,
        });
    }

    match candidates.len() {
        0 => Ok(Identification::Unidentified {
            why: if catalog_has_platform(conn, evidence.platform_id)? {
                Incompleteness::HashesDisagree
            } else {
                Incompleteness::NoCatalogForPlatform
            },
        }),
        1 => {
            let found = candidates
                .into_iter()
                .next()
                .unwrap_or_else(|| unreachable!());
            // Verified only when every track agreed *and* we hashed everything
            // the medium has.
            let why = if hashed < total {
                Some(Incompleteness::TracksUnhashed { hashed, total })
            } else if !fully_checked {
                Some(Incompleteness::PrimaryHashOnly)
            } else {
                None
            };
            Ok(match why {
                Some(why) => Identification::Unique {
                    media_id: found.media_id,
                    release_id: found.release_id,
                    why,
                },
                None => Identification::Complete {
                    media_id: found.media_id,
                    release_id: found.release_id,
                },
            })
        }
        _ => Ok(Identification::Ambiguous { candidates }),
    }
}

/// Every catalog medium that shares at least one track with what we hashed.
///
/// The candidate list behind [`Identification::Ambiguous`]. This is what makes
/// partial evidence useful without letting it identify anything: two discs
/// sharing a data track both stay on the list until an audio track separates
/// them.
fn partial_candidates(
    conn: &Connection,
    evidence: &Evidence<'_>,
) -> Result<Vec<Candidate>, OperationError> {
    let mut ids = std::collections::BTreeSet::new();
    for track in &evidence.tracks {
        for media_id in crate::queries::match_media_ids_by_track_hash(
            conn,
            evidence.platform_id,
            track.size,
            &track.crc32,
            Some(track.sha1.as_str()),
        )? {
            ids.insert(media_id);
        }
        for matched in crate::queries::match_media_by_hash(
            conn,
            evidence.platform_id,
            track.size,
            Some(track.crc32.as_str()),
            Some(track.sha1.as_str()),
        )? {
            ids.insert(matched.media.id);
        }
    }
    load_candidates(conn, &ids.into_iter().collect::<Vec<_>>())
}

/// The catalog's own description of each candidate, for a chooser that must
/// not invent names.
fn load_candidates(conn: &Connection, ids: &[String]) -> Result<Vec<Candidate>, OperationError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT m.id,m.release_id,r.work_id,r.title,r.region,r.revision,r.variant
         FROM media m JOIN releases r ON r.id=m.release_id
         WHERE m.id IN ({placeholders}) ORDER BY m.id"
    );
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(rusqlite::params_from_iter(ids.iter()), |row| {
        Ok(Candidate {
            media_id: row.get(0)?,
            release_id: row.get(1)?,
            work_id: row.get(2)?,
            game: row.get(3)?,
            region: row.get(4)?,
            revision: row.get(5)?,
            variant: row.get(6)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Whether any catalog entry exists for a platform.
///
/// Distinguishes "your hashes disagree with the catalog" from "there is no
/// catalog to disagree with", which are different problems with different
/// fixes and used to render identically.
///
/// Public because scraping asks the same question: without a catalog entry to
/// speak for a game there is no DAT title or digest to search on, only
/// whatever the file happens to hash to, and the user is owed a warning that
/// results will be worse.
pub fn catalog_has_platform(conn: &Connection, platform_id: &str) -> Result<bool, OperationError> {
    if platform_id.is_empty() {
        return Ok(true);
    }
    let count: i64 = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM releases WHERE platform_id=?1)",
        [platform_id],
        |row| row.get(0),
    )?;
    Ok(count != 0)
}
