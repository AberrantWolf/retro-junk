//! Archive maintenance commands: refresh the projection, verify integrity,
//! identify carriers against the catalog.

use std::path::Path;
use std::sync::atomic::AtomicBool;

use retro_junk_archive::CollectionProfile;

use super::OpCtx;

#[derive(Debug, Clone)]
pub struct AddReleaseFileRequest {
    pub release_id: retro_junk_archive::ArchiveReleaseId,
    pub source_file: std::path::PathBuf,
    pub category: retro_junk_archive::ReleaseFileCategory,
    pub asset_type: String,
    pub source: String,
    pub source_url: String,
    pub caption: String,
}

#[derive(Debug, Clone)]
pub struct AddPhysicalCopyFileRequest {
    pub physical_copy_id: retro_junk_archive::PhysicalCopyId,
    pub source_file: std::path::PathBuf,
    pub category: retro_junk_archive::PhysicalCopyFileCategory,
    pub asset_type: String,
    pub source: String,
    pub caption: String,
}

#[derive(Debug, Clone)]
pub struct UpdateCollectionDetailsRequest {
    pub physical_copy_manifest_path: std::path::PathBuf,
    pub carrier_manifest_path: std::path::PathBuf,
    pub label: String,
    pub condition: String,
    pub notes: String,
    pub date_acquired: String,
    pub provenance: String,
    pub playable_policy: Option<retro_junk_archive::DesiredPlayablePolicy>,
}

/// Persist collection manifest edits below the UI boundary, then rebuild the
/// projection. The structural release projector will eventually make the last
/// step release-scoped; keeping the write here already prevents synchronous
/// archive I/O and lock ownership from leaking into the view.
pub fn update_collection_details(
    profile: &CollectionProfile,
    db_path: &Path,
    request: &UpdateCollectionDetailsRequest,
    ctx: &OpCtx,
) -> Result<String, String> {
    {
        let _archive_lock = retro_junk_archive::ArchiveLock::acquire(&profile.archive_root)
            .map_err(|error| error.to_string())?;
        retro_junk_archive::advance_projection_generation(&profile.archive_root)
            .map_err(|error| error.to_string())?;
        let mut physical_copy: retro_junk_archive::PhysicalCopyManifest =
            retro_junk_archive::read_toml(&request.physical_copy_manifest_path)
                .map_err(|error| error.to_string())?;
        physical_copy.label.clone_from(&request.label);
        physical_copy.condition.clone_from(&request.condition);
        physical_copy.notes.clone_from(&request.notes);
        physical_copy
            .date_acquired
            .clone_from(&request.date_acquired);
        physical_copy.provenance.clone_from(&request.provenance);
        retro_junk_archive::write_toml_atomic(&request.physical_copy_manifest_path, &physical_copy)
            .map_err(|error| error.to_string())?;

        let mut carrier: retro_junk_archive::CarrierManifest =
            retro_junk_archive::read_toml(&request.carrier_manifest_path)
                .map_err(|error| error.to_string())?;
        carrier.playable_policy.clone_from(&request.playable_policy);
        retro_junk_archive::write_toml_atomic(&request.carrier_manifest_path, &carrier)
            .map_err(|error| error.to_string())?;
    }
    refresh_archive(profile, Some(db_path), false, ctx)
}

/// Archive one release-level supporting file and update only that release's
/// supporting-file projection.
pub fn add_release_file(
    profile: &CollectionProfile,
    db_path: &Path,
    request: &AddReleaseFileRequest,
    cancel: &AtomicBool,
) -> Result<bool, String> {
    let added = {
        let _archive_lock = retro_junk_archive::ArchiveLock::acquire(&profile.archive_root)
            .map_err(|error| error.to_string())?;
        retro_junk_archive::add_release_files(
            &profile.archive_root,
            &[retro_junk_archive::NewReleaseFile {
                release_id: request.release_id,
                source_file: &request.source_file,
                category: request.category,
                asset_type: &request.asset_type,
                source: &request.source,
                source_url: &request.source_url,
                caption: &request.caption,
            }],
            cancel,
        )
        .map_err(|error| error.to_string())?
        .into_iter()
        .next()
        .is_some_and(|result| result.added)
    };
    if added {
        reconcile_release_files(profile, db_path, &[request.release_id], cancel)?;
    }
    Ok(added)
}

/// Archive one physical-copy supporting file and update only its owning
/// release's supporting-file projection.
pub fn add_physical_copy_file(
    profile: &CollectionProfile,
    db_path: &Path,
    request: &AddPhysicalCopyFileRequest,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let connection = retro_junk_db::open_database(db_path).map_err(|error| error.to_string())?;
    let release_id: String = connection
        .query_row(
            "SELECT archive_release_id FROM physical_copies WHERE id=?1",
            [request.physical_copy_id.to_string()],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let release_id = release_id
        .parse::<retro_junk_archive::ArchiveReleaseId>()
        .map_err(|error| error.to_string())?;
    drop(connection);
    {
        let _archive_lock = retro_junk_archive::ArchiveLock::acquire(&profile.archive_root)
            .map_err(|error| error.to_string())?;
        retro_junk_archive::add_physical_copy_file(
            &profile.archive_root,
            retro_junk_archive::NewPhysicalCopyFile {
                physical_copy_id: request.physical_copy_id,
                source_file: &request.source_file,
                category: request.category,
                asset_type: &request.asset_type,
                source: &request.source,
                caption: &request.caption,
            },
            cancel,
        )
        .map_err(|error| error.to_string())?;
    }
    reconcile_release_files(profile, db_path, &[release_id], cancel)
}

/// Refresh the rebuildable archive projection, optionally appending a fresh
/// integrity verification first. The archive lock serializes this with every
/// authoritative manifest mutation; `SQLite` readers retain the prior complete
/// projection until the replacement transaction commits.
pub fn refresh_archive(
    profile: &CollectionProfile,
    db_path: Option<&Path>,
    verify: bool,
    ctx: &OpCtx,
) -> Result<String, String> {
    if !verify
        && let Some(db_path) = db_path
        && let Ok(connection) = retro_junk_db::open_database(db_path)
        && let Ok(source_generation) =
            retro_junk_archive::projection_generation(&profile.archive_root)
        && retro_junk_db::archive_profile_projection_is_current(
            &connection,
            &profile.profile_id.to_string(),
            source_generation,
        )
        .unwrap_or(false)
    {
        return Ok("Archive index is already current".to_owned());
    }
    let _archive_lock = retro_junk_archive::ArchiveLock::acquire(&profile.archive_root)
        .map_err(|error| error.to_string())?;
    retro_junk_archive::upgrade_legacy_regional_physical_platforms(&profile.archive_root)
        .map_err(|error| error.to_string())?;
    (ctx.progress)(
        "Scanning archive manifests",
        retro_junk_io::ProgressUnit::Items,
        0,
        0,
    );
    let mut snapshot =
        retro_junk_archive::scan_archive_cancellable(&profile.archive_root, ctx.cancel)
            .map_err(|error| error.to_string())?;
    let mut verified_dump_count = 0;
    if verify {
        let report = retro_junk_lib::archive_ops::verify_archive_integrity(
            &snapshot,
            None,
            ctx.progress,
            ctx.cancel,
        )
        .map_err(|error| error.to_string())?;
        verified_dump_count = report.checked;
        // Verification appended evidence after the first snapshot.
        // Rescan once so the projection includes those records.
        snapshot = retro_junk_archive::scan_archive_cancellable(&profile.archive_root, ctx.cancel)
            .map_err(|error| error.to_string())?;
    }
    if let Some(db_path) = db_path {
        (ctx.progress)(
            "Applying archive projection",
            retro_junk_io::ProgressUnit::Items,
            0,
            0,
        );
        let mut connection =
            retro_junk_db::open_database(db_path).map_err(|error| error.to_string())?;
        retro_junk_db::reconcile_archive_snapshot(
            &mut connection,
            &snapshot,
            &profile.playable_root,
            &profile.workspace_root,
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(if verify {
        format!("Verified {verified_dump_count} preservation dump(s)")
    } else {
        "Refreshed archive index".to_owned()
    })
}

/// Bring the projection back in step with the archive after something wrote
/// to it: rescan the archive, pull any artwork that already sits beside the
/// playable files into the archive, and reconcile.
///
/// Adopting artwork adds files to the archive, so the snapshot taken before it
/// is already out of date — hence the second scan before reconciling.
///
/// The caller is expected to already hold the archive lock if it needs the
/// whole surrounding operation serialized; this function does not take it.
pub fn reindex_after_change(
    profile: &CollectionProfile,
    db_path: &Path,
    media_dir_setting: &str,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<(), String> {
    let mut snapshot = retro_junk_archive::scan_archive(&profile.archive_root)
        .map_err(|error| error.to_string())?;
    let mut connection = crate::queries::open_catalog(db_path)?;
    let adopted = super::assets::adopt_playable_artwork(
        &connection,
        &snapshot,
        profile,
        media_dir_setting,
        cancel,
    )?;
    if adopted > 0 {
        log::info!("Adopted {adopted} existing playable artwork file(s) into the archive");
        snapshot = retro_junk_archive::scan_archive(&profile.archive_root)
            .map_err(|error| error.to_string())?;
    }
    retro_junk_db::reconcile_archive_snapshot(
        &mut connection,
        &snapshot,
        &profile.playable_root,
        &profile.workspace_root,
    )
    .map_err(|error| error.to_string())
}

/// Reconcile only release-level supporting files for known archive changes.
/// Artwork publication cannot affect carriers, dumps, or playable evidence,
/// so a full archive projection rebuild would be unrelated work.
pub fn reconcile_release_files(
    profile: &CollectionProfile,
    db_path: &Path,
    release_ids: &[retro_junk_archive::ArchiveReleaseId],
    cancel: &AtomicBool,
) -> Result<(), String> {
    let mut releases = Vec::with_capacity(release_ids.len());
    for release_id in release_ids {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("operation cancelled".to_owned());
        }
        releases.push(
            retro_junk_archive::scan_archive_release(&profile.archive_root, *release_id)
                .map_err(|error| error.to_string())?,
        );
    }
    let mut connection =
        retro_junk_db::open_database(db_path).map_err(|error| error.to_string())?;
    retro_junk_db::reconcile_archive_supporting_files(
        &mut connection,
        &profile.archive_root,
        &releases,
    )
    .map_err(|error| error.to_string())
}

/// Resolve archived carriers against the current catalog and persist exact
/// carrier identities. Compatible carriers from different mastering records
/// remain grouped under a work-level parent.
pub fn identify_carriers(
    profile: &CollectionProfile,
    db_path: &Path,
    ctx: &OpCtx,
) -> Result<String, String> {
    let _archive_lock = retro_junk_archive::ArchiveLock::acquire(&profile.archive_root)
        .map_err(|error| error.to_string())?;
    let snapshot = retro_junk_archive::scan_archive(&profile.archive_root)
        .map_err(|error| error.to_string())?;
    let mut connection =
        retro_junk_db::open_database(db_path).map_err(|error| error.to_string())?;
    let report = retro_junk_lib::archive_ops::identify_archived_carriers(
        &retro_junk_lib::archive_ops::IdentifyCarriersRequest {
            snapshot: &snapshot,
            selection: retro_junk_lib::archive_ops::IdentifySelection::StaleOnly,
            only_dump: None,
            redumper_path: Path::new(""),
            workspace_root: &profile.processing_workspace_root(),
        },
        &connection,
        ctx.progress,
        ctx.cancel,
    )
    .map_err(|error| error.to_string())?;
    if report.selected == 0 {
        return Ok("All carriers already have current catalog identification".to_owned());
    }
    let snapshot = retro_junk_archive::scan_archive(&profile.archive_root)
        .map_err(|error| error.to_string())?;
    retro_junk_db::reconcile_archive_snapshot(
        &mut connection,
        &snapshot,
        &profile.playable_root,
        &profile.workspace_root,
    )
    .map_err(|error| error.to_string())?;
    Ok(format!(
        "Identified {} carrier(s); {} ambiguous; {} unmatched; {} failed",
        report.identified, report.ambiguous, report.unmatched, report.failed
    ))
}
