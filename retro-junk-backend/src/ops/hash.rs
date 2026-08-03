//! Hash verification for library entries.
//!
//! Flattens the selected entries into per-file work items (one per disc for
//! multi-disc sets), computes CRC32/SHA-1 for each — with track-aware CUE
//! hashing, optional local staging for network-hosted CHDs, and byte-level
//! progress — then resolves the whole batch against the catalog database
//! with clustered queries and judges complete-disc verification per track
//! set. The frontend only schedules the call and delivers the results.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use retro_junk_core::RomAnalyzer;
use retro_junk_dat::FileHashes;
use retro_junk_io::ProgressUnit;
use retro_junk_lib::{AnalysisContext, Platform, hasher};

use super::OpCtx;
use crate::library::{DiscVerification, EntryHashResult, LibraryEntry};

#[cfg(test)]
#[path = "hash_tests.rs"]
mod tests;

/// 4 MB throttle — only send a progress update when at least this many new bytes
/// have been processed since the last report.
const PROGRESS_THROTTLE: u64 = 4 * 1024 * 1024;

/// A single unit of hash work: either a whole entry or one disc of a multi-disc entry.
pub struct HashWork {
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
/// multi-disc entries get one item per disc. Also returns a snapshot of the
/// entries that produced work, in the same order, so the caller can pair
/// results back to them.
pub fn collect_hash_work<'a>(
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
    workspace_root: &Path,
    stage_large_files: bool,
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
    let staged = if is_chd && stage_large_files {
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
    if is_chd && stage_large_files {
        phase(
            format!("Decoding and hashing {}", item.entry_name),
            item.file_size,
            item.file_size.saturating_mul(2),
        );
    }
    log::debug!("compute_hashes: calling hasher for {}", item.path.display());
    let hash_progress = |done: u64, total: u64| {
        if is_chd && stage_large_files {
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

/// Everything a hash run needs beyond the work items themselves.
pub struct HashRequest {
    pub platform: Platform,
    /// Flattened work items from [`collect_hash_work`].
    pub work: Vec<HashWork>,
    /// Where to stage network-hosted files for seek-heavy verification.
    pub workspace_root: PathBuf,
    /// Stage CHDs to `workspace_root` before hashing (network mode).
    pub stage_large_files: bool,
}

/// One work item that could not be hashed, with the user-facing reason.
pub struct HashFailure {
    pub entry_id: retro_junk_db::LibraryEntryId,
    pub entry_name: String,
    pub error: String,
}

/// What a hash run hands back to the frontend.
pub struct HashReport {
    /// Successful per-file results, grouped by owning entry, with catalog
    /// candidates already resolved and disc verification judged.
    pub results_by_entry: HashMap<retro_junk_db::LibraryEntryId, Vec<EntryHashResult>>,
    pub failures: Vec<HashFailure>,
    /// True when the run stopped early; completed results are discarded so a
    /// cancelled run never writes partial batches.
    pub cancelled: bool,
}

impl HashReport {
    fn cancelled(failures: Vec<HashFailure>) -> Self {
        Self {
            results_by_entry: HashMap::new(),
            failures,
            cancelled: true,
        }
    }
}

/// Compute hashes for the requested work items and match them against the
/// catalog database (opened from `db_path` on this thread) in one clustered
/// query pass — the frontend never performs per-entry catalog lookups while
/// applying the results.
pub fn compute_entry_hashes(
    context: &AnalysisContext,
    db_path: Option<&Path>,
    request: HashRequest,
    ctx: &OpCtx,
) -> HashReport {
    let HashRequest {
        platform,
        mut work,
        workspace_root,
        stage_large_files,
    } = request;

    // The description most recently announced via a phase change; throttled
    // numeric updates re-report it because every progress callback carries
    // its phase.
    let current_phase = RefCell::new("Computing hashes".to_string());
    let report_progress = |current: u64, total: u64| {
        (ctx.progress)(&current_phase.borrow(), ProgressUnit::Bytes, current, total);
    };

    for item in &mut work {
        if ctx.cancelled() {
            return HashReport::cancelled(Vec::new());
        }
        item.file_size = std::fs::metadata(&item.path).map_or(0, |metadata| metadata.len());
    }
    let mut total_bytes: u64 = work.iter().map(|item| item.file_size).sum();
    report_progress(0, total_bytes);
    let Some(registered) = context.get_by_platform(platform) else {
        return HashReport {
            results_by_entry: HashMap::new(),
            failures: Vec::new(),
            cancelled: false,
        };
    };

    let mut bytes_completed: u64 = 0;
    let last_reported = Cell::new(0u64);
    let mut completed = Vec::new();
    let mut failures = Vec::new();

    for item in &work {
        if ctx.cancelled() {
            return HashReport::cancelled(failures);
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
            if total_changed || current.saturating_sub(last_reported.get()) >= PROGRESS_THROTTLE {
                last_reported.set(current);
                report_progress(current, effective_operation_total.get());
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
            *current_phase.borrow_mut() = description;
            report_progress(current, effective_operation_total.get());
        };

        match hash_one(
            item,
            registered.analyzer.as_ref(),
            &throttled_progress,
            &report_phase,
            &workspace_root,
            stage_large_files,
            ctx.cancel,
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
                failures.push(HashFailure {
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
        report_progress(bytes_completed, total_bytes);
        last_reported.set(bytes_completed);
    }

    (ctx.progress)(
        &format!(
            "Matching {} hashed file(s) against the catalog",
            completed.len()
        ),
        ProgressUnit::Items,
        0,
        0,
    );

    // Resolve the completed cluster with one SQL query pass.
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
                    .map(|candidate: &retro_junk_db::CatalogMediaMatch| candidate.media.id.clone())
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
            let tracks = retro_junk_db::find_media_tracks_for_media_ids(&conn, &media_ids).ok()?;
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
            let full_ids =
                fully_matching_disc_media_ids(local_tracks, &catalog_matches, &tracks_by_media);
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

    HashReport {
        results_by_entry,
        failures,
        cancelled: false,
    }
}
