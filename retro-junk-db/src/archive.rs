//! Rebuildable `SQLite` projection of portable preservation manifests.

use std::path::Path;

use retro_junk_archive::{
    ArchiveIndexSnapshot, RepresentationFormat, RepresentationRole, TrackDigest, playable_presence,
    preservation_presence,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

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
    pub desired_playable_count: u64,
    pub satisfied_playable_count: u64,
    pub integrity_verified_count: u64,
    pub reproduction_verified_count: u64,
    pub catalog_verified_count: u64,
    pub round_trip_verified_count: u64,
    /// Logical catalog discs expected for this release. Zero means the
    /// release is not sufficiently catalog-bound to claim completeness.
    pub expected_disc_count: u64,
    /// Expected discs which have a present preservation master and current,
    /// complete catalog verification in the best matching physical copy.
    pub verified_disc_count: u64,
    pub archive_complete: bool,
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

/// Follow a playable moved from an internal archive platform directory to the
/// active frontend's equivalent system directory. Portable build evidence is
/// historical and remains unchanged; only this rebuildable projection points
/// at the file's current location.
fn projected_playable_path(
    playable_root: &Path,
    platform_id: &str,
    region: &str,
    input_manifest_sha256: &str,
    evidence: &retro_junk_archive::BuildEvidence,
) -> (String, retro_junk_archive::RepresentationPresence) {
    let presence = playable_presence(playable_root, input_manifest_sha256, evidence);
    if presence != retro_junk_archive::RepresentationPresence::Missing {
        return (evidence.relative_output_path.clone(), presence);
    }
    let historical = Path::new(&evidence.relative_output_path);
    let mut components = historical.components();
    let Some(first) = components.next() else {
        return (evidence.relative_output_path.clone(), presence);
    };
    let Some(first) = first.as_os_str().to_str() else {
        return (evidence.relative_output_path.clone(), presence);
    };
    let frontend_directory = retro_junk_frontend::esde::system_directory(platform_id, Some(region));
    if first.eq_ignore_ascii_case(&frontend_directory) {
        return (evidence.relative_output_path.clone(), presence);
    }
    let mut projected_evidence = evidence.clone();
    projected_evidence.relative_output_path = Path::new(&frontend_directory)
        .join(components.as_path())
        .to_string_lossy()
        .replace('\\', "/");
    let projected_presence =
        playable_presence(playable_root, input_manifest_sha256, &projected_evidence);
    if projected_presence == retro_junk_archive::RepresentationPresence::Present {
        log::info!(
            "Resolved moved playable {} -> {}",
            evidence.relative_output_path,
            projected_evidence.relative_output_path
        );
        (projected_evidence.relative_output_path, projected_presence)
    } else {
        (evidence.relative_output_path.clone(), presence)
    }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteCatalogMediaMatch {
    pub media_id: String,
    pub release_id: String,
    pub work_id: String,
    pub game: String,
    pub source: String,
    pub source_version: String,
    pub platform_id: String,
    pub region: String,
    pub revision: String,
    pub variant: String,
    pub serial: String,
    pub sequence_number: u32,
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
                          WHERE il.source_type=m.dat_source
                          ORDER BY il.imported_at DESC,il.id DESC LIMIT 1),''),
                r.platform_id,r.region,r.revision,r.variant,m.media_serial,m.disc_number
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
    match_catalog_file_inner(conn, platform_id, actual)
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
                          WHERE il.source_type=m.dat_source
                          ORDER BY il.imported_at DESC,il.id DESC LIMIT 1),''),
                r.platform_id,r.region,r.revision,r.variant,m.media_serial,m.disc_number
         FROM media m JOIN releases r ON r.id=m.release_id
         WHERE (?1='' OR r.platform_id=?1) AND m.file_size=?2
           AND (m.sha1<>'' OR m.md5<>'' OR m.crc32<>'')
           AND (m.sha1='' OR m.sha1=lower(?3))
           AND (m.md5='' OR m.md5=lower(?4))
           AND (m.crc32='' OR m.crc32=lower(?5))
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
            })
        },
    )?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Connect the legacy playable-library projection to catalog/archive identity
/// using already-computed strong hashes. This does not make the library row
/// authoritative and is safe to rebuild.
pub fn bind_library_entries_by_hash(
    conn: &Connection,
    platform_id: &str,
    actual: &retro_junk_archive::FileDigests,
    catalog_media_id: &str,
    representation_id: Option<&str>,
    match_method: &str,
) -> Result<usize, OperationError> {
    if catalog_media_id.is_empty() {
        return Ok(0);
    }
    conn.execute(
        "INSERT OR REPLACE INTO library_entry_media_bindings(library_entry_id,catalog_media_id,representation_id,match_method)
         SELECT e.id,?4,?5,?6
         FROM library_entries e JOIN library_consoles c ON c.id=e.console_id
         WHERE (lower(c.folder_name)=lower(?1) OR lower(c.platform)=lower(?1))
           AND e.data_size=?2
           AND ((e.sha1<>'' AND e.sha1=lower(?3))
                OR (e.md5<>'' AND e.md5=lower(?7))
                OR (e.crc32<>'' AND e.crc32=lower(?8)))",
        params![
            platform_id,
            i64::try_from(actual.size).unwrap_or(i64::MAX),
            actual.sha1,
            catalog_media_id,
            representation_id,
            match_method,
            actual.md5,
            actual.crc32,
        ],
    )
    .map_err(Into::into)
}

/// Match only when the complete ordered track set agrees. A single matching
/// track is deliberately insufficient evidence for a multi-track disc.
pub fn match_complete_catalog_media(
    conn: &Connection,
    platform_id: &str,
    actual: &[TrackDigest],
) -> Result<Vec<CompleteCatalogMediaMatch>, OperationError> {
    match_complete_catalog_media_inner(conn, platform_id, actual)
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
                          WHERE il.source_type=m.dat_source
                          ORDER BY il.imported_at DESC,il.id DESC LIMIT 1),''),
                r.platform_id,r.region,r.revision,r.variant,m.media_serial,m.disc_number
         FROM media m JOIN releases r ON r.id=m.release_id
         WHERE (?1='' OR r.platform_id=?1) AND m.file_size=?2
           AND NOT EXISTS(SELECT 1 FROM media_tracks mt WHERE mt.media_id=m.id)
           AND (m.sha1<>'' OR m.md5<>'' OR m.crc32<>'')
           AND (m.sha1='' OR m.sha1=lower(?3))
           AND (m.md5='' OR m.md5=lower(?4))
           AND (m.crc32='' OR m.crc32=lower(?5))
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
                          WHERE il.source_type=m.dat_source
                          ORDER BY il.imported_at DESC,il.id DESC LIMIT 1),''),
                r.platform_id,r.region,r.revision,r.variant,m.media_serial,m.disc_number
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
                        || expected.crc32.eq_ignore_ascii_case(&actual.crc32))
                    && (expected.md5.is_empty() || expected.md5.eq_ignore_ascii_case(&actual.md5))
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
#[allow(clippy::too_many_lines)]
pub fn reconcile_archive_snapshot(
    conn: &mut Connection,
    snapshot: &ArchiveIndexSnapshot,
    playable_root: &Path,
    workspace_root: &Path,
) -> Result<(), OperationError> {
    let tx = conn.transaction()?;
    let profile_id = snapshot.manifest.profile_id.to_string();
    // Policies intentionally have no polymorphic foreign key. Remove carrier
    // policies owned by this projection before cascading its archive rows, or
    // a rebuild would leave stale rows and collide on reinsertion.
    tx.execute(
        "DELETE FROM playable_policies WHERE scope_type IN ('carrier','carrier_override') AND (
             scope_id IN (
                 SELECT c.id FROM carriers c
                 JOIN physical_copies pc ON pc.id=c.physical_copy_id
                 JOIN archive_releases ar ON ar.id=pc.archive_release_id
                 WHERE ar.profile_id=?1
             ) OR NOT EXISTS(SELECT 1 FROM carriers c WHERE c.id=scope_id)
         )",
        [&profile_id],
    )?;
    tx.execute("DELETE FROM archive_profiles WHERE id=?1", [&profile_id])?;
    tx.execute(
        "INSERT INTO archive_profiles(id,display_name,manifest_path,manifest_sha256,archive_root,playable_root,workspace_root,indexed_at)
         VALUES(?1,?2,'retro-junk-archive.toml',?3,?4,?5,?6,datetime('now'))",
        params![
            profile_id,
            snapshot.manifest.display_name,
            snapshot.manifest_sha256,
            snapshot.root.to_string_lossy(),
            playable_root.to_string_lossy(),
            workspace_root.to_string_lossy(),
        ],
    )?;

    for release in &snapshot.releases {
        let catalog_release = existing_id(
            &tx,
            "releases",
            &release.manifest.catalog_binding.catalog_release_id,
        )?;
        let catalog_work = if release.manifest.catalog_binding.catalog_work_id.is_empty() {
            catalog_release
                .as_deref()
                .map(|release_id| {
                    tx.query_row(
                        "SELECT work_id FROM releases WHERE id=?1",
                        [release_id],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
                })
                .transpose()?
                .flatten()
        } else {
            existing_id(
                &tx,
                "works",
                &release.manifest.catalog_binding.catalog_work_id,
            )?
        };
        let binding_state = if catalog_release.is_some() {
            "resolved"
        } else if catalog_work.is_some() {
            "carrier_resolved"
        } else {
            "unresolved"
        };
        tx.execute(
            "INSERT INTO archive_releases(id,profile_id,catalog_work_id,catalog_release_id,platform_id,title,region,revision,variant,manifest_path,manifest_sha256,binding_state)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                release.manifest.archive_release_id.to_string(),
                profile_id,
                catalog_work,
                catalog_release,
                release.manifest.platform_id,
                release.manifest.title,
                release.manifest.region,
                release.manifest.revision,
                release.manifest.variant,
                relative(&snapshot.root, &release.directory.join("release.toml")),
                release.manifest_sha256,
                binding_state,
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
                let catalog_media = existing_id(
                    &tx,
                    "media",
                    &carrier.manifest.catalog_binding.catalog_media_id,
                )?;
                let media_binding = if catalog_media.is_some() {
                    "resolved"
                } else {
                    "unresolved"
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
                    let current_verification = |kind| {
                        dump.verifications.iter().any(|verification| {
                            verification.evidence.kind == kind
                                && verification.evidence.outcome
                                    == retro_junk_archive::VerificationOutcome::Verified
                                && verification.evidence.input_manifest_sha256
                                    == dump.manifest_sha256
                        })
                    };
                    let integrity_state =
                        if current_verification(retro_junk_archive::VerificationKind::Integrity) {
                            "verified"
                        } else {
                            "unknown"
                        };
                    let catalog_state =
                        if current_verification(retro_junk_archive::VerificationKind::Catalog) {
                            "verified"
                        } else {
                            "not_attempted"
                        };
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
                        "INSERT INTO representations(id,carrier_id,dump_id,role,format,location_role,relative_path,presence_state,input_manifest_sha256,catalog_verified,round_trip_verified)
                         VALUES(?1,?2,?3,?4,?5,'archive',?6,?7,?8,?9,0)",
                        params![
                            dump.manifest.representation_id.to_string(),
                            carrier.manifest.carrier_id.to_string(),
                            dump.manifest.dump_id.to_string(),
                            role_key(&RepresentationRole::PreservationMaster),
                            format,
                            relative_dump,
                            master_presence.as_str(),
                            dump.manifest_sha256,
                            current_verification(retro_junk_archive::VerificationKind::Catalog),
                        ],
                    )?;
                    for file in &dump.manifest.files {
                        tx.execute(
                            "INSERT INTO representation_files(representation_id,relative_path,file_size,sha256)
                             VALUES(?1,?2,?3,?4)",
                            params![
                                dump.manifest.representation_id.to_string(),
                                file.path,
                                file.size,
                                file.sha256,
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
                    for build in &dump.builds {
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
                                    "INSERT INTO representation_files(representation_id,relative_path,file_size,sha256)
                                     VALUES(?1,?2,?3,?4)",
                                    params![
                                        intermediate.representation_id.to_string(),
                                        file.path,
                                        file.size,
                                        file.sha256,
                                    ],
                                )?;
                            }
                        }
                        let (playable_relative_path, presence) = projected_playable_path(
                            playable_root,
                            &release.manifest.platform_id,
                            &release.manifest.region,
                            &dump.manifest_sha256,
                            &build.evidence,
                        );
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

    // Rebuild the bridge between the playable-library projection and archival
    // carriers from their shared, normalized catalog hashes. In particular,
    // cartridge library hashes omit format headers (for example iNES), just as
    // the catalog does; comparing archive-file sizes here would miss them.
    tx.execute(
        "DELETE FROM library_entry_media_bindings
         WHERE match_method IN (
             'archive_projection',
             'archive_output_path',
             'archive_release_projection'
         )",
        [],
    )?;
    // A playable build is already provenance evidence: its manifest records
    // the exact output path below the profile's playable root. Bind a scanned
    // library row at that path directly instead of asking CHD (or another
    // derivative container) to reproduce the preservation dump's raw hashes.
    tx.execute(
        "INSERT OR REPLACE INTO library_entry_media_bindings(
             library_entry_id,catalog_media_id,representation_id,match_method)
         SELECT DISTINCT le.id,c.catalog_media_id,rep.id,'archive_output_path'
         FROM representations rep
         JOIN carriers c ON c.id=rep.carrier_id
         JOIN physical_copies pc ON pc.id=c.physical_copy_id
         JOIN archive_releases ar ON ar.id=pc.archive_release_id
         JOIN archive_profiles ap ON ap.id=ar.profile_id
         JOIN library_roots lr ON lr.root_path=ap.playable_root
         JOIN library_consoles lc ON lc.root_id=lr.id
         JOIN library_entries le ON le.console_id=lc.id
         WHERE rep.role='playable'
           AND rep.location_role='playable'
           AND rep.presence_state='present'
           AND c.catalog_media_id IS NOT NULL
           AND replace(rep.relative_path,'\\','/')=
               replace(lc.folder_name || '/' || substr(le.entry_key,6),'\\','/')",
        [],
    )?;
    tx.execute(
        "INSERT OR REPLACE INTO library_entry_media_bindings(library_entry_id,catalog_media_id,representation_id,match_method)
         SELECT DISTINCT le.id,c.catalog_media_id,
                (SELECT rep.id FROM representations rep
                 WHERE rep.carrier_id=c.id AND rep.role='playable'
                   AND replace(rep.relative_path,'\\','/')=
                       replace(lc.folder_name || '/' || substr(le.entry_key,6),'\\','/')
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
                OR (le.crc32<>'' AND m.crc32<>'' AND le.crc32=m.crc32))",
        [],
    )?;
    // Disc containers and M3U sets often cannot expose Redump's raw per-track
    // hashes cheaply (or at all), but analysis can still identify their exact
    // DAT game names from serials. Bind the logical library entry to every
    // archived carrier in that catalog release. This makes a multi-disc set
    // one archived/playable game without pretending that one matched disc is
    // evidence that the other archive carriers are verified.
    tx.execute(
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
             library_entry_id,catalog_media_id,representation_id,match_method)
         SELECT DISTINCT er.library_entry_id,c.catalog_media_id,NULL,'archive_release_projection'
         FROM entry_releases er
         JOIN media m ON m.release_id=er.release_id
         JOIN carriers c ON c.catalog_media_id=m.id
         JOIN physical_copies pc ON pc.id=c.physical_copy_id
         JOIN archive_releases ar ON ar.id=pc.archive_release_id
         JOIN archive_profiles ap ON ap.id=ar.profile_id
         JOIN library_entries le ON le.id=er.library_entry_id
         JOIN library_consoles lc ON lc.id=le.console_id
         JOIN library_roots lr ON lr.id=lc.root_id
         WHERE ap.playable_root=lr.root_path",
        [],
    )?;
    tx.commit()?;
    Ok(())
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
                      AS playable_present_count
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
            desired_playable_count: row.get(13)?,
            satisfied_playable_count: row.get(14)?,
            integrity_verified_count: row.get(15)?,
            reproduction_verified_count: row.get(16)?,
            catalog_verified_count: row.get(17)?,
            round_trip_verified_count: row.get(18)?,
            expected_disc_count: 0,
            verified_disc_count: 0,
            archive_complete: false,
        })
    })?;
    let mut summaries = rows.collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    let completeness = archive_release_completeness(conn, profile_id, "")?;
    for summary in &mut summaries {
        if let Some((expected, verified)) = completeness
            .get(&summary.archive_release_id)
            .map(|(expected, verified)| (*expected, *verified))
        {
            summary.expected_disc_count = expected;
            summary.verified_disc_count = verified;
            summary.archive_complete = expected > 0 && verified == expected;
        }
    }
    Ok(summaries)
}

const ARCHIVE_RELEASE_COMPLETENESS_SQL: &str = "WITH candidate_media AS (
             SELECT ar.id AS archive_release_id,m.id AS media_id,m.disc_number
             FROM archive_releases ar
             JOIN archive_profiles ap ON ap.id=ar.profile_id
             JOIN media m ON m.release_id=ar.catalog_release_id
             WHERE (?1='' OR ar.profile_id=?1)
               AND (?2='' OR ap.playable_root=?2)
               AND ar.catalog_release_id IS NOT NULL
             UNION ALL
             SELECT ar.id,m.id,m.disc_number
             FROM archive_releases ar
             JOIN archive_profiles ap ON ap.id=ar.profile_id
             CROSS JOIN releases r INDEXED BY idx_releases_natural
             JOIN media m ON m.release_id=r.id
             WHERE (?1='' OR ar.profile_id=?1)
               AND (?2='' OR ap.playable_root=?2)
               AND ar.catalog_release_id IS NULL
               AND ar.catalog_work_id IS NOT NULL
               AND r.work_id=ar.catalog_work_id
               AND r.platform_id=ar.platform_id
               AND r.region=ar.region
         ),
         release_expected AS (
             SELECT archive_release_id AS id,
                    CASE WHEN MAX(disc_number)>0
                         THEN COUNT(DISTINCT CASE WHEN disc_number>0 THEN disc_number END)
                         ELSE 1 END AS expected_count,
                    MAX(disc_number)>0 AS numbered
             FROM candidate_media
             GROUP BY archive_release_id
         ),
         per_copy AS (
             SELECT re.id AS archive_release_id,pc.id AS physical_copy_id,
                    COUNT(DISTINCT CASE WHEN ve.id IS NOT NULL THEN
                         CASE WHEN re.numbered
                              THEN CASE WHEN m.disc_number>0 THEN m.disc_number END
                              ELSE 0 END
                    END) AS verified_count
             FROM release_expected re
             JOIN physical_copies pc ON pc.archive_release_id=re.id
             LEFT JOIN carriers c ON c.physical_copy_id=pc.id
             LEFT JOIN candidate_media cm
                ON cm.archive_release_id=re.id AND cm.media_id=c.catalog_media_id
             LEFT JOIN media m ON m.id=cm.media_id
             LEFT JOIN representations rep ON rep.carrier_id=c.id
                AND rep.role='preservation_master' AND rep.presence_state='present'
             LEFT JOIN verification_events ve ON ve.representation_id=rep.id
                AND ve.kind='catalog' AND ve.outcome='verified'
                AND ve.input_manifest_sha256=rep.input_manifest_sha256
                AND (ve.complete_track_set=1 OR NOT EXISTS(
                     SELECT 1 FROM media_tracks mt WHERE mt.media_id=m.id))
             GROUP BY re.id,pc.id
         )
         SELECT re.id,re.expected_count,COALESCE(MAX(pc.verified_count),0)
         FROM release_expected re
         LEFT JOIN per_copy pc ON pc.archive_release_id=re.id
         GROUP BY re.id,re.expected_count";

pub(crate) fn archive_release_completeness(
    conn: &Connection,
    profile_id: &str,
    playable_root: &str,
) -> Result<std::collections::HashMap<String, (u64, u64)>, OperationError> {
    let mut statement = conn.prepare(ARCHIVE_RELEASE_COMPLETENESS_SQL)?;
    statement
        .query_map(params![profile_id, playable_root], |row| {
            Ok((
                row.get::<_, String>(0)?,
                (row.get::<_, u64>(1)?, row.get::<_, u64>(2)?),
            ))
        })?
        .collect::<Result<_, _>>()
        .map_err(Into::into)
}

#[cfg(test)]
mod query_plan_tests {
    use super::*;

    #[test]
    fn work_level_completeness_never_scans_the_catalog_media_table() {
        let connection = crate::schema::open_memory().unwrap();
        let explain = format!("EXPLAIN QUERY PLAN {ARCHIVE_RELEASE_COMPLETENESS_SQL}");
        let mut statement = connection.prepare(&explain).unwrap();
        let details = statement
            .query_map(params!["profile", ""], |row| row.get::<_, String>(3))
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
            "completeness regressed to a full media scan: {details:#?}"
        );
    }
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

fn existing_id(
    tx: &Transaction<'_>,
    table: &str,
    candidate: &str,
) -> Result<Option<String>, OperationError> {
    if candidate.is_empty() {
        return Ok(None);
    }
    let query = format!("SELECT id FROM {table} WHERE id=?1");
    tx.query_row(&query, [candidate], |row| row.get(0))
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
