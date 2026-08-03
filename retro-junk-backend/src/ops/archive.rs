//! Archive maintenance commands: refresh the projection, verify integrity,
//! identify carriers against the catalog.

use std::path::Path;

use retro_junk_archive::CollectionProfile;

use super::OpCtx;

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
    let _archive_lock = retro_junk_archive::ArchiveLock::acquire(&profile.archive_root)
        .map_err(|error| error.to_string())?;
    retro_junk_archive::upgrade_legacy_regional_physical_platforms(&profile.archive_root)
        .map_err(|error| error.to_string())?;
    let mut snapshot = retro_junk_archive::scan_archive(&profile.archive_root)
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
        snapshot = retro_junk_archive::scan_archive(&profile.archive_root)
            .map_err(|error| error.to_string())?;
    }
    if let Some(db_path) = db_path {
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
