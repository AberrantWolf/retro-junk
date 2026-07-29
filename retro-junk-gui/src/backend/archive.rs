use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::app::RetroJunkApp;
use crate::state::{
    AppMessage, BackgroundOperation, OperationKind, ProgressDisplay, next_operation_id,
};

/// Refresh the rebuildable archive projection, optionally appending a fresh
/// integrity verification first. The archive lock serializes this with every
/// authoritative manifest mutation; `SQLite` readers retain the prior complete
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

/// Reproduce unbound Redumper masters, resolve complete track sets against the
/// current catalog, and persist exact carrier identities. Compatible carriers
/// from different mastering records remain grouped under a work-level parent.
pub(crate) fn start_catalog_identification_operation(
    app: &mut RetroJunkApp,
    profile: &retro_junk_archive::CollectionProfile,
) {
    let Some(db_path) = app.db_path.clone() else {
        app.push_error(
            "Identify archived carriers",
            "Catalog database is unavailable",
        );
        return;
    };
    let op_id = next_operation_id();
    let cancel = Arc::new(AtomicBool::new(false));
    app.operations.push(BackgroundOperation::new(
        op_id,
        "Identifying archived carriers".to_owned(),
        Arc::clone(&cancel),
        OperationKind::Hash,
        "archive".to_owned(),
        ProgressDisplay::Count,
    ));
    let sender = app.message_tx.clone();
    let profile = profile.clone();
    let handle = std::thread::spawn(move || {
        let result = (|| -> Result<String, String> {
            let _archive_lock = retro_junk_archive::ArchiveLock::acquire(&profile.archive_root)
                .map_err(|error| error.to_string())?;
            let snapshot = retro_junk_archive::scan_archive(&profile.archive_root)
                .map_err(|error| error.to_string())?;
            let candidates = snapshot
                .releases
                .iter()
                .flat_map(|release| {
                    release.physical_copies.iter().flat_map(move |copy| {
                        copy.carriers.iter().filter_map(move |carrier| {
                            carrier
                                .dumps
                                .iter()
                                .rev()
                                .find(|dump| {
                                    dump.manifest.format
                                        == retro_junk_archive::RepresentationFormat::RedumperRaw
                                        && (carrier
                                            .manifest
                                            .catalog_binding
                                            .catalog_media_id
                                            .is_empty()
                                            || !retro_junk_archive::dump_catalog_verified(dump))
                                })
                                .map(|dump| (release, carrier, dump))
                        })
                    })
                })
                .collect::<Vec<_>>();
            if candidates.is_empty() {
                return Ok(
                    "All Redumper carriers already have current catalog identification".to_owned(),
                );
            }
            let redumper = retro_junk_archive::Redumper::detect(std::path::Path::new(""))
                .map_err(|error| error.to_string())?;
            let mut connection =
                retro_junk_db::open_database(&db_path).map_err(|error| error.to_string())?;
            let _ = sender.send(AppMessage::OperationProgress {
                op_id,
                current: 0,
                total: candidates.len() as u64,
            });
            let mut identified = 0_usize;
            let mut ambiguous = 0_usize;
            let mut unmatched = 0_usize;
            let mut failed = 0_usize;
            let candidate_count = candidates.len() as u64;
            for (index, (release, carrier, dump)) in candidates.into_iter().enumerate() {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    return Err("Identification cancelled".to_owned());
                }
                let audit = match redumper.audit(
                    &dump.directory.join("raw"),
                    &profile.processing_workspace_root(),
                    &cancel,
                ) {
                    Ok(audit) => audit,
                    Err(error) => {
                        failed += 1;
                        log::warn!(
                            "Could not reproduce archived dump {} for catalog identification: {error}",
                            dump.manifest.dump_id
                        );
                        let _ = sender.send(AppMessage::OperationProgress {
                            op_id,
                            current: (index + 1) as u64,
                            total: candidate_count,
                        });
                        continue;
                    }
                };
                let matches = retro_junk_db::match_complete_catalog_media(
                    &connection,
                    &release.manifest.platform_id,
                    &audit.tracks,
                )
                .map_err(|error| error.to_string())?;
                match matches.as_slice() {
                    [catalog_match] => {
                        let binding = retro_junk_archive::CatalogBinding {
                            catalog_work_id: catalog_match.work_id.clone(),
                            catalog_release_id: catalog_match.release_id.clone(),
                            catalog_media_id: catalog_match.media_id.clone(),
                            source: catalog_match.source.clone(),
                            dat_name: catalog_match.game.clone(),
                            source_version: catalog_match.source_version.clone(),
                            serials: if catalog_match.serial.is_empty() {
                                Vec::new()
                            } else {
                                vec![catalog_match.serial.clone()]
                            },
                            expected_tracks: audit.tracks.clone(),
                        };
                        retro_junk_archive::bind_carrier_to_catalog(
                            &release.directory.join("release.toml"),
                            &carrier.directory.join("carrier.toml"),
                            &binding,
                        )
                        .map_err(|error| error.to_string())?;
                        append_catalog_evidence(dump, catalog_match, &audit)?;
                        identified += 1;
                    }
                    [] => unmatched += 1,
                    _ => ambiguous += 1,
                }
                let _ = sender.send(AppMessage::OperationProgress {
                    op_id,
                    current: (index + 1) as u64,
                    total: candidate_count,
                });
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
                "Identified {identified} carrier(s); {ambiguous} ambiguous; {unmatched} unmatched; {failed} failed"
            ))
        })();
        let _ = sender.send(AppMessage::ArchiveOperationComplete { op_id, result });
    });
    app.op_threads.insert(op_id, handle);
}

fn append_catalog_evidence(
    dump: &retro_junk_archive::IndexedDump,
    catalog_match: &retro_junk_db::CompleteCatalogMediaMatch,
    audit: &retro_junk_archive::RedumperAudit,
) -> Result<(), String> {
    let verification_id = retro_junk_archive::VerificationId::new();
    let evidence = retro_junk_archive::VerificationEvidence {
        schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
        verification_id,
        representation_id: dump.manifest.representation_id,
        performed_at: chrono::Utc::now().to_rfc3339(),
        input_manifest_sha256: dump.manifest_sha256.clone(),
        kind: retro_junk_archive::VerificationKind::Catalog,
        outcome: retro_junk_archive::VerificationOutcome::Verified,
        tool: Some(audit.tool.clone()),
        catalog: Some(retro_junk_archive::CatalogEvidence {
            source: catalog_match.source.clone(),
            system: catalog_match.platform_id.clone(),
            version: catalog_match.source_version.clone(),
            game: catalog_match.game.clone(),
            complete_track_set: true,
        }),
        tracks: audit
            .tracks
            .iter()
            .map(|track| retro_junk_archive::TrackVerification {
                number: track.number,
                size: track.size,
                expected_sha1: track.sha1.clone(),
                actual_sha1: track.sha1.clone(),
                matched: true,
            })
            .collect(),
        detail: format!(
            "Complete track set matched catalog media {}",
            catalog_match.media_id
        ),
    };
    let evidence_directory = dump.directory.join("evidence");
    std::fs::create_dir_all(&evidence_directory).map_err(|error| error.to_string())?;
    retro_junk_archive::write_json_new(
        &evidence_directory.join(format!("verification-{verification_id}.json")),
        &evidence,
    )
    .map_err(|error| error.to_string())
}
