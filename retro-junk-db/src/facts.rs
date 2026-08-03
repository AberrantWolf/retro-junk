//! Semantics-free completion facts.
//!
//! This module answers "what is true" about each archive release — what the
//! manifests claim, what the catalog resolves, what is on disk, what evidence
//! exists — and nothing else. What those facts *mean* (complete, incomplete,
//! needs attention) is decided in exactly one place, the backend's completion
//! fold. SQL here stores and fetches facts; it does not define status.
//!
//! The per-dump verification states (`dump_events.integrity_state`,
//! `catalog_state`) are already the single strict derivation from evidence
//! files — see `retro_junk_archive::evidence` — so this module reads them
//! rather than re-deriving the rule from `verification_events`.

use std::collections::HashMap;

use rusqlite::Connection;

use crate::OperationError;

/// What the catalog says a complete set of discs looks like for a release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpectedDiscs {
    /// Distinct numbered discs, or 1 for single-medium releases.
    pub count: u64,
    /// Whether the catalog numbers the discs (multi-disc sets) — decides
    /// whether verification is counted per disc number or per carrier.
    pub numbered: bool,
}

/// Facts about one carrier (one physical disc/cartridge in one copy).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CarrierFacts {
    pub carrier_id: String,
    pub physical_copy_id: String,
    /// The catalog medium this carrier resolved to, when it did.
    pub catalog_media_id: Option<String>,
    /// The medium id the carrier's manifest claims, resolved or not.
    pub claimed_media_id: String,
    /// The bound medium's disc number (0 when the catalog doesn't number it).
    pub disc_number: Option<i64>,
    /// Preservation-master representations recorded for this carrier.
    pub masters_recorded: u64,
    /// Of those, how many are present on disk right now (per the last
    /// projection pass).
    pub masters_present: u64,
    /// Whether any current dump evidence verifies the stored bytes.
    pub integrity_verified: bool,
    /// Whether current evidence catalog-verifies the dump (complete track
    /// set enforced at the evidence layer).
    pub catalog_verified: bool,
}

/// A built playable, with everything the naming rule needs to say whether
/// its name is still the right one.
///
/// The comparison itself is not made here: naming is a rule about what a
/// file should be called, and this module only reports what is true.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayableNameFacts {
    pub representation_id: String,
    /// Path relative to the playable root, as the projection recorded it.
    pub relative_path: String,
    /// The catalog's name for the game, empty when the carrier is unbound.
    pub dat_name: String,
    /// The catalog's filename for the medium (or its largest track).
    pub rom_name: String,
    /// Whether the catalog stores this medium as separate track files.
    pub medium_has_tracks: bool,
    pub disc_number: u32,
}

/// Everything the completion fold needs to know about one archive release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseFacts {
    pub archive_release_id: String,
    pub platform_id: String,
    pub title: String,
    pub region: String,
    pub revision: String,
    /// Resolved catalog identity, when the local catalog has the row.
    pub catalog_release_id: Option<String>,
    pub catalog_work_id: Option<String>,
    /// What the manifest on disk claims, independent of local resolution.
    pub claimed_release_id: String,
    pub claimed_work_id: String,
    /// Present when the catalog can say how many discs a complete set has.
    pub expected_discs: Option<ExpectedDiscs>,
    pub carriers: Vec<CarrierFacts>,
    /// Playable-policy scopes asking for a playable, and how many are
    /// satisfied by a present playable in the requested format.
    pub desired_playables: u64,
    pub satisfied_playables: u64,
    /// Playables whose evidence says they exist but whose file is missing.
    pub missing_playables: u64,
    /// Artwork/video asset types archived for this release.
    pub archived_asset_types: Vec<String>,
    /// Built playables and their naming inputs, for the conformance check.
    pub playable_names: Vec<PlayableNameFacts>,
}

/// How many of a release's expected discs one physical copy covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyDiscCoverage {
    pub physical_copy_id: String,
    /// Discs whose preservation master is present on disk.
    pub archived: u64,
    /// Discs that are catalog-verified *and* whose master is present. A
    /// verification of bytes that are no longer there proves nothing about
    /// what the collection currently holds.
    pub verified: u64,
}

/// The one disc-counting rule.
///
/// A release is complete when a *single* physical copy covers every expected
/// disc — discs from different copies do not pool, because half of one boxed
/// set plus half of another is not a playable game. A disc counts once no
/// matter how many carriers hold it, and a carrier whose master is missing
/// from disk cannot vouch for its disc.
///
/// Everything that needs disc counts calls this: the completion fold that
/// decides what the UI shows, and the convergence derivation that decides
/// what to build and why a build is blocked. There is no second definition.
#[must_use]
pub fn copy_disc_coverage(facts: &ReleaseFacts) -> Vec<CopyDiscCoverage> {
    let Some(expected) = facts.expected_discs else {
        return Vec::new();
    };
    // Ordered so the result is stable for callers that compare or display it.
    let mut order: Vec<&str> = Vec::new();
    let mut archived: HashMap<&str, std::collections::HashSet<i64>> = HashMap::new();
    let mut verified: HashMap<&str, std::collections::HashSet<i64>> = HashMap::new();
    for carrier in &facts.carriers {
        if carrier.masters_present == 0 {
            continue;
        }
        let disc_key = if expected.numbered {
            match carrier.disc_number {
                Some(number) if number > 0 => number,
                // A carrier bound to an unnumbered medium in a numbered set
                // cannot say which disc it is.
                _ => continue,
            }
        } else {
            0
        };
        let copy = carrier.physical_copy_id.as_str();
        if !archived.contains_key(copy) {
            order.push(copy);
        }
        archived.entry(copy).or_default().insert(disc_key);
        if carrier.catalog_verified {
            verified.entry(copy).or_default().insert(disc_key);
        }
    }
    order
        .into_iter()
        .map(|copy| CopyDiscCoverage {
            physical_copy_id: copy.to_owned(),
            archived: archived.get(copy).map_or(0, |discs| discs.len() as u64),
            verified: verified.get(copy).map_or(0, |discs| discs.len() as u64),
        })
        .collect()
}

/// Expected discs covered by the best single physical copy.
#[must_use]
pub fn archived_disc_count(facts: &ReleaseFacts) -> u64 {
    copy_disc_coverage(facts)
        .into_iter()
        .map(|copy| copy.archived)
        .max()
        .unwrap_or(0)
}

/// Catalog-verified discs held by the best single physical copy.
#[must_use]
pub fn verified_disc_count(facts: &ReleaseFacts) -> u64 {
    copy_disc_coverage(facts)
        .into_iter()
        .map(|copy| copy.verified)
        .max()
        .unwrap_or(0)
}

/// Expected discs per release: direct for release-bound rows, through the
/// natural key for work-bound ones.
const EXPECTED_DISCS_SQL: &str = "SELECT ar.id,
                    CASE WHEN MAX(m.disc_number)>0
                         THEN COUNT(DISTINCT CASE WHEN m.disc_number>0 THEN m.disc_number END)
                         ELSE 1 END,
                    MAX(m.disc_number)>0
             FROM archive_releases ar
             JOIN archive_profiles ap ON ap.id=ar.profile_id
             JOIN media m ON m.release_id=ar.catalog_release_id
             WHERE (?1='' OR ar.profile_id=?1)
               AND (?2='' OR ap.playable_root=?2)
               AND ar.catalog_release_id IS NOT NULL
             GROUP BY ar.id
             UNION ALL
             SELECT ar.id,
                    CASE WHEN MAX(m.disc_number)>0
                         THEN COUNT(DISTINCT CASE WHEN m.disc_number>0 THEN m.disc_number END)
                         ELSE 1 END,
                    MAX(m.disc_number)>0
             FROM archive_releases ar
             JOIN archive_profiles ap ON ap.id=ar.profile_id
             CROSS JOIN releases r INDEXED BY idx_releases_natural
             JOIN media m ON m.release_id=r.id
             WHERE r.work_id=ar.catalog_work_id
               AND r.platform_id=ar.platform_id
               AND r.region=ar.region
               AND (?1='' OR ar.profile_id=?1)
               AND (?2='' OR ap.playable_root=?2)
               AND ar.catalog_release_id IS NULL
               AND ar.catalog_work_id IS NOT NULL
             GROUP BY ar.id";

/// Which archive releases to gather facts for.
///
/// Both selectors are matched as "empty means any", so a profile scope and a
/// playable-root scope compose: the Library page asks by playable root (it
/// knows the console's root, not the profile id), while the Collection page
/// and convergence ask by profile.
#[derive(Debug, Clone, Default)]
pub struct FactsScope {
    pub profile_id: String,
    pub playable_root: String,
}

impl FactsScope {
    #[must_use]
    pub fn profile(profile_id: &str) -> Self {
        Self {
            profile_id: profile_id.to_owned(),
            playable_root: String::new(),
        }
    }

    #[must_use]
    pub fn playable_root(playable_root: &str) -> Self {
        Self {
            profile_id: String::new(),
            playable_root: playable_root.to_owned(),
        }
    }
}

/// Fetch completion facts for every archive release in a profile.
pub fn release_completion_facts(
    conn: &Connection,
    profile_id: &str,
) -> Result<Vec<ReleaseFacts>, OperationError> {
    release_facts_in_scope(conn, &FactsScope::profile(profile_id))
}

/// Fetch completion facts for the releases a scope selects, keyed by
/// archive release id.
pub fn release_facts_by_id(
    conn: &Connection,
    scope: &FactsScope,
) -> Result<HashMap<String, ReleaseFacts>, OperationError> {
    Ok(release_facts_in_scope(conn, scope)?
        .into_iter()
        .map(|facts| (facts.archive_release_id.clone(), facts))
        .collect())
}

#[allow(clippy::too_many_lines)]
pub fn release_facts_in_scope(
    conn: &Connection,
    scope: &FactsScope,
) -> Result<Vec<ReleaseFacts>, OperationError> {
    let profile_id = scope.profile_id.as_str();
    let playable_root = scope.playable_root.as_str();
    // Base rows.
    let mut releases: Vec<ReleaseFacts> = Vec::new();
    let mut index_of: HashMap<String, usize> = HashMap::new();
    {
        let mut statement = conn.prepare(
            "SELECT ar.id,ar.platform_id,ar.title,ar.region,ar.revision,
                    ar.catalog_release_id,ar.catalog_work_id,
                    ar.claimed_release_id,ar.claimed_work_id
             FROM archive_releases ar
             JOIN archive_profiles ap ON ap.id=ar.profile_id
             WHERE (?1='' OR ar.profile_id=?1)
               AND (?2='' OR ap.playable_root=?2)
             ORDER BY ar.platform_id,ar.title COLLATE NOCASE,ar.id",
        )?;
        for row in statement.query_map([profile_id, playable_root], |row| {
            Ok(ReleaseFacts {
                archive_release_id: row.get(0)?,
                platform_id: row.get(1)?,
                title: row.get(2)?,
                region: row.get(3)?,
                revision: row.get(4)?,
                catalog_release_id: row.get(5)?,
                catalog_work_id: row.get(6)?,
                claimed_release_id: row.get(7)?,
                claimed_work_id: row.get(8)?,
                expected_discs: None,
                carriers: Vec::new(),
                desired_playables: 0,
                satisfied_playables: 0,
                missing_playables: 0,
                archived_asset_types: Vec::new(),
                playable_names: Vec::new(),
            })
        })? {
            let facts = row?;
            index_of.insert(facts.archive_release_id.clone(), releases.len());
            releases.push(facts);
        }
    }

    // Expected discs from the catalog: directly for release-bound rows, via
    // the natural key for work-bound rows.
    {
        let mut statement = conn.prepare(EXPECTED_DISCS_SQL)?;
        for row in statement.query_map([profile_id, playable_root], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, bool>(2)?,
            ))
        })? {
            let (id, count, numbered) = row?;
            if let Some(&index) = index_of.get(&id) {
                releases[index].expected_discs = Some(ExpectedDiscs { count, numbered });
            }
        }
    }

    // Carrier facts, with master and evidence rollups.
    {
        let mut statement = conn.prepare(
            "SELECT pc.archive_release_id,c.id,pc.id,
                    c.catalog_media_id,c.claimed_media_id,m.disc_number,
                    (SELECT COUNT(*) FROM representations rep
                     WHERE rep.carrier_id=c.id AND rep.role='preservation_master'),
                    (SELECT COUNT(*) FROM representations rep
                     WHERE rep.carrier_id=c.id AND rep.role='preservation_master'
                       AND rep.presence_state='present'),
                    EXISTS(SELECT 1 FROM dump_events de
                           WHERE de.carrier_id=c.id AND de.integrity_state='verified'),
                    EXISTS(SELECT 1 FROM dump_events de
                           WHERE de.carrier_id=c.id AND de.catalog_state='verified')
             FROM physical_copies pc
             JOIN archive_releases ar ON ar.id=pc.archive_release_id
             JOIN archive_profiles ap ON ap.id=ar.profile_id
             JOIN carriers c ON c.physical_copy_id=pc.id
             LEFT JOIN media m ON m.id=c.catalog_media_id
             WHERE (?1='' OR ar.profile_id=?1)
               AND (?2='' OR ap.playable_root=?2)
             ORDER BY pc.archive_release_id,pc.id,c.sequence_number,c.id",
        )?;
        for row in statement.query_map([profile_id, playable_root], |row| {
            Ok((
                row.get::<_, String>(0)?,
                CarrierFacts {
                    carrier_id: row.get(1)?,
                    physical_copy_id: row.get(2)?,
                    catalog_media_id: row.get(3)?,
                    claimed_media_id: row.get(4)?,
                    disc_number: row.get(5)?,
                    masters_recorded: row.get(6)?,
                    masters_present: row.get(7)?,
                    integrity_verified: row.get(8)?,
                    catalog_verified: row.get(9)?,
                },
            ))
        })? {
            let (id, carrier) = row?;
            if let Some(&index) = index_of.get(&id) {
                releases[index].carriers.push(carrier);
            }
        }
    }

    // Playable policy demand and satisfaction, and missing built playables.
    {
        let mut statement = conn.prepare(
            "SELECT pc.archive_release_id,
                    COUNT(DISTINCT pp.scope_id),
                    COUNT(DISTINCT CASE
                         WHEN rep.role='playable'
                          AND rep.presence_state='present'
                          AND rep.format=pp.format
                         THEN c.id END),
                    (SELECT COUNT(*) FROM representations mrep
                     JOIN carriers mc ON mc.id=mrep.carrier_id
                     JOIN physical_copies mpc ON mpc.id=mc.physical_copy_id
                     WHERE mpc.archive_release_id=pc.archive_release_id
                       AND mrep.role='playable' AND mrep.presence_state='missing')
             FROM physical_copies pc
             JOIN archive_releases ar ON ar.id=pc.archive_release_id
             JOIN archive_profiles ap ON ap.id=ar.profile_id
             JOIN carriers c ON c.physical_copy_id=pc.id
             JOIN playable_policies pp
               ON pp.scope_type='carrier' AND pp.scope_id=c.id
             LEFT JOIN representations rep ON rep.carrier_id=c.id
             WHERE (?1='' OR ar.profile_id=?1)
               AND (?2='' OR ap.playable_root=?2)
             GROUP BY pc.archive_release_id",
        )?;
        for row in statement.query_map([profile_id, playable_root], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, u64>(2)?,
                row.get::<_, u64>(3)?,
            ))
        })? {
            let (id, desired, satisfied, missing) = row?;
            if let Some(&index) = index_of.get(&id) {
                releases[index].desired_playables = desired;
                releases[index].satisfied_playables = satisfied;
                releases[index].missing_playables = missing;
            }
        }
    }

    // Built playables, with the catalog's naming inputs for each.
    {
        let mut statement = conn.prepare(
            "SELECT pc.archive_release_id,rep.id,rep.relative_path,
                    COALESCE(m.dat_name,''),COALESCE(m.rom_name,''),
                    EXISTS(SELECT 1 FROM media_tracks mt WHERE mt.media_id=m.id),
                    COALESCE(c.sequence_number,0)
             FROM representations rep
             JOIN carriers c ON c.id=rep.carrier_id
             JOIN physical_copies pc ON pc.id=c.physical_copy_id
             JOIN archive_releases ar ON ar.id=pc.archive_release_id
             JOIN archive_profiles ap ON ap.id=ar.profile_id
             LEFT JOIN media m ON m.id=c.catalog_media_id
             WHERE (?1='' OR ar.profile_id=?1)
               AND (?2='' OR ap.playable_root=?2)
               AND rep.role='playable'
               AND rep.presence_state='present'
               AND rep.relative_path<>''
             ORDER BY pc.archive_release_id,rep.id",
        )?;
        for row in statement.query_map([profile_id, playable_root], |row| {
            Ok((
                row.get::<_, String>(0)?,
                PlayableNameFacts {
                    representation_id: row.get(1)?,
                    relative_path: row.get(2)?,
                    dat_name: row.get(3)?,
                    rom_name: row.get(4)?,
                    medium_has_tracks: row.get(5)?,
                    disc_number: row.get::<_, i64>(6)?.try_into().unwrap_or(0),
                },
            ))
        })? {
            let (id, playable) = row?;
            if let Some(&index) = index_of.get(&id) {
                releases[index].playable_names.push(playable);
            }
        }
    }

    // Archived artwork types.
    {
        let mut statement = conn.prepare(
            "SELECT f.archive_release_id,f.asset_type
             FROM archive_release_files f
             JOIN archive_releases ar ON ar.id=f.archive_release_id
             JOIN archive_profiles ap ON ap.id=ar.profile_id
             WHERE (?1='' OR ar.profile_id=?1)
               AND (?2='' OR ap.playable_root=?2)
               AND f.category IN ('artwork','video')
             ORDER BY f.archive_release_id,f.asset_type",
        )?;
        for row in statement.query_map([profile_id, playable_root], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })? {
            let (id, asset_type) = row?;
            if let Some(&index) = index_of.get(&id) {
                releases[index].archived_asset_types.push(asset_type);
            }
        }
    }

    Ok(releases)
}

#[cfg(test)]
mod query_plan_tests {
    use super::*;

    /// The work-level arm resolves through the natural-key index. Without
    /// the explicit index choice the planner picks a full media scan, which
    /// on a real catalog is the difference between a page that opens and one
    /// that hangs.
    #[test]
    fn work_level_expectation_never_scans_the_catalog_media_table() {
        let connection = crate::schema::open_memory().unwrap();
        let explain = format!("EXPLAIN QUERY PLAN {EXPECTED_DISCS_SQL}");
        let mut statement = connection.prepare(&explain).unwrap();
        let details = statement
            .query_map(rusqlite::params!["profile", ""], |row| {
                row.get::<_, String>(3)
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            details
                .iter()
                .any(|detail| detail.contains("idx_releases_natural")),
            "work lookup lost its catalog release index: {details:#?}"
        );
        assert!(
            details
                .iter()
                .all(|detail| detail != "SCAN m" && !detail.starts_with("SCAN m ")),
            "expected-disc lookup regressed to a full media scan: {details:#?}"
        );
    }
}
