use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::app::RetroJunkApp;
use crate::state::{
    AppMessage, BackgroundOperation, OperationKind, ProgressDisplay, next_operation_id,
};

/// Refresh the rebuildable archive projection, optionally appending a fresh
/// integrity verification first. The archive lock serializes this with every
/// authoritative manifest mutation; SQLite readers retain the prior complete
/// projection until the replacement transaction commits.
pub(crate) fn start_archive_operation(
    app: &mut RetroJunkApp,
    profile: &retro_junk_archive::CollectionProfile,
    verify: bool,
) {
    let op_id = next_operation_id();
    let cancel = Arc::new(AtomicBool::new(false));
    app.operations.push(BackgroundOperation::new(
        op_id,
        if verify {
            "Verifying archive"
        } else {
            "Refreshing archive index"
        }
        .to_owned(),
        Arc::clone(&cancel),
        OperationKind::Hash,
        "archive".to_owned(),
        ProgressDisplay::Count,
    ));
    let sender = app.message_tx.clone();
    let profile = profile.clone();
    let db_path = app.db_path.clone();
    let handle = std::thread::spawn(move || {
        let result = (|| -> Result<String, String> {
            let _archive_lock = retro_junk_archive::ArchiveLock::acquire(&profile.archive_root)
                .map_err(|error| error.to_string())?;
            retro_junk_archive::upgrade_legacy_regional_physical_platforms(&profile.archive_root)
                .map_err(|error| error.to_string())?;
            let mut snapshot = retro_junk_archive::scan_archive(&profile.archive_root)
                .map_err(|error| error.to_string())?;
            let mut verified_dump_count = 0;
            if verify {
                let dumps = snapshot
                    .releases
                    .iter()
                    .flat_map(|release| &release.physical_copies)
                    .flat_map(|item| &item.carriers)
                    .flat_map(|medium| &medium.dumps)
                    .collect::<Vec<_>>();
                verified_dump_count = dumps.len();
                let _ = sender.send(AppMessage::OperationProgress {
                    op_id,
                    current: 0,
                    total: dumps.len() as u64,
                });
                for (index, dump) in dumps.iter().enumerate() {
                    let report = retro_junk_archive::verify_dump_integrity(
                        &dump.directory,
                        &dump.manifest,
                        &cancel,
                    )
                    .map_err(|error| error.to_string())?;
                    let verification_id = retro_junk_archive::VerificationId::new();
                    let evidence = retro_junk_archive::VerificationEvidence {
                        schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
                        verification_id,
                        representation_id: dump.manifest.representation_id,
                        performed_at: chrono::Utc::now().to_rfc3339(),
                        input_manifest_sha256: dump.manifest_sha256.clone(),
                        kind: retro_junk_archive::VerificationKind::Integrity,
                        outcome: if report.is_verified() {
                            retro_junk_archive::VerificationOutcome::Verified
                        } else {
                            retro_junk_archive::VerificationOutcome::Failed
                        },
                        tool: None,
                        catalog: None,
                        tracks: Vec::new(),
                        detail: report
                            .failures
                            .iter()
                            .map(|failure| format!("{}: {}", failure.path, failure.reason))
                            .collect::<Vec<_>>()
                            .join("; "),
                    };
                    let evidence_directory = dump.directory.join("evidence");
                    std::fs::create_dir_all(&evidence_directory)
                        .map_err(|error| error.to_string())?;
                    retro_junk_archive::write_json_new(
                        &evidence_directory.join(format!("verification-{verification_id}.json")),
                        &evidence,
                    )
                    .map_err(|error| error.to_string())?;
                    let _ = sender.send(AppMessage::OperationProgress {
                        op_id,
                        current: (index + 1) as u64,
                        total: dumps.len() as u64,
                    });
                }
                // Verification appended evidence after the first snapshot.
                // Rescan once so the projection includes those new records.
                snapshot = retro_junk_archive::scan_archive(&profile.archive_root)
                    .map_err(|error| error.to_string())?;
            }
            if let Some(db_path) = db_path {
                let mut connection =
                    retro_junk_db::open_database(&db_path).map_err(|error| error.to_string())?;
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
        })();
        let _ = sender.send(AppMessage::ArchiveOperationComplete { op_id, result });
    });
    app.op_threads.insert(op_id, handle);
}
