//! Convergence derivation: compute, from the SQL projection alone, every
//! action that moves a profile toward "fully verified archive, fully
//! playable set, current frontend projections".
//!
//! This is the one definition of pending work. The Library view, `sync`,
//! `status`, and the daemon all consume it; executors re-validate against
//! the authoritative manifests when they run, so a stale projection fails
//! safe rather than building the wrong thing. Derivation is pure SQL — no
//! archive walk, cheap enough for a 30-second daemon tick over a network
//! mount.

use std::collections::{BTreeMap, BTreeSet};

use retro_junk_frontend::{AssetSelection, AssetType};
use rusqlite::{Connection, OptionalExtension, params};

use crate::library::{
    ArchivedPlayableGap, ArchivedScrapeIdentity, GapScope, LibraryError, ScrapeIdentityTier,
    query_playable_gaps,
};

/// What a proposed action would do. `#[non_exhaustive]`: miximage staleness
/// derivation arrives in a later phase.
///
/// Declaration order is worker stage order, and — through the derived `Ord` —
/// both the sort order of derived actions and the chip order in the GUI
/// backlog strip. Keep it in the order work actually happens.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ActionKind {
    /// Re-hash a preservation dump's stored bytes and append evidence.
    VerifyIntegrity,
    /// Catalog-verify a single-file master by normalized hashes.
    VerifyCatalog,
    /// Reproduce a Redumper raw master and bind its complete track set.
    AuditRedumper,
    /// Build the preferred playable representation for a release (includes
    /// the playlist when the set completes, plus asset/gamelist projection).
    BuildPlayable,
    /// Fetch missing artwork from `ScreenScraper` into the archive.
    Scrape,
    /// Project archived artwork originals to the frontend media tree.
    ProjectAssets,
    /// Upsert the release's ES-DE gamelist entry.
    SyncGamelist,
}

impl ActionKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::VerifyIntegrity => "verify_integrity",
            Self::VerifyCatalog => "verify_catalog",
            Self::AuditRedumper => "audit_redumper",
            Self::BuildPlayable => "build",
            Self::Scrape => "scrape",
            Self::ProjectAssets => "project_assets",
            Self::SyncGamelist => "sync_gamelist",
        }
    }

    /// All kinds in worker stage order: verification unlocks builds, builds
    /// unlock projections.
    #[must_use]
    pub fn all() -> &'static [ActionKind] {
        &[
            Self::VerifyIntegrity,
            Self::VerifyCatalog,
            Self::AuditRedumper,
            Self::BuildPlayable,
            Self::Scrape,
            Self::ProjectAssets,
            Self::SyncGamelist,
        ]
    }
}

impl std::str::FromStr for ActionKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "verify_integrity" | "verify-integrity" | "integrity" => Ok(Self::VerifyIntegrity),
            "verify_catalog" | "verify-catalog" | "catalog" => Ok(Self::VerifyCatalog),
            "audit_redumper" | "audit-redumper" | "audit" => Ok(Self::AuditRedumper),
            "build" | "build_playable" => Ok(Self::BuildPlayable),
            "scrape" | "artwork" => Ok(Self::Scrape),
            "project_assets" | "project" => Ok(Self::ProjectAssets),
            "sync_gamelist" | "gamelist" => Ok(Self::SyncGamelist),
            other => Err(format!("unknown action kind '{other}'")),
        }
    }
}

/// What an action operates on. Serialized onto the string-typed
/// coordination tables via [`WorkTarget::kind`] and [`WorkTarget::id`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkTarget {
    /// A preservation dump (`dump_events.id`).
    Dump(String),
    /// An archive release (`archive_releases.id`).
    Release(String),
    /// A filesystem path (incoming packages).
    Path(String),
}

impl WorkTarget {
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Dump(_) => "dump",
            Self::Release(_) => "release",
            Self::Path(_) => "path",
        }
    }

    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::Dump(id) | Self::Release(id) | Self::Path(id) => id,
        }
    }
}

/// Why a derived action cannot run unattended right now. Blocked actions
/// are reported, never silently dropped — the matrix's stop boundaries made
/// visible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockedReason {
    /// No effective playable policy for the carrier/platform; converging
    /// requires the user to pick a preferred format first.
    NoPolicy,
    /// The archive does not hold every catalog-expected carrier.
    IncompleteArchive { have: u32, need: u32 },
    /// Nothing identifies the release to a scraper — no serial, no complete
    /// hash triple, not even a filename — so there is nothing to look up.
    NoScrapeIdentity,
}

impl std::fmt::Display for BlockedReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoPolicy => write!(f, "no preferred playable format is set"),
            Self::IncompleteArchive { have, need } => {
                write!(f, "archive holds {have} of {need} expected discs")
            }
            Self::NoScrapeIdentity => {
                write!(f, "no catalog identity to look up")
            }
        }
    }
}

/// One derived action.
#[derive(Debug, Clone)]
pub struct ProposedAction {
    pub kind: ActionKind,
    pub target: WorkTarget,
    pub profile_id: String,
    /// Archive (physical) platform of the owning release.
    pub platform_id: String,
    /// Frontend system directory the outputs belong to.
    pub playable_platform_id: String,
    /// Human-readable label ("Title (region)" or "Title (region) — disc 2").
    pub label: String,
    pub blocked: Option<BlockedReason>,
    /// Build payload for `BuildPlayable` actions.
    pub build: Option<ArchivedPlayableGap>,
}

/// What to derive over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    AllProfiles,
    Profile(String),
    Platform {
        profile_id: String,
        platform_id: String,
    },
    Release {
        archive_release_id: String,
    },
}

/// Derive every pending convergence action in scope, in worker stage order
/// (verifications, then builds, then projections).
pub fn derive_convergence(
    conn: &Connection,
    scope: &Scope,
    expected_assets: &AssetSelection,
) -> Result<Vec<ProposedAction>, LibraryError> {
    let mut actions = Vec::new();
    for profile_id in profiles_in_scope(conn, scope)? {
        derive_dump_actions(conn, &profile_id, &mut actions)?;
        derive_build_actions(conn, &profile_id, &mut actions)?;
        derive_scrape_actions(conn, &profile_id, expected_assets, &mut actions)?;
        derive_projection_actions(conn, &profile_id, &mut actions)?;
    }
    // Scope narrowing happens after derivation: the queries are
    // profile-scoped and cheap, and release/platform filters compose better
    // in one place than woven through every statement.
    match scope {
        Scope::AllProfiles | Scope::Profile(_) => {}
        Scope::Platform { platform_id, .. } => {
            actions.retain(|action| action.platform_id.eq_ignore_ascii_case(platform_id));
        }
        Scope::Release { archive_release_id } => {
            actions.retain(|action| {
                action.release_id(conn).as_deref() == Some(archive_release_id.as_str())
            });
        }
    }
    actions.sort_by_key(|action| action.kind);
    Ok(actions)
}

impl ProposedAction {
    /// The archive release an action belongs to.
    fn release_id(&self, conn: &Connection) -> Option<String> {
        release_for_target(conn, &self.target)
    }
}

/// The archive release a work target belongs to: direct for release
/// targets, resolved through the owning carrier for dump targets, and none
/// for filesystem paths (incoming packages are not archived yet).
///
/// One definition, so derivation's scope filter and the error/claim
/// surfaces group work the same way.
#[must_use]
pub fn release_for_target(conn: &Connection, target: &WorkTarget) -> Option<String> {
    match target {
        WorkTarget::Release(id) => Some(id.clone()),
        WorkTarget::Dump(dump_id) => conn
            .query_row(
                "SELECT ar.id FROM dump_events de
                 JOIN carriers c ON c.id=de.carrier_id
                 JOIN physical_copies pc ON pc.id=c.physical_copy_id
                 JOIN archive_releases ar ON ar.id=pc.archive_release_id
                 WHERE de.id=?1",
                [dump_id.as_str()],
                |row| row.get(0),
            )
            .optional()
            .ok()
            .flatten(),
        WorkTarget::Path(_) => None,
    }
}

/// Every open error, grouped by the archive release it belongs to.
///
/// Verification errors are recorded against a dump, but the UI shows one
/// row per release, so the grouping has to happen somewhere; doing it here
/// keeps [`release_for_target`] the only place that knows the join.
pub fn errors_by_release(
    conn: &Connection,
) -> Result<BTreeMap<String, Vec<(ActionKind, crate::work::WorkError)>>, LibraryError> {
    let mut grouped: BTreeMap<String, Vec<_>> = BTreeMap::new();
    let errors = crate::work::list_work_errors(conn).map_err(|error| match error {
        crate::operations::OperationError::Sqlite(error) => LibraryError::Sqlite(error),
        other => LibraryError::InvalidScanState(other.to_string()),
    })?;
    for error in errors {
        let Ok(kind) = error.action_kind.parse::<ActionKind>() else {
            continue;
        };
        let target = match error.target_kind.as_str() {
            "dump" => WorkTarget::Dump(error.target_id.clone()),
            "release" => WorkTarget::Release(error.target_id.clone()),
            _ => WorkTarget::Path(error.target_id.clone()),
        };
        if let Some(release_id) = release_for_target(conn, &target) {
            grouped.entry(release_id).or_default().push((kind, error));
        }
    }
    Ok(grouped)
}

fn profiles_in_scope(conn: &Connection, scope: &Scope) -> Result<Vec<String>, LibraryError> {
    let profiles = match scope {
        Scope::Profile(profile_id) | Scope::Platform { profile_id, .. } => {
            vec![profile_id.clone()]
        }
        Scope::Release { archive_release_id } => conn
            .query_row(
                "SELECT profile_id FROM archive_releases WHERE id=?1",
                [archive_release_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .into_iter()
            .collect(),
        Scope::AllProfiles => {
            let mut statement = conn.prepare("SELECT id FROM archive_profiles ORDER BY id")?;
            statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?
        }
    };
    Ok(profiles)
}

/// Newest dump per carrier with its verification states — the source rows
/// for the three verification kinds.
const NEWEST_DUMPS_SQL: &str = "
    SELECT de.id, de.format, de.integrity_state, de.catalog_state,
           COALESCE(c.catalog_media_id,''), ar.platform_id, ar.region, ar.title,
           c.sequence_number,
           (SELECT COUNT(*) FROM representation_files rf
            WHERE rf.representation_id=de.representation_id)
    FROM dump_events de
    JOIN carriers c ON c.id=de.carrier_id
    JOIN physical_copies pc ON pc.id=c.physical_copy_id
    JOIN archive_releases ar ON ar.id=pc.archive_release_id
    WHERE ar.profile_id=?1
      AND de.id=(SELECT newest.id FROM dump_events newest
                 WHERE newest.carrier_id=c.id
                 ORDER BY newest.captured_at DESC, newest.id DESC LIMIT 1)";

fn derive_dump_actions(
    conn: &Connection,
    profile_id: &str,
    actions: &mut Vec<ProposedAction>,
) -> Result<(), LibraryError> {
    let mut statement = conn.prepare(NEWEST_DUMPS_SQL)?;
    let rows = statement
        .query_map([profile_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, i64>(9)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (
        dump_id,
        format,
        integrity_state,
        catalog_state,
        catalog_media_id,
        platform_id,
        region,
        title,
        sequence,
        file_count,
    ) in rows
    {
        let label = dump_label(&title, &region, sequence);
        let playable_platform_id =
            retro_junk_frontend::esde::system_directory(&platform_id, Some(&region));
        let base = |kind: ActionKind| ProposedAction {
            kind,
            target: WorkTarget::Dump(dump_id.clone()),
            profile_id: profile_id.to_owned(),
            platform_id: platform_id.clone(),
            playable_platform_id: playable_platform_id.clone(),
            label: label.clone(),
            blocked: None,
            build: None,
        };
        if integrity_state != "verified" {
            actions.push(base(ActionKind::VerifyIntegrity));
        }
        let redumper_raw = format == "redumper_raw";
        if redumper_raw && (catalog_media_id.is_empty() || catalog_state != "verified") {
            actions.push(base(ActionKind::AuditRedumper));
        }
        if !redumper_raw && file_count == 1 && catalog_state != "verified" {
            actions.push(base(ActionKind::VerifyCatalog));
        }
    }
    Ok(())
}

fn derive_build_actions(
    conn: &Connection,
    profile_id: &str,
    actions: &mut Vec<ProposedAction>,
) -> Result<(), LibraryError> {
    let (gaps, _completeness) = query_playable_gaps(
        conn,
        &GapScope::Profile {
            profile_id: profile_id.to_owned(),
        },
    )?;
    for gap in gaps {
        if !gap.needs_playable && !gap.needs_playlist {
            // The gap list also carries incomplete-but-satisfied releases
            // for display; there is nothing to execute for them.
            continue;
        }
        let Some((platform_id, region)) = release_identity(conn, &gap.archive_release_id)? else {
            continue;
        };
        let blocked = if gap.needs_playable && gap.preferred_format.is_none() {
            Some(BlockedReason::NoPolicy)
        } else if gap.needs_playable && !gap.buildable {
            Some(BlockedReason::IncompleteArchive {
                have: gap.archived_disc_count,
                need: gap.expected_disc_count,
            })
        } else {
            None
        };
        actions.push(ProposedAction {
            kind: ActionKind::BuildPlayable,
            target: WorkTarget::Release(gap.archive_release_id.clone()),
            profile_id: profile_id.to_owned(),
            platform_id: platform_id.clone(),
            playable_platform_id: retro_junk_frontend::esde::system_directory(
                &platform_id,
                Some(&region),
            ),
            label: dump_label(&gap.title, &gap.region, 0),
            blocked,
            build: Some(gap),
        });
    }
    Ok(())
}

/// Releases with archived artwork and a present playable output owe current
/// frontend projections. The executor's projection is idempotent (existing
/// identical files are current, not errors); the worker throttles how often
/// these re-run.
const PROJECTION_CANDIDATES_SQL: &str = "
    SELECT DISTINCT ar.id, ar.platform_id, ar.region, ar.title,
           EXISTS(SELECT 1 FROM archive_release_files arf
                  WHERE arf.archive_release_id=ar.id AND arf.category='artwork')
    FROM archive_releases ar
    JOIN physical_copies pc ON pc.archive_release_id=ar.id
    JOIN carriers c ON c.physical_copy_id=pc.id
    JOIN representations rep ON rep.carrier_id=c.id
    WHERE ar.profile_id=?1
      AND rep.role='playable' AND rep.presence_state='present'";

fn derive_projection_actions(
    conn: &Connection,
    profile_id: &str,
    actions: &mut Vec<ProposedAction>,
) -> Result<(), LibraryError> {
    let mut statement = conn.prepare(PROJECTION_CANDIDATES_SQL)?;
    let rows = statement
        .query_map([profile_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, bool>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (release_id, platform_id, region, title, has_artwork) in rows {
        let playable_platform_id =
            retro_junk_frontend::esde::system_directory(&platform_id, Some(&region));
        let label = dump_label(&title, &region, 0);
        if has_artwork {
            actions.push(ProposedAction {
                kind: ActionKind::ProjectAssets,
                target: WorkTarget::Release(release_id.clone()),
                profile_id: profile_id.to_owned(),
                platform_id: platform_id.clone(),
                playable_platform_id: playable_platform_id.clone(),
                label: label.clone(),
                blocked: None,
                build: None,
            });
        }
        actions.push(ProposedAction {
            kind: ActionKind::SyncGamelist,
            target: WorkTarget::Release(release_id),
            profile_id: profile_id.to_owned(),
            platform_id,
            playable_platform_id,
            label,
            blocked: None,
            build: None,
        });
    }
    Ok(())
}

/// Every release's artwork holdings, whether or not it has any. The `LEFT
/// JOIN` is what makes a release with no artwork at all appear — an inner
/// join would silently exclude exactly the releases most in need of a scrape.
const RELEASE_ARTWORK_SQL: &str = "
    SELECT ar.id, ar.platform_id, ar.region, ar.title, COALESCE(arf.asset_type,'')
    FROM archive_releases ar
    LEFT JOIN archive_release_files arf
      ON arf.archive_release_id=ar.id AND arf.category IN ('artwork','video')
    WHERE ar.profile_id=?1";

/// One release's artwork position against the expected set.
#[derive(Debug, Clone)]
pub struct ScrapeGap {
    pub archive_release_id: String,
    pub platform_id: String,
    pub region: String,
    pub title: String,
    /// Expected types the archive already holds.
    pub have: Vec<AssetType>,
    /// Expected types it does not.
    pub missing: Vec<AssetType>,
    /// How well this release can be identified to a scraper.
    pub identity: ScrapeIdentityTier,
}

impl ScrapeGap {
    #[must_use]
    pub fn expected(&self) -> usize {
        self.have.len() + self.missing.len()
    }
}

/// Compare every release's archived artwork against the expected set.
///
/// One definition of "what artwork does this release owe", shared by
/// derivation, the summary's done count, and the executor. `Miximage` is
/// excluded throughout: it is composed locally from the others, so expecting
/// it from a scraper would make every release permanently incomplete.
pub fn scrape_gaps(
    conn: &Connection,
    profile_id: &str,
    expected: &AssetSelection,
) -> Result<Vec<ScrapeGap>, LibraryError> {
    let expected: Vec<AssetType> = expected
        .types
        .iter()
        .copied()
        .filter(|asset_type| *asset_type != AssetType::Miximage)
        .collect();
    if expected.is_empty() {
        return Ok(Vec::new());
    }
    let identities = crate::library::query_archived_scrape_identities(conn, profile_id)?;

    let mut statement = conn.prepare(RELEASE_ARTWORK_SQL)?;
    let rows = statement
        .query_map([profile_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut releases: BTreeMap<String, (String, String, String, BTreeSet<AssetType>)> =
        BTreeMap::new();
    for (release_id, platform_id, region, title, asset_type) in rows {
        let entry = releases
            .entry(release_id)
            .or_insert_with(|| (platform_id, region, title, BTreeSet::new()));
        if let Some(asset_type) = AssetType::from_archive_name(&asset_type) {
            entry.3.insert(asset_type);
        }
    }

    Ok(releases
        .into_iter()
        .map(|(release_id, (platform_id, region, title, held))| {
            let (have, missing) = expected
                .iter()
                .partition::<Vec<_>, _>(|asset_type| held.contains(asset_type));
            ScrapeGap {
                identity: identities
                    .get(&release_id)
                    .map_or(ScrapeIdentityTier::None, ArchivedScrapeIdentity::tier),
                archive_release_id: release_id,
                platform_id,
                region,
                title,
                have,
                missing,
            }
        })
        .collect())
}

/// Releases missing expected artwork owe a scrape.
///
/// Unlike the projection kinds this does *not* require a present playable:
/// an archive-only release is scrapeable from its catalog identity, and
/// waiting for a build before fetching its box art would leave the collection
/// view blank for everything not yet built.
fn derive_scrape_actions(
    conn: &Connection,
    profile_id: &str,
    expected: &AssetSelection,
    actions: &mut Vec<ProposedAction>,
) -> Result<(), LibraryError> {
    for gap in scrape_gaps(conn, profile_id, expected)? {
        if gap.missing.is_empty() {
            continue;
        }
        actions.push(ProposedAction {
            kind: ActionKind::Scrape,
            target: WorkTarget::Release(gap.archive_release_id.clone()),
            profile_id: profile_id.to_owned(),
            playable_platform_id: retro_junk_frontend::esde::system_directory(
                &gap.platform_id,
                Some(&gap.region),
            ),
            label: dump_label(&gap.title, &gap.region, 0),
            blocked: (gap.identity == ScrapeIdentityTier::None)
                .then_some(BlockedReason::NoScrapeIdentity),
            platform_id: gap.platform_id,
            build: None,
        });
    }
    Ok(())
}

fn release_identity(
    conn: &Connection,
    archive_release_id: &str,
) -> Result<Option<(String, String)>, LibraryError> {
    Ok(conn
        .query_row(
            "SELECT platform_id, region FROM archive_releases WHERE id=?1",
            [archive_release_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?)
}

fn dump_label(title: &str, region: &str, sequence: i64) -> String {
    let mut label = if region.trim().is_empty() {
        title.to_owned()
    } else {
        format!("{title} ({region})")
    };
    if sequence > 0 {
        use std::fmt::Write as _;
        let _ = write!(label, " — disc {sequence}");
    }
    label
}

// ── Summary ────────────────────────────────────────────────────────────────

/// Per-kind convergence counts. `done` counts satisfied targets derivable
/// from the projection; `pending`/`blocked` come from derivation; `errored`
/// and `running` from the coordination tables.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KindCounts {
    pub done: u64,
    pub pending: u64,
    pub blocked: u64,
    pub errored: u64,
    pub running: u64,
}

#[derive(Debug, Clone, Default)]
pub struct ConvergenceSummary {
    pub per_kind: BTreeMap<ActionKind, KindCounts>,
    pub open_suggestions: u64,
}

/// The one aggregation behind `status`, daemon logging, and (later) the GUI
/// backlog strip.
pub fn summarize_convergence(
    conn: &Connection,
    scope: &Scope,
    expected_assets: &AssetSelection,
) -> Result<ConvergenceSummary, LibraryError> {
    let mut summary = ConvergenceSummary::default();
    for action in derive_convergence(conn, scope, expected_assets)? {
        let counts = summary.per_kind.entry(action.kind).or_default();
        if action.blocked.is_some() {
            counts.blocked += 1;
        } else {
            counts.pending += 1;
        }
    }
    for kind in ActionKind::all() {
        let counts = summary.per_kind.entry(*kind).or_default();
        counts.errored = count_rows(
            conn,
            "SELECT COUNT(*) FROM work_errors WHERE action_kind=?1",
            kind.as_str(),
        )?;
        counts.running = count_rows(
            conn,
            &format!(
                "SELECT COUNT(*) FROM work_claims WHERE action_kind=?1
                 AND since >= datetime('now','-{} minutes')",
                crate::work::CLAIM_TIMEOUT_MINUTES
            ),
            kind.as_str(),
        )?;
    }
    // Done counts for the verification kinds fall straight out of the
    // projected dump states; builds count satisfied complete releases.
    for profile_id in profiles_in_scope(conn, scope)? {
        let mut statement = conn.prepare(NEWEST_DUMPS_SQL)?;
        let rows = statement
            .query_map([profile_id.as_str()], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(9)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for (format, integrity_state, catalog_state, file_count) in rows {
            if integrity_state == "verified" {
                summary
                    .per_kind
                    .entry(ActionKind::VerifyIntegrity)
                    .or_default()
                    .done += 1;
            }
            if catalog_state == "verified" {
                let kind = if format == "redumper_raw" {
                    ActionKind::AuditRedumper
                } else {
                    ActionKind::VerifyCatalog
                };
                summary.per_kind.entry(kind).or_default().done += 1;
            } else if format != "redumper_raw" && file_count != 1 {
                // Multi-file non-redumper dumps have no automated catalog
                // path; they are neither done nor pending.
            }
        }
        let satisfied: u64 = conn.query_row(
            "SELECT COUNT(DISTINCT ar.id) FROM archive_releases ar
             JOIN physical_copies pc ON pc.archive_release_id=ar.id
             JOIN carriers c ON c.physical_copy_id=pc.id
             JOIN representations rep ON rep.carrier_id=c.id
             WHERE ar.profile_id=?1 AND rep.role='playable'
               AND rep.presence_state='present'",
            [profile_id.as_str()],
            |row| row.get(0),
        )?;
        summary
            .per_kind
            .entry(ActionKind::BuildPlayable)
            .or_default()
            .done += satisfied;
        let fully_scraped = scrape_gaps(conn, &profile_id, expected_assets)?
            .into_iter()
            .filter(|gap| gap.missing.is_empty())
            .count() as u64;
        summary.per_kind.entry(ActionKind::Scrape).or_default().done += fully_scraped;
    }
    summary.open_suggestions = conn.query_row(
        "SELECT COUNT(*) FROM suggestions WHERE resolved_at IS NULL",
        [],
        |row| row.get(0),
    )?;
    Ok(summary)
}

fn count_rows(conn: &Connection, sql: &str, param: &str) -> Result<u64, LibraryError> {
    Ok(conn.query_row(sql, params![param], |row| row.get(0))?)
}
