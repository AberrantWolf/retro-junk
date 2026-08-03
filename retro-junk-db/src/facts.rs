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
}

/// The one disc-counting rule.
///
/// Counts catalog-verified discs from a release's facts: the best physical
/// copy wins, a disc counts once no matter how many carriers hold it, and
/// only a carrier whose master is present on disk can vouch for its disc.
/// Both the completion fold (display) and convergence derivation (what to
/// build, and why something is blocked) call this — there is no second
/// definition anywhere.
#[must_use]
pub fn verified_disc_count(facts: &ReleaseFacts) -> u64 {
    let Some(expected) = facts.expected_discs else {
        return 0;
    };
    let mut best = 0_u64;
    let mut copies: HashMap<&str, std::collections::HashSet<i64>> = HashMap::new();
    for carrier in &facts.carriers {
        if !(carrier.catalog_verified && carrier.masters_present > 0) {
            continue;
        }
        let disc_key = if expected.numbered {
            match carrier.disc_number {
                Some(number) if number > 0 => number,
                // A verified carrier bound to an unnumbered medium in a
                // numbered set cannot say which disc it is.
                _ => continue,
            }
        } else {
            0
        };
        copies
            .entry(carrier.physical_copy_id.as_str())
            .or_default()
            .insert(disc_key);
    }
    for discs in copies.values() {
        best = best.max(discs.len() as u64);
    }
    best
}

/// Fetch completion facts for every archive release in a profile.
#[allow(clippy::too_many_lines)]
pub fn release_completion_facts(
    conn: &Connection,
    profile_id: &str,
) -> Result<Vec<ReleaseFacts>, OperationError> {
    // Base rows.
    let mut releases: Vec<ReleaseFacts> = Vec::new();
    let mut index_of: HashMap<String, usize> = HashMap::new();
    {
        let mut statement = conn.prepare(
            "SELECT id,platform_id,title,region,revision,
                    catalog_release_id,catalog_work_id,
                    claimed_release_id,claimed_work_id
             FROM archive_releases
             WHERE profile_id=?1
             ORDER BY platform_id,title COLLATE NOCASE,id",
        )?;
        for row in statement.query_map([profile_id], |row| {
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
        let mut statement = conn.prepare(
            "SELECT ar.id,
                    CASE WHEN MAX(m.disc_number)>0
                         THEN COUNT(DISTINCT CASE WHEN m.disc_number>0 THEN m.disc_number END)
                         ELSE 1 END,
                    MAX(m.disc_number)>0
             FROM archive_releases ar
             JOIN media m ON m.release_id=ar.catalog_release_id
             WHERE ar.profile_id=?1 AND ar.catalog_release_id IS NOT NULL
             GROUP BY ar.id
             UNION ALL
             SELECT ar.id,
                    CASE WHEN MAX(m.disc_number)>0
                         THEN COUNT(DISTINCT CASE WHEN m.disc_number>0 THEN m.disc_number END)
                         ELSE 1 END,
                    MAX(m.disc_number)>0
             FROM archive_releases ar
             JOIN releases r ON r.work_id=ar.catalog_work_id
                            AND r.platform_id=ar.platform_id
                            AND r.region=ar.region
             JOIN media m ON m.release_id=r.id
             WHERE ar.profile_id=?1
               AND ar.catalog_release_id IS NULL
               AND ar.catalog_work_id IS NOT NULL
             GROUP BY ar.id",
        )?;
        for row in statement.query_map([profile_id], |row| {
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
             JOIN carriers c ON c.physical_copy_id=pc.id
             LEFT JOIN media m ON m.id=c.catalog_media_id
             WHERE ar.profile_id=?1
             ORDER BY pc.archive_release_id,pc.id,c.sequence_number,c.id",
        )?;
        for row in statement.query_map([profile_id], |row| {
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
             JOIN carriers c ON c.physical_copy_id=pc.id
             JOIN playable_policies pp
               ON pp.scope_type='carrier' AND pp.scope_id=c.id
             LEFT JOIN representations rep ON rep.carrier_id=c.id
             WHERE ar.profile_id=?1
             GROUP BY pc.archive_release_id",
        )?;
        for row in statement.query_map([profile_id], |row| {
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

    // Archived artwork types.
    {
        let mut statement = conn.prepare(
            "SELECT f.archive_release_id,f.asset_type
             FROM archive_release_files f
             JOIN archive_releases ar ON ar.id=f.archive_release_id
             WHERE ar.profile_id=?1 AND f.category IN ('artwork','video')
             ORDER BY f.archive_release_id,f.asset_type",
        )?;
        for row in statement.query_map([profile_id], |row| {
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
