use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::Ordering;

use retro_junk_core::RomAnalyzer;
use retro_junk_dat::FileHashes;
use retro_junk_lib::hasher;

use crate::app::RetroJunkApp;
use crate::backend::worker::spawn_background_op;
use crate::state::{
    AppMessage, DiscVerification, EntryHashResult, LibraryEntry, OperationKind, ProgressDisplay,
};

/// 4 MB throttle — only send a progress update when at least this many new bytes
/// have been processed since the last report.
const PROGRESS_THROTTLE: u64 = 4 * 1024 * 1024;

/// A single unit of hash work: either a whole entry or one disc of a multi-disc entry.
struct HashWork {
    entry_id: retro_junk_db::LibraryEntryId,
    entry_name: String,
    path: PathBuf,
    file_size: u64,
    is_disc: bool,
    identification: Option<retro_junk_core::RomIdentification>,
}

struct HashOutcome {
    primary: FileHashes,
    cue_tracks: Option<Vec<retro_junk_lib::disc_hash::DiscTrackHashes>>,
    disc_verification: DiscVerification,
}

struct CompletedHash {
    entry_id: retro_junk_db::LibraryEntryId,
    path: PathBuf,
    is_disc: bool,
    identification: Option<retro_junk_core::RomIdentification>,
    outcome: HashOutcome,
}

/// Flatten entries into hash work items — single-file entries get one item,
/// multi-disc entries get one item per disc.
fn collect_hash_work<'a>(
    entries: impl Iterator<Item = &'a LibraryEntry>,
    include_cached: bool,
) -> (Vec<HashWork>, Vec<LibraryEntry>) {
    let entries: Vec<_> = entries
        .filter_map(|entry| entry.id.map(|id| (id, entry)))
        .filter(|(_, entry)| include_cached || !entry_has_complete_hashes(entry))
        .collect();
    let snapshots = entries.iter().map(|(_, entry)| (*entry).clone()).collect();
    let work = entries
        .into_iter()
        .flat_map(|(entry_id, entry)| {
            let name = entry.game_entry.display_name().to_string();
            log::debug!(
                "compute_hashes: entry '{}', disc_identifications={}, status={:?}",
                name,
                entry
                    .disc_identifications
                    .as_ref()
                    .map_or(0, std::vec::Vec::len),
                entry.status
            );
            if let Some(ref discs) = entry.disc_identifications {
                discs
                    .iter()
                    .map(|d| HashWork {
                        entry_id,
                        entry_name: name.clone(),
                        path: d.path.clone(),
                        file_size: 0,
                        is_disc: true,
                        identification: Some(d.identification.clone()),
                    })
                    .collect::<Vec<_>>()
            } else {
                let path = entry.game_entry.analysis_path().to_path_buf();
                vec![HashWork {
                    entry_id,
                    entry_name: name,
                    path,
                    file_size: 0,
                    is_disc: false,
                    identification: entry.identification.clone(),
                }]
            }
        })
        .collect();
    (work, snapshots)
}

fn entry_has_complete_hashes(entry: &LibraryEntry) -> bool {
    entry.disc_identifications.as_ref().map_or_else(
        || entry.hashes.is_some(),
        |discs| !discs.is_empty() && discs.iter().all(|disc| disc.hashes.is_some()),
    )
}

/// Open and hash a single work item, forwarding scaled byte progress
/// (absolute across the whole operation) to `progress`.
fn hash_one(
    item: &HashWork,
    analyzer: &dyn RomAnalyzer,
    progress: &dyn Fn(u64, u64),
    phase: &dyn Fn(String, u64, u64),
    workspace_root: &std::path::Path,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<HashOutcome, String> {
    log::debug!("compute_hashes: opening file {}", item.path.display());
    if item
        .path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("cue"))
    {
        phase(
            format!("Hashing disc tracks for {}", item.entry_name),
            0,
            item.file_size,
        );
        match retro_junk_lib::disc_hash::hash_cue_disc(&item.path, progress) {
            Ok(disc) => {
                return Ok(HashOutcome {
                    primary: disc.primary,
                    cue_tracks: Some(disc.tracks),
                    // Catalog comparison upgrades this only after every track
                    // has matched the same media entry.
                    disc_verification: DiscVerification::Incomplete,
                });
            }
            Err(layout_error) => {
                // Preserve identification value for damaged descriptors while
                // preventing Track 1 from ever upgrading the whole disc.
                let mut file = std::fs::File::open(&item.path).map_err(|e| e.to_string())?;
                let mut primary = hasher::compute_crc32_sha1_with_progress(
                    &mut file,
                    analyzer,
                    progress,
                    Some(&item.path),
                )
                .map_err(|e| e.to_string())?;
                primary.warnings.push(format!(
                    "Invalid CUE layout: {layout_error}. Data-track hash may identify the game, but the disc is not verified."
                ));
                return Ok(HashOutcome {
                    primary,
                    cue_tracks: None,
                    disc_verification: DiscVerification::InvalidLayout,
                });
            }
        }
    }
    let is_chd = item
        .path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("chd"));
    let staged = if is_chd {
        phase(
            format!("Caching {} locally", item.entry_name),
            0,
            item.file_size.saturating_mul(2),
        );
        log::info!(
            "Staging CHD locally before seek-heavy verification: {}",
            item.path.display()
        );
        let mut staged_bytes = 0_u64;
        Some(
            retro_junk_io::stage_package(&item.path, workspace_root, cancel, |bytes| {
                staged_bytes = staged_bytes.saturating_add(bytes);
                progress(staged_bytes, item.file_size.saturating_mul(2));
            })
            .map_err(|error| error.to_string())?,
        )
    } else {
        phase(format!("Hashing {}", item.entry_name), 0, item.file_size);
        None
    };
    let hash_path = staged.as_ref().map_or(item.path.as_path(), |package| {
        package.local_source.as_path()
    });
    let mut file = std::io::BufReader::with_capacity(
        8 * 1024 * 1024,
        std::fs::File::open(hash_path).map_err(|e| e.to_string())?,
    );
    if is_chd {
        phase(
            format!("Decoding and hashing {}", item.entry_name),
            item.file_size,
            item.file_size.saturating_mul(2),
        );
    }
    log::debug!("compute_hashes: calling hasher for {}", item.path.display());
    let hash_progress = |done: u64, total: u64| {
        if is_chd {
            progress(
                item.file_size.saturating_add(done),
                item.file_size.saturating_add(total),
            );
        } else {
            progress(done, total);
        }
    };
    let primary = hasher::compute_crc32_sha1_with_progress(
        &mut file,
        analyzer,
        &hash_progress,
        Some(hash_path),
    )
    .map_err(|e| e.to_string())?;
    Ok(HashOutcome {
        primary,
        cue_tracks: None,
        disc_verification: DiscVerification::NotApplicable,
    })
}

fn replace_component_total(operation_total: u64, old_total: u64, new_total: u64) -> u64 {
    operation_total
        .saturating_sub(old_total)
        .saturating_add(new_total)
}

fn hashes_match_catalog_track(local: &FileHashes, file_size: i64, crc32: &str, sha1: &str) -> bool {
    let crc_matches = u64::try_from(file_size).ok() == Some(local.data_size)
        && !local.crc32.is_empty()
        && local.crc32.eq_ignore_ascii_case(crc32);
    let local_sha1 = local.sha1.as_deref().unwrap_or_default();
    crc_matches || (!local_sha1.is_empty() && local_sha1.eq_ignore_ascii_case(sha1))
}

fn disc_matches_candidate(
    local: &[retro_junk_lib::disc_hash::DiscTrackHashes],
    candidate: &retro_junk_db::CatalogMediaMatch,
    tracks_by_media: &HashMap<String, Vec<retro_junk_db::MediaTrack>>,
) -> bool {
    let Some(expected) = tracks_by_media.get(&candidate.media.id) else {
        return local.len() == 1
            && hashes_match_catalog_track(
                &local[0].hashes,
                candidate.media.file_size,
                &candidate.media.crc32,
                &candidate.media.sha1,
            );
    };
    local.len() == expected.len()
        && local.iter().all(|track| {
            expected
                .iter()
                .find(|expected| expected.track_number == i32::from(track.track_number))
                .is_some_and(|expected| {
                    hashes_match_catalog_track(
                        &track.hashes,
                        expected.file_size,
                        &expected.crc32,
                        &expected.sha1,
                    )
                })
        })
}

fn fully_matching_disc_media_ids(
    local: &[retro_junk_lib::disc_hash::DiscTrackHashes],
    candidates: &[retro_junk_db::CatalogMediaMatch],
    tracks_by_media: &HashMap<String, Vec<retro_junk_db::MediaTrack>>,
) -> HashSet<String> {
    candidates
        .iter()
        .filter(|candidate| disc_matches_candidate(local, candidate, tracks_by_media))
        .map(|candidate| candidate.media.id.clone())
        .collect()
}

fn describe_incomplete_disc(
    local: &[retro_junk_lib::disc_hash::DiscTrackHashes],
    candidates: &[retro_junk_db::CatalogMediaMatch],
    tracks_by_media: &HashMap<String, Vec<retro_junk_db::MediaTrack>>,
) -> Vec<String> {
    let candidate = candidates.iter().find(|candidate| {
        local.iter().any(|track| {
            tracks_by_media
                .get(&candidate.media.id)
                .is_some_and(|expected| {
                    expected.iter().any(|expected| {
                        hashes_match_catalog_track(
                            &track.hashes,
                            expected.file_size,
                            &expected.crc32,
                            &expected.sha1,
                        )
                    })
                })
                || hashes_match_catalog_track(
                    &track.hashes,
                    candidate.media.file_size,
                    &candidate.media.crc32,
                    &candidate.media.sha1,
                )
        })
    });
    let Some(candidate) = candidate.or_else(|| candidates.first()) else {
        return vec![
            "Incomplete disc verification: no catalog media matches the complete local track set."
                .to_string(),
        ];
    };
    let Some(expected) = tracks_by_media.get(&candidate.media.id) else {
        return vec![format!(
            "Incomplete disc verification: the local track set does not match {}.",
            candidate.media.dat_name
        )];
    };

    let mut warnings = Vec::new();
    for expected_track in expected {
        match local
            .iter()
            .find(|track| i32::from(track.track_number) == expected_track.track_number)
        {
            None => warnings.push(format!(
                "Incomplete disc: DAT Track {} is missing ({}).",
                expected_track.track_number, expected_track.track_name
            )),
            Some(local_track)
                if !hashes_match_catalog_track(
                    &local_track.hashes,
                    expected_track.file_size,
                    &expected_track.crc32,
                    &expected_track.sha1,
                ) =>
            {
                warnings.push(format!(
                    "Incomplete disc: Track {} does not match the DAT fingerprint ({}).",
                    expected_track.track_number, expected_track.track_name
                ));
            }
            Some(_) => {}
        }
    }
    for local_track in local {
        if !expected
            .iter()
            .any(|track| track.track_number == i32::from(local_track.track_number))
        {
            warnings.push(format!(
                "Incomplete disc: local Track {} is not present in the matched DAT entry.",
                local_track.track_number
            ));
        }
    }
    if warnings.is_empty() {
        warnings.push(format!(
            "Incomplete disc verification: the local track set does not match {}.",
            candidate.media.dat_name
        ));
    }
    warnings
}

/// Compute hashes for selected entries in the active console.
pub fn compute_hashes_for_selection(app: &mut RetroJunkApp, console_idx: usize) {
    compute_hashes_for_selection_inner(app, console_idx, false);
}

/// Recompute hashes even when the current source fingerprint already has a
/// durable result. This is deliberately separate from the normal action so a
/// verification click does not reread unchanged network-hosted media.
pub fn recompute_hashes_for_selection(app: &mut RetroJunkApp, console_idx: usize) {
    compute_hashes_for_selection_inner(app, console_idx, true);
}

fn compute_hashes_for_selection_inner(
    app: &mut RetroJunkApp,
    console_idx: usize,
    include_cached: bool,
) {
    let console = &app.browser.consoles[console_idx];

    log::debug!(
        "compute_hashes_for_selection: console_idx={}, selected_entries={:?}, total_entries={}",
        console_idx,
        app.ui_state.selected_entries,
        console.entries.len()
    );

    let (work, snapshots) = collect_hash_work(
        app.ui_state
            .selected_entries
            .iter()
            .copied()
            .filter_map(|id| console.entry_by_id(id)),
        include_cached,
    );

    spawn_hash_work(app, console_idx, work, snapshots);
}

fn spawn_hash_work(
    app: &mut RetroJunkApp,
    console_idx: usize,
    work: Vec<HashWork>,
    snapshots: Vec<LibraryEntry>,
) {
    let console = &app.browser.consoles[console_idx];
    let platform = console.platform;
    let folder_name = console.folder_name.clone();

    if work.is_empty() {
        return;
    }

    let context = app.context.clone();
    let db_path = app.db_path.clone();
    let workspace_root = app
        .settings
        .library
        .active_profile()
        .map_or_else(retro_junk_io::default_transient_workspace, |profile| {
            profile.workspace_root.clone()
        });
    let description = format!("Computing hashes ({} files)", work.len());
    let scope = folder_name.clone();

    spawn_background_op(
        app,
        description,
        OperationKind::Hash,
        scope,
        ProgressDisplay::Bytes,
        move |op_id, cancel, tx| {
            let mut work = work;
            for item in &mut work {
                if cancel.load(Ordering::Relaxed) {
                    let _ = tx.send(AppMessage::OperationComplete { op_id });
                    return;
                }
                item.file_size = std::fs::metadata(&item.path).map_or(0, |metadata| metadata.len());
            }
            let mut total_bytes: u64 = work.iter().map(|item| item.file_size).sum();
            let _ = tx.send(AppMessage::OperationProgress {
                op_id,
                current: 0,
                total: total_bytes,
            });
            let Some(registered) = context.get_by_platform(platform) else {
                let _ = tx.send(AppMessage::OperationComplete { op_id });
                return;
            };

            let mut bytes_completed: u64 = 0;
            let last_reported = Cell::new(0u64);
            let mut completed = Vec::new();

            for item in &work {
                if cancel.load(Ordering::Relaxed) {
                    let _ = tx.send(AppMessage::OperationComplete { op_id });
                    return;
                }

                let file_base = bytes_completed;
                let effective_item_total = Cell::new(item.file_size);
                let effective_operation_total = Cell::new(total_bytes);
                let throttled_progress = |file_bytes_done: u64, file_total: u64| {
                    let mut total_changed = false;
                    if file_total != effective_item_total.get() {
                        effective_operation_total.set(replace_component_total(
                            effective_operation_total.get(),
                            effective_item_total.replace(file_total),
                            file_total,
                        ));
                        total_changed = true;
                    }
                    let current = file_base.saturating_add(file_bytes_done);
                    if total_changed
                        || current.saturating_sub(last_reported.get()) >= PROGRESS_THROTTLE
                    {
                        last_reported.set(current);
                        let _ = tx.send(AppMessage::OperationProgress {
                            op_id,
                            current,
                            total: effective_operation_total.get(),
                        });
                    }
                };
                let report_phase = |description: String, file_bytes_done: u64, file_total: u64| {
                    if file_total != effective_item_total.get() {
                        effective_operation_total.set(replace_component_total(
                            effective_operation_total.get(),
                            effective_item_total.replace(file_total),
                            file_total,
                        ));
                    }
                    let current = file_base.saturating_add(file_bytes_done);
                    last_reported.set(current);
                    let _ = tx.send(AppMessage::OperationPhase {
                        op_id,
                        description,
                        display: ProgressDisplay::Bytes,
                        current,
                        total: effective_operation_total.get(),
                    });
                };

                match hash_one(
                    item,
                    registered.analyzer.as_ref(),
                    &throttled_progress,
                    &report_phase,
                    &workspace_root,
                    &cancel,
                ) {
                    Ok(outcome) => {
                        log::debug!(
                            "compute_hashes: success for {}, crc32={}, data_size={}",
                            item.path.display(),
                            outcome.primary.crc32,
                            outcome.primary.data_size
                        );
                        completed.push(CompletedHash {
                            entry_id: item.entry_id,
                            path: item.path.clone(),
                            is_disc: item.is_disc,
                            identification: item.identification.clone(),
                            outcome,
                        });
                    }
                    Err(error) => {
                        let _ = tx.send(AppMessage::HashFailed {
                            folder_name: folder_name.clone(),
                            entry_id: item.entry_id,
                            entry_name: item.entry_name.clone(),
                            error,
                        });
                    }
                }

                // Container-aware hashers report the logical bytes they read
                // (for example, a CUE's referenced BIN track or CHD sectors).
                // Carry that corrected size into the remainder of the batch.
                total_bytes = effective_operation_total.get();
                bytes_completed = bytes_completed.saturating_add(effective_item_total.get());
                let _ = tx.send(AppMessage::OperationProgress {
                    op_id,
                    current: bytes_completed,
                    total: total_bytes,
                });
                last_reported.set(bytes_completed);
            }

            let _ = tx.send(AppMessage::OperationPhase {
                op_id,
                description: format!(
                    "Matching {} hashed file(s) against the catalog",
                    completed.len()
                ),
                display: ProgressDisplay::Count,
                current: 0,
                total: 0,
            });

            // Resolve the completed cluster with one SQL query. The UI only
            // applies the already-fetched candidates and never performs an
            // N+1 catalog lookup while draining hash messages.
            let mut queries = Vec::new();
            let mut query_owners = Vec::new();
            for (owner, completed) in completed.iter().enumerate() {
                if let Some(tracks) = completed.outcome.cue_tracks.as_ref() {
                    for track in tracks {
                        queries.push(retro_junk_db::CatalogHashQuery {
                            file_size: track.hashes.data_size,
                            crc32: track.hashes.crc32.clone(),
                            sha1: track.hashes.sha1.clone().unwrap_or_default(),
                        });
                        query_owners.push(owner);
                    }
                } else {
                    let hashes = &completed.outcome.primary;
                    queries.push(retro_junk_db::CatalogHashQuery {
                        file_size: hashes.data_size,
                        crc32: hashes.crc32.clone(),
                        sha1: hashes.sha1.clone().unwrap_or_default(),
                    });
                    query_owners.push(owner);
                }
            }
            let serial_queries: Vec<_> = completed
                .iter()
                .map(|completed| {
                    completed
                        .identification
                        .as_ref()
                        .and_then(|identification| {
                            retro_junk_lib::catalog_match::catalog_serial_key(
                                registered.analyzer.as_ref(),
                                identification,
                            )
                        })
                        .unwrap_or_default()
                })
                .collect();
            let (matches, tracks_by_media) = db_path
                .as_deref()
                .and_then(|path| retro_junk_db::open_database(path).ok())
                .and_then(|conn| {
                    let mut flat_hash_matches = Vec::with_capacity(queries.len());
                    for cluster in queries.chunks(200) {
                        flat_hash_matches.extend(
                            retro_junk_db::match_media_by_hashes(
                                &conn,
                                registered.analyzer.short_name(),
                                cluster,
                            )
                            .ok()?,
                        );
                    }
                    let mut hash_matches = vec![Vec::new(); completed.len()];
                    for (owner, candidates) in query_owners.iter().copied().zip(flat_hash_matches) {
                        let owned = &mut hash_matches[owner];
                        let mut ids: HashSet<String> = owned
                            .iter()
                            .map(|candidate: &retro_junk_db::CatalogMediaMatch| {
                                candidate.media.id.clone()
                            })
                            .collect();
                        for candidate in candidates {
                            if ids.insert(candidate.media.id.clone()) {
                                owned.push(candidate);
                            }
                        }
                    }
                    let mut serial_matches = Vec::with_capacity(serial_queries.len());
                    for cluster in serial_queries.chunks(200) {
                        serial_matches.extend(
                            retro_junk_db::match_media_by_serials(
                                &conn,
                                registered.analyzer.short_name(),
                                cluster,
                            )
                            .ok()?,
                        );
                    }
                    let merged: Vec<Vec<retro_junk_db::CatalogMediaMatch>> = hash_matches
                        .into_iter()
                        .zip(serial_matches)
                        .map(|(mut hash_matches, serial_matches)| {
                            let mut media_ids: HashSet<String> = hash_matches
                                .iter()
                                .map(|candidate| candidate.media.id.clone())
                                .collect();
                            for candidate in serial_matches {
                                if media_ids.insert(candidate.media.id.clone()) {
                                    hash_matches.push(candidate);
                                }
                            }
                            hash_matches
                        })
                        .collect();
                    let media_ids: Vec<String> = merged
                        .iter()
                        .flatten()
                        .map(|candidate| candidate.media.id.clone())
                        .collect::<HashSet<_>>()
                        .into_iter()
                        .collect();
                    let tracks =
                        retro_junk_db::find_media_tracks_for_media_ids(&conn, &media_ids).ok()?;
                    let mut tracks_by_media: HashMap<String, Vec<retro_junk_db::MediaTrack>> =
                        HashMap::new();
                    for track in tracks {
                        tracks_by_media
                            .entry(track.media_id.clone())
                            .or_default()
                            .push(track);
                    }
                    Some((merged, tracks_by_media))
                })
                .unwrap_or_else(|| (vec![Vec::new(); completed.len()], HashMap::new()));
            let mut results_by_entry: HashMap<retro_junk_db::LibraryEntryId, Vec<EntryHashResult>> =
                HashMap::new();
            for (mut completed, mut catalog_matches) in completed.into_iter().zip(matches) {
                if let Some(local_tracks) = completed.outcome.cue_tracks.as_ref() {
                    let full_ids = fully_matching_disc_media_ids(
                        local_tracks,
                        &catalog_matches,
                        &tracks_by_media,
                    );
                    if full_ids.is_empty() {
                        completed
                            .outcome
                            .primary
                            .warnings
                            .extend(describe_incomplete_disc(
                                local_tracks,
                                &catalog_matches,
                                &tracks_by_media,
                            ));
                        completed.outcome.disc_verification = DiscVerification::Incomplete;
                    } else {
                        catalog_matches.retain(|candidate| full_ids.contains(&candidate.media.id));
                        completed.outcome.disc_verification = DiscVerification::Complete;
                    }
                }
                results_by_entry
                    .entry(completed.entry_id)
                    .or_default()
                    .push(EntryHashResult {
                        disc_path: completed.is_disc.then_some(completed.path),
                        hashes: completed.outcome.primary,
                        catalog_matches,
                        disc_verification: completed.outcome.disc_verification,
                    });
            }
            for entry in snapshots {
                let Some(entry_id) = entry.id else {
                    continue;
                };
                let Some(results) = results_by_entry.remove(&entry_id) else {
                    continue;
                };
                let _ = tx.send(AppMessage::EntryHashBatchComplete {
                    folder_name: folder_name.clone(),
                    entry: Box::new(entry),
                    results,
                });
            }

            let _ = tx.send(AppMessage::OperationComplete { op_id });
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_hash_work_reuses_complete_results_but_force_includes_them() {
        let mut cached = crate::test_support::test_entry(
            retro_junk_lib::scanner::GameEntry::SingleFile("cached.nes".into()),
        );
        cached.hashes = Some(FileHashes {
            crc32: "12345678".into(),
            sha1: Some("abc".into()),
            md5: None,
            data_size: 3,
            warnings: Vec::new(),
        });
        let missing = crate::test_support::test_entry(
            retro_junk_lib::scanner::GameEntry::SingleFile("missing.nes".into()),
        );

        let (normal, _) = collect_hash_work([&cached, &missing].into_iter(), false);
        assert_eq!(normal.len(), 1);
        assert_eq!(normal[0].entry_name, "missing.nes");

        let (forced, _) = collect_hash_work([&cached, &missing].into_iter(), true);
        assert_eq!(forced.len(), 2);
    }

    fn disc_candidate() -> retro_junk_db::CatalogMediaMatch {
        retro_junk_db::CatalogMediaMatch {
            media: retro_junk_catalog::types::Media {
                id: "game-media".into(),
                release_id: "game-release".into(),
                media_serial: "SLUS-00000".into(),
                disc_number: 0,
                disc_label: String::new(),
                revision: String::new(),
                status: retro_junk_catalog::types::MediaStatus::Verified,
                tag: None,
                dat_name: "Game (USA)".into(),
                rom_name: "Game (USA) (Track 1).bin".into(),
                dat_source: "redump".into(),
                file_size: 100,
                crc32: "11111111".into(),
                sha1: String::new(),
                md5: String::new(),
                created_at: String::new(),
                updated_at: String::new(),
            },
            platform_id: "ps1".into(),
            region: "usa".into(),
            release_revision: String::new(),
            release_title: "Game".into(),
            cover_title: String::new(),
            screen_title: String::new(),
        }
    }

    fn local_track(
        number: u8,
        size: u64,
        crc32: &str,
    ) -> retro_junk_lib::disc_hash::DiscTrackHashes {
        retro_junk_lib::disc_hash::DiscTrackHashes {
            track_number: number,
            is_data: number == 1,
            hashes: FileHashes {
                crc32: crc32.into(),
                sha1: None,
                md5: None,
                data_size: size,
                warnings: Vec::new(),
            },
        }
    }

    #[test]
    fn replacing_a_container_estimate_keeps_the_batch_total_consistent() {
        let initial = 100 + 200;
        let after_cue_expands_to_bin = replace_component_total(initial, 100, 700);
        assert_eq!(after_cue_expands_to_bin, 900);
        assert_eq!(
            replace_component_total(after_cue_expands_to_bin, 200, 150),
            850
        );
    }

    #[test]
    fn cue_hash_progress_reports_referenced_bin_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let cue_path = dir.path().join("Game.cue");
        let bin_path = dir.path().join("Game.bin");
        let bin_size = 128 * 1024;
        std::fs::write(&bin_path, vec![0_u8; bin_size]).unwrap();
        std::fs::write(
            &cue_path,
            "FILE \"Game.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n",
        )
        .unwrap();
        let cue_size = std::fs::metadata(&cue_path).unwrap().len();
        let item = HashWork {
            entry_id: retro_junk_db::LibraryEntryId(1),
            entry_name: "Game".into(),
            path: cue_path,
            file_size: cue_size,
            is_disc: false,
            identification: None,
        };
        let progress = Cell::new((0_u64, 0_u64));

        let hashes = hash_one(
            &item,
            &retro_junk_sony::Ps1Analyzer,
            &|done, total| {
                progress.set((done, total));
            },
            &|_, _, _| {},
            dir.path(),
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();

        assert_eq!(hashes.primary.data_size, bin_size as u64);
        assert_eq!(progress.get(), (bin_size as u64, bin_size as u64));
        assert!(progress.get().1 > cue_size);
    }

    #[test]
    fn malformed_combined_bin_cue_recovers_data_track_but_cannot_verify_disc() {
        let dir = tempfile::tempdir().unwrap();
        let cue_path = dir.path().join("game.cue");
        let bin_path = dir.path().join("game.bin");
        let mut bin = Vec::new();
        for _ in 0..2 {
            let mut sector = vec![0_u8; 2352];
            sector[..12].copy_from_slice(&retro_junk_disc::CD_SYNC_PATTERN);
            sector[24] = 1;
            bin.extend(sector);
        }
        bin.extend(vec![0x55_u8; 3 * 2352]);
        std::fs::write(&bin_path, bin).unwrap();
        std::fs::write(
            &cue_path,
            "FILE \"game.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n  TRACK 02 AUDIO\n    INDEX 01 00:00:00\nFILE \"game.bin\" BINARY\n",
        )
        .unwrap();
        let item = HashWork {
            entry_id: retro_junk_db::LibraryEntryId(1),
            entry_name: "Game".into(),
            path: cue_path,
            file_size: 0,
            is_disc: false,
            identification: None,
        };

        let result = hash_one(
            &item,
            &retro_junk_sony::Ps1Analyzer,
            &|_, _| {},
            &|_, _, _| {},
            dir.path(),
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();

        assert_eq!(result.primary.data_size, 2 * 2352);
        assert_eq!(result.disc_verification, DiscVerification::InvalidLayout);
        assert!(result.cue_tracks.is_none());
        assert!(
            result
                .primary
                .warnings
                .iter()
                .any(|warning| warning.contains("Invalid CUE layout"))
        );
    }

    #[test]
    fn complete_catalog_track_assignment_is_required_for_disc_verification() {
        let candidate = disc_candidate();
        let local = vec![
            local_track(1, 100, "11111111"),
            local_track(2, 50, "22222222"),
        ];
        let tracks = vec![
            retro_junk_db::MediaTrack {
                media_id: candidate.media.id.clone(),
                track_number: 1,
                track_name: "Game (Track 1).bin".into(),
                file_size: 100,
                crc32: "11111111".into(),
                sha1: String::new(),
                md5: String::new(),
            },
            retro_junk_db::MediaTrack {
                media_id: candidate.media.id.clone(),
                track_number: 2,
                track_name: "Game (Track 2).bin".into(),
                file_size: 50,
                crc32: "22222222".into(),
                sha1: String::new(),
                md5: String::new(),
            },
        ];
        let tracks_by_media = HashMap::from([(candidate.media.id.clone(), tracks)]);

        assert_eq!(
            fully_matching_disc_media_ids(
                &local,
                std::slice::from_ref(&candidate),
                &tracks_by_media
            ),
            HashSet::from([candidate.media.id.clone()])
        );

        let missing_audio = &local[..1];
        assert!(
            fully_matching_disc_media_ids(
                missing_audio,
                std::slice::from_ref(&candidate),
                &tracks_by_media
            )
            .is_empty()
        );
        assert!(
            describe_incomplete_disc(
                missing_audio,
                std::slice::from_ref(&candidate),
                &tracks_by_media
            )
            .iter()
            .any(|warning| warning.contains("Track 2 is missing"))
        );
    }
}
