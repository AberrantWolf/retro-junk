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
        "SELECT DISTINCT m.id,m.release_id,r.title,m.dat_source,
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
            game: row.get(2)?,
            source: row.get(3)?,
            source_version: row.get(4)?,
            platform_id: row.get(5)?,
            region: row.get(6)?,
            revision: row.get(7)?,
            variant: row.get(8)?,
            serial: row.get(9)?,
            sequence_number: u32::try_from(row.get::<_, i64>(10)?).unwrap_or(0),
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
        "SELECT m.id,m.release_id,r.title,m.dat_source,
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
                game: row.get(2)?,
                source: row.get(3)?,
                source_version: row.get(4)?,
                platform_id: row.get(5)?,
                region: row.get(6)?,
                revision: row.get(7)?,
                variant: row.get(8)?,
                serial: row.get(9)?,
                sequence_number: u32::try_from(row.get::<_, i64>(10)?).unwrap_or(0),
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

fn match_complete_catalog_media_inner(
    conn: &Connection,
    platform_id: &str,
    actual: &[TrackDigest],
) -> Result<Vec<CompleteCatalogMediaMatch>, OperationError> {
    if actual.is_empty() || actual.iter().any(|track| track.sha1.is_empty()) {
        return Ok(Vec::new());
    }
    let mut candidates = conn.prepare(
        "SELECT DISTINCT m.id,m.release_id,r.title,m.dat_source,
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
                game: row.get(2)?,
                source: row.get(3)?,
                source_version: row.get(4)?,
                platform_id: row.get(5)?,
                region: row.get(6)?,
                revision: row.get(7)?,
                variant: row.get(8)?,
                serial: row.get(9)?,
                sequence_number: u32::try_from(row.get::<_, i64>(10)?).unwrap_or(0),
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
        let binding_state = if catalog_release.is_some() {
            "resolved"
        } else {
            "unresolved"
        };
        tx.execute(
            "INSERT INTO archive_releases(id,profile_id,catalog_release_id,platform_id,title,region,revision,variant,manifest_path,manifest_sha256,binding_state)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                release.manifest.archive_release_id.to_string(),
                profile_id,
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
                        .find(|default| default.platform_id == release.manifest.platform_id)
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
                        let presence = playable_presence(
                            playable_root,
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
                                build.evidence.relative_output_path,
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
                                build.evidence.relative_output_path,
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
        "DELETE FROM library_entry_media_bindings WHERE match_method='archive_projection'",
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
    let carrier_filter = "SELECT c.id FROM carriers c
         JOIN physical_copies pc ON pc.id=c.physical_copy_id
         JOIN archive_releases ar ON ar.id=pc.archive_release_id
         WHERE ar.profile_id=?1 AND lower(ar.platform_id)=lower(?2)
           AND NOT EXISTS(SELECT 1 FROM playable_policies marker
                          WHERE marker.scope_type='carrier_override' AND marker.scope_id=c.id)";
    let removed = tx.execute(
        &format!(
            "DELETE FROM playable_policies WHERE scope_type='carrier' AND scope_id IN ({carrier_filter})"
        ),
        params![profile_id, platform_id],
    )?;
    let inserted = if let Some(policy) = policy {
        tx.execute(
            &format!(
                "INSERT INTO playable_policies(scope_type,scope_id,format,retain_intermediate,allow_unverified,options_json)
                 SELECT 'carrier',inherited.id,?3,?4,?5,?6 FROM ({carrier_filter}) inherited"
            ),
            params![
                profile_id,
                platform_id,
                format_key(&policy.format),
                policy.retain_canonical_intermediate,
                policy.allow_unverified,
                serde_json::to_string(&policy.options)
                    .map_err(|error| OperationError::InvalidData(error.to_string()))?,
            ],
        )?
    } else {
        0
    };
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

pub fn list_archive_release_summaries(
    conn: &Connection,
    profile_id: &str,
) -> Result<Vec<ArchiveReleaseSummary>, OperationError> {
    let mut statement = conn.prepare(
        "SELECT ar.id,ar.catalog_release_id,ar.platform_id,ar.title,ar.region,ar.revision,
                COUNT(DISTINCT ci.id),COUNT(DISTINCT cm.id),COUNT(DISTINCT de.id),
                COUNT(DISTINCT CASE WHEN rep.role='preservation_master' THEN rep.id END),
                COUNT(DISTINCT CASE WHEN rep.role='preservation_master' AND rep.presence_state='present' THEN rep.id END),
                COUNT(DISTINCT CASE WHEN rep.role='playable' THEN rep.id END),
                COUNT(DISTINCT CASE WHEN rep.role='playable' AND rep.presence_state='present' THEN rep.id END),
                COUNT(DISTINCT pp.scope_id),
                COUNT(DISTINCT CASE WHEN pp.scope_id IS NOT NULL AND rep.role='playable' AND rep.presence_state='present' AND rep.format=pp.format THEN cm.id END),
                COUNT(DISTINCT CASE WHEN ve.kind='integrity' AND ve.outcome='verified' AND ve.input_manifest_sha256=rep.input_manifest_sha256 THEN rep.id END),
                COUNT(DISTINCT CASE WHEN ve.kind='reproduction' AND ve.outcome='verified' AND ve.input_manifest_sha256=rep.input_manifest_sha256 THEN rep.id END),
                COUNT(DISTINCT CASE WHEN (ve.kind='catalog' AND ve.outcome='verified' AND ve.input_manifest_sha256=rep.input_manifest_sha256) OR rep.catalog_verified=1 THEN rep.id END),
                COUNT(DISTINCT CASE WHEN (ve.kind='round_trip' AND ve.outcome='verified' AND ve.input_manifest_sha256=rep.input_manifest_sha256) OR rep.round_trip_verified=1 THEN rep.id END)
         FROM archive_releases ar
         LEFT JOIN physical_copies ci ON ci.archive_release_id=ar.id
         LEFT JOIN carriers cm ON cm.physical_copy_id=ci.id
         LEFT JOIN dump_events de ON de.carrier_id=cm.id
         LEFT JOIN representations rep ON rep.carrier_id=cm.id
         LEFT JOIN playable_policies pp ON pp.scope_type='carrier' AND pp.scope_id=cm.id
         LEFT JOIN verification_events ve ON ve.representation_id=rep.id
         WHERE ar.profile_id=?1
         GROUP BY ar.id
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
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
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
