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
use rusqlite::{Connection, OptionalExtension};

use crate::library::{
    ArchivedPlayableGap, ArchivedScrapeIdentity, GapScope, LibraryError, ScrapeIdentityTier,
    query_forced_playable_gap, query_playable_gaps,
};
use crate::projection_state::{ProjectionOf, projection_is_current};

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
    /// Find a playable output that moved out from under its build evidence and
    /// record where it now lives. Ordered before [`Self::BuildPlayable`] on
    /// purpose: a moved file that is not re-adopted first reads as a build gap
    /// and gets rebuilt beside itself.
    AdoptPlayable,
    /// Build the preferred playable representation for a release (includes
    /// the playlist when the set completes, plus asset/gamelist projection).
    BuildPlayable,
    /// Rename a built playable whose name is no longer what the catalog
    /// calls it — the usual cause is a playable built before the naming
    /// rule was corrected, or a DAT that has since renamed the game.
    /// Ordered after building so a file is never renamed and then rebuilt
    /// under its old name in the same pass.
    RenamePlayable,
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
            Self::AdoptPlayable => "adopt_playable",
            Self::BuildPlayable => "build",
            Self::RenamePlayable => "rename_playable",
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
            Self::AdoptPlayable,
            Self::BuildPlayable,
            Self::RenamePlayable,
            Self::Scrape,
            Self::ProjectAssets,
            Self::SyncGamelist,
        ]
    }
}

impl ActionKind {
    /// Every spelling that names this kind, the canonical one first.
    ///
    /// One list, read by both the parser below and anything that offers the
    /// choices to a person — the CLI's `--only` and its help text. Two lists
    /// would eventually disagree about what is valid, and the way that shows
    /// up is a documented spelling being rejected.
    #[must_use]
    pub fn spellings(self) -> &'static [&'static str] {
        match self {
            Self::VerifyIntegrity => &["verify_integrity", "verify-integrity", "integrity"],
            Self::VerifyCatalog => &["verify_catalog", "verify-catalog", "catalog"],
            Self::AuditRedumper => &["audit_redumper", "audit-redumper", "audit"],
            Self::AdoptPlayable => &["adopt_playable", "adopt-playable", "adopt"],
            Self::BuildPlayable => &["build", "build_playable"],
            Self::RenamePlayable => &["rename_playable", "rename-playable", "rename"],
            Self::Scrape => &["scrape", "artwork"],
            Self::ProjectAssets => &["project_assets", "project"],
            Self::SyncGamelist => &["sync_gamelist", "gamelist"],
        }
    }
}

impl std::str::FromStr for ActionKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let spelling = value.trim().to_ascii_lowercase();
        Self::all()
            .iter()
            .copied()
            .find(|kind| kind.spellings().contains(&spelling.as_str()))
            .ok_or_else(|| format!("unknown action kind '{spelling}'"))
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
    /// One frontend system folder — that is, one `gamelist.xml` — written
    /// `"{profile_id}/{directory}"`.
    ///
    /// A gamelist is per folder, not per game, so this is the honest size of
    /// the work: targeting a release meant a folder holding a hundred games
    /// derived a hundred actions that rewrote one file. The profile is part of
    /// the id because two collections can both have a `psx` folder and their
    /// claims must not collide.
    Console(String),
    /// A filesystem path (incoming packages).
    Path(String),
}

impl WorkTarget {
    /// A console target for one profile's system folder.
    #[must_use]
    pub fn console(profile_id: &str, directory: &str) -> Self {
        Self::Console(format!("{profile_id}/{directory}"))
    }

    /// The profile and folder a console target names, or `None` for any other
    /// kind of target.
    #[must_use]
    pub fn console_parts(&self) -> Option<(&str, &str)> {
        match self {
            Self::Console(id) => id.split_once('/'),
            _ => None,
        }
    }

    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Dump(_) => "dump",
            Self::Release(_) => "release",
            Self::Console(_) => "console",
            Self::Path(_) => "path",
        }
    }

    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::Dump(id) | Self::Release(id) | Self::Console(id) | Self::Path(id) => id,
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
    /// A hand-picked set of archive releases — a multi-row selection in the
    /// GUI. Empty means nothing is in scope, not everything.
    Releases(Vec<String>),
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
        derive_adoption_actions(conn, &profile_id, &mut actions)?;
        derive_build_actions(conn, &profile_id, &mut actions)?;
        derive_scrape_actions(conn, &profile_id, expected_assets, &mut actions)?;
        derive_projection_actions(conn, &profile_id, &mut actions)?;
        derive_gamelist_actions(conn, &profile_id, &mut actions)?;
    }
    // Scope narrowing happens after derivation: the queries are
    // profile-scoped and cheap, and release/platform filters compose better
    // in one place than woven through every statement.
    let wanted = ScopeReleases::for_scope(scope);
    actions.retain(|action| wanted.admits(conn, action));
    actions.sort_by_key(|action| action.kind);
    Ok(actions)
}

/// Which archive releases a scope narrows to, resolved once for a whole
/// derivation rather than re-queried per action.
///
/// The per-action form ran one `release_for_target` query for every action in
/// the profile's entire backlog, on every GUI badge click — for a scope that
/// names a single release.
enum ScopeReleases {
    /// Nothing is filtered out.
    Everything,
    /// Only actions belonging to one of these releases.
    Only(BTreeSet<String>),
    /// Only actions whose owning release is on this archive platform.
    Platform(String),
}

impl ScopeReleases {
    fn for_scope(scope: &Scope) -> Self {
        match scope {
            Scope::AllProfiles | Scope::Profile(_) => Self::Everything,
            Scope::Platform { platform_id, .. } => Self::Platform(platform_id.clone()),
            Scope::Release { archive_release_id } => {
                Self::Only(std::iter::once(archive_release_id.clone()).collect())
            }
            Scope::Releases(ids) => Self::Only(ids.iter().cloned().collect()),
        }
    }

    fn admits(&self, conn: &Connection, action: &ProposedAction) -> bool {
        match self {
            Self::Everything => true,
            Self::Platform(platform_id) => {
                // A console action spans a whole frontend folder, which is
                // where one archive platform's releases land; the folder name
                // it carries is the answer for that platform.
                if action.target.console_parts().is_some() {
                    action
                        .playable_platform_id
                        .eq_ignore_ascii_case(platform_id)
                        || releases_for_target(conn, &action.target).iter().any(|id| {
                            release_platform(conn, id)
                                .is_some_and(|found| found.eq_ignore_ascii_case(platform_id))
                        })
                } else {
                    action.platform_id.eq_ignore_ascii_case(platform_id)
                }
            }
            Self::Only(wanted) => releases_for_target(conn, &action.target)
                .iter()
                .any(|id| wanted.contains(id)),
        }
    }
}

fn release_platform(conn: &Connection, archive_release_id: &str) -> Option<String> {
    conn.query_row(
        "SELECT platform_id FROM archive_releases WHERE id=?1",
        [archive_release_id],
        |row| row.get(0),
    )
    .optional()
    .ok()
    .flatten()
}

/// The archive release a work target belongs to, when it belongs to exactly
/// one. Prefer [`releases_for_target`], which also answers for the targets that
/// span several.
#[must_use]
pub fn release_for_target(conn: &Connection, target: &WorkTarget) -> Option<String> {
    releases_for_target(conn, target).into_iter().next()
}

/// Every archive release a work target covers.
///
/// Direct for release targets; resolved through the owning carrier for dump
/// targets; every release publishing into the folder for console targets; and
/// none for filesystem paths, since an incoming package is not archived yet.
///
/// One definition, so derivation's scope filter and the error and claim
/// surfaces group work the same way. A console target resolving to many is what
/// keeps a gamelist failure visible on the rows it affects: attributing it to
/// no release at all would drop it silently, which is how those surfaces lose
/// errors today.
#[must_use]
pub fn releases_for_target(conn: &Connection, target: &WorkTarget) -> Vec<String> {
    let collect = |sql: &str, parameters: &[&dyn rusqlite::ToSql]| -> Vec<String> {
        let Ok(mut statement) = conn.prepare(sql) else {
            return Vec::new();
        };
        let Ok(rows) = statement.query_map(parameters, |row| row.get::<_, String>(0)) else {
            return Vec::new();
        };
        rows.filter_map(Result::ok).collect()
    };
    match target {
        WorkTarget::Release(id) => vec![id.clone()],
        WorkTarget::Dump(dump_id) => collect(
            "SELECT ar.id FROM dump_events de
             JOIN carriers c ON c.id=de.carrier_id
             JOIN physical_copies pc ON pc.id=c.physical_copy_id
             JOIN archive_releases ar ON ar.id=pc.archive_release_id
             WHERE de.id=?1",
            &[&dump_id.as_str()],
        ),
        WorkTarget::Console(_) => match target.console_parts() {
            Some((profile_id, directory)) => collect(
                CONSOLE_RELEASES_SQL,
                &[&profile_id as &dyn rusqlite::ToSql, &directory],
            ),
            None => Vec::new(),
        },
        WorkTarget::Path(_) => Vec::new(),
    }
}

/// Every open error, grouped by the archive release it belongs to.
///
/// Verification errors are recorded against a dump and gamelist errors against
/// a whole folder, but the UI shows one row per release, so the grouping has to
/// happen somewhere; doing it here keeps [`releases_for_target`] the only place
/// that knows the joins. A folder's failure appears on every release in it,
/// which is honest: none of them reached the frontend.
pub fn errors_by_release(
    conn: &Connection,
    actions: &[ProposedAction],
) -> Result<BTreeMap<String, Vec<(ActionKind, crate::work::WorkError)>>, LibraryError> {
    let mut grouped: BTreeMap<String, Vec<_>> = BTreeMap::new();
    for error in active_work_errors(conn, actions)? {
        let Ok(kind) = error.action_kind.parse::<ActionKind>() else {
            continue;
        };
        for release_id in
            releases_for_target(conn, &stored_target(&error.target_kind, &error.target_id))
        {
            grouped
                .entry(release_id)
                .or_default()
                .push((kind, error.clone()));
        }
    }
    Ok(grouped)
}

/// Open failures whose exact action/target is still owed by the current
/// projection. `work_errors` records the latest failed attempt, but the
/// projection is the authority on whether there remains anything to do: a
/// later manual fix, adoption, or successful equivalent operation can satisfy
/// the target without executing the same claim again.
fn active_work_errors(
    conn: &Connection,
    actions: &[ProposedAction],
) -> Result<Vec<crate::work::WorkError>, LibraryError> {
    let active = actions
        .iter()
        .map(|action| {
            (
                action.kind.as_str(),
                action.target.kind(),
                action.target.id(),
            )
        })
        .collect::<BTreeSet<_>>();
    let errors = crate::work::list_work_errors(conn).map_err(|error| match error {
        crate::operations::OperationError::Sqlite(error) => LibraryError::Sqlite(error),
        other => LibraryError::InvalidScanState(other.to_string()),
    })?;
    Ok(errors
        .into_iter()
        .filter(|error| {
            active.contains(&(
                error.action_kind.as_str(),
                error.target_kind.as_str(),
                error.target_id.as_str(),
            ))
        })
        .collect())
}

/// Rebuild a work target from the two strings the coordination tables store.
///
/// The inverse of [`WorkTarget::kind`] and [`WorkTarget::id`], and the one
/// place that mapping is written down — an unrecognized kind becomes a path
/// target, which resolves to no release rather than to the wrong one.
#[must_use]
pub fn stored_target(target_kind: &str, target_id: &str) -> WorkTarget {
    match target_kind {
        "dump" => WorkTarget::Dump(target_id.to_owned()),
        "release" => WorkTarget::Release(target_id.to_owned()),
        "console" => WorkTarget::Console(target_id.to_owned()),
        _ => WorkTarget::Path(target_id.to_owned()),
    }
}

/// Every blocked action, grouped by the archive release it belongs to.
///
/// [`BlockedReason`]'s own doc comment says blocked actions are reported,
/// never silently dropped — this is what lets a UI honor that: the worker
/// already skips a blocked action before it ever reaches the executor, so
/// without this a click on one produces no error and no effect, and the
/// reason `derive_convergence` computed is never seen by anyone.
pub fn blocked_by_release(
    conn: &Connection,
    scope: &Scope,
    expected_assets: &AssetSelection,
) -> Result<BTreeMap<String, Vec<(ActionKind, BlockedReason)>>, LibraryError> {
    let actions = derive_convergence(conn, scope, expected_assets)?;
    blocked_by_release_for_actions(conn, &actions)
}

/// Group the blocked subset of one already-derived action set.
pub fn blocked_by_release_for_actions(
    conn: &Connection,
    actions: &[ProposedAction],
) -> Result<BTreeMap<String, Vec<(ActionKind, BlockedReason)>>, LibraryError> {
    let mut grouped: BTreeMap<String, Vec<_>> = BTreeMap::new();
    for action in actions {
        let Some(reason) = action.blocked.clone() else {
            continue;
        };
        for release_id in releases_for_target(conn, &action.target) {
            grouped
                .entry(release_id)
                .or_default()
                .push((action.kind, reason.clone()));
        }
    }
    Ok(grouped)
}

/// The collections a scope touches, in a stable order.
pub fn profiles_for_scope(conn: &Connection, scope: &Scope) -> Result<Vec<String>, LibraryError> {
    profiles_in_scope(conn, scope)
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
        Scope::Releases(ids) => {
            // A selection can in principle span collections; derive over each
            // once, in a stable order.
            let mut profiles = BTreeSet::new();
            for id in ids {
                if let Some(profile_id) = conn
                    .query_row(
                        "SELECT profile_id FROM archive_releases WHERE id=?1",
                        [id.as_str()],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()?
                {
                    profiles.insert(profile_id);
                }
            }
            profiles.into_iter().collect()
        }
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
        let identify =
            crate::archive::needs_catalog_identification(&catalog_state, &catalog_media_id);
        if redumper_raw && identify {
            actions.push(base(ActionKind::AuditRedumper));
        }
        if !redumper_raw && file_count == 1 && identify {
            actions.push(base(ActionKind::VerifyCatalog));
        }
    }
    Ok(())
}

/// Releases whose playable files and the archive's record of them disagree, in
/// the two ways a content match can settle.
///
/// First: a recorded playable whose file is not there. `missing` is
/// deliberately narrower than "not present" — a `stale` output belongs to
/// superseded evidence and a `modified` one is an integrity question, neither
/// of which re-adoption answers.
///
/// Second: a carrier with *no* playable at all, beside an unbound library file
/// carrying that carrier's catalog digests. That is a collection assembled
/// before the archive existed — the file was never built here, so there is no
/// output digest to search for, but the catalog medium the carrier verified
/// against identifies it just as well.
const ORPHANED_PLAYABLE_SQL: &str = "
    SELECT DISTINCT ar.id, ar.platform_id, ar.region, ar.title
    FROM archive_releases ar
    JOIN physical_copies pc ON pc.archive_release_id=ar.id
    JOIN carriers c ON c.physical_copy_id=pc.id
    JOIN representations rep ON rep.carrier_id=c.id
    WHERE ar.profile_id=?1
      AND rep.role='playable' AND rep.presence_state='missing'
    UNION
    SELECT DISTINCT ar.id, ar.platform_id, ar.region, ar.title
    FROM archive_releases ar
    JOIN archive_profiles ap ON ap.id=ar.profile_id
    JOIN physical_copies pc ON pc.archive_release_id=ar.id
    JOIN carriers c ON c.physical_copy_id=pc.id
    JOIN media m ON m.id=c.catalog_media_id
    JOIN library_roots lr ON lr.root_path=ap.playable_root
    JOIN library_consoles lc ON lc.root_id=lr.id
    JOIN library_entries le ON le.console_id=lc.id
    WHERE ar.profile_id=?1
      AND NOT EXISTS(
          SELECT 1 FROM representations rep
          WHERE rep.carrier_id=c.id AND rep.role='playable')
      AND ((le.sha1<>'' AND m.sha1<>'')
           OR (le.md5<>'' AND m.md5<>'')
           OR (le.crc32<>'' AND m.crc32<>''))
      AND (le.sha1='' OR m.sha1='' OR le.sha1=m.sha1)
      AND (le.md5='' OR m.md5='' OR le.md5=m.md5)
      AND (le.crc32='' OR m.crc32=''
           OR (le.crc32=m.crc32 AND le.data_size=m.file_size))
      AND NOT EXISTS(
          SELECT 1 FROM library_entry_media_bindings b
          WHERE b.library_entry_id=le.id AND b.carrier_id IS NOT NULL)";

fn derive_adoption_actions(
    conn: &Connection,
    profile_id: &str,
    actions: &mut Vec<ProposedAction>,
) -> Result<(), LibraryError> {
    let mut statement = conn.prepare(ORPHANED_PLAYABLE_SQL)?;
    let rows = statement
        .query_map([profile_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (release_id, platform_id, region, title) in rows {
        actions.push(ProposedAction {
            kind: ActionKind::AdoptPlayable,
            target: WorkTarget::Release(release_id),
            profile_id: profile_id.to_owned(),
            playable_platform_id: retro_junk_frontend::esde::system_directory(
                &platform_id,
                Some(&region),
            ),
            platform_id,
            label: dump_label(&title, &region, 0),
            blocked: None,
            build: None,
        });
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

/// Force a rebuild of one release's playable representation, regardless of
/// whether it currently reads as satisfied.
///
/// The normal derivation above skips a release once its preferred playable
/// looks present, off of caches — a projected representation row, a bound
/// library entry — that can go stale without the archive knowing: a moved
/// or regenerated file `AdoptPlayable` could not relink, or a scan binding
/// left over from before the file changed. This is the escape hatch: it
/// bypasses only the "already satisfied" belief, not a genuine blocker.
/// [`BlockedReason`] still applies the same way — forcing cannot build
/// without a preferred format or a complete archive, only skip the belief
/// that nothing is owed.
pub fn forced_build_action(
    conn: &Connection,
    archive_release_id: &str,
) -> Result<Option<ProposedAction>, LibraryError> {
    let Some((profile_id, platform_id, region)) = conn
        .query_row(
            "SELECT profile_id, platform_id, region FROM archive_releases WHERE id=?1",
            [archive_release_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?
    else {
        return Ok(None);
    };
    let Some(gap) = query_forced_playable_gap(conn, &profile_id, archive_release_id)? else {
        return Ok(None);
    };
    let blocked = if gap.preferred_format.is_none() {
        Some(BlockedReason::NoPolicy)
    } else if !gap.buildable {
        Some(BlockedReason::IncompleteArchive {
            have: gap.archived_disc_count,
            need: gap.expected_disc_count,
        })
    } else {
        None
    };
    Ok(Some(ProposedAction {
        kind: ActionKind::BuildPlayable,
        target: WorkTarget::Release(archive_release_id.to_owned()),
        profile_id,
        playable_platform_id: retro_junk_frontend::esde::system_directory(
            &platform_id,
            Some(&region),
        ),
        platform_id,
        label: dump_label(&gap.title, &gap.region, 0),
        blocked,
        build: Some(gap),
    }))
}

/// Releases with archived artwork and a present playable output owe current
/// frontend projections.
///
/// `category IN ('artwork','video')` matches [`RELEASE_ARTWORK_SQL`]: gating on
/// `'artwork'` alone meant a release holding only a video never projected it,
/// while the scrape derivation counted that video as artwork the release had.
const PROJECTION_CANDIDATES_SQL: &str = "
    SELECT DISTINCT ar.id, ar.platform_id, ar.region, ar.title,
           EXISTS(SELECT 1 FROM archive_release_files arf
                  WHERE arf.archive_release_id=ar.id
                    AND arf.presence_state='present'
                    AND arf.category IN ('artwork','video')) AS has_artwork
    FROM archive_releases ar
    JOIN physical_copies pc ON pc.archive_release_id=ar.id
    JOIN carriers c ON c.physical_copy_id=pc.id
    JOIN representations rep ON rep.carrier_id=c.id
    WHERE ar.profile_id=?1
      AND rep.role='playable' AND rep.presence_state='present'";

/// The frontend folders one profile publishes into, taken from where its
/// playable files actually are.
///
/// Presence resolution already wrote the resolved location into
/// `representations.relative_path`, so this is the folder holding the file
/// rather than the folder the naming rule would compute — which matters for a
/// release filed somewhere the rule would not pick today, whose gamelist is the
/// one in the folder it is really in.
const CONSOLE_FOLDERS_SQL: &str = "
    SELECT DISTINCT substr(rep.relative_path, 1, instr(rep.relative_path,'/')-1)
    FROM archive_releases ar
    JOIN physical_copies pc ON pc.archive_release_id=ar.id
    JOIN carriers c ON c.physical_copy_id=pc.id
    JOIN representations rep ON rep.carrier_id=c.id
    WHERE ar.profile_id=?1
      AND rep.role='playable' AND rep.presence_state='present'
      AND instr(rep.relative_path,'/') > 0
    ORDER BY 1";

/// Every release publishing into one folder — the releases a console's gamelist
/// lists, and the rows a console-scoped error or failure belongs to.
const CONSOLE_RELEASES_SQL: &str = "
    SELECT DISTINCT ar.id
    FROM archive_releases ar
    JOIN physical_copies pc ON pc.archive_release_id=ar.id
    JOIN carriers c ON c.physical_copy_id=pc.id
    JOIN representations rep ON rep.carrier_id=c.id
    WHERE ar.profile_id=?1
      AND rep.role='playable' AND rep.presence_state='present'
      AND substr(rep.relative_path, 1, instr(rep.relative_path,'/')-1) = ?2
    ORDER BY ar.id";

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
        if !has_artwork || projection_is_current(conn, ProjectionOf::assets(&release_id))? {
            continue;
        }
        actions.push(ProposedAction {
            kind: ActionKind::ProjectAssets,
            target: WorkTarget::Release(release_id),
            profile_id: profile_id.to_owned(),
            playable_platform_id: retro_junk_frontend::esde::system_directory(
                &platform_id,
                Some(&region),
            ),
            platform_id,
            label: dump_label(&title, &region, 0),
            blocked: None,
            build: None,
        });
    }
    Ok(())
}

/// One gamelist action per frontend folder, not per game.
fn derive_gamelist_actions(
    conn: &Connection,
    profile_id: &str,
    actions: &mut Vec<ProposedAction>,
) -> Result<(), LibraryError> {
    for directory in console_folders(conn, profile_id)? {
        if projection_is_current(conn, ProjectionOf::gamelist(profile_id, &directory))? {
            continue;
        }
        actions.push(ProposedAction {
            kind: ActionKind::SyncGamelist,
            target: WorkTarget::console(profile_id, &directory),
            profile_id: profile_id.to_owned(),
            // A folder is a frontend identity; the archive platforms landing in
            // it are whatever they are, so there is no single one to name.
            platform_id: String::new(),
            playable_platform_id: directory.clone(),
            label: directory,
            blocked: None,
            build: None,
        });
    }
    Ok(())
}

/// A projection is done when it is current, which is exactly the candidates the
/// derivation did not propose.
///
/// Counting them the other way — as always outstanding, because they are always
/// safe to redo — is what made a fully converged library report hundreds of
/// pending projections forever.
fn count_projections_done(
    conn: &Connection,
    profile_id: &str,
    summary: &mut ConvergenceSummary,
) -> Result<(), LibraryError> {
    let projectable: u64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM ({PROJECTION_CANDIDATES_SQL}) WHERE has_artwork"),
        [profile_id],
        |row| row.get(0),
    )?;
    let assets = summary
        .per_kind
        .entry(ActionKind::ProjectAssets)
        .or_default();
    assets.done += projectable.saturating_sub(assets.pending);

    let folders = u64::try_from(console_folders(conn, profile_id)?.len()).unwrap_or(u64::MAX);
    let gamelists = summary
        .per_kind
        .entry(ActionKind::SyncGamelist)
        .or_default();
    gamelists.done += folders.saturating_sub(gamelists.pending);
    Ok(())
}

fn console_folders(conn: &Connection, profile_id: &str) -> Result<Vec<String>, LibraryError> {
    let mut statement = conn.prepare(CONSOLE_FOLDERS_SQL)?;
    let folders = statement
        .query_map([profile_id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(folders)
}

/// Every release's artwork holdings, whether or not it has any. The `LEFT
/// JOIN` is what makes a release with no artwork at all appear — an inner
/// join would silently exclude exactly the releases most in need of a scrape.
const RELEASE_ARTWORK_SQL: &str = "
    SELECT ar.id, ar.platform_id, ar.region, ar.title, COALESCE(arf.asset_type,'')
    FROM archive_releases ar
    LEFT JOIN archive_release_files arf
      ON arf.archive_release_id=ar.id AND arf.presence_state='present'
     AND arf.category IN ('artwork','video')
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
    /// Tried against these exact bytes and settled on no single catalog
    /// medium — nothing matched, or several did.
    ///
    /// These are deliberately not `pending`: re-deriving the same answer costs
    /// a full reproduction of the dump. Counted separately so they stay visible
    /// instead of vanishing from the backlog as though they never existed.
    pub unresolved: u64,
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
    let actions = derive_convergence(conn, scope, expected_assets)?;
    summarize_convergence_for_actions(conn, scope, expected_assets, &actions)
}

/// Aggregate one already-derived action set. Keeping this beside
/// [`derive_convergence`] lets compound backend reads derive pending work once
/// and use that same answer for counts, errors, and blocked reasons.
pub fn summarize_convergence_for_actions(
    conn: &Connection,
    scope: &Scope,
    expected_assets: &AssetSelection,
    actions: &[ProposedAction],
) -> Result<ConvergenceSummary, LibraryError> {
    let mut summary = ConvergenceSummary::default();
    for action in actions {
        let counts = summary.per_kind.entry(action.kind).or_default();
        if action.blocked.is_some() {
            counts.blocked += 1;
        } else {
            counts.pending += 1;
        }
    }
    let active_errors = active_work_errors(conn, actions)?;
    let active_claims = active_work_claim_counts(conn, actions)?;
    for kind in ActionKind::all() {
        let counts = summary.per_kind.entry(*kind).or_default();
        counts.errored = active_errors
            .iter()
            .filter(|error| error.action_kind == kind.as_str())
            .count()
            .try_into()
            .unwrap_or(u64::MAX);
        counts.running = active_claims.get(kind).copied().unwrap_or(0);
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
            let catalog_kind = if format == "redumper_raw" {
                ActionKind::AuditRedumper
            } else {
                ActionKind::VerifyCatalog
            };
            if catalog_state == crate::archive::CATALOG_VERIFIED {
                summary.per_kind.entry(catalog_kind).or_default().done += 1;
            } else if catalog_state == crate::archive::CATALOG_UNRESOLVED {
                summary.per_kind.entry(catalog_kind).or_default().unresolved += 1;
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
        count_projections_done(conn, &profile_id, &mut summary)?;
        // Adoption is repair, not a stage every release passes through: a
        // release whose playables are all where their evidence says needs no
        // adoption and counts as done, so the chip reads 0 pending rather
        // than an empty backlog of nothing.
        summary
            .per_kind
            .entry(ActionKind::AdoptPlayable)
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

fn active_work_claim_counts(
    conn: &Connection,
    actions: &[ProposedAction],
) -> Result<BTreeMap<ActionKind, u64>, LibraryError> {
    let active = actions
        .iter()
        .map(|action| {
            (
                action.kind.as_str().to_owned(),
                action.target.kind().to_owned(),
                action.target.id().to_owned(),
            )
        })
        .collect::<BTreeSet<_>>();
    let mut statement = conn.prepare(&format!(
        "SELECT action_kind,target_kind,target_id FROM work_claims
         WHERE since >= datetime('now','-{} minutes')",
        crate::work::CLAIM_TIMEOUT_MINUTES
    ))?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut counts = BTreeMap::new();
    for row in rows {
        let key = row?;
        if !active.contains(&key) {
            continue;
        }
        let Ok(kind) = key.0.parse::<ActionKind>() else {
            continue;
        };
        *counts.entry(kind).or_default() += 1;
    }
    Ok(counts)
}
