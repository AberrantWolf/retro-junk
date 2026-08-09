//! Rebuildable `SQLite` projection of portable preservation manifests.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use retro_junk_archive::{
    ArchiveIndexSnapshot, RepresentationFormat, RepresentationRole, TrackDigest,
    preservation_presence,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use sha2::{Digest, Sha256};

use crate::operations::OperationError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveReleaseSummary {
    pub archive_release_id: String,
    pub catalog_release_id: Option<String>,
    pub platform_id: String,
    pub title: String,
    pub region: String,
    pub revision: String,
    pub physical_copy_count: u64,
    pub carrier_count: u64,
    pub dump_count: u64,
    pub preservation_count: u64,
    pub preservation_present_count: u64,
    pub playable_count: u64,
    pub playable_present_count: u64,
    /// Playable representations whose recorded file is not there at all —
    /// distinct from `stale` (superseded evidence) and `modified` (an
    /// integrity question). Only these can be recovered by content.
    pub playable_missing_count: u64,
    pub desired_playable_count: u64,
    pub satisfied_playable_count: u64,
    pub integrity_verified_count: u64,
    pub reproduction_verified_count: u64,
    pub catalog_verified_count: u64,
    pub round_trip_verified_count: u64,
}

/// A playable representation's path, relative to the profile's playable root.
/// Separators are normalized so a projection written on Windows still matches
/// rows scanned on Unix.
const PLAYABLE_PATH: &str = "replace(rep.relative_path,'\\','/')";

/// The same path for a library row: its console folder plus the path its entry
/// key carries — `file:` for one file, `set:` for the directory a multi-disc
/// set stands for.
const LIBRARY_ENTRY_PATH: &str = "replace(lc.folder_name || '/' ||
     CASE WHEN le.entry_key LIKE 'set:%' THEN substr(le.entry_key,5)
          ELSE substr(le.entry_key,6) END,'\\','/')";

/// "This scanned library row *is* the file this representation points at."
///
/// Exact identity, for the questions that need it: which file the archive's
/// evidence names, and whose hashes it already knows.
fn playable_path_is_library_entry() -> String {
    format!("{PLAYABLE_PATH}={LIBRARY_ENTRY_PATH}")
}

/// "This scanned library row *holds* the file this representation points at."
///
/// A multi-disc row is one library entry standing for a directory of disc
/// images, so each archived disc inside it belongs to that row. Ownership, not
/// identity: use this to decide what an archive already accounts for.
fn playable_path_is_within_library_entry() -> String {
    format!(
        "({PLAYABLE_PATH}={LIBRARY_ENTRY_PATH}
          OR substr({PLAYABLE_PATH},1,length({LIBRARY_ENTRY_PATH})+1)
             ={LIBRARY_ENTRY_PATH} || '/')"
    )
}

/// The join from a playable representation (`rep`) to the library rows that
/// could hold it: through its carrier's release to the profile whose playable
/// root is the scanned library root.
const PLAYABLE_REPRESENTATION_TO_LIBRARY_ENTRY_JOIN: &str = "\
     JOIN carriers c ON c.id=rep.carrier_id
         JOIN physical_copies pc ON pc.id=c.physical_copy_id
         JOIN archive_releases ar ON ar.id=pc.archive_release_id
         JOIN archive_profiles ap ON ap.id=ar.profile_id
         JOIN library_roots lr ON lr.root_path=ap.playable_root
         JOIN library_consoles lc ON lc.root_id=lr.id
         JOIN library_entries le ON le.console_id=lc.id";

/// Every (library row, archived carrier) binding, with the archive release and
/// profile that own the carrier.
///
/// This is the one definition of "this scanned playable file is an archived
/// carrier's own copy". It deliberately says nothing about the catalog: an
/// unbound archive still owns the playable its build evidence produced, and a
/// carrier no catalog on this machine can name must not lose its playable in
/// the meantime.
const ARCHIVE_BOUND_LIBRARY_ROWS: &str = "\
     SELECT binding.library_entry_id AS library_entry_id,
            binding.representation_id AS representation_id,
            binding_carrier.id AS carrier_id,
            binding_release.id AS archive_release_id,
            binding_profile.playable_root AS playable_root
     FROM library_entry_media_bindings binding
     JOIN carriers binding_carrier ON binding_carrier.id=binding.carrier_id
     JOIN physical_copies binding_copy ON binding_copy.id=binding_carrier.physical_copy_id
     JOIN archive_releases binding_release ON binding_release.id=binding_copy.archive_release_id
     JOIN archive_profiles binding_profile ON binding_profile.id=binding_release.profile_id";

/// The scanned playable root a library row lives under, for correlating it
/// with the archive profile that projects into that root.
fn library_entry_playable_root(entry: &str) -> String {
    format!(
        "(SELECT root_scope.root_path FROM library_consoles console_scope
          JOIN library_roots root_scope ON root_scope.id=console_scope.root_id
          WHERE console_scope.id={entry}.console_id)"
    )
}

/// `FROM (…) bound` over the archived-carrier bindings.
pub(crate) fn archive_bound_rows_from() -> String {
    format!("FROM ({ARCHIVE_BOUND_LIBRARY_ROWS}) bound")
}

/// The predicate restricting `bound` to one library row's own bindings, in the
/// archive profile that projects into that row's playable root.
///
/// `entry` is the alias the caller gave `library_entries`.
pub(crate) fn archive_bound_rows_where(entry: &str) -> String {
    format!(
        "bound.library_entry_id={entry}.id AND bound.playable_root={}",
        library_entry_playable_root(entry)
    )
}

/// Whether a library row is already an archived carrier's own playable copy.
pub(crate) fn library_entry_is_archived(entry: &str) -> String {
    format!(
        "EXISTS(SELECT 1 {} WHERE {})",
        archive_bound_rows_from(),
        archive_bound_rows_where(entry)
    )
}

/// A library entry the archive's own evidence can name without any catalog.
///
/// The identity is the game name a catalog verification agreed on, recorded in
/// the archive beside the dump it verified. It reaches a library row only when
/// the playable build derived from that dump is still present at its recorded
/// path and size.
/// Which library rows an evidence derivation covers. Status-writing paths pass
/// the narrowest scope they own, so a per-entry commit does not re-derive the
/// whole console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveEvidenceScope {
    All,
    Console(crate::LibraryConsoleId),
    Entry(crate::LibraryEntryId),
}

impl ArchiveEvidenceScope {
    /// `(console_id, entry_id)` filter parameters; zero means "no filter".
    const fn filters(self) -> (u64, u64) {
        match self {
            Self::All => (0, 0),
            Self::Console(console) => (console.0, 0),
            Self::Entry(entry) => (0, entry.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveEvidenceIdentity {
    pub library_entry_id: i64,
    pub entry_key: String,
    /// Catalog game name recorded by the dump's catalog verification.
    pub game_name: String,
    pub representation_id: String,
    pub archive_release_id: String,
}

/// The platform id the catalog keys releases by, for an archive or library
/// platform name that may be regional (`famicom`, `snesna`, `super-famicom`).
/// Unknown names pass through so an unrecognized platform simply matches
/// nothing rather than matching everything.
fn catalog_platform_id(platform_id: &str) -> String {
    platform_id
        .parse::<retro_junk_core::Platform>()
        .map_or_else(
            |_| platform_id.to_owned(),
            |platform| platform.short_name().to_owned(),
        )
}

fn same_platform(left: &str, right: &str) -> bool {
    if left.eq_ignore_ascii_case(right) {
        return true;
    }
    match (
        left.parse::<retro_junk_core::Platform>(),
        right.parse::<retro_junk_core::Platform>(),
    ) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

/// Follow a playable from the location its evidence records to the active
/// frontend's equivalent system directory. Portable build evidence is
/// historical and remains unchanged; only this rebuildable projection points
/// at the file's current location.
///
/// The resolution itself lives in [`retro_junk_archive::resolve_playable`], so
/// the archive's orphan scan cannot reach a different answer than this. All
/// this layer adds is the frontend system directory, which is the one thing
/// the archive crate has no business knowing.
fn projected_playable_path(
    playable_root: &Path,
    platform_id: &str,
    region: &str,
    input_manifest_sha256: &str,
    evidence: &retro_junk_archive::BuildEvidence,
) -> (String, retro_junk_archive::RepresentationPresence) {
    retro_junk_archive::resolve_playable(
        playable_root,
        input_manifest_sha256,
        evidence,
        &frontend_system_directory(platform_id, region),
    )
}

/// The frontend system directory a release's playable outputs belong to.
///
/// One definition, shared with the archive-side orphan scan through
/// [`playable_system_directory`], so both resolve the same file.
fn frontend_system_directory(platform_id: &str, region: &str) -> String {
    retro_junk_frontend::esde::system_directory(platform_id, Some(region))
}

/// The mapping the archive's orphan scan needs to resolve outputs the way the
/// projection does.
///
/// Exposed rather than duplicated: `retro-junk-archive` deliberately does not
/// depend on `retro-junk-frontend`, so callers that bridge the two pass this
/// in instead of inventing their own answer.
#[must_use]
pub fn playable_system_directory(platform_id: &str, region: &str) -> String {
    frontend_system_directory(platform_id, region)
}

/// Choose the build evidence that describes each output file's current state.
///
/// `evidence/` is append-only, so rebuilding a derivative (a newer chdman, a
/// changed recipe, a corrected filename) leaves several records for one
/// derivative. A representation is the current state of a file, so only the
/// newest record in each build lineage may project —
/// [`retro_junk_archive::current_build_evidence`] owns that rule, and the
/// archive-side orphan scan reads it the same way.
///
/// The path dedup below is a second, narrower guard: two *different* lineages
/// (redumps of one carrier) can converge on one canonical output name, and the
/// projection's `UNIQUE(location_role, relative_path)` will not have that.
/// Builds arrive in time order, so the last one wins.
///
/// Returns the indices to project, mapped to the path and presence already
/// resolved for them.
fn current_builds_by_output(
    dump: &retro_junk_archive::IndexedDump,
    playable_root: &Path,
    platform_id: &str,
    region: &str,
    input_manifest_sha256: &str,
) -> HashMap<usize, (String, retro_junk_archive::RepresentationPresence)> {
    let builds = &dump.builds;
    let current_lineages = retro_junk_archive::current_build_evidence(dump)
        .into_iter()
        .map(|evidence| evidence.build_id)
        .collect::<std::collections::HashSet<_>>();
    let mut current: HashMap<String, usize> = HashMap::new();
    let mut projected = HashMap::with_capacity(builds.len());
    for (index, build) in builds.iter().enumerate() {
        if !current_lineages.contains(&build.evidence.build_id) {
            log::debug!(
                "Superseded build {} for {}; a later build of the same derivative replaces it",
                build.evidence.build_id,
                build.evidence.relative_output_path
            );
            continue;
        }
        let resolved = projected_playable_path(
            playable_root,
            platform_id,
            region,
            input_manifest_sha256,
            &build.evidence,
        );
        if let Some(superseded) = current.insert(resolved.0.clone(), index) {
            log::info!(
                "Superseded build {} for {}; projecting {}",
                builds[superseded].evidence.build_id,
                resolved.0,
                build.evidence.build_id
            );
        }
        projected.insert(index, resolved);
    }
    // A representation row is the current state of one file, so two live
    // records claiming one representation id cannot both be projected — the
    // insert would collide on the primary key and take the whole reconcile
    // with it. That is not hypothetical: a rename that wrote the wrong format
    // started a second lineage beside the one it meant to supersede, and the
    // records it left behind are still on disk in any archive it touched.
    // The newest record is the truth about where the file is now; the rest is
    // history, which is what `evidence/` is for.
    let mut newest_by_representation: HashMap<String, usize> = HashMap::new();
    for index in current.into_values() {
        let evidence = &builds[index].evidence;
        let id = evidence.child_representation_id.to_string();
        match newest_by_representation.get(&id) {
            Some(&held) if !supersedes(&builds[held].evidence, evidence) => {}
            Some(&held) => {
                log::info!(
                    "Build {} and {} both claim representation {id}; projecting the newer",
                    builds[held].evidence.build_id,
                    evidence.build_id
                );
                newest_by_representation.insert(id, index);
            }
            None => {
                newest_by_representation.insert(id, index);
            }
        }
    }
    newest_by_representation
        .into_values()
        .filter_map(|index| projected.get(&index).map(|value| (index, value.clone())))
        .collect()
}

/// Whether `candidate` is the later of two build records.
///
/// Timestamps decide it; the build id breaks a tie so the projection is
/// stable rather than depending on directory order.
fn supersedes(
    held: &retro_junk_archive::BuildEvidence,
    candidate: &retro_junk_archive::BuildEvidence,
) -> bool {
    (
        candidate.performed_at.as_str(),
        candidate.build_id.to_string(),
    ) > (held.performed_at.as_str(), held.build_id.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveCollectionDetails {
    pub archive_release_id: String,
    pub platform_id: String,
    pub title: String,
    pub region: String,
    pub revision: String,
    pub variant: String,
    pub catalog_release_id: Option<String>,
    pub release_binding_state: String,
    pub catalog_source: String,
    pub physical_copy_id: String,
    pub physical_copy_manifest_path: String,
    pub label: String,
    pub condition: String,
    pub notes: String,
    pub date_acquired: String,
    pub provenance: String,
    pub carrier_manifest_path: String,
    pub carrier_kind: String,
    pub carrier_serial: String,
    pub carrier_binding_state: String,
    pub desired_format: Option<String>,
    pub retain_intermediate: bool,
    pub allow_unverified: bool,
}

/// A dump's catalog identification matched exactly one medium.
pub const CATALOG_VERIFIED: &str = "verified";
/// Identification ran against these exact bytes and settled on no single
/// medium — nothing matched, or several did. An answer, just not a binding.
pub const CATALOG_UNRESOLVED: &str = "unresolved";
/// Identification has not run against these bytes.
pub const CATALOG_NOT_ATTEMPTED: &str = "not_attempted";

/// Whether identification has work left to do for a projected dump.
///
/// The expensive half of identifying a disc is reproducing its tracks, so this
/// asks only about states that a fresh reproduction would change: never tried,
/// or tried successfully but the carrier somehow carries no medium id (an
/// inconsistency worth repairing). A dump that already settled on "no single
/// match" is left alone until its bytes change — the archive-side rule this
/// mirrors is `retro_junk_archive::dump_catalog_attempted`.
///
/// Re-running after a *catalog* change is a deliberate act, not a derived one:
/// `archive redumper-audit` audits regardless of state.
#[must_use]
pub fn needs_catalog_identification(catalog_state: &str, catalog_media_id: &str) -> bool {
    match catalog_state {
        CATALOG_VERIFIED => catalog_media_id.is_empty(),
        CATALOG_UNRESOLVED => false,
        _ => true,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteCatalogMediaMatch {
    pub media_id: String,
    pub release_id: String,
    pub work_id: String,
    pub game: String,
    /// Media-level DAT name and member filename. These are retained separately
    /// from the release title because playable naming must distinguish a
    /// whole multi-track medium from the track file used for hash matching.
    pub dat_name: String,
    pub rom_name: String,
    pub source: String,
    pub source_version: String,
    pub platform_id: String,
    pub region: String,
    pub revision: String,
    pub variant: String,
    pub serial: String,
    pub sequence_number: u32,
    /// Whether the catalog stores this medium as a set of separate tracks.
    ///
    /// A single-file dump can match such a medium on its primary (largest
    /// track) digests alone, which identifies the game but verifies only one
    /// track of it. Callers recording catalog evidence need that distinction:
    /// claiming a complete track set from a single-file match is exactly what
    /// `complete_track_set` exists to prevent.
    pub medium_has_tracks: bool,
}

/// How many numbered discs the catalog records for a release, never less
/// than 1. A release whose media carry no disc numbers is one "disc".
///
/// This is the release's *total*, as distinct from any one medium's
/// `sequence_number` (its position). Playable builds need the total to decide
/// playlist layout, so handing them a position instead silently truncates
/// multi-disc sets.
pub fn release_disc_count(conn: &Connection, release_id: &str) -> Result<u32, OperationError> {
    let count: u32 = conn.query_row(
        "SELECT COUNT(DISTINCT CASE WHEN disc_number>0 THEN disc_number END)
         FROM media WHERE release_id = ?1",
        params![release_id],
        |row| row.get(0),
    )?;
    Ok(count.max(1))
}

/// Resolve a normalized serial across every platform for archive auto-import.
pub fn match_catalog_serial_any_platform(
    conn: &Connection,
    serial: &str,
) -> Result<Vec<CompleteCatalogMediaMatch>, OperationError> {
    let serial_key = serial.to_ascii_uppercase().replace([' ', '-'], "");
    if serial_key.is_empty() {
        return Ok(Vec::new());
    }
    let mut statement = conn.prepare(
        "SELECT DISTINCT m.id,m.release_id,r.work_id,r.title,m.dat_source,
                COALESCE((SELECT il.source_version FROM import_log il
                          WHERE il.source_type=m.dat_source AND il.source_name=m.dat_system
                          ORDER BY il.imported_at DESC,il.id DESC LIMIT 1),''),
                r.platform_id,r.region,r.revision,r.variant,m.media_serial,m.disc_number,
                EXISTS(SELECT 1 FROM media_tracks mt WHERE mt.media_id=m.id),m.dat_name,m.rom_name
         FROM media m JOIN releases r ON r.id=m.release_id
         LEFT JOIN media_serial_keys msk ON msk.media_id=m.id
         WHERE msk.serial_key=?1 OR
               upper(replace(replace(r.game_serial,'-',''),' ',''))=?1
         ORDER BY r.platform_id,r.region,m.disc_number,m.id",
    )?;
    let rows = statement.query_map([serial_key], |row| {
        Ok(CompleteCatalogMediaMatch {
            media_id: row.get(0)?,
            release_id: row.get(1)?,
            work_id: row.get(2)?,
            game: row.get(3)?,
            source: row.get(4)?,
            source_version: row.get(5)?,
            platform_id: row.get(6)?,
            region: row.get(7)?,
            revision: row.get(8)?,
            variant: row.get(9)?,
            serial: row.get(10)?,
            sequence_number: u32::try_from(row.get::<_, i64>(11)?).unwrap_or(0),
            medium_has_tracks: row.get::<_, i64>(12)? != 0,
            dat_name: row.get(13)?,
            rom_name: row.get(14)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Match a single-file preservation image against all available strong
/// catalog digests. Every catalog digest that is present must agree.
pub fn match_catalog_file(
    conn: &Connection,
    platform_id: &str,
    actual: &retro_junk_archive::FileDigests,
) -> Result<Vec<CompleteCatalogMediaMatch>, OperationError> {
    match_catalog_file_inner(conn, &catalog_platform_id(platform_id), actual)
}

pub fn match_catalog_file_any_platform(
    conn: &Connection,
    actual: &retro_junk_archive::FileDigests,
) -> Result<Vec<CompleteCatalogMediaMatch>, OperationError> {
    match_catalog_file_inner(conn, "", actual)
}

fn match_catalog_file_inner(
    conn: &Connection,
    platform_id: &str,
    actual: &retro_junk_archive::FileDigests,
) -> Result<Vec<CompleteCatalogMediaMatch>, OperationError> {
    let mut statement = conn.prepare(
        "SELECT m.id,m.release_id,r.work_id,r.title,m.dat_source,
                COALESCE((SELECT il.source_version FROM import_log il
                          WHERE il.source_type=m.dat_source AND il.source_name=m.dat_system
                          ORDER BY il.imported_at DESC,il.id DESC LIMIT 1),''),
                r.platform_id,r.region,r.revision,r.variant,m.media_serial,m.disc_number,
                EXISTS(SELECT 1 FROM media_tracks mt WHERE mt.media_id=m.id),m.dat_name,m.rom_name
         FROM media m JOIN releases r ON r.id=m.release_id
         WHERE (?1='' OR r.platform_id=?1) AND m.file_size=?2
           AND (m.sha1<>'' OR m.md5<>'' OR m.crc32<>'')
           -- An empty digest on *either* side means \"not available to
           -- compare\", never \"mismatch\", and both sides must bring at
           -- least one. The archive's own track evidence records SHA-1 only,
           -- so demanding the CRC-32 the catalog happens to hold made every
           -- such medium permanently unmatchable.
           AND (?3<>'' OR ?4<>'' OR ?5<>'')
           AND (m.sha1='' OR ?3='' OR m.sha1=lower(?3))
           AND (m.md5='' OR ?4='' OR m.md5=lower(?4))
           AND (m.crc32='' OR ?5='' OR m.crc32=lower(?5))
         ORDER BY m.id",
    )?;
    let rows = statement.query_map(
        params![
            platform_id,
            i64::try_from(actual.size).unwrap_or(i64::MAX),
            actual.sha1,
            actual.md5,
            actual.crc32,
        ],
        |row| {
            Ok(CompleteCatalogMediaMatch {
                media_id: row.get(0)?,
                release_id: row.get(1)?,
                work_id: row.get(2)?,
                game: row.get(3)?,
                source: row.get(4)?,
                source_version: row.get(5)?,
                platform_id: row.get(6)?,
                region: row.get(7)?,
                revision: row.get(8)?,
                variant: row.get(9)?,
                serial: row.get(10)?,
                sequence_number: u32::try_from(row.get::<_, i64>(11)?).unwrap_or(0),
                medium_has_tracks: row.get::<_, i64>(12)? != 0,
                dat_name: row.get(13)?,
                rom_name: row.get(14)?,
            })
        },
    )?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// What a hash match proves about a library row: the archived carrier it is a
/// copy of, the catalog medium it matches, or both.
///
/// A carrier alone is enough. An archive that no catalog can name still owns
/// the files its own evidence accounts for.
#[derive(Debug, Clone, Copy, Default)]
pub struct LibraryEntryBinding<'a> {
    pub carrier_id: Option<&'a str>,
    pub catalog_media_id: &'a str,
    pub representation_id: Option<&'a str>,
    pub match_method: &'a str,
}

/// Connect the legacy playable-library projection to catalog/archive identity
/// using already-computed strong hashes. This does not make the library row
/// authoritative and is safe to rebuild.
///
/// Every id a caller hands in names a row this database may simply not have —
/// a carrier from an archive this projection has not indexed yet, or a medium
/// from a DAT this machine has not imported. Storing one anyway is refused
/// outright by the foreign keys, which would fail the caller's whole run over
/// one file, so each reference is resolved first and only what exists is
/// written:
///
/// - a carrier the projection does not hold yet means there is nothing to bind
///   to, so nothing is written (reindexing the archive will bind it);
/// - the carrier row's own catalog medium wins over the caller's, because
///   reindexing derived it from the archive's recorded digests, which is the
///   more direct evidence;
/// - with no carrier, the caller's medium is used if this catalog holds it.
pub fn bind_library_entries_by_hash(
    conn: &Connection,
    platform_id: &str,
    actual: &retro_junk_archive::FileDigests,
    binding: &LibraryEntryBinding<'_>,
) -> Result<usize, OperationError> {
    let carrier_medium = match binding.carrier_id {
        Some(carrier) => {
            let recorded = conn
                .query_row(
                    "SELECT catalog_media_id FROM carriers WHERE id=?1",
                    [carrier],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()?;
            let Some(recorded) = recorded else {
                log::warn!(
                    "Not binding library rows to carrier {carrier}: this catalog has no such carrier yet"
                );
                return Ok(0);
            };
            recorded
        }
        None => None,
    };
    let catalog_media_id = match carrier_medium {
        Some(recorded) => Some(recorded),
        None => existing_id(conn, "media", binding.catalog_media_id)?,
    };
    let representation_id = match binding.representation_id {
        Some(representation) => existing_id(conn, "representations", representation)?,
        None => None,
    };
    if catalog_media_id.is_none() && binding.carrier_id.is_none() {
        return Ok(0);
    }
    conn.execute(
        "INSERT OR REPLACE INTO library_entry_media_bindings(library_entry_id,carrier_id,catalog_media_id,representation_id,match_method)
         SELECT e.id,?4,?5,?6,?7
         FROM library_entries e JOIN library_consoles c ON c.id=e.console_id
         WHERE (lower(c.folder_name)=lower(?1) OR lower(c.platform)=lower(?1))
           AND e.data_size=?2
           AND ((e.sha1<>'' AND e.sha1=lower(?3))
                OR (e.md5<>'' AND e.md5=lower(?8))
                OR (e.crc32<>'' AND e.crc32=lower(?9)))",
        params![
            platform_id,
            i64::try_from(actual.size).unwrap_or(i64::MAX),
            actual.sha1,
            binding.carrier_id,
            catalog_media_id,
            representation_id,
            binding.match_method,
            actual.md5,
            actual.crc32,
        ],
    )
    .map_err(Into::into)
}

/// A scanned playable file the archive does not yet account for.
#[derive(Debug, Clone)]
pub struct UnboundPlayableRow {
    /// Path below the profile's playable root, as the projection spells it.
    pub relative_path: String,
    pub sha1: String,
    pub crc32: String,
    pub md5: String,
    pub data_size: u64,
}

/// Scanned playable files under `playable_root` that no archived carrier
/// claims, with the digests the library already read for them.
///
/// This is the input to adopting a playable nobody built here: a file that
/// predates the archive, or arrived with a collection, but is provably a
/// derivative of an archived carrier. Rows with no digest at all are excluded
/// — there is nothing to prove anything with.
pub fn unbound_playable_rows(
    conn: &Connection,
    playable_root: &str,
) -> Result<Vec<UnboundPlayableRow>, OperationError> {
    let sql = format!(
        "SELECT {LIBRARY_ENTRY_PATH},le.sha1,le.crc32,le.md5,le.data_size
         FROM library_entries le
         JOIN library_consoles lc ON lc.id=le.console_id
         JOIN library_roots lr ON lr.id=lc.root_id
         WHERE lr.root_path=?1
           AND (le.sha1<>'' OR le.crc32<>'')
           AND NOT EXISTS(
               SELECT 1 FROM library_entry_media_bindings b
               WHERE b.library_entry_id=le.id AND b.carrier_id IS NOT NULL)"
    );
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map([playable_root], |row| {
        Ok(UnboundPlayableRow {
            relative_path: row.get(0)?,
            sha1: row.get(1)?,
            crc32: row.get(2)?,
            md5: row.get(3)?,
            data_size: u64::try_from(row.get::<_, i64>(4)?).unwrap_or(0),
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Which catalog medium a carrier is, from the digests recorded beside it.
///
/// This is the only way a carrier gets a catalog identity. The archive names no
/// catalog row anywhere — an id built from a game's title moved every time a
/// title was corrected, and an archive written on one machine named rows a
/// differently versioned import on another machine never created. The digests
/// do not move, so they are asked instead, in order of how directly they answer
/// the question:
///
/// 1. The track set the carrier's own binding recorded, which is what the
///    catalog agreed to when it was identified.
/// 2. The track set a catalog verification recorded against a current dump.
/// 3. The digests of a dump's single archived file — a cartridge, where the one
///    file *is* the whole medium and there are no tracks to compare.
///
/// Deliberately conservative throughout: an ambiguous match resolves to nothing
/// rather than guessing between candidates — unless a person has already
/// chosen, which `chosen` carries.
fn resolved_catalog_media(
    conn: &Connection,
    platform_id: &str,
    carrier: &retro_junk_archive::IndexedCarrier,
    chosen: &crate::disambiguation::Disambiguations,
) -> Result<Option<String>, OperationError> {
    let platform = catalog_platform_id(platform_id);

    let recorded = &carrier.manifest.catalog_binding.expected_tracks;
    if !recorded.is_empty()
        && let Some(id) = matched_track_set(conn, &platform, recorded, carrier, chosen)?
    {
        return Ok(Some(id));
    }

    for dump in &carrier.dumps {
        // One definition of "the archive calls this dump catalog-verified",
        // shared with every other consumer — including the shape rule that
        // rescues cartridge evidence written before the completeness flag.
        if retro_junk_archive::dump_catalog_evidence(dump).is_none() {
            continue;
        }
        for verification in &dump.verifications {
            let evidence = &verification.evidence;
            if evidence.kind != retro_junk_archive::VerificationKind::Catalog
                || evidence.outcome != retro_junk_archive::VerificationOutcome::Verified
                || evidence.input_manifest_sha256 != dump.manifest_sha256
            {
                continue;
            }
            if evidence.tracks.is_empty() {
                continue;
            }
            let tracks = evidence
                .tracks
                .iter()
                .filter(|track| track.matched)
                .map(|track| TrackDigest {
                    number: track.number,
                    size: track.size,
                    crc32: String::new(),
                    md5: String::new(),
                    sha1: track.actual_sha1.clone(),
                })
                .collect::<Vec<_>>();
            if tracks.len() != evidence.tracks.len() {
                continue;
            }
            if let Some(id) = matched_track_set(conn, &platform, &tracks, carrier, chosen)? {
                return Ok(Some(id));
            }
        }
    }

    // A cartridge has no tracks: its one archived file is the whole medium, and
    // its digests are in the dump manifest whether or not anything ever ran a
    // catalog pass over it.
    for dump in &carrier.dumps {
        if let [file] = dump.manifest.files.as_slice()
            && let Some(id) = matched_single_file(conn, &platform, file)?
        {
            log::debug!(
                "Carrier {} is catalog medium {id}, by its one archived file's digests",
                carrier.manifest.carrier_id
            );
            return Ok(Some(id));
        }
    }
    Ok(None)
}

/// The catalog medium a complete ordered track set names, if exactly one does —
/// or the one a person picked, when several fit.
fn matched_track_set(
    conn: &Connection,
    platform: &str,
    tracks: &[TrackDigest],
    carrier: &retro_junk_archive::IndexedCarrier,
    chosen: &crate::disambiguation::Disambiguations,
) -> Result<Option<String>, OperationError> {
    match match_complete_catalog_media(conn, platform, tracks)?.as_slice() {
        [matched] => {
            log::debug!(
                "Carrier {} is catalog medium {}, by its recorded track digests",
                carrier.manifest.carrier_id,
                matched.media_id
            );
            Ok(Some(matched.media_id.clone()))
        }
        // Several catalog entries fit these bytes. The tool will not choose,
        // but a person may already have — and their answer is kept beside the
        // collection precisely so it survives this projection being rebuilt.
        candidates if candidates.len() > 1 => {
            if let Some(chosen) = chosen.chosen_for_any(
                candidates
                    .iter()
                    .map(|candidate| candidate.media_id.as_str()),
            ) {
                log::info!(
                    "Carrier {} resolves to catalog medium {chosen}, chosen by hand from {} \
                     possible entries",
                    carrier.manifest.carrier_id,
                    candidates.len()
                );
                return Ok(Some(chosen.to_owned()));
            }
            Ok(None)
        }
        _ => Ok(None),
    }
}

/// The catalog medium one archived file's recorded digests name, if exactly
/// one does.
///
/// The archive stores *raw* file digests while the catalog stores the payload
/// (an iNES or copier header is skipped), so this resolves only formats whose
/// stored bytes already are the payload. A headered dump simply stays
/// unresolved rather than being force-matched on digests describing different
/// bytes. A medium held as separate tracks is refused too: one file cannot be
/// a complete match for it.
fn matched_single_file(
    conn: &Connection,
    platform_id: &str,
    file: &retro_junk_archive::ArchivedFile,
) -> Result<Option<String>, OperationError> {
    if file.crc32.is_empty() && file.sha1.is_empty() {
        return Ok(None);
    }
    let digests = retro_junk_archive::FileDigests {
        size: file.size,
        crc32: file.crc32.clone(),
        md5: file.md5.clone(),
        sha1: file.sha1.clone(),
        sha256: file.sha256.clone(),
    };
    match match_catalog_file(conn, platform_id, &digests)?.as_slice() {
        [matched] if !matched.medium_has_tracks => Ok(Some(matched.media_id.clone())),
        _ => Ok(None),
    }
}

/// Match only when the complete ordered track set agrees. A single matching
/// track is deliberately insufficient evidence for a multi-track disc.
pub fn match_complete_catalog_media(
    conn: &Connection,
    platform_id: &str,
    actual: &[TrackDigest],
) -> Result<Vec<CompleteCatalogMediaMatch>, OperationError> {
    match_complete_catalog_media_inner(conn, &catalog_platform_id(platform_id), actual)
}

pub fn match_complete_catalog_media_any_platform(
    conn: &Connection,
    actual: &[TrackDigest],
) -> Result<Vec<CompleteCatalogMediaMatch>, OperationError> {
    match_complete_catalog_media_inner(conn, "", actual)
}

fn match_single_track_catalog_media(
    conn: &Connection,
    platform_id: &str,
    track: &TrackDigest,
) -> Result<Vec<CompleteCatalogMediaMatch>, OperationError> {
    let mut statement = conn.prepare(
        "SELECT m.id,m.release_id,r.work_id,r.title,m.dat_source,
                COALESCE((SELECT il.source_version FROM import_log il
                          WHERE il.source_type=m.dat_source AND il.source_name=m.dat_system
                          ORDER BY il.imported_at DESC,il.id DESC LIMIT 1),''),
                r.platform_id,r.region,r.revision,r.variant,m.media_serial,m.disc_number,
                EXISTS(SELECT 1 FROM media_tracks mt WHERE mt.media_id=m.id),m.dat_name,m.rom_name
         FROM media m JOIN releases r ON r.id=m.release_id
         WHERE (?1='' OR r.platform_id=?1) AND m.file_size=?2
           AND NOT EXISTS(SELECT 1 FROM media_tracks mt WHERE mt.media_id=m.id)
           AND (m.sha1<>'' OR m.md5<>'' OR m.crc32<>'')
           -- An empty digest on *either* side means \"not available to
           -- compare\", never \"mismatch\", and both sides must bring at
           -- least one. The archive's own track evidence records SHA-1 only,
           -- so demanding the CRC-32 the catalog happens to hold made every
           -- such medium permanently unmatchable.
           AND (?3<>'' OR ?4<>'' OR ?5<>'')
           AND (m.sha1='' OR ?3='' OR m.sha1=lower(?3))
           AND (m.md5='' OR ?4='' OR m.md5=lower(?4))
           AND (m.crc32='' OR ?5='' OR m.crc32=lower(?5))
         ORDER BY m.id",
    )?;
    let rows = statement.query_map(
        params![
            platform_id,
            i64::try_from(track.size).unwrap_or(i64::MAX),
            track.sha1,
            track.md5,
            track.crc32,
        ],
        |row| {
            Ok(CompleteCatalogMediaMatch {
                media_id: row.get(0)?,
                release_id: row.get(1)?,
                work_id: row.get(2)?,
                game: row.get(3)?,
                source: row.get(4)?,
                source_version: row.get(5)?,
                platform_id: row.get(6)?,
                region: row.get(7)?,
                revision: row.get(8)?,
                variant: row.get(9)?,
                serial: row.get(10)?,
                sequence_number: u32::try_from(row.get::<_, i64>(11)?).unwrap_or(0),
                medium_has_tracks: row.get::<_, i64>(12)? != 0,
                dat_name: row.get(13)?,
                rom_name: row.get(14)?,
            })
        },
    )?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn match_complete_catalog_media_inner(
    conn: &Connection,
    platform_id: &str,
    actual: &[TrackDigest],
) -> Result<Vec<CompleteCatalogMediaMatch>, OperationError> {
    if actual.is_empty() || actual.iter().any(|track| track.sha1.is_empty()) {
        return Ok(Vec::new());
    }
    // Single-track Redump games have one non-CUE ROM in the DAT. The catalog
    // importer stores that ROM directly on `media`; only games with multiple
    // non-CUE ROMs receive `media_tracks` rows. Treat a primary media digest as
    // the complete set only when that medium has no track rows, so one matching
    // data track can never verify a catalogued multi-track disc.
    if let [track] = actual {
        return match_single_track_catalog_media(conn, platform_id, track);
    }
    let mut candidates = conn.prepare(
        "SELECT DISTINCT m.id,m.release_id,r.work_id,r.title,m.dat_source,
                COALESCE((SELECT il.source_version FROM import_log il
                          WHERE il.source_type=m.dat_source AND il.source_name=m.dat_system
                          ORDER BY il.imported_at DESC,il.id DESC LIMIT 1),''),
                r.platform_id,r.region,r.revision,r.variant,m.media_serial,m.disc_number,
                EXISTS(SELECT 1 FROM media_tracks mt WHERE mt.media_id=m.id),m.dat_name,m.rom_name
         FROM media_tracks mt
         JOIN media m ON m.id=mt.media_id
         JOIN releases r ON r.id=m.release_id
         WHERE (?1='' OR r.platform_id=?1) AND mt.file_size=?2 AND mt.sha1=lower(?3)
         ORDER BY m.id",
    )?;
    let first = &actual[0];
    let rows = candidates.query_map(
        params![
            platform_id,
            i64::try_from(first.size).unwrap_or(i64::MAX),
            first.sha1,
        ],
        |row| {
            Ok(CompleteCatalogMediaMatch {
                media_id: row.get(0)?,
                release_id: row.get(1)?,
                work_id: row.get(2)?,
                game: row.get(3)?,
                source: row.get(4)?,
                source_version: row.get(5)?,
                platform_id: row.get(6)?,
                region: row.get(7)?,
                revision: row.get(8)?,
                variant: row.get(9)?,
                serial: row.get(10)?,
                sequence_number: u32::try_from(row.get::<_, i64>(11)?).unwrap_or(0),
                medium_has_tracks: row.get::<_, i64>(12)? != 0,
                dat_name: row.get(13)?,
                rom_name: row.get(14)?,
            })
        },
    )?;
    let candidates = rows.collect::<Result<Vec<_>, _>>()?;
    let mut matches = Vec::new();
    for candidate in candidates {
        let mut statement = conn.prepare(
            "SELECT track_number,file_size,crc32,md5,sha1 FROM media_tracks
             WHERE media_id=?1 ORDER BY track_number",
        )?;
        let expected = statement
            .query_map([&candidate.media_id], |row| {
                Ok(TrackDigest {
                    number: u32::try_from(row.get::<_, i64>(0)?).unwrap_or(u32::MAX),
                    size: u64::try_from(row.get::<_, i64>(1)?).unwrap_or(u64::MAX),
                    crc32: row.get(2)?,
                    md5: row.get(3)?,
                    sha1: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let complete = expected.len() == actual.len()
            && expected.iter().zip(actual).all(|(expected, actual)| {
                expected.number == actual.number
                    && expected.size == actual.size
                    && expected.sha1.eq_ignore_ascii_case(&actual.sha1)
                    && (expected.crc32.is_empty()
                        || actual.crc32.is_empty()
                        || expected.crc32.eq_ignore_ascii_case(&actual.crc32))
                    && (expected.md5.is_empty()
                        || actual.md5.is_empty()
                        || expected.md5.eq_ignore_ascii_case(&actual.md5))
            });
        if complete {
            matches.push(candidate);
        }
    }
    Ok(matches)
}

/// Replace one profile's rebuildable index from its authoritative manifests.
///
/// The filesystem has already committed before this transaction begins. A DB
/// failure can therefore be repaired by calling this function again.
fn release_projection_fingerprint(
    release: &retro_junk_archive::IndexedRelease,
    playable_root: &Path,
) -> String {
    let mut digest = Sha256::new();
    digest.update(release.directory.to_string_lossy().as_bytes());
    digest.update(release.manifest_sha256.as_bytes());
    for file in &release.supporting_files {
        digest.update(file.directory.to_string_lossy().as_bytes());
        digest.update(file.manifest_sha256.as_bytes());
        digest.update(file.manifest.file.sha256.as_bytes());
    }
    for physical_copy in &release.physical_copies {
        digest.update(physical_copy.directory.to_string_lossy().as_bytes());
        digest.update(physical_copy.manifest_sha256.as_bytes());
        for file in &physical_copy.supporting_files {
            digest.update(file.directory.to_string_lossy().as_bytes());
            digest.update(file.manifest_sha256.as_bytes());
            digest.update(file.manifest.file.sha256.as_bytes());
        }
        for carrier in &physical_copy.carriers {
            digest.update(carrier.directory.to_string_lossy().as_bytes());
            digest.update(carrier.manifest_sha256.as_bytes());
            for dump in &carrier.dumps {
                digest.update(dump.directory.to_string_lossy().as_bytes());
                digest.update(dump.manifest_sha256.as_bytes());
                for file in &dump.manifest.files {
                    let path = dump.directory.join("raw").join(&file.path);
                    match std::fs::symlink_metadata(path) {
                        Ok(metadata) => {
                            digest.update([1]);
                            digest.update(metadata.len().to_le_bytes());
                        }
                        Err(_) => digest.update([0]),
                    }
                }
                for verification in &dump.verifications {
                    if let Ok(bytes) = serde_json::to_vec(&verification.evidence) {
                        digest.update(bytes);
                    }
                }
                for build in &dump.builds {
                    if let Ok(bytes) = serde_json::to_vec(&build.evidence) {
                        digest.update(bytes);
                    }
                    let (path, presence) = projected_playable_path(
                        playable_root,
                        &release.manifest.platform_id,
                        &release.manifest.region,
                        &dump.manifest_sha256,
                        &build.evidence,
                    );
                    digest.update(path.as_bytes());
                    digest.update(presence.as_str().as_bytes());
                }
            }
        }
    }
    format!("{:x}", digest.finalize())
}

/// Version of the rule that resolves persisted archive hashes to catalog
/// media. Bumping this revisits every carrier once after a matching bug is
/// fixed, without pretending the archive manifests themselves changed.
const CATALOG_RESOLUTION_VERSION: u64 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CatalogProjectionGeneration {
    import: u64,
    resolver: u64,
}

impl CatalogProjectionGeneration {
    const fn encoded(self) -> u64 {
        (self.import << 16) | self.resolver
    }
}

impl std::fmt::Display for CatalogProjectionGeneration {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "i{}r{}", self.import, self.resolver)
    }
}

/// Catalog state that can change archive identity.
///
/// Import history is intentionally not the generation: importing an identical
/// DAT is an audit event, but it changes no catalog identity and must not force
/// a full archive reprojection. The high bits advance only after an import that
/// created or updated catalog media; the low bits advance when the resolver's
/// interpretation of already-persisted hash evidence changes.
fn catalog_projection_generation(
    conn: &Connection,
) -> Result<CatalogProjectionGeneration, OperationError> {
    let material_import: u64 = conn.query_row(
        "SELECT COALESCE(MAX(id),0) FROM import_log
         WHERE records_created<>0 OR records_updated<>0",
        [],
        |row| row.get(0),
    )?;
    Ok(CatalogProjectionGeneration {
        import: material_import,
        resolver: CATALOG_RESOLUTION_VERSION,
    })
}

#[allow(clippy::too_many_lines)]
pub fn reconcile_archive_snapshot(
    conn: &mut Connection,
    snapshot: &ArchiveIndexSnapshot,
    playable_root: &Path,
    workspace_root: &Path,
) -> Result<(), OperationError> {
    let profile_id = snapshot.manifest.profile_id.to_string();
    let catalog_generation = catalog_projection_generation(conn)?;
    log::debug!("Archive catalog projection generation: {catalog_generation}");
    let existing_profile: Option<(String, String, String, String, u64)> = conn
        .query_row(
            "SELECT manifest_sha256,archive_root,playable_root,workspace_root,catalog_generation
             FROM archive_profiles WHERE id=?1",
            [&profile_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()?;
    let force_all = existing_profile.as_ref().is_none_or(
        |(
            manifest_sha256,
            projected_archive,
            projected_playable,
            projected_workspace,
            projected_catalog,
        )| {
            manifest_sha256 != &snapshot.manifest_sha256
                || projected_archive != snapshot.root.to_string_lossy().as_ref()
                || projected_playable != playable_root.to_string_lossy().as_ref()
                || projected_workspace != workspace_root.to_string_lossy().as_ref()
                || *projected_catalog != catalog_generation.encoded()
        },
    );
    let fingerprints = snapshot
        .releases
        .iter()
        .map(|release| {
            (
                release.manifest.archive_release_id.to_string(),
                release_projection_fingerprint(release, playable_root),
            )
        })
        .collect::<HashMap<_, _>>();
    let existing_releases = conn
        .prepare("SELECT id,projection_fingerprint FROM archive_releases WHERE profile_id=?1")?
        .query_map([&profile_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<HashMap<_, _>, _>>()?;
    let changed = fingerprints
        .iter()
        .filter(|(id, fingerprint)| force_all || existing_releases.get(*id) != Some(*fingerprint))
        .map(|(id, _)| id.clone())
        .collect::<HashSet<_>>();
    let removed = existing_releases
        .keys()
        .filter(|id| !fingerprints.contains_key(*id))
        .cloned()
        .collect::<HashSet<_>>();

    let tx = conn.transaction()?;
    // Decisions a person made about ambiguous carriers. Read from beside the
    // collection rather than from this projection, which is about to be
    // rewritten — that is the whole reason they are not stored here.
    let chosen = crate::disambiguation::Disambiguations::load(
        &retro_junk_archive::collection_root_for(&snapshot.root, playable_root),
    )
    .unwrap_or_default();
    for release_id in changed.iter().chain(&removed) {
        tx.execute(
            "DELETE FROM playable_policies
             WHERE scope_type IN ('carrier','carrier_override')
               AND scope_id IN (
                   SELECT c.id FROM carriers c
                   JOIN physical_copies pc ON pc.id=c.physical_copy_id
                   WHERE pc.archive_release_id=?1
               )",
            [release_id],
        )?;
        tx.execute("DELETE FROM archive_releases WHERE id=?1", [release_id])?;
    }
    tx.execute(
        "DELETE FROM playable_policies
         WHERE scope_type IN ('carrier','carrier_override')
           AND NOT EXISTS(SELECT 1 FROM carriers c WHERE c.id=scope_id)",
        [],
    )?;
    tx.execute(
        "INSERT INTO archive_profiles(id,display_name,manifest_path,manifest_sha256,archive_root,playable_root,workspace_root,source_generation,catalog_generation,indexed_at)
         VALUES(?1,?2,'retro-junk-archive.toml',?3,?4,?5,?6,?7,?8,datetime('now'))
         ON CONFLICT(id) DO UPDATE SET
           display_name=excluded.display_name,
           manifest_sha256=excluded.manifest_sha256,
           archive_root=excluded.archive_root,
           playable_root=excluded.playable_root,
           workspace_root=excluded.workspace_root,
           source_generation=excluded.source_generation,
           catalog_generation=excluded.catalog_generation,
           indexed_at=excluded.indexed_at",
        params![
            profile_id,
            snapshot.manifest.display_name,
            snapshot.manifest_sha256,
            snapshot.root.to_string_lossy(),
            playable_root.to_string_lossy(),
            workspace_root.to_string_lossy(),
            retro_junk_archive::projection_generation(&snapshot.root).unwrap_or(0),
            catalog_generation.encoded(),
        ],
    )?;

    for release in snapshot
        .releases
        .iter()
        .filter(|release| changed.contains(&release.manifest.archive_release_id.to_string()))
    {
        // A release has no catalog identity of its own to read: its manifest
        // describes what the archive holds, and the catalog identity comes up
        // from its carriers once they have each resolved by content. That
        // happens in `bind_releases_from_carriers`, after every carrier below
        // has been projected — so the row starts unbound and is filled in.
        tx.execute(
            "INSERT INTO archive_releases(id,profile_id,catalog_work_id,catalog_release_id,platform_id,title,region,revision,variant,manifest_path,manifest_sha256,projection_fingerprint,binding_state)
             VALUES(?1,?2,NULL,NULL,?3,?4,?5,?6,?7,?8,?9,?10,'unbound')",
            params![
                release.manifest.archive_release_id.to_string(),
                profile_id,
                release.manifest.platform_id,
                release.manifest.title,
                release.manifest.region,
                release.manifest.revision,
                release.manifest.variant,
                relative(&snapshot.root, &release.directory.join("release.toml")),
                release.manifest_sha256,
                fingerprints[&release.manifest.archive_release_id.to_string()],
            ],
        )?;
        for file in &release.supporting_files {
            tx.execute(
                "INSERT INTO archive_release_files(id,archive_release_id,category,asset_type,relative_path,file_size,sha256,source,source_url,caption,captured_at,manifest_path,manifest_sha256)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
                params![
                    file.manifest.supporting_file_id.to_string(),
                    release.manifest.archive_release_id.to_string(),
                    release_file_category_key(file.manifest.category),
                    file.manifest.asset_type,
                    relative(&snapshot.root, &file.directory.join(&file.manifest.file.path)),
                    file.manifest.file.size,
                    file.manifest.file.sha256,
                    file.manifest.source,
                    file.manifest.source_url,
                    file.manifest.caption,
                    file.manifest.captured_at,
                    relative(&snapshot.root, &file.directory.join("supporting-file.toml")),
                    file.manifest_sha256,
                ],
            )?;
        }
        for physical_copy in &release.physical_copies {
            tx.execute(
                "INSERT INTO physical_copies(id,archive_release_id,copy_number,owner_id,label,condition,notes,date_acquired,provenance,manifest_path,manifest_sha256)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                params![
                    physical_copy.manifest.physical_copy_id.to_string(),
                    release.manifest.archive_release_id.to_string(),
                    physical_copy.manifest.copy_number,
                    physical_copy.manifest.owner_id,
                    physical_copy.manifest.label,
                    physical_copy.manifest.condition,
                    physical_copy.manifest.notes,
                    physical_copy.manifest.date_acquired,
                    physical_copy.manifest.provenance,
                    relative(&snapshot.root, &physical_copy.directory.join("physical-copy.toml")),
                    physical_copy.manifest_sha256,
                ],
            )?;
            for file in &physical_copy.supporting_files {
                tx.execute(
                    "INSERT INTO physical_copy_files(id,physical_copy_id,category,asset_type,relative_path,sha256,caption,source)
                     VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                    params![
                        file.manifest.supporting_file_id.to_string(),
                        physical_copy.manifest.physical_copy_id.to_string(),
                        physical_copy_file_category_key(file.manifest.category),
                        file.manifest.asset_type,
                        relative(
                            &snapshot.root,
                            &file.directory.join(&file.manifest.file.path),
                        ),
                        file.manifest.file.sha256,
                        file.manifest.caption,
                        file.manifest.source,
                    ],
                )?;
            }
            for carrier in &physical_copy.carriers {
                // Which catalog medium a carrier is comes from the digests the
                // archive itself recorded, every time — never from an id
                // written into the manifest. Ids used to be derived from the
                // game's title, so an archive written on one machine named
                // rows a differently versioned import on another machine never
                // created; the digests are the same everywhere.
                let (catalog_media, media_binding) = match resolved_catalog_media(
                    &tx,
                    &release.manifest.platform_id,
                    carrier,
                    &chosen,
                )? {
                    Some(id) => (Some(id), "bound"),
                    None => (None, "unbound"),
                };
                tx.execute(
                    "INSERT INTO carriers(id,physical_copy_id,catalog_media_id,kind,serial,sequence_number,label,manifest_path,manifest_sha256,binding_state)
                     VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                    params![
                        carrier.manifest.carrier_id.to_string(),
                        physical_copy.manifest.physical_copy_id.to_string(),
                        catalog_media,
                        carrier_kind_key(&carrier.manifest.kind),
                        carrier.manifest.serial,
                        carrier.manifest.sequence_number,
                        carrier.manifest.label,
                        relative(&snapshot.root, &carrier.directory.join("carrier.toml")),
                        carrier.manifest_sha256,
                        media_binding,
                    ],
                )?;
                let effective_policy = carrier.manifest.playable_policy.as_ref().or_else(|| {
                    snapshot
                        .manifest
                        .platform_defaults
                        .iter()
                        .find(|default| {
                            default
                                .platform_id
                                .eq_ignore_ascii_case(&release.manifest.platform_id)
                        })
                        .or_else(|| {
                            snapshot.manifest.platform_defaults.iter().find(|default| {
                                same_platform(&default.platform_id, &release.manifest.platform_id)
                            })
                        })
                        .map(|default| &default.policy)
                });
                if let Some(policy) = carrier.manifest.playable_policy.as_ref() {
                    insert_projected_policy(
                        &tx,
                        "carrier_override",
                        &carrier.manifest.carrier_id.to_string(),
                        policy,
                    )?;
                }
                if let Some(policy) = effective_policy {
                    insert_projected_policy(
                        &tx,
                        "carrier",
                        &carrier.manifest.carrier_id.to_string(),
                        policy,
                    )?;
                }
                for dump in &carrier.dumps {
                    let format = format_key(&dump.manifest.format);
                    let relative_dump = relative(&snapshot.root, &dump.directory);
                    let master_presence = preservation_presence(&dump.directory, &dump.manifest);
                    let integrity_state = if retro_junk_archive::dump_has_current_evidence(
                        dump,
                        retro_junk_archive::VerificationKind::Integrity,
                    ) {
                        "verified"
                    } else {
                        "unknown"
                    };
                    // One rule decides "catalog-verified"; the identity the
                    // catalog agreed on comes from that same evidence rather
                    // than from a second, drifting predicate.
                    let catalog_evidence = retro_junk_archive::dump_catalog_evidence(dump);
                    // "Tried and got nowhere" is a third state, not the same as
                    // never having tried. Collapsing the two made every disc the
                    // catalog cannot resolve look untouched, so convergence
                    // proposed a fresh reproduction for it on every single run —
                    // a full copy and split of the raw dump each time.
                    let catalog_state = if catalog_evidence.is_some() {
                        CATALOG_VERIFIED
                    } else if retro_junk_archive::dump_catalog_attempted(dump) {
                        CATALOG_UNRESOLVED
                    } else {
                        CATALOG_NOT_ATTEMPTED
                    };
                    let catalog_game = catalog_evidence.map_or("", |catalog| catalog.game.as_str());
                    tx.execute(
                        "INSERT INTO dump_events(id,carrier_id,representation_id,format,captured_at,manifest_path,manifest_sha256,integrity_state,catalog_state)
                         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                        params![
                            dump.manifest.dump_id.to_string(),
                            carrier.manifest.carrier_id.to_string(),
                            dump.manifest.representation_id.to_string(),
                            format,
                            dump.manifest.captured_at,
                            relative(&snapshot.root, &dump.directory.join("dump.toml")),
                            dump.manifest_sha256,
                            integrity_state,
                            catalog_state,
                        ],
                    )?;
                    tx.execute(
                        "INSERT INTO representations(id,carrier_id,dump_id,role,format,location_role,relative_path,presence_state,input_manifest_sha256,catalog_verified,catalog_game,round_trip_verified)
                         VALUES(?1,?2,?3,?4,?5,'archive',?6,?7,?8,?9,?10,0)",
                        params![
                            dump.manifest.representation_id.to_string(),
                            carrier.manifest.carrier_id.to_string(),
                            dump.manifest.dump_id.to_string(),
                            role_key(&RepresentationRole::PreservationMaster),
                            format,
                            relative_dump,
                            master_presence.as_str(),
                            dump.manifest_sha256,
                            catalog_evidence.is_some(),
                            catalog_game,
                        ],
                    )?;
                    for file in &dump.manifest.files {
                        tx.execute(
                            "INSERT INTO representation_files(representation_id,relative_path,file_size,sha256,crc32,md5,sha1)
                             VALUES(?1,?2,?3,?4,?5,?6,?7)",
                            params![
                                dump.manifest.representation_id.to_string(),
                                file.path,
                                file.size,
                                file.sha256,
                                file.crc32,
                                file.md5,
                                file.sha1,
                            ],
                        )?;
                    }
                    for verification in &dump.verifications {
                        let catalog = verification.evidence.catalog.as_ref();
                        tx.execute(
                            "INSERT INTO verification_events(id,representation_id,kind,outcome,performed_at,input_manifest_sha256,evidence_path,catalog_source,catalog_version,catalog_game,complete_track_set,detail)
                             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                            params![
                                verification.evidence.verification_id.to_string(),
                                verification.evidence.representation_id.to_string(),
                                verification_kind_key(verification.evidence.kind),
                                verification_outcome_key(verification.evidence.outcome),
                                verification.evidence.performed_at,
                                verification.evidence.input_manifest_sha256,
                                relative(&snapshot.root, &verification.path),
                                catalog.map_or("", |value| value.source.as_str()),
                                catalog.map_or("", |value| value.version.as_str()),
                                catalog.map_or("", |value| value.game.as_str()),
                                catalog.is_some_and(|value| value.complete_track_set),
                                verification.evidence.detail,
                            ],
                        )?;
                    }
                    // Build evidence is append-only history: rebuilding a
                    // derivative — or re-adopting one that moved — appends a
                    // second record for it. A representation row is the
                    // *current* state of one file, so only the newest build in
                    // each lineage is projected and the superseded records stay
                    // in `evidence/`.
                    let projected_builds = current_builds_by_output(
                        dump,
                        playable_root,
                        &release.manifest.platform_id,
                        &release.manifest.region,
                        &dump.manifest_sha256,
                    );
                    for (index, build) in dump.builds.iter().enumerate() {
                        let Some((playable_relative_path, presence)) =
                            projected_builds.get(&index).cloned()
                        else {
                            continue;
                        };
                        let child_id = build.evidence.child_representation_id.to_string();
                        if let Some(intermediate) = &build.evidence.canonical_intermediate {
                            let intermediate_directory =
                                dump.directory.join(&intermediate.relative_path);
                            let intermediate_presence = retro_junk_archive::archived_files_presence(
                                &intermediate_directory.join("raw"),
                                &intermediate.files,
                            );
                            let intermediate_relative =
                                relative(&snapshot.root, &intermediate_directory);
                            tx.execute(
                                "INSERT INTO representations(id,carrier_id,dump_id,role,format,location_role,relative_path,presence_state,input_manifest_sha256,content_size,recipe_version)
                                 VALUES(?1,?2,NULL,'canonical_intermediate',?3,'archive',?4,?5,?6,?7,?8)",
                                params![
                                    intermediate.representation_id.to_string(),
                                    carrier.manifest.carrier_id.to_string(),
                                    format_key(&intermediate.format),
                                    intermediate_relative,
                                    intermediate_presence.as_str(),
                                    build.evidence.input_manifest_sha256,
                                    intermediate.files.iter().map(|file| file.size).sum::<u64>(),
                                    build.evidence.recipe_version,
                                ],
                            )?;
                            for file in &intermediate.files {
                                tx.execute(
                                    "INSERT INTO representation_files(representation_id,relative_path,file_size,sha256,crc32,md5,sha1)
                                     VALUES(?1,?2,?3,?4,?5,?6,?7)",
                                    params![
                                        intermediate.representation_id.to_string(),
                                        file.path,
                                        file.size,
                                        file.sha256,
                                        file.crc32,
                                        file.md5,
                                        file.sha1,
                                    ],
                                )?;
                            }
                        }
                        tx.execute(
                            "INSERT INTO representations(id,carrier_id,dump_id,role,format,location_role,relative_path,presence_state,input_manifest_sha256,content_sha256,content_size,catalog_verified,round_trip_verified,recipe_version)
                             VALUES(?1,?2,NULL,'playable',?3,'playable',?4,?5,?6,?7,?8,?9,?10,?11)",
                            params![
                                child_id,
                                carrier.manifest.carrier_id.to_string(),
                                format_key(&build.evidence.format),
                                playable_relative_path,
                                presence.as_str(),
                                build.evidence.input_manifest_sha256,
                                build.evidence.output_sha256,
                                build.evidence.output_size,
                                build.evidence.catalog_verified,
                                build.evidence.round_trip_verified,
                                build.evidence.recipe_version,
                            ],
                        )?;
                        tx.execute(
                            "INSERT INTO representation_files(representation_id,relative_path,file_size,sha256)
                             VALUES(?1,?2,?3,?4)",
                            params![
                                child_id,
                                playable_relative_path,
                                build.evidence.output_size,
                                build.evidence.output_sha256,
                            ],
                        )?;
                        tx.execute(
                            "INSERT INTO derivations(id,parent_representation_id,child_representation_id,recipe_version,evidence_path,created_at)
                             VALUES(?1,?2,?3,?4,?5,?6)",
                            params![
                                build.evidence.build_id.to_string(),
                                build.evidence.parent_representation_id.to_string(),
                                child_id,
                                build.evidence.recipe_version,
                                relative(&snapshot.root, &build.path),
                                build.evidence.performed_at,
                            ],
                        )?;
                    }
                }
            }
        }
    }

    bind_releases_from_carriers(&tx)?;
    rebuild_library_entry_bindings(&tx)?;
    apply_archive_derivations(&tx, ArchiveEvidenceScope::All)?;
    // The user's own decisions, kept beside the collection because this
    // database is device-local and rebuilt from DATs. Applying them here is
    // what makes a mark made on one machine mean something on the next.
    apply_collection_marks(
        &tx,
        &retro_junk_archive::collection_root_for(&snapshot.root, playable_root),
    )?;
    tx.commit()?;
    Ok(())
}

/// Refresh only the supporting-file rows for releases whose artwork or other
/// release-level media changed.
///
/// The release, carrier, dump, and representation projections are unaffected
/// by adding artwork, so rebuilding the complete archive projection here is
/// both unnecessary and disproportionately expensive.
pub fn reconcile_archive_supporting_files(
    conn: &mut Connection,
    archive_root: &Path,
    releases: &[retro_junk_archive::IndexedRelease],
) -> Result<(), OperationError> {
    let tx = conn.transaction()?;
    for release in releases {
        let release_id = release.manifest.archive_release_id.to_string();
        let playable_root: String = tx.query_row(
            "SELECT ap.playable_root FROM archive_profiles ap
             JOIN archive_releases ar ON ar.profile_id=ap.id
             WHERE ar.id=?1",
            [&release_id],
            |row| row.get(0),
        )?;
        tx.execute(
            "DELETE FROM archive_release_files WHERE archive_release_id=?1",
            [&release_id],
        )?;
        tx.execute(
            "DELETE FROM physical_copy_files
             WHERE physical_copy_id IN (
                 SELECT id FROM physical_copies WHERE archive_release_id=?1
             )",
            [&release_id],
        )?;
        for file in &release.supporting_files {
            tx.execute(
                "INSERT INTO archive_release_files(id,archive_release_id,category,asset_type,relative_path,file_size,sha256,source,source_url,caption,captured_at,manifest_path,manifest_sha256)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
                params![
                    file.manifest.supporting_file_id.to_string(),
                    release_id,
                    release_file_category_key(file.manifest.category),
                    file.manifest.asset_type,
                    relative(archive_root, &file.directory.join(&file.manifest.file.path)),
                    file.manifest.file.size,
                    file.manifest.file.sha256,
                    file.manifest.source,
                    file.manifest.source_url,
                    file.manifest.caption,
                    file.manifest.captured_at,
                    relative(archive_root, &file.directory.join("supporting-file.toml")),
                    file.manifest_sha256,
                ],
            )?;
        }
        for physical_copy in &release.physical_copies {
            for file in &physical_copy.supporting_files {
                tx.execute(
                    "INSERT INTO physical_copy_files(id,physical_copy_id,category,asset_type,relative_path,sha256,caption,source)
                     VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                    params![
                        file.manifest.supporting_file_id.to_string(),
                        physical_copy.manifest.physical_copy_id.to_string(),
                        physical_copy_file_category_key(file.manifest.category),
                        file.manifest.asset_type,
                        relative(
                            archive_root,
                            &file.directory.join(&file.manifest.file.path),
                        ),
                        file.manifest.file.sha256,
                        file.manifest.caption,
                        file.manifest.source,
                    ],
                )?;
            }
        }
        tx.execute(
            "UPDATE archive_releases SET projection_fingerprint=?1 WHERE id=?2",
            params![
                release_projection_fingerprint(release, Path::new(&playable_root)),
                release_id,
            ],
        )?;
    }
    if let Some(release) = releases.first() {
        tx.execute(
            "UPDATE archive_profiles
             SET source_generation=?1,indexed_at=datetime('now')
             WHERE id=(SELECT profile_id FROM archive_releases WHERE id=?2)",
            params![
                retro_junk_archive::projection_generation(archive_root).unwrap_or(0),
                release.manifest.archive_release_id.to_string(),
            ],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Give a release the catalog identity its own carriers proved by content.
///
/// This is how an archive release gets a catalog identity at all. Nothing on
/// disk names one: a release manifest describes what the archive holds, and
/// each carrier under it resolves to a catalog medium from its recorded
/// digests. The release those media belong to is the release.
///
/// Only unanimity binds a release id. Carriers from different mastering
/// records legitimately disagree — a boxed set whose discs came from two
/// pressings is one owned thing but two catalog releases — and that case is
/// bound at the work level instead, which is the honest answer rather than
/// picking one of the two.
fn bind_releases_from_carriers(conn: &Connection) -> Result<(), OperationError> {
    let mut statement = conn.prepare(
        "SELECT ar.id,
                COUNT(DISTINCT m.release_id),
                MIN(m.release_id),
                COUNT(DISTINCT r.work_id),
                MIN(r.work_id)
         FROM archive_releases ar
         JOIN physical_copies pc ON pc.archive_release_id=ar.id
         JOIN carriers c ON c.physical_copy_id=pc.id
         JOIN media m ON m.id=c.catalog_media_id
         JOIN releases r ON r.id=m.release_id
         WHERE ar.catalog_release_id IS NULL
         GROUP BY ar.id",
    )?;
    let by_carriers = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for (archive_release_id, release_count, release_id, work_count, work_id) in by_carriers {
        // One work, one release: the carriers describe a single mastering.
        if release_count == 1 && work_count == 1 {
            conn.execute(
                "UPDATE archive_releases
                 SET catalog_release_id=?1,catalog_work_id=?2,binding_state='bound'
                 WHERE id=?3",
                params![release_id, work_id, archive_release_id],
            )?;
            log::debug!(
                "Archive release {archive_release_id} is catalog release {} by its carriers' content",
                release_id.unwrap_or_default()
            );
        } else if work_count == 1 {
            // Mixed masterings of one game: work-level is the honest answer.
            conn.execute(
                "UPDATE archive_releases
                 SET catalog_work_id=?1,binding_state='bound'
                 WHERE id=?2 AND catalog_work_id IS NULL",
                params![work_id, archive_release_id],
            )?;
        }
    }
    Ok(())
}

/// Rebuild the bridge between the playable-library projection and archival
/// carriers.
///
/// Every rule here is derived from committed projection rows, so this is safe
/// to re-run at any time — after a reindex, or once during a schema migration
/// that changes what a binding means.
///
/// A binding is always to the *carrier*: the archived thing whose evidence
/// produced (or matches) the scanned file. The carrier's catalog medium rides
/// along when it has one, but is never what makes the file archived.
// Three binding rules in one place, deliberately: what makes a scanned file an
// archived carrier's own copy should be readable end to end.
#[allow(clippy::too_many_lines)]
pub(crate) fn rebuild_library_entry_bindings(conn: &Connection) -> Result<(), OperationError> {
    rebuild_library_entry_bindings_scoped(conn, None)
}

/// Rebuild archive bindings for only the library rows a scan actually
/// changed. New rows need this before archive-derived naming runs; waiting for
/// a whole archive refresh made their result depend on a later UI message.
pub(crate) fn rebuild_library_entry_bindings_for_entries(
    conn: &Connection,
    entries: &[crate::library::LibraryEntryId],
) -> Result<(), OperationError> {
    for entry in entries {
        rebuild_library_entry_bindings_scoped(conn, Some(entry.0))?;
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn rebuild_library_entry_bindings_scoped(
    conn: &Connection,
    entry_id: Option<u64>,
) -> Result<(), OperationError> {
    let entry_filter = entry_id.map_or_else(String::new, |id| format!(" AND le.id={id}"));
    let binding_filter =
        entry_id.map_or_else(String::new, |id| format!(" AND library_entry_id={id}"));
    conn.execute(
        &format!(
            "DELETE FROM library_entry_media_bindings
         WHERE match_method IN (
             'archive_projection',
             'archive_output_path',
             'archive_release_projection'
         ){binding_filter}"
        ),
        [],
    )?;
    // A playable build is already provenance evidence: its manifest records
    // the exact output path below the profile's playable root. Bind a scanned
    // library row that holds that path directly instead of asking CHD (or
    // another derivative container) to reproduce the preservation dump's raw
    // hashes. A multi-disc row holds one such output per archived disc.
    conn.execute(
        &format!(
            "INSERT OR REPLACE INTO library_entry_media_bindings(
             library_entry_id,carrier_id,catalog_media_id,representation_id,match_method)
         SELECT DISTINCT le.id,c.id,c.catalog_media_id,rep.id,'archive_output_path'
         FROM representations rep
         {PLAYABLE_REPRESENTATION_TO_LIBRARY_ENTRY_JOIN}
         WHERE rep.role='playable'
           AND rep.location_role='playable'
           AND rep.presence_state='present'
           AND {holds_playable}{entry_filter}",
            holds_playable = playable_path_is_within_library_entry()
        ),
        [],
    )?;
    // Matching on shared, normalized catalog hashes. In particular, cartridge
    // library hashes omit format headers (for example iNES), just as the
    // catalog does; comparing archive-file sizes here would miss them.
    conn.execute(
        &format!(
            "INSERT OR REPLACE INTO library_entry_media_bindings(library_entry_id,carrier_id,catalog_media_id,representation_id,match_method)
         SELECT DISTINCT le.id,c.id,c.catalog_media_id,
                (SELECT rep.id FROM representations rep
                 WHERE rep.carrier_id=c.id AND rep.role='playable'
                   AND {holds_playable}
                 ORDER BY rep.id LIMIT 1),
                'archive_projection'
         FROM carriers c
         JOIN physical_copies pc ON pc.id=c.physical_copy_id
         JOIN archive_releases ar ON ar.id=pc.archive_release_id
         JOIN media m ON m.id=c.catalog_media_id
         JOIN releases cr ON cr.id=m.release_id
         JOIN library_consoles lc
           ON lower(lc.folder_name)=lower(cr.platform_id)
              OR lower(lc.platform)=lower(cr.platform_id)
         JOIN library_entries le ON le.console_id=lc.id AND le.data_size=m.file_size
         WHERE c.catalog_media_id IS NOT NULL
           AND ((le.sha1<>'' AND m.sha1<>'' AND le.sha1=m.sha1)
                OR (le.md5<>'' AND m.md5<>'' AND le.md5=m.md5)
                OR (le.crc32<>'' AND m.crc32<>'' AND le.crc32=m.crc32))
           {entry_filter}",
            holds_playable = playable_path_is_within_library_entry()
        ),
        [],
    )?;
    // Disc containers and M3U sets often cannot expose Redump's raw per-track
    // hashes cheaply (or at all), but analysis can still identify their exact
    // DAT game names from serials. Bind the logical library entry to every
    // archived carrier in that catalog release. This makes a multi-disc set
    // one archived/playable game without pretending that one matched disc is
    // evidence that the other archive carriers are verified.
    conn.execute(
        &format!(
            "WITH entry_releases(library_entry_id,release_id) AS (
             SELECT DISTINCT le.id,m.release_id
             FROM library_entries le
             JOIN library_consoles lc ON lc.id=le.console_id
             JOIN media m ON m.dat_name=le.dat_game_name
             JOIN releases r ON r.id=m.release_id
             WHERE le.dat_game_name<>''
               AND (lower(lc.folder_name)=lower(r.platform_id)
                    OR lower(lc.platform)=lower(r.platform_id))
               AND (SELECT COUNT(DISTINCT unique_media.release_id)
                    FROM media unique_media
                    JOIN releases unique_release ON unique_release.id=unique_media.release_id
                    WHERE unique_media.dat_name=le.dat_game_name
                      AND (lower(lc.folder_name)=lower(unique_release.platform_id)
                           OR lower(lc.platform)=lower(unique_release.platform_id)))=1
             UNION
             SELECT DISTINCT le.id,m.release_id
             FROM library_entries le
             JOIN library_consoles lc ON lc.id=le.console_id
             JOIN json_each(le.disc_identifications_json) disc
             JOIN media m ON m.dat_name=json_extract(disc.value,'$.dat_match.game_name')
             JOIN releases r ON r.id=m.release_id
             WHERE json_valid(le.disc_identifications_json)
               AND json_extract(disc.value,'$.dat_match.game_name') IS NOT NULL
               AND (lower(lc.folder_name)=lower(r.platform_id)
                    OR lower(lc.platform)=lower(r.platform_id))
               AND (SELECT COUNT(DISTINCT unique_media.release_id)
                    FROM media unique_media
                    JOIN releases unique_release ON unique_release.id=unique_media.release_id
                    WHERE unique_media.dat_name=
                          json_extract(disc.value,'$.dat_match.game_name')
                      AND (lower(lc.folder_name)=lower(unique_release.platform_id)
                           OR lower(lc.platform)=lower(unique_release.platform_id)))=1
         )
         INSERT OR REPLACE INTO library_entry_media_bindings(
             library_entry_id,carrier_id,catalog_media_id,representation_id,match_method)
         SELECT DISTINCT er.library_entry_id,c.id,c.catalog_media_id,NULL,
                'archive_release_projection'
         FROM entry_releases er
         JOIN media m ON m.release_id=er.release_id
         JOIN carriers c ON c.catalog_media_id=m.id
         JOIN physical_copies pc ON pc.id=c.physical_copy_id
         JOIN archive_releases ar ON ar.id=pc.archive_release_id
         JOIN archive_profiles ap ON ap.id=ar.profile_id
         JOIN library_entries le ON le.id=er.library_entry_id
         JOIN library_consoles lc ON lc.id=le.console_id
         JOIN library_roots lr ON lr.id=lc.root_id
         WHERE ap.playable_root=lr.root_path
           {entry_filter}"
        ),
        [],
    )?;
    Ok(())
}

/// Library rows the archive's own evidence can name, without consulting the
/// catalog tables at all.
///
/// This is what makes an archive portable between machines: the dump's catalog
/// verification recorded which catalog game it matched, and the build evidence
/// recorded where that dump's playable output was written. A machine that has
/// never imported a DAT can still say what the file is.
///
pub fn archive_evidence_identities(
    conn: &Connection,
    scope: ArchiveEvidenceScope,
) -> Result<Vec<ArchiveEvidenceIdentity>, OperationError> {
    let mut statement = conn.prepare(&format!(
        "SELECT le.id,le.entry_key,master.catalog_game,rep.id,ar.id
         FROM representations rep
         {PLAYABLE_REPRESENTATION_TO_LIBRARY_ENTRY_JOIN}
         JOIN derivations d ON d.child_representation_id=rep.id
         JOIN representations master ON master.id=d.parent_representation_id
         WHERE rep.role='playable'
           AND rep.location_role='playable'
           AND rep.presence_state='present'
           AND master.catalog_verified=1
           AND master.catalog_game<>''
           AND (?1=0 OR lc.id=?1)
           AND (?2=0 OR le.id=?2)
           AND {is_playable}
         ORDER BY le.id",
        is_playable = playable_path_is_library_entry()
    ))?;
    let (console_filter, entry_filter) = scope.filters();
    let rows = statement.query_map(params![console_filter, entry_filter], |row| {
        Ok(ArchiveEvidenceIdentity {
            library_entry_id: row.get(0)?,
            entry_key: row.get(1)?,
            game_name: row.get(2)?,
            representation_id: row.get(3)?,
            archive_release_id: row.get(4)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Name library rows the catalog could not, from archive evidence.
///
/// Only rows analysis left unidentified are touched: a live catalog hash
/// comparison is stronger evidence about the bytes on disk than a recorded
/// verification, so a catalog verdict always wins, and user tags are never
/// overwritten. Returns the rows this pass named.
pub fn apply_archive_evidence_identities(
    conn: &Connection,
    scope: ArchiveEvidenceScope,
) -> Result<Vec<crate::LibraryEntryId>, OperationError> {
    let identities = archive_evidence_identities(conn, scope)?;
    if identities.is_empty() {
        return Ok(Vec::new());
    }
    let mut statement = conn.prepare(
        "UPDATE library_entries
         SET status='matched',dat_game_name=?2,dat_match_method='archive_evidence',
             revision=revision+1
         WHERE id=?1 AND tag='' AND dat_game_name=''
           AND status IN ('unknown','unrecognized')",
    )?;
    let mut updated = Vec::new();
    for identity in &identities {
        if statement.execute(params![identity.library_entry_id, identity.game_name])? > 0 {
            updated.push(crate::LibraryEntryId(
                u64::try_from(identity.library_entry_id).unwrap_or_default(),
            ));
        }
    }
    if !updated.is_empty() {
        log::info!(
            "Named {} library row(s) from archive evidence",
            updated.len()
        );
    }
    Ok(updated)
}

/// Hashes the archive already computed for a file the library also holds.
///
/// A playable representation whose recorded content SHA-256 equals its single
/// archived master file's is the same bytes under another name, so the master's
/// digests describe the library file exactly — no second read required.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdoptableHashes {
    pub library_entry_id: i64,
    pub platform_id: String,
    pub digests: retro_junk_archive::FileDigests,
}

/// Library rows whose hashes are already recorded in the archive.
///
/// Restricted to rows with no locally computed hashes, and to playables that
/// are byte-identical mirrors of a single-file master: a derived container
/// (CHD, RVZ) shares no bytes with its master, so the master's digests say
/// nothing about the playable file.
pub fn adoptable_archive_hashes(
    conn: &Connection,
    scope: ArchiveEvidenceScope,
) -> Result<Vec<AdoptableHashes>, OperationError> {
    let mut statement = conn.prepare(&format!(
        "SELECT le.id,ar.platform_id,mf.file_size,mf.crc32,mf.md5,mf.sha1,mf.sha256
         FROM representations rep
         {PLAYABLE_REPRESENTATION_TO_LIBRARY_ENTRY_JOIN}
         JOIN derivations d ON d.child_representation_id=rep.id
         JOIN representations master ON master.id=d.parent_representation_id
         JOIN representation_files mf ON mf.representation_id=master.id
         WHERE rep.role='playable'
           AND rep.location_role='playable'
           AND rep.presence_state='present'
           AND rep.content_sha256<>''
           AND mf.sha256=rep.content_sha256
           AND mf.crc32<>'' AND mf.sha1<>''
           AND le.crc32='' AND le.sha1='' AND le.md5=''
           AND le.tag=''
           AND (SELECT COUNT(*) FROM representation_files sibling
                WHERE sibling.representation_id=master.id)=1
           AND (?1=0 OR lc.id=?1)
           AND (?2=0 OR le.id=?2)
           AND {is_playable}
         UNION
         -- A derivative that is not a byte-identical mirror — a CHD of a
         -- multi-track disc — can never satisfy the digest equality above, so
         -- disc rows kept asking to be read even though the archive had
         -- already answered. Round-trip verification decompressed this
         -- derivative and compared it back against the master, and the
         -- master's complete track set matched this catalog medium; the file
         -- therefore holds the catalog's bytes, and reading it again cannot
         -- produce a different answer. Both flags are required: either alone
         -- would be a guess.
         SELECT le.id,ar.platform_id,m.file_size,m.crc32,m.md5,m.sha1,''
         FROM representations rep
         {PLAYABLE_REPRESENTATION_TO_LIBRARY_ENTRY_JOIN}
         JOIN media m ON m.id=c.catalog_media_id
         WHERE rep.role='playable'
           AND rep.location_role='playable'
           AND rep.presence_state='present'
           AND rep.round_trip_verified=1
           AND rep.catalog_verified=1
           AND m.crc32<>'' AND m.sha1<>''
           AND le.crc32='' AND le.sha1='' AND le.md5=''
           AND le.tag=''
           AND (?1=0 OR lc.id=?1)
           AND (?2=0 OR le.id=?2)
           AND {is_playable}
         ORDER BY 1",
        is_playable = playable_path_is_library_entry()
    ))?;
    let (console_filter, entry_filter) = scope.filters();
    let rows = statement.query_map(params![console_filter, entry_filter], |row| {
        Ok(AdoptableHashes {
            library_entry_id: row.get(0)?,
            platform_id: row.get(1)?,
            digests: retro_junk_archive::FileDigests {
                size: row.get::<_, i64>(2)?.try_into().unwrap_or_default(),
                crc32: row.get(3)?,
                md5: row.get(4)?,
                sha1: row.get(5)?,
                sha256: row.get(6)?,
            },
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Fill a library row's hash cache from the archive instead of re-reading the
/// file, and name it from the catalog medium those hashes identify.
///
/// Adoption is deliberately conditional on the recorded digests matching
/// exactly one catalog medium. The archive stores raw file digests while the
/// library hashes format-aware payloads (an iNES or copier header is skipped),
/// so recorded digests are only interchangeable with computed ones when the
/// catalog itself confirms they describe a known dump. Anything ambiguous or
/// unmatched is left for a real hash pass to read.
///
/// Rows filled this way record `hash_source='archive_evidence'`: they are a
/// cache of what the archive proved, not evidence that the bytes on disk were
/// read on this machine.
pub fn adopt_archive_hashes(
    conn: &Connection,
    scope: ArchiveEvidenceScope,
) -> Result<Vec<crate::LibraryEntryId>, OperationError> {
    let candidates = adoptable_archive_hashes(conn, scope)?;
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let mut statement = conn.prepare(
        "UPDATE library_entries
         SET crc32=?2,sha1=?3,md5=?4,data_size=?5,hash_source='archive_evidence',
             status='matched',dat_game_name=?6,dat_rom_name=?7,dat_match_method='crc32',
             ambiguous_candidates_json=NULL,revision=revision+1
         WHERE id=?1 AND tag='' AND crc32='' AND sha1='' AND md5=''",
    )?;
    let mut adopted = Vec::new();
    for candidate in &candidates {
        // Archives file releases under regional platform directories
        // (`super-famicom`); the catalog keys them by canonical platform.
        let candidates = match_catalog_file(
            conn,
            catalog_platform_id(&candidate.platform_id).as_str(),
            &candidate.digests,
        )?;
        let [confirmed] = candidates.as_slice() else {
            continue;
        };
        let (dat_name, rom_name) = catalog_media_names(conn, &confirmed.media_id)?;
        let updated = statement.execute(params![
            candidate.library_entry_id,
            candidate.digests.crc32,
            candidate.digests.sha1,
            candidate.digests.md5,
            i64::try_from(candidate.digests.size).unwrap_or(i64::MAX),
            dat_name,
            rom_name,
        ])?;
        if updated > 0 {
            adopted.push(crate::LibraryEntryId(
                u64::try_from(candidate.library_entry_id).unwrap_or_default(),
            ));
        }
    }
    if !adopted.is_empty() {
        log::info!(
            "Adopted archive hashes for {} library row(s) without re-reading them",
            adopted.len()
        );
    }
    Ok(adopted)
}

/// Everything the archive can tell the library without reading a file: the
/// hashes it already computed, then the identities its evidence established for
/// whatever the catalog still could not name.
pub fn apply_archive_derivations(
    conn: &Connection,
    scope: ArchiveEvidenceScope,
) -> Result<Vec<crate::LibraryEntryId>, OperationError> {
    let mut touched = adopt_archive_hashes(conn, scope)?;
    for id in apply_archive_evidence_identities(conn, scope)? {
        if !touched.contains(&id) {
            touched.push(id);
        }
    }
    Ok(touched)
}

fn catalog_media_names(
    conn: &Connection,
    media_id: &str,
) -> Result<(String, String), OperationError> {
    conn.query_row(
        "SELECT dat_name,rom_name FROM media WHERE id=?1",
        [media_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .map_err(Into::into)
}

fn insert_projected_policy(
    tx: &Transaction<'_>,
    scope_type: &str,
    scope_id: &str,
    policy: &retro_junk_archive::DesiredPlayablePolicy,
) -> Result<(), OperationError> {
    tx.execute(
        "INSERT INTO playable_policies(scope_type,scope_id,format,retain_intermediate,allow_unverified,options_json)
         VALUES(?1,?2,?3,?4,?5,?6)",
        params![
            scope_type,
            scope_id,
            format_key(&policy.format),
            policy.retain_canonical_intermediate,
            policy.allow_unverified,
            serde_json::to_string(&policy.options)
                .map_err(|error| OperationError::InvalidData(error.to_string()))?,
        ],
    )?;
    Ok(())
}

/// Ordered on-disk paths of the already-built playable discs for a release,
/// for playlist-only builds. Fails unless the set is complete and ordered —
/// a playlist over a partial set would claim completeness the files can't
/// back.
pub fn existing_playable_disc_paths(
    conn: &Connection,
    archive_release_id: &str,
    playable_root: &std::path::Path,
    expected_disc_count: u32,
) -> Result<Vec<std::path::PathBuf>, OperationError> {
    let mut statement = conn.prepare(
        "SELECT DISTINCT m.disc_number,e.game_entry_json,lc.folder_path
         FROM archive_releases ar
         JOIN physical_copies pc ON pc.archive_release_id=ar.id
         JOIN carriers c ON c.physical_copy_id=pc.id
         JOIN media m ON m.id=c.catalog_media_id AND m.disc_number>0
         JOIN library_entries e ON e.dat_game_name=m.dat_name
         JOIN library_consoles lc ON lc.id=e.console_id
         JOIN library_roots lr ON lr.id=lc.root_id
         WHERE ar.id=?1 AND lr.root_path=?2
         ORDER BY m.disc_number",
    )?;
    let rows = statement
        .query_map(
            (archive_release_id, playable_root.to_string_lossy().as_ref()),
            |row| {
                Ok((
                    row.get::<_, u32>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    let mut paths = Vec::new();
    for (index, (disc_number, json, folder)) in rows.into_iter().enumerate() {
        if disc_number != u32::try_from(index + 1).unwrap_or(u32::MAX) {
            return Err(OperationError::InvalidData(
                "existing playable discs are not a complete ordered set".to_owned(),
            ));
        }
        let value: serde_json::Value = serde_json::from_str(&json)
            .map_err(|error| OperationError::InvalidData(error.to_string()))?;
        let path = value
            .get("SingleFile")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                OperationError::InvalidData(
                    "an existing disc is not a single-file playable entry".to_owned(),
                )
            })?;
        let path = std::path::PathBuf::from(path);
        paths.push(if path.is_absolute() {
            path
        } else {
            std::path::PathBuf::from(folder).join(path)
        });
    }
    if paths.len() != expected_disc_count as usize {
        return Err(OperationError::InvalidData(format!(
            "found {} of {expected_disc_count} existing playable discs",
            paths.len()
        )));
    }
    Ok(paths)
}

/// When this profile's archive projection was last committed, or `None` if it
/// has never been reconciled. Lets startup paint the committed projection
/// immediately instead of rescanning an archive that only changes through the
/// tool anyway.
pub fn archive_profile_indexed_at(
    conn: &Connection,
    profile_id: &str,
) -> Result<Option<String>, OperationError> {
    let indexed_at = conn
        .query_row(
            "SELECT indexed_at FROM archive_profiles WHERE id=?1",
            [profile_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    Ok(indexed_at)
}

pub fn archive_profile_source_generation(
    conn: &Connection,
    profile_id: &str,
) -> Result<Option<u64>, OperationError> {
    let generation = conn
        .query_row(
            "SELECT source_generation FROM archive_profiles WHERE id=?1",
            [profile_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(generation)
}

/// Whether a profile projection represents both the current archive tree and
/// the current catalog identity generation.
///
/// Archive bytes and catalog rows are independent inputs. Checking only the
/// archive generation leaves hash-resolvable carriers unbound after a DAT
/// import until some unrelated archive mutation happens to trigger a rescan.
pub fn archive_profile_projection_is_current(
    conn: &Connection,
    profile_id: &str,
    source_generation: u64,
) -> Result<bool, OperationError> {
    let projected = conn
        .query_row(
            "SELECT source_generation,catalog_generation FROM archive_profiles WHERE id=?1",
            [profile_id],
            |row| Ok((row.get::<_, u64>(0)?, row.get::<_, u64>(1)?)),
        )
        .optional()?;
    let catalog_generation = catalog_projection_generation(conn)?;
    Ok(projected.is_some_and(|(source, catalog)| {
        source == source_generation && catalog == catalog_generation.encoded()
    }))
}

/// Increment the rebuildable policy projection after the authoritative root
/// manifest changes. Explicit carrier overrides are left untouched; only
/// carriers inheriting this platform default are updated.
pub fn update_projected_platform_policy(
    conn: &mut Connection,
    profile_id: &str,
    platform_id: &str,
    policy: Option<&retro_junk_archive::DesiredPlayablePolicy>,
    root_manifest_sha256: &str,
) -> Result<usize, OperationError> {
    let tx = conn.transaction()?;
    let carrier_ids = {
        let mut statement = tx.prepare(
            "SELECT c.id,ar.platform_id FROM carriers c
             JOIN physical_copies pc ON pc.id=c.physical_copy_id
             JOIN archive_releases ar ON ar.id=pc.archive_release_id
             WHERE ar.profile_id=?1
               AND NOT EXISTS(SELECT 1 FROM playable_policies marker
                              WHERE marker.scope_type='carrier_override' AND marker.scope_id=c.id)",
        )?;
        statement
            .query_map([profile_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter_map(|(carrier_id, release_platform)| {
                same_platform(&release_platform, platform_id).then_some(carrier_id)
            })
            .collect::<Vec<_>>()
    };
    let mut removed = 0;
    let mut inserted = 0;
    let options_json = policy
        .map(|policy| serde_json::to_string(&policy.options))
        .transpose()
        .map_err(|error| OperationError::InvalidData(error.to_string()))?;
    for carrier_id in &carrier_ids {
        removed += tx.execute(
            "DELETE FROM playable_policies WHERE scope_type='carrier' AND scope_id=?1",
            [carrier_id],
        )?;
        if let Some(policy) = policy {
            inserted += tx.execute(
                "INSERT INTO playable_policies(scope_type,scope_id,format,retain_intermediate,allow_unverified,options_json)
                 VALUES('carrier',?1,?2,?3,?4,?5)",
                params![
                    carrier_id,
                    format_key(&policy.format),
                    policy.retain_canonical_intermediate,
                    policy.allow_unverified,
                    options_json.as_deref().unwrap_or("{}"),
                ],
            )?;
        }
    }
    let profiles = tx.execute(
        "UPDATE archive_profiles SET manifest_sha256=?2,indexed_at=datetime('now') WHERE id=?1",
        params![profile_id, root_manifest_sha256],
    )?;
    if profiles != 1 {
        return Err(OperationError::InvalidData(format!(
            "archive profile {profile_id} is not projected"
        )));
    }
    tx.commit()?;
    Ok(removed.max(inserted))
}

fn verification_kind_key(kind: retro_junk_archive::VerificationKind) -> &'static str {
    match kind {
        retro_junk_archive::VerificationKind::Integrity => "integrity",
        retro_junk_archive::VerificationKind::Reproduction => "reproduction",
        retro_junk_archive::VerificationKind::Catalog => "catalog",
        retro_junk_archive::VerificationKind::RoundTrip => "round_trip",
    }
}

fn verification_outcome_key(outcome: retro_junk_archive::VerificationOutcome) -> &'static str {
    match outcome {
        retro_junk_archive::VerificationOutcome::Verified => "verified",
        retro_junk_archive::VerificationOutcome::Unmatched => "unmatched",
        retro_junk_archive::VerificationOutcome::Ambiguous => "ambiguous",
        retro_junk_archive::VerificationOutcome::Failed => "failed",
    }
}

#[allow(clippy::too_many_lines)]
pub fn list_archive_release_summaries(
    conn: &Connection,
    profile_id: &str,
) -> Result<Vec<ArchiveReleaseSummary>, OperationError> {
    let mut statement = conn.prepare(
        "WITH copy_rollup AS (
             SELECT archive_release_id,COUNT(*) AS physical_copy_count
             FROM physical_copies GROUP BY archive_release_id
         ),
         carrier_scope AS (
             SELECT pc.archive_release_id,c.id AS carrier_id
             FROM physical_copies pc
             JOIN carriers c ON c.physical_copy_id=pc.id
         ),
         carrier_rollup AS (
             SELECT archive_release_id,COUNT(*) AS carrier_count
             FROM carrier_scope GROUP BY archive_release_id
         ),
         dump_rollup AS (
             SELECT cs.archive_release_id,COUNT(de.id) AS dump_count
             FROM carrier_scope cs
             JOIN dump_events de ON de.carrier_id=cs.carrier_id
             GROUP BY cs.archive_release_id
         ),
         representation_rollup AS (
             SELECT cs.archive_release_id,
                    COUNT(CASE WHEN rep.role='preservation_master' THEN 1 END)
                      AS preservation_count,
                    COUNT(CASE WHEN rep.role='preservation_master'
                                AND rep.presence_state='present' THEN 1 END)
                      AS preservation_present_count,
                    COUNT(CASE WHEN rep.role='playable' THEN 1 END)
                      AS playable_count,
                    COUNT(CASE WHEN rep.role='playable'
                                AND rep.presence_state='present' THEN 1 END)
                      AS playable_present_count,
                    COUNT(CASE WHEN rep.role='playable'
                                AND rep.presence_state='missing' THEN 1 END)
                      AS playable_missing_count
             FROM carrier_scope cs
             JOIN representations rep ON rep.carrier_id=cs.carrier_id
             GROUP BY cs.archive_release_id
         ),
         policy_rollup AS (
             SELECT cs.archive_release_id,
                    COUNT(DISTINCT pp.scope_id) AS desired_playable_count,
                    COUNT(DISTINCT CASE
                         WHEN rep.role='playable'
                          AND rep.presence_state='present'
                          AND rep.format=pp.format
                         THEN cs.carrier_id END) AS satisfied_playable_count
             FROM carrier_scope cs
             JOIN playable_policies pp
               ON pp.scope_type='carrier' AND pp.scope_id=cs.carrier_id
             LEFT JOIN representations rep ON rep.carrier_id=cs.carrier_id
             GROUP BY cs.archive_release_id
         ),
         verification_rollup AS (
             SELECT cs.archive_release_id,
                    COUNT(DISTINCT CASE
                         WHEN ve.kind='integrity' AND ve.outcome='verified'
                          AND ve.input_manifest_sha256=rep.input_manifest_sha256
                         THEN rep.id END) AS integrity_verified_count,
                    COUNT(DISTINCT CASE
                         WHEN ve.kind='reproduction' AND ve.outcome='verified'
                          AND ve.input_manifest_sha256=rep.input_manifest_sha256
                         THEN rep.id END) AS reproduction_verified_count,
                    COUNT(DISTINCT CASE
                         WHEN (ve.kind='catalog' AND ve.outcome='verified'
                               AND ve.input_manifest_sha256=rep.input_manifest_sha256)
                           OR rep.catalog_verified=1
                         THEN rep.id END) AS catalog_verified_count,
                    COUNT(DISTINCT CASE
                         WHEN (ve.kind='round_trip' AND ve.outcome='verified'
                               AND ve.input_manifest_sha256=rep.input_manifest_sha256)
                           OR rep.round_trip_verified=1
                         THEN rep.id END) AS round_trip_verified_count
             FROM carrier_scope cs
             JOIN representations rep ON rep.carrier_id=cs.carrier_id
             LEFT JOIN verification_events ve ON ve.representation_id=rep.id
             GROUP BY cs.archive_release_id
         )
         SELECT ar.id,ar.catalog_release_id,ar.platform_id,ar.title,ar.region,ar.revision,
                COALESCE(cr.physical_copy_count,0),
                COALESCE(car.carrier_count,0),
                COALESCE(dr.dump_count,0),
                COALESCE(rr.preservation_count,0),
                COALESCE(rr.preservation_present_count,0),
                COALESCE(rr.playable_count,0),
                COALESCE(rr.playable_present_count,0),
                COALESCE(rr.playable_missing_count,0),
                COALESCE(pr.desired_playable_count,0),
                COALESCE(pr.satisfied_playable_count,0),
                COALESCE(vr.integrity_verified_count,0),
                COALESCE(vr.reproduction_verified_count,0),
                COALESCE(vr.catalog_verified_count,0),
                COALESCE(vr.round_trip_verified_count,0)
         FROM archive_releases ar
         LEFT JOIN copy_rollup cr ON cr.archive_release_id=ar.id
         LEFT JOIN carrier_rollup car ON car.archive_release_id=ar.id
         LEFT JOIN dump_rollup dr ON dr.archive_release_id=ar.id
         LEFT JOIN representation_rollup rr ON rr.archive_release_id=ar.id
         LEFT JOIN policy_rollup pr ON pr.archive_release_id=ar.id
         LEFT JOIN verification_rollup vr ON vr.archive_release_id=ar.id
         WHERE ar.profile_id=?1
         ORDER BY ar.platform_id,ar.title COLLATE NOCASE,ar.id",
    )?;
    let rows = statement.query_map([profile_id], |row| {
        Ok(ArchiveReleaseSummary {
            archive_release_id: row.get(0)?,
            catalog_release_id: row.get(1)?,
            platform_id: row.get(2)?,
            title: row.get(3)?,
            region: row.get(4)?,
            revision: row.get(5)?,
            physical_copy_count: row.get(6)?,
            carrier_count: row.get(7)?,
            dump_count: row.get(8)?,
            preservation_count: row.get(9)?,
            preservation_present_count: row.get(10)?,
            playable_count: row.get(11)?,
            playable_present_count: row.get(12)?,
            playable_missing_count: row.get(13)?,
            desired_playable_count: row.get(14)?,
            satisfied_playable_count: row.get(15)?,
            integrity_verified_count: row.get(16)?,
            reproduction_verified_count: row.get(17)?,
            catalog_verified_count: row.get(18)?,
            round_trip_verified_count: row.get(19)?,
        })
    })?;
    let summaries = rows.collect::<Result<Vec<_>, _>>()?;
    Ok(summaries)
}

pub fn load_archive_collection_details(
    conn: &Connection,
    archive_release_id: &str,
) -> Result<Option<ArchiveCollectionDetails>, OperationError> {
    conn.query_row(
        "SELECT ar.id,ar.platform_id,ar.title,ar.region,ar.revision,ar.variant,
                ar.catalog_release_id,ar.binding_state,COALESCE(m.dat_source,''),
                pc.id,pc.manifest_path,pc.label,pc.condition,pc.notes,pc.date_acquired,pc.provenance,
                c.manifest_path,c.kind,c.serial,c.binding_state,
                pp.format,COALESCE(pp.retain_intermediate,0),COALESCE(pp.allow_unverified,0)
         FROM archive_releases ar
         JOIN physical_copies pc ON pc.archive_release_id=ar.id
         JOIN carriers c ON c.physical_copy_id=pc.id
         LEFT JOIN media m ON m.id=c.catalog_media_id
         LEFT JOIN playable_policies pp ON pp.scope_type='carrier' AND pp.scope_id=c.id
         WHERE ar.id=?1
         ORDER BY pc.copy_number,c.sequence_number,c.id
         LIMIT 1",
        [archive_release_id],
        |row| {
            Ok(ArchiveCollectionDetails {
                archive_release_id: row.get(0)?,
                platform_id: row.get(1)?,
                title: row.get(2)?,
                region: row.get(3)?,
                revision: row.get(4)?,
                variant: row.get(5)?,
                catalog_release_id: row.get(6)?,
                release_binding_state: row.get(7)?,
                catalog_source: row.get(8)?,
                physical_copy_id: row.get(9)?,
                physical_copy_manifest_path: row.get(10)?,
                label: row.get(11)?,
                condition: row.get(12)?,
                notes: row.get(13)?,
                date_acquired: row.get(14)?,
                provenance: row.get(15)?,
                carrier_manifest_path: row.get(16)?,
                carrier_kind: row.get(17)?,
                carrier_serial: row.get(18)?,
                carrier_binding_state: row.get(19)?,
                desired_format: row.get(20)?,
                retain_intermediate: row.get(21)?,
                allow_unverified: row.get(22)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

/// Which archive release each catalog release is held by, in one profile.
///
/// The manifests on disk name no catalog row, so this is where that mapping
/// lives: the projection derived it from each release's carriers, by content.
/// Callers that have a catalog release id and want the archived thing it stands
/// for — adopting already-downloaded artwork, say — ask here rather than
/// reading an id out of a manifest that no longer carries one.
pub fn archive_releases_by_catalog_release(
    conn: &Connection,
    profile_id: &str,
) -> Result<HashMap<String, String>, OperationError> {
    let mut statement = conn.prepare(
        "SELECT catalog_release_id,id FROM archive_releases
         WHERE profile_id=?1 AND catalog_release_id IS NOT NULL",
    )?;
    let rows = statement.query_map([profile_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    rows.collect::<Result<HashMap<_, _>, _>>()
        .map_err(Into::into)
}

/// The candidate id, but only if `table` actually holds a row with it —
/// otherwise nothing, so callers never store a reference that the foreign keys
/// will reject. Takes a plain connection so a transaction can pass itself.
fn existing_id(
    conn: &Connection,
    table: &str,
    candidate: &str,
) -> Result<Option<String>, OperationError> {
    if candidate.is_empty() {
        return Ok(None);
    }
    let query = format!("SELECT id FROM {table} WHERE id=?1");
    conn.query_row(&query, [candidate], |row| row.get(0))
        .optional()
        .map_err(Into::into)
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn format_key(format: &RepresentationFormat) -> String {
    match format {
        RepresentationFormat::RedumperRaw => "redumper_raw".to_owned(),
        RepresentationFormat::Rom => "rom".to_owned(),
        RepresentationFormat::CueBin => "cue_bin".to_owned(),
        RepresentationFormat::Iso => "iso".to_owned(),
        RepresentationFormat::Chd => "chd".to_owned(),
        RepresentationFormat::Rvz => "rvz".to_owned(),
        RepresentationFormat::Other(value) => format!("other:{value}"),
    }
}

fn role_key(role: &RepresentationRole) -> &'static str {
    match role {
        RepresentationRole::PreservationMaster => "preservation_master",
        RepresentationRole::CanonicalIntermediate => "canonical_intermediate",
        RepresentationRole::Playable => "playable",
    }
}

fn carrier_kind_key(kind: &retro_junk_archive::CarrierKind) -> String {
    match kind {
        retro_junk_archive::CarrierKind::OpticalDisc => "optical_disc".to_owned(),
        retro_junk_archive::CarrierKind::Cartridge => "cartridge".to_owned(),
        retro_junk_archive::CarrierKind::Card => "card".to_owned(),
        retro_junk_archive::CarrierKind::Tape => "tape".to_owned(),
        retro_junk_archive::CarrierKind::FloppyDisk => "floppy_disk".to_owned(),
        retro_junk_archive::CarrierKind::Unknown => "unknown".to_owned(),
        retro_junk_archive::CarrierKind::Other(value) => format!("other:{value}"),
    }
}

const fn release_file_category_key(
    category: retro_junk_archive::ReleaseFileCategory,
) -> &'static str {
    match category {
        retro_junk_archive::ReleaseFileCategory::Artwork => "artwork",
        retro_junk_archive::ReleaseFileCategory::Video => "video",
        retro_junk_archive::ReleaseFileCategory::Document => "document",
        retro_junk_archive::ReleaseFileCategory::Metadata => "metadata",
    }
}

const fn physical_copy_file_category_key(
    category: retro_junk_archive::PhysicalCopyFileCategory,
) -> &'static str {
    match category {
        retro_junk_archive::PhysicalCopyFileCategory::Photo => "photo",
        retro_junk_archive::PhysicalCopyFileCategory::Provenance => "provenance",
        retro_junk_archive::PhysicalCopyFileCategory::Document => "document",
    }
}

/// What one pass of applying the collection's marks did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MarkReport {
    /// Marks that produced (or refreshed) a catalog entry.
    pub applied: usize,
    /// Marks whose parent work this catalog does not have yet. Kept, not lost:
    /// importing the right DAT later makes them resolve.
    pub deferred: usize,
    /// Library rows bound to what a mark established.
    pub bound: usize,
}

/// Rebuild the catalog rows the collection's marks describe, and point the
/// library rows carrying that content at them.
///
/// Marks are the user's own decisions — a homebrew title they own, a mod they
/// applied — which nothing outside the collection can derive. They live beside
/// the archive rather than in this database precisely because this database is
/// device-local and rebuildable, so applying them is a normal part of building
/// it rather than a migration.
///
/// Idempotent: every id involved is derived from the mark's contents, so a
/// second pass rewrites the same rows.
pub fn apply_collection_marks(
    conn: &Connection,
    collection_root: &std::path::Path,
) -> Result<MarkReport, OperationError> {
    let marks = retro_junk_archive::load_marks(collection_root)
        .map_err(|error| OperationError::InvalidField(error.to_string()))?;
    let mut report = MarkReport::default();
    for mark in &marks {
        // A region correction names no catalog entry to create; it is applied
        // straight to the rows holding those bytes.
        if mark.kind == retro_junk_archive::MarkKind::RegionOverride {
            report.applied += 1;
            conn.execute(
                "UPDATE library_entries SET region_override=?1,revision=revision+1
                 WHERE region_override<>?1 AND data_size=?2
                   AND ((sha1<>'' AND sha1=?3) OR (crc32<>'' AND crc32=?4))",
                params![
                    mark.region,
                    i64::try_from(mark.content.size).unwrap_or(0),
                    mark.content.sha1,
                    mark.content.crc32,
                ],
            )?;
            continue;
        }
        let Some(applied) = crate::operations::apply_collection_mark(conn, mark)? else {
            report.deferred += 1;
            continue;
        };
        report.applied += 1;
        let digests = retro_junk_archive::FileDigests {
            size: mark.content.size,
            crc32: mark.content.crc32.clone(),
            md5: mark.content.md5.clone(),
            sha1: mark.content.sha1.clone(),
            sha256: String::new(),
        };
        // The library row keeps the user's tag, which is what stops the file
        // reading as an unidentified stranger on a machine that never saw the
        // decision being made.
        report.bound += bind_library_entries_by_hash(
            conn,
            &mark.platform_id,
            &digests,
            &LibraryEntryBinding {
                catalog_media_id: &applied.media_id,
                match_method: "collection_mark",
                ..Default::default()
            },
        )?;
        conn.execute(
            "UPDATE library_entries SET tag=?1,revision=revision+1
             WHERE tag<>?1 AND data_size=?2
               AND ((sha1<>'' AND sha1=?3) OR (crc32<>'' AND crc32=?4))",
            params![
                applied.tag,
                i64::try_from(mark.content.size).unwrap_or(0),
                mark.content.sha1,
                mark.content.crc32,
            ],
        )?;
    }
    if report.applied > 0 || report.deferred > 0 {
        log::info!(
            "Collection marks: {} applied, {} awaiting their parent DAT, {} library row(s) bound",
            report.applied,
            report.deferred,
            report.bound
        );
    }
    Ok(report)
}

#[cfg(test)]
mod catalog_generation_tests {
    use super::*;

    #[test]
    fn generation_format_is_owned_and_human_readable() {
        assert_eq!(
            CatalogProjectionGeneration {
                import: 100,
                resolver: 3,
            }
            .to_string(),
            "i100r3"
        );
    }

    #[test]
    fn a_material_catalog_import_invalidates_an_unchanged_archive_projection() {
        let conn = crate::open_memory().unwrap();
        let current = catalog_projection_generation(&conn).unwrap().encoded();
        conn.execute(
            "INSERT INTO archive_profiles(id,display_name,manifest_path,manifest_sha256,archive_root,source_generation,catalog_generation)
             VALUES('profile','Profile','archive.toml','sha','/archive',7,?1)",
            [current],
        )
        .unwrap();
        assert!(archive_profile_projection_is_current(&conn, "profile", 7).unwrap());

        conn.execute(
            "INSERT INTO import_log(source_type,source_name,imported_at,records_updated)
             VALUES('redump','Sega - Saturn','2026-08-09',1)",
            [],
        )
        .unwrap();
        assert!(!archive_profile_projection_is_current(&conn, "profile", 7).unwrap());
    }
}
