//! Shared archive orchestrations.
//!
//! A CLI and GUI action that claim the same destination must call the same
//! verification/build/projection implementation. This module owns the
//! per-destination orchestration that previously lived inline (and diverged)
//! in `retro-junk-cli/commands/archive.rs` and the GUI archive backends:
//! integrity verification, catalog identification of archived carriers, and
//! release-aware playable builds. Callers own locking and projection
//! reconcile; these functions mutate the archive (append-only evidence,
//! catalog bindings, playable outputs) and report what happened.
//!
//! Every function keeps the established contract:
//! `progress: &PhaseProgressFn<'_>` + `cancelled: &AtomicBool`.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use retro_junk_archive::{
    ArchiveIndexSnapshot, CatalogBinding, CatalogEvidence, IndexedCarrier, IndexedDump,
    IndexedRelease, RepresentationFormat, TrackDigest, TrackVerification, VerificationEvidence,
    VerificationId, VerificationKind, VerificationOutcome, write_json_new,
};
use retro_junk_io::{PhaseProgressFn, ProgressUnit};

use crate::playable_build::{
    CatalogVerificationRequest, PlayableBuildError, PlayableBuildRequest, build_playable,
    verify_dump_against_catalog,
};

#[derive(Debug, thiserror::Error)]
pub enum ArchiveOpsError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Build(#[from] PlayableBuildError),
    #[error("operation cancelled")]
    Cancelled,
}

impl ArchiveOpsError {
    fn msg(error: impl std::fmt::Display) -> Self {
        Self::Message(error.to_string())
    }
}

/// Frontend output roots resolved once per operation.
///
/// The GUI resolves these from its settings (empty assets setting = the
/// `{root}-media` sibling convention); the CLI resolves them from flags with
/// the same sibling defaults. Everything below this type deals only in
/// resolved roots.
#[derive(Debug, Clone)]
pub struct FrontendRoots {
    pub playable_root: PathBuf,
    pub media_root: PathBuf,
    pub metadata_root: PathBuf,
}

impl FrontendRoots {
    /// Resolve from user settings strings (GUI convention: empty assets
    /// setting means the sibling `-media` directory; the metadata setting is
    /// a path relative to the playable root, `"."` meaning inline).
    #[must_use]
    pub fn from_settings(
        playable_root: &Path,
        assets_dir_setting: &str,
        metadata_dir_setting: &str,
    ) -> Self {
        let media_root = if assets_dir_setting.is_empty() {
            crate::util::default_media_dir(playable_root)
        } else {
            resolve_root(playable_root, assets_dir_setting)
        };
        let metadata_root = if metadata_dir_setting.is_empty() {
            playable_root.to_path_buf()
        } else {
            resolve_root(playable_root, metadata_dir_setting)
        };
        Self {
            playable_root: playable_root.to_path_buf(),
            media_root,
            metadata_root,
        }
    }
}

fn resolve_root(playable_root: &Path, setting: &str) -> PathBuf {
    let p = Path::new(setting);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        playable_root.join(p)
    }
}

// ── Integrity verification ─────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct IntegrityRunReport {
    pub checked: usize,
    pub failed: usize,
    /// Per-dump failure detail (dump id, joined failure reasons).
    pub failures: Vec<(String, String)>,
}

/// Re-hash every archived preservation file in `snapshot` (optionally one
/// dump) and append integrity evidence, verified or failed.
///
/// The caller decides locking and whether to rescan + reconcile afterwards.
pub fn verify_archive_integrity(
    snapshot: &ArchiveIndexSnapshot,
    only_dump: Option<&str>,
    progress: &PhaseProgressFn<'_>,
    cancelled: &AtomicBool,
) -> Result<IntegrityRunReport, ArchiveOpsError> {
    let dumps = all_dumps(snapshot)
        .filter(|(_, _, dump)| only_dump.is_none_or(|id| dump.manifest.dump_id.to_string() == id))
        .map(|(_, _, dump)| dump)
        .collect::<Vec<_>>();
    let total = dumps.len() as u64;
    let mut report = IntegrityRunReport::default();
    progress("Verifying stored bytes", ProgressUnit::Items, 0, total);
    for (index, dump) in dumps.into_iter().enumerate() {
        if cancelled.load(Ordering::Relaxed) {
            return Err(ArchiveOpsError::Cancelled);
        }
        let integrity =
            retro_junk_archive::verify_dump_integrity(&dump.directory, &dump.manifest, cancelled)
                .map_err(ArchiveOpsError::msg)?;
        report.checked += 1;
        let detail = if integrity.is_verified() {
            format!(
                "SHA-256 verified {} stored file(s), {} byte(s)",
                integrity.checked_files, integrity.checked_bytes
            )
        } else {
            integrity
                .failures
                .iter()
                .map(|failure| format!("{}: {}", failure.path, failure.reason))
                .collect::<Vec<_>>()
                .join("; ")
        };
        if !integrity.is_verified() {
            report.failed += 1;
            report
                .failures
                .push((dump.manifest.dump_id.to_string(), detail.clone()));
        }
        append_evidence(
            dump,
            &VerificationEvidence {
                schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
                verification_id: VerificationId::new(),
                representation_id: dump.manifest.representation_id,
                performed_at: chrono::Utc::now().to_rfc3339(),
                input_manifest_sha256: dump.manifest_sha256.clone(),
                kind: VerificationKind::Integrity,
                outcome: if integrity.is_verified() {
                    VerificationOutcome::Verified
                } else {
                    VerificationOutcome::Failed
                },
                tool: None,
                catalog: None,
                tracks: Vec::new(),
                detail,
            },
        )?;
        progress(
            "Verifying stored bytes",
            ProgressUnit::Items,
            (index + 1) as u64,
            total,
        );
    }
    Ok(report)
}

// ── Catalog identification ─────────────────────────────────────────────────

/// Which dumps an identification run should look at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentifySelection {
    /// Redumper-raw dumps that are unbound or lack current catalog evidence
    /// (the convergence set — nothing already identified is retouched).
    StaleOnly,
    /// Every Redumper-raw dump (a full audit pass).
    All,
}

#[derive(Debug, Default)]
pub struct IdentifyReport {
    pub selected: usize,
    pub identified: usize,
    pub unmatched: usize,
    pub ambiguous: usize,
    pub failed: usize,
}

/// One identification run's inputs.
pub struct IdentifyCarriersRequest<'a> {
    pub snapshot: &'a ArchiveIndexSnapshot,
    pub selection: IdentifySelection,
    pub only_dump: Option<&'a str>,
    /// Empty = detect redumper on PATH.
    pub redumper_path: &'a Path,
    pub workspace_root: &'a Path,
}

/// Reproduce Redumper raw masters, match complete track sets against the
/// catalog, bind unique matches to their carriers, and append evidence for
/// **every** attempt — verified, unmatched, ambiguous, and failed alike.
/// Ambiguity never auto-binds.
///
/// Single-file masters go through [`verify_catalog_files`] instead.
// One loop, one decision table per candidate; splitting it would scatter the
// evidence-outcome mapping this module exists to keep in one place.
#[allow(clippy::too_many_lines)]
pub fn identify_archived_carriers(
    request: &IdentifyCarriersRequest<'_>,
    conn: &retro_junk_db::Connection,
    progress: &PhaseProgressFn<'_>,
    cancelled: &AtomicBool,
) -> Result<IdentifyReport, ArchiveOpsError> {
    let IdentifyCarriersRequest {
        snapshot,
        selection,
        only_dump,
        redumper_path,
        workspace_root,
    } = *request;
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
                            dump.manifest.format == RepresentationFormat::RedumperRaw
                                && match selection {
                                    IdentifySelection::All => true,
                                    // A dump identification already reached a
                                    // conclusion about is left alone, whether
                                    // that conclusion bound a carrier or not.
                                    // The exception repairs an inconsistency:
                                    // evidence says these bytes matched a
                                    // catalog entry, but the carrier records
                                    // nothing that entry said, so the binding
                                    // needs redoing.
                                    IdentifySelection::StaleOnly => {
                                        !retro_junk_archive::dump_catalog_attempted(dump)
                                            || (retro_junk_archive::dump_catalog_verified(dump)
                                                && carrier
                                                    .manifest
                                                    .catalog_binding
                                                    .dat_name
                                                    .is_empty())
                                    }
                                }
                        })
                        .map(|dump| (release, carrier, dump))
                })
            })
        })
        .filter(|(_, _, dump)| only_dump.is_none_or(|id| dump.manifest.dump_id.to_string() == id))
        .collect::<Vec<_>>();
    let mut report = IdentifyReport {
        selected: candidates.len(),
        ..IdentifyReport::default()
    };
    if candidates.is_empty() {
        return Ok(report);
    }
    let total = candidates.len() as u64;
    progress(
        "Identifying archived carriers",
        ProgressUnit::Items,
        0,
        total,
    );
    for (index, (release, carrier, dump)) in candidates.into_iter().enumerate() {
        if cancelled.load(Ordering::Relaxed) {
            return Err(ArchiveOpsError::Cancelled);
        }
        // Reproducing one disc takes minutes and moves gigabytes, so the inner
        // phases report their own progress rather than leaving the caller
        // staring at an unchanged "dump 0 of 1" for the whole run.
        let prepared = crate::redumper_cache::prepare(
            redumper_path,
            &dump.directory.join("raw"),
            workspace_root,
            &dump.manifest_sha256,
            progress,
            cancelled,
        );
        match prepared {
            Ok(prepared) => {
                let audit = prepared.audit().clone();
                // Evidence records what happened, and on a cache hit no tool
                // ran — the earlier attempt already filed its own reproduction
                // record. The catalog record below is written either way,
                // because matching against the catalog did just happen.
                if !prepared.reused() {
                    append_reproduction_evidence(dump, &audit)?;
                }
                let matches = retro_junk_db::match_complete_catalog_media(
                    conn,
                    &release.manifest.platform_id,
                    &audit.tracks,
                )
                .map_err(ArchiveOpsError::msg)?;
                let unique = matches.len() == 1;
                // Only an identified disc has a build ahead of it. For anything
                // else, holding on to several hundred megabytes of split output
                // would grow the workspace on every run for no benefit.
                if unique {
                    prepared.keep();
                } else {
                    prepared.discard();
                }
                let (outcome, catalog, detail) = match matches.as_slice() {
                    [catalog_match] => {
                        bind_carrier(conn, release, carrier, catalog_match, &audit.tracks)?;
                        report.identified += 1;
                        (
                            VerificationOutcome::Verified,
                            // The match above required the complete ordered
                            // track set, so the evidence may say so.
                            Some(catalog_evidence(
                                catalog_match,
                                &release.manifest.platform_id,
                                true,
                            )),
                            format!(
                                "Complete track set matched catalog media {}",
                                catalog_match.media_id
                            ),
                        )
                    }
                    [] => {
                        report.unmatched += 1;
                        (
                            VerificationOutcome::Unmatched,
                            None,
                            "Raw master reproduced a track set, but no complete catalog match \
                             was found"
                                .to_owned(),
                        )
                    }
                    _ => {
                        report.ambiguous += 1;
                        (
                            VerificationOutcome::Ambiguous,
                            None,
                            format!(
                                "Raw master reproduced a track set matching {} catalog media",
                                matches.len()
                            ),
                        )
                    }
                };
                append_evidence(
                    dump,
                    &VerificationEvidence {
                        schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
                        verification_id: VerificationId::new(),
                        representation_id: dump.manifest.representation_id,
                        performed_at: chrono::Utc::now().to_rfc3339(),
                        input_manifest_sha256: dump.manifest_sha256.clone(),
                        kind: VerificationKind::Catalog,
                        outcome,
                        tool: Some(audit.tool.clone()),
                        catalog,
                        tracks: audit
                            .tracks
                            .iter()
                            .map(|track| TrackVerification {
                                number: track.number,
                                size: track.size,
                                expected_sha1: if unique {
                                    track.sha1.clone()
                                } else {
                                    String::new()
                                },
                                actual_sha1: track.sha1.clone(),
                                matched: unique,
                            })
                            .collect(),
                        detail,
                    },
                )?;
            }
            Err(error) => {
                report.failed += 1;
                log::warn!(
                    "Could not reproduce archived dump {} for catalog identification: {error}",
                    dump.manifest.dump_id
                );
                append_evidence(
                    dump,
                    &VerificationEvidence {
                        schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
                        verification_id: VerificationId::new(),
                        representation_id: dump.manifest.representation_id,
                        performed_at: chrono::Utc::now().to_rfc3339(),
                        input_manifest_sha256: dump.manifest_sha256.clone(),
                        kind: VerificationKind::Reproduction,
                        outcome: VerificationOutcome::Failed,
                        tool: None,
                        catalog: None,
                        tracks: Vec::new(),
                        detail: error.to_string(),
                    },
                )?;
            }
        }
        progress(
            "Identifying archived carriers",
            ProgressUnit::Items,
            (index + 1) as u64,
            total,
        );
    }
    Ok(report)
}

/// Match single-file preservation masters (cartridges, ISOs) against the
/// catalog by normalized logical-payload hashes, bind unique matches, and
/// append evidence for every attempt.
// Same shape as `identify_archived_carriers`: one loop, one decision table.
#[allow(clippy::too_many_lines)]
pub fn verify_catalog_files(
    snapshot: &ArchiveIndexSnapshot,
    conn: &retro_junk_db::Connection,
    analyzers: &crate::AnalysisContext,
    only_dump: Option<&str>,
    progress: &PhaseProgressFn<'_>,
    cancelled: &AtomicBool,
) -> Result<IdentifyReport, ArchiveOpsError> {
    // Every stored master is a candidate. A single-file master matches on
    // its own digests; a cue/bin master matches on its complete ordered track
    // set, read from the digests the dump manifest already records. Only a
    // raw redumper image is excluded, because its tracks do not exist as
    // files until the image is reproduced — that is what `identify` is for.
    let candidates = all_dumps(snapshot)
        .filter(|(_, _, dump)| only_dump.is_none_or(|id| dump.manifest.dump_id.to_string() == id))
        .filter(|(_, _, dump)| {
            !dump.manifest.files.is_empty()
                && dump.manifest.format != RepresentationFormat::RedumperRaw
        })
        .collect::<Vec<_>>();
    let mut report = IdentifyReport {
        selected: candidates.len(),
        ..IdentifyReport::default()
    };
    let total = candidates.len() as u64;
    progress(
        "Catalog-verifying file masters",
        ProgressUnit::Items,
        0,
        total,
    );
    for (index, (release, carrier, dump)) in candidates.into_iter().enumerate() {
        if cancelled.load(Ordering::Relaxed) {
            return Err(ArchiveOpsError::Cancelled);
        }
        if dump.manifest.files.len() > 1 {
            verify_track_set(dump, release, carrier, conn, &mut report)?;
            progress(
                "Catalog-verifying file masters",
                ProgressUnit::Items,
                (index + 1) as u64,
                total,
            );
            continue;
        }
        let [file] = dump.manifest.files.as_slice() else {
            unreachable!("filtered to non-empty dumps, and multi-file handled above");
        };
        let input_path = dump.directory.join("raw").join(&file.path);
        let raw = retro_junk_archive::hash_file_digests(&input_path, cancelled)
            .map_err(ArchiveOpsError::msg)?;
        let actual =
            if let Some(console) = analyzers.get_by_short_name(&release.manifest.platform_id) {
                let mut input = std::fs::File::open(&input_path)?;
                let hashes = crate::hasher::compute_all_hashes(
                    &mut input,
                    console.analyzer.as_ref(),
                    Some(&input_path),
                )
                .map_err(ArchiveOpsError::msg)?;
                retro_junk_archive::FileDigests {
                    size: hashes.data_size,
                    crc32: hashes.crc32,
                    md5: hashes.md5.unwrap_or_default(),
                    sha1: hashes.sha1.unwrap_or_default(),
                    sha256: raw.sha256,
                }
            } else {
                raw
            };
        let matches =
            retro_junk_db::match_catalog_file(conn, &release.manifest.platform_id, &actual)
                .map_err(ArchiveOpsError::msg)?;
        let (outcome, catalog, detail) = match matches.as_slice() {
            [catalog_match] => {
                bind_carrier(conn, release, carrier, catalog_match, &[])?;
                report.identified += 1;
                (
                    VerificationOutcome::Verified,
                    // Single-file match: complete only when the catalog stores
                    // the medium as one file too. A medium held as separate
                    // tracks can still match here on its primary (largest
                    // track) digests, which identifies the game while
                    // verifying one track of it — and recording that as a
                    // complete set is exactly what this flag exists to
                    // prevent.
                    Some(catalog_evidence(
                        catalog_match,
                        &release.manifest.platform_id,
                        !catalog_match.medium_has_tracks,
                    )),
                    format!(
                        "File hashes matched catalog media {}",
                        catalog_match.media_id
                    ),
                )
            }
            [] => {
                report.unmatched += 1;
                (
                    VerificationOutcome::Unmatched,
                    None,
                    "No catalog file matched size and available CRC32/MD5/SHA-1 hashes".to_owned(),
                )
            }
            _ => {
                report.ambiguous += 1;
                (
                    VerificationOutcome::Ambiguous,
                    None,
                    format!("File hashes matched {} catalog media", matches.len()),
                )
            }
        };
        let unique = matches.len() == 1;
        append_evidence(
            dump,
            &VerificationEvidence {
                schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
                verification_id: VerificationId::new(),
                representation_id: dump.manifest.representation_id,
                performed_at: chrono::Utc::now().to_rfc3339(),
                input_manifest_sha256: dump.manifest_sha256.clone(),
                kind: VerificationKind::Catalog,
                outcome,
                tool: None,
                catalog,
                tracks: vec![TrackVerification {
                    number: 1,
                    size: actual.size,
                    expected_sha1: if unique {
                        actual.sha1.clone()
                    } else {
                        String::new()
                    },
                    actual_sha1: actual.sha1,
                    matched: unique,
                }],
                detail,
            },
        )?;
        progress(
            "Catalog-verifying file masters",
            ProgressUnit::Items,
            (index + 1) as u64,
            total,
        );
    }
    Ok(report)
}

// ── Release-aware playable build ───────────────────────────────────────────

/// One release's build job, driven by the derived
/// [`retro_junk_db::ArchivedPlayableGap`].
pub struct ReleaseBuildRequest<'a> {
    pub gap: &'a retro_junk_db::ArchivedPlayableGap,
    pub archive_root: PathBuf,
    pub workspace_root: PathBuf,
    pub roots: FrontendRoots,
    pub format: RepresentationFormat,
    pub playable_platform_id: String,
    pub chdman_path: PathBuf,
    pub redumper_path: PathBuf,
    pub dolphin_tool_path: PathBuf,
    pub options: BTreeMap<String, String>,
    /// Project archived artwork to the frontend media tree afterwards.
    pub project_assets: bool,
    /// Upsert the ES-DE gamelist entry afterwards.
    pub update_gamelist: bool,
}

pub struct ReleaseBuildOutcome {
    pub built: Vec<PathBuf>,
    pub playlist: Option<PathBuf>,
    /// Snapshot taken after all archive mutations — reuse for reconcile
    /// instead of rescanning.
    pub snapshot: ArchiveIndexSnapshot,
}

/// Verify prerequisites release-wide, build every needy carrier, create the
/// playlist when the set is complete, then project assets and upsert the
/// gamelist entry.
///
/// Verification failure stops the release before any new derivative is
/// published. The caller owns the archive lock and the projection reconcile
/// (feed `outcome.snapshot` to `reconcile_archive_snapshot`).
// One release's pipeline — prerequisites, builds, playlist, projections —
// reads as a single unit; the matrix invariant it implements is release-wide.
#[allow(clippy::too_many_lines)]
pub fn build_release_playable(
    request: &ReleaseBuildRequest<'_>,
    conn: &retro_junk_db::Connection,
    progress: &PhaseProgressFn<'_>,
    cancelled: &AtomicBool,
) -> Result<ReleaseBuildOutcome, ArchiveOpsError> {
    let gap = request.gap;
    // Verify every prerequisite first. A bad or mismatched disc stops the
    // release before any new playable derivatives are published.
    for carrier in &gap.carriers {
        if carrier.catalog_verified || (gap.allow_unverified && gap.needs_playable) {
            continue;
        }
        let media_id = carrier.catalog_media_id.as_deref().ok_or_else(|| {
            ArchiveOpsError::Message(format!("{} has no catalog disc binding", gap.title))
        })?;
        let (media, tracks) = expected_tracks_for_media(conn, media_id)?;
        let dump_id = carrier.dump_id.clone().ok_or_else(|| {
            ArchiveOpsError::Message(format!("{} has no preservation dump", gap.title))
        })?;
        let disc_label = disc_label(carrier.sequence_number, "Disc");
        verify_dump_against_catalog(
            &CatalogVerificationRequest {
                archive_root: request.archive_root.clone(),
                workspace_root: request.workspace_root.clone(),
                dump_id,
                redumper_path: request.redumper_path.clone(),
                expected_tracks: tracks,
                catalog: CatalogEvidence {
                    source: media.dat_source,
                    system: request.playable_platform_id.clone(),
                    version: String::new(),
                    game: media.dat_name,
                    complete_track_set: true,
                },
            },
            &|description, unit, current, total| {
                progress(
                    &format!("{disc_label}: {description}"),
                    unit,
                    current,
                    total,
                );
            },
            cancelled,
        )?;
    }

    // Playlist-only jobs collect the already-built disc files up front.
    let existing_playlist_files = if gap.needs_playlist && !gap.needs_playable {
        Some(
            retro_junk_db::existing_playable_disc_paths(
                conn,
                &gap.archive_release_id,
                &request.roots.playable_root,
                gap.expected_disc_count,
            )
            .map_err(ArchiveOpsError::msg)?,
        )
    } else {
        None
    };

    // Canonical output names from the catalog media bound to each carrier.
    let mut canonical_names = HashMap::new();
    for carrier in &gap.carriers {
        let Some(media_id) = carrier.catalog_media_id.as_deref() else {
            continue;
        };
        let Some(media) =
            retro_junk_db::get_media_by_id(conn, media_id).map_err(ArchiveOpsError::msg)?
        else {
            continue;
        };
        // A playable is the whole medium. For a multi-track disc the catalog's
        // `rom_name` is its largest *track* file, so naming the container after
        // it produced "… (Track 1).chd" — and the same stem then reached the
        // scraped media and the frontend entry derived from it.
        let multi_track = !retro_junk_db::find_media_tracks(conn, media_id)
            .map_err(ArchiveOpsError::msg)?
            .is_empty();
        canonical_names.insert(
            carrier.carrier_id.clone(),
            crate::naming::CanonicalName {
                dat_name: media.dat_name,
                rom_name: media.rom_name,
                medium_has_tracks: multi_track,
                title: gap.title.clone(),
                region: gap.region.clone(),
                disc_number: carrier.sequence_number,
                expected_disc_count: gap.expected_disc_count,
                ..Default::default()
            },
        );
    }
    let canonical_release_name = canonical_names
        .values()
        .next()
        .map(crate::naming::canonical_release_stem)
        .unwrap_or_default();

    let mut built = Vec::new();
    for carrier in &gap.carriers {
        if !carrier.needs_playable {
            continue;
        }
        let dump_id = carrier.dump_id.clone().ok_or_else(|| {
            ArchiveOpsError::Message(format!("{} has no preservation dump", gap.title))
        })?;
        let disc_label = disc_label(carrier.sequence_number, "Game");
        let outcome = build_playable(
            &PlayableBuildRequest {
                archive_root: request.archive_root.clone(),
                playable_root: request.roots.playable_root.clone(),
                workspace_root: request.workspace_root.clone(),
                dump_id,
                format: request.format.clone(),
                chdman_path: request.chdman_path.clone(),
                redumper_path: request.redumper_path.clone(),
                dolphin_tool_path: request.dolphin_tool_path.clone(),
                allow_unverified: gap.allow_unverified,
                retain_intermediate: gap.retain_intermediate,
                options: request.options.clone(),
                playable_platform_id: request.playable_platform_id.clone(),
                canonical_name: canonical_names
                    .get(&carrier.carrier_id)
                    .cloned()
                    .unwrap_or_else(|| crate::naming::CanonicalName {
                        title: gap.title.clone(),
                        region: gap.region.clone(),
                        disc_number: carrier.sequence_number,
                        expected_disc_count: gap.expected_disc_count,
                        ..Default::default()
                    }),
            },
            &|description, unit, current, total| {
                progress(
                    &format!("{disc_label}: {description}"),
                    unit,
                    current,
                    total,
                );
            },
            cancelled,
        )?;
        built.push(outcome.output);
    }

    let playlist = if let Some(files) = existing_playlist_files {
        Some(write_release_playlist(
            &request.roots.playable_root,
            &request.playable_platform_id,
            &gap.title,
            &gap.region,
            &canonical_release_name,
            &files,
        )?)
    } else {
        None
    };

    // One post-mutation scan serves asset projection, the gamelist upsert,
    // and the caller's reconcile.
    let snapshot =
        retro_junk_archive::scan_archive(&request.archive_root).map_err(ArchiveOpsError::msg)?;
    let indexed_release = snapshot
        .releases
        .iter()
        .find(|item| item.manifest.archive_release_id.to_string() == gap.archive_release_id);
    if let Some(release) = indexed_release {
        if request.project_assets {
            let media_directory = request.roots.media_root.join(&request.playable_platform_id);
            crate::archive_assets::project_release_assets(
                release,
                &media_directory,
                &crate::archive_assets::release_media_stems(release),
                cancelled,
            )
            .map_err(ArchiveOpsError::msg)?;
        }
        if request.update_gamelist {
            crate::archive_assets::sync_esde_gamelist_for_release(
                release,
                &request.roots.playable_root,
                &request.roots.metadata_root,
                &request.roots.media_root,
            )
            .map_err(ArchiveOpsError::msg)?;
        }
    }
    Ok(ReleaseBuildOutcome {
        built,
        playlist,
        snapshot,
    })
}

// ── Adopting moved playables ───────────────────────────────────────────────

/// One adoption pass over an archive snapshot.
#[derive(Clone, Copy)]
pub struct AdoptionRequest<'a> {
    pub snapshot: &'a ArchiveIndexSnapshot,
    pub playable_root: &'a Path,
    /// Restrict to one archive release; `None` sweeps the whole snapshot.
    pub only_release: Option<&'a str>,
    /// Report what would be adopted without appending any evidence.
    pub dry_run: bool,
}

#[derive(Debug, Default)]
pub struct AdoptionReport {
    /// Current playable builds whose recorded output was not where the
    /// evidence said.
    pub orphaned: usize,
    /// (release label, old relative path, new relative path).
    pub adopted: Vec<(String, String, String)>,
    /// Orphans whose bytes are nowhere under the playable root — genuinely
    /// deleted, and a real gap for the build stage to fill.
    pub unresolved: Vec<(String, String)>,
}

/// Re-adopt playable outputs that moved out from under their build evidence.
///
/// The recorded path is how a scanned library row is bound back to the carrier
/// that produced it, so a rename outside the archive silently splits one game
/// into "archived only" plus "playable only". The recorded SHA-256 still
/// identifies the file: find it by content and append evidence naming where it
/// now lives, which both restores the binding and records the rename the
/// archive was never told about.
///
/// Content matching runs once per moved file, not once per projection: the new
/// evidence means the next scan resolves it by path like any other build.
///
/// This runs unattended regardless of the automation policy — it produces
/// nothing, it only corrects where the archive believes an existing file is.
/// See `retro_junk_backend::worker::daemon_may_run`.
///
/// The caller owns the archive lock and the reconcile that follows.
pub fn adopt_moved_playables(
    request: &AdoptionRequest<'_>,
    progress: &PhaseProgressFn<'_>,
    cancelled: &AtomicBool,
) -> Result<AdoptionReport, ArchiveOpsError> {
    let AdoptionRequest {
        snapshot,
        playable_root,
        only_release,
        dry_run,
    } = *request;
    // The same resolution the projection uses, so a playable that merely lives
    // under the frontend's system directory is found rather than re-adopted.
    let orphans = retro_junk_archive::orphaned_playables(
        snapshot,
        playable_root,
        &retro_junk_db::playable_system_directory,
    )
    .into_iter()
    .filter(|orphan| only_release.is_none_or(|id| orphan.archive_release_id.to_string() == id))
    .collect::<Vec<_>>();
    let mut report = AdoptionReport {
        orphaned: orphans.len(),
        ..AdoptionReport::default()
    };
    if orphans.is_empty() {
        return Ok(report);
    }

    // A scoped repair searches only the system directories the orphans name;
    // an unscoped sweep is the user asking for the whole library to be
    // reconciled, including files moved between systems.
    let directories = if only_release.is_some() {
        retro_junk_archive::search_directories(playable_root, &orphans)
    } else {
        vec![playable_root.to_path_buf()]
    };
    progress("Indexing the playable library", ProgressUnit::Items, 0, 0);
    let by_size = retro_junk_archive::index_by_size(&directories).map_err(ArchiveOpsError::msg)?;
    // Paths current evidence already names, so two byte-identical outputs
    // cannot both adopt one file. Adoptions add to it as they land.
    let mut claimed = retro_junk_archive::claimed_playable_paths(snapshot);

    let total = orphans.len() as u64;
    for (index, orphan) in orphans.iter().enumerate() {
        if cancelled.load(Ordering::Relaxed) {
            return Err(ArchiveOpsError::Cancelled);
        }
        progress(
            &format!("Locating {} by content", orphan.label),
            ProgressUnit::Items,
            index as u64,
            total,
        );
        let located = retro_junk_archive::locate_by_content(
            playable_root,
            orphan,
            &by_size,
            &claimed,
            cancelled,
        )
        .map_err(ArchiveOpsError::msg)?;
        let Some(relative_path) = located else {
            report.unresolved.push((
                orphan.label.clone(),
                orphan.evidence.relative_output_path.clone(),
            ));
            continue;
        };
        if dry_run {
            report.adopted.push((
                orphan.label.clone(),
                orphan.evidence.relative_output_path.clone(),
                relative_path.clone(),
            ));
            // Still claim it: a dry run must report the same one-file-one-
            // representation assignment the real run would make.
            claimed.insert(relative_path);
            continue;
        }
        retro_junk_archive::record_adoption(orphan, &relative_path)
            .map_err(ArchiveOpsError::msg)?;
        log::info!(
            "Adopted moved playable for {}: {} -> {relative_path}",
            orphan.label,
            orphan.evidence.relative_output_path
        );
        claimed.insert(relative_path.clone());
        report.adopted.push((
            orphan.label.clone(),
            orphan.evidence.relative_output_path.clone(),
            relative_path,
        ));
    }
    progress("Adoption complete", ProgressUnit::Items, total, total);
    Ok(report)
}

/// Adopt playable files the archive never built but can prove it owns.
///
/// A collection assembled before the archive existed is full of these: a CHD
/// sitting beside a preservation master of the same disc, with no build
/// evidence connecting them. Neither existing adoption path reaches it —
/// [`adopt_moved_playables`] needs a recorded output digest to search for, and
/// the byte-identical-to-master pass cannot match a compressed container
/// against raw master bytes.
///
/// The proof is already on both sides. A catalog verification records the
/// dump's complete ordered track set; the library records each disc image's
/// data-track digest. When a scanned file's digest and size equal one of a
/// carrier's verified tracks, that file *is* a derivative of that carrier, and
/// build evidence can say so. From then on it behaves like anything the
/// pipeline built: bound by path, re-adopted if it moves.
///
/// Conservative by construction: only current complete-track evidence counts,
/// a carrier that already has a current build is left alone, and a digest
/// matching more than one unbound file adopts none of them.
#[allow(clippy::too_many_lines)]
pub fn adopt_unbuilt_playables(
    request: &AdoptionRequest<'_>,
    conn: &retro_junk_db::Connection,
    progress: &PhaseProgressFn<'_>,
    cancelled: &AtomicBool,
) -> Result<AdoptionReport, ArchiveOpsError> {
    let AdoptionRequest {
        snapshot,
        playable_root,
        only_release,
        dry_run,
    } = *request;
    let mut report = AdoptionReport::default();
    let candidates = retro_junk_db::unbound_playable_rows(conn, &playable_root.to_string_lossy())
        .map_err(ArchiveOpsError::msg)?;
    if candidates.is_empty() {
        return Ok(report);
    }

    for release in &snapshot.releases {
        if only_release.is_some_and(|id| release.manifest.archive_release_id.to_string() != id) {
            continue;
        }
        let label = if release.manifest.region.is_empty() {
            release.manifest.title.clone()
        } else {
            format!("{} ({})", release.manifest.title, release.manifest.region)
        };
        for carrier in release
            .physical_copies
            .iter()
            .flat_map(|copy| &copy.carriers)
        {
            for dump in &carrier.dumps {
                if cancelled.load(Ordering::Relaxed) {
                    return Err(ArchiveOpsError::Cancelled);
                }
                // A carrier that already has a current derivative is not
                // missing one; if that derivative moved, adoption-by-digest is
                // the wrong repair for it.
                if !retro_junk_archive::current_build_evidence(dump).is_empty() {
                    continue;
                }
                let Some(tracks) = verified_track_digests(dump) else {
                    continue;
                };
                let matches = candidates
                    .iter()
                    .filter(|row| tracks.iter().any(|track| track_matches_row(track, row)))
                    .collect::<Vec<_>>();
                let [row] = matches.as_slice() else {
                    if matches.len() > 1 {
                        log::warn!(
                            "{label}: {} unbound playable files match one track set; adopting none",
                            matches.len()
                        );
                    }
                    continue;
                };
                report.orphaned += 1;
                progress(
                    &format!("Adopting {}", row.relative_path),
                    ProgressUnit::Items,
                    0,
                    0,
                );
                if dry_run {
                    report
                        .adopted
                        .push((label.clone(), String::new(), row.relative_path.clone()));
                    continue;
                }
                let path = playable_root.join(&row.relative_path);
                let (output_size, output_sha256) =
                    retro_junk_archive::sha256_file(&path, cancelled)
                        .map_err(ArchiveOpsError::msg)?;
                let evidence = retro_junk_archive::BuildEvidence {
                    schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
                    build_id: retro_junk_archive::BuildId::new(),
                    parent_representation_id: dump.manifest.representation_id,
                    child_representation_id: retro_junk_archive::RepresentationId::new(),
                    performed_at: chrono::Utc::now().to_rfc3339(),
                    input_manifest_sha256: dump.manifest_sha256.clone(),
                    recipe_version: 1,
                    format: playable_format(&row.relative_path),
                    relative_output_path: row.relative_path.clone(),
                    output_sha256,
                    output_size,
                    catalog_verified: true,
                    // Nothing here reproduced the disc from this file; the
                    // claim is "its data track is this carrier's", no more.
                    round_trip_verified: false,
                    tool: Some(retro_junk_archive::ToolRecord {
                        name: retro_junk_archive::ADOPTION_TOOL.to_owned(),
                        version: env!("CARGO_PKG_VERSION").to_owned(),
                        build: String::new(),
                    }),
                    omitted_features: Vec::new(),
                    canonical_intermediate: None,
                };
                retro_junk_archive::write_build_evidence(&dump.directory, &evidence)
                    .map_err(ArchiveOpsError::msg)?;
                log::info!(
                    "{label}: adopted unbuilt playable {} for dump {}",
                    row.relative_path,
                    dump.manifest.dump_id
                );
                report
                    .adopted
                    .push((label.clone(), String::new(), row.relative_path.clone()));
            }
        }
    }
    Ok(report)
}

/// One playable file proven to be the same bytes as one archived master.
pub struct IdenticalAdoption<'a> {
    pub dump: &'a retro_junk_archive::IndexedDump,
    /// The carrier the dump belongs to; what the library row is bound to.
    pub carrier: &'a retro_junk_archive::IndexedCarrier,
    pub platform_id: &'a str,
    /// Where the file sits under the playable root.
    pub relative_path: &'a str,
    /// Digests already computed for that file, which the caller has confirmed
    /// equal the master's.
    pub digests: &'a retro_junk_archive::FileDigests,
}

/// Record that a playable file is a derivative of an archived master it is
/// byte-identical to, and bind the library row to that carrier.
///
/// Two callers reach this with the same conclusion by different routes: the
/// adoption sweep, which found exactly one master with these bytes, and a
/// person resolving a review row where the sweep found several and would not
/// choose. Nothing about the recording differs between those, so it happens in
/// one place — otherwise a reviewed adoption could quietly produce weaker
/// evidence than an automatic one.
///
/// The evidence claims a round trip because being byte-identical to the master
/// *is* the round trip: this file reproduces it exactly, with nothing omitted.
pub fn adopt_identical_playable(
    adoption: &IdenticalAdoption<'_>,
    conn: &retro_junk_db::Connection,
) -> Result<retro_junk_archive::BuildEvidence, ArchiveOpsError> {
    let dump = adoption.dump;
    let evidence = retro_junk_archive::BuildEvidence {
        schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
        build_id: retro_junk_archive::BuildId::new(),
        parent_representation_id: dump.manifest.representation_id,
        child_representation_id: retro_junk_archive::RepresentationId::new(),
        performed_at: chrono::Utc::now().to_rfc3339(),
        input_manifest_sha256: dump.manifest_sha256.clone(),
        recipe_version: 1,
        format: dump.manifest.format.clone(),
        relative_output_path: adoption.relative_path.to_owned(),
        output_sha256: adoption.digests.sha256.clone(),
        output_size: adoption.digests.size,
        catalog_verified: retro_junk_archive::dump_catalog_verified(dump),
        round_trip_verified: true,
        tool: None,
        omitted_features: Vec::new(),
        canonical_intermediate: None,
    };
    retro_junk_archive::write_build_evidence(&dump.directory, &evidence)
        .map_err(ArchiveOpsError::msg)?;
    let carrier_id = adoption.carrier.manifest.carrier_id.to_string();
    retro_junk_db::bind_library_entries_by_hash(
        conn,
        adoption.platform_id,
        adoption.digests,
        &retro_junk_db::LibraryEntryBinding {
            // The adopted file is byte-identical to this carrier's master, so
            // it belongs to the carrier whether or not the carrier is
            // catalog-bound. Its catalog medium comes from the carrier row
            // rather than from the manifest here: the manifest's medium id was
            // minted against whichever DAT version archived the carrier, which
            // is often not one this catalog ever imported.
            carrier_id: Some(&carrier_id),
            match_method: "archive_adoption",
            ..Default::default()
        },
    )
    .map_err(ArchiveOpsError::msg)?;
    Ok(evidence)
}

/// The complete verified track set of a dump's current catalog evidence.
fn verified_track_digests(
    dump: &retro_junk_archive::IndexedDump,
) -> Option<Vec<retro_junk_archive::TrackVerification>> {
    dump.verifications.iter().find_map(|verification| {
        let evidence = &verification.evidence;
        let current = evidence.kind == retro_junk_archive::VerificationKind::Catalog
            && evidence.outcome == retro_junk_archive::VerificationOutcome::Verified
            && evidence.input_manifest_sha256 == dump.manifest_sha256
            && evidence
                .catalog
                .as_ref()
                .is_some_and(|catalog| catalog.complete_track_set)
            && !evidence.tracks.is_empty()
            && evidence.tracks.iter().all(|track| track.matched);
        current.then(|| evidence.tracks.clone())
    })
}

/// Whether a scanned library row is this verified track.
///
/// Size *and* SHA-1 both, because a disc image's data track is the only thing
/// the library hashes for a container: size alone is a coincidence, and SHA-1
/// alone would ignore that the row might describe a different representation.
fn track_matches_row(
    track: &retro_junk_archive::TrackVerification,
    row: &retro_junk_db::UnboundPlayableRow,
) -> bool {
    !track.actual_sha1.is_empty()
        && !row.sha1.is_empty()
        && track.size == row.data_size
        && track.actual_sha1.eq_ignore_ascii_case(&row.sha1)
}

/// The representation format a playable file's extension names.
fn playable_format(relative_path: &str) -> RepresentationFormat {
    Path::new(relative_path)
        .extension()
        .and_then(|value| value.to_str())
        .and_then(|value| value.parse().ok())
        .unwrap_or(RepresentationFormat::Rom)
}

/// Write an ordered `.m3u` playlist over already-present disc files.
/// Idempotent: an existing playlist is current, not an error.
pub fn write_release_playlist(
    playable_root: &Path,
    playable_platform_id: &str,
    title: &str,
    region: &str,
    canonical_release_name: &str,
    files: &[PathBuf],
) -> Result<PathBuf, ArchiveOpsError> {
    let display_name = if region.is_empty() {
        title.to_owned()
    } else {
        format!("{title} ({region})")
    };
    let stem = if canonical_release_name.trim().is_empty() {
        display_name
    } else {
        canonical_release_name.to_owned()
    };
    let directory = playable_root
        .join(retro_junk_archive::slugify(playable_platform_id))
        .join(format!("{stem}.m3u"));
    std::fs::create_dir_all(&directory)?;
    let playlist = directory.join(format!("{stem}.m3u"));
    if playlist.is_file() {
        return Ok(playlist);
    }
    let contents = files
        .iter()
        .map(|file| {
            pathdiff::diff_paths(file, &directory)
                .unwrap_or_else(|| file.clone())
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let temporary = directory.join(format!(
        ".playlist-{}.tmp",
        retro_junk_archive::BuildId::new()
    ));
    if let Err(error) =
        std::fs::write(&temporary, contents).and_then(|()| std::fs::rename(&temporary, &playlist))
    {
        let _ = std::fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(playlist)
}

// ── Shared helpers ─────────────────────────────────────────────────────────

/// Catalog media row plus its expected track digests, falling back to the
/// single-file digest row for trackless media. The one definition shared by
/// prerequisite verification everywhere.
pub fn expected_tracks_for_media(
    conn: &retro_junk_db::Connection,
    media_id: &str,
) -> Result<(retro_junk_db::Media, Vec<TrackDigest>), ArchiveOpsError> {
    let media = retro_junk_db::get_media_by_id(conn, media_id)
        .map_err(ArchiveOpsError::msg)?
        .ok_or_else(|| {
            ArchiveOpsError::Message(format!("Catalog medium {media_id} was not found"))
        })?;
    let mut tracks = retro_junk_db::find_media_tracks(conn, media_id)
        .map_err(ArchiveOpsError::msg)?
        .into_iter()
        .map(|track| TrackDigest {
            number: u32::try_from(track.track_number).unwrap_or(0),
            size: u64::try_from(track.file_size).unwrap_or(0),
            crc32: track.crc32,
            md5: track.md5,
            sha1: track.sha1,
        })
        .collect::<Vec<_>>();
    if tracks.is_empty() && media.file_size > 0 {
        tracks.push(TrackDigest {
            number: 1,
            size: u64::try_from(media.file_size).unwrap_or(0),
            crc32: media.crc32.clone(),
            md5: media.md5.clone(),
            sha1: media.sha1.clone(),
        });
    }
    Ok((media, tracks))
}

fn all_dumps(
    snapshot: &ArchiveIndexSnapshot,
) -> impl Iterator<Item = (&IndexedRelease, &IndexedCarrier, &IndexedDump)> {
    snapshot.releases.iter().flat_map(|release| {
        release.physical_copies.iter().flat_map(move |copy| {
            copy.carriers.iter().flat_map(move |carrier| {
                carrier
                    .dumps
                    .iter()
                    .map(move |dump| (release, carrier, dump))
            })
        })
    })
}

fn disc_label(sequence_number: u32, fallback: &str) -> String {
    if sequence_number > 0 {
        format!("Disc {sequence_number}")
    } else {
        fallback.to_owned()
    }
}

/// `complete_track_set` is the caller's claim, because only the caller knows
/// what kind of match just happened: a whole-ordered-track-set match covers
/// the medium by construction, while a single-file match covers it only when
/// the catalog stores the medium as one file too.
fn catalog_evidence(
    catalog_match: &retro_junk_db::CompleteCatalogMediaMatch,
    platform_id: &str,
    complete_track_set: bool,
) -> CatalogEvidence {
    CatalogEvidence {
        source: catalog_match.source.clone(),
        system: platform_id.to_owned(),
        version: catalog_match.source_version.clone(),
        game: catalog_match.game.clone(),
        complete_track_set,
    }
}

fn bind_carrier(
    conn: &retro_junk_db::Connection,
    release: &IndexedRelease,
    carrier: &IndexedCarrier,
    catalog_match: &retro_junk_db::CompleteCatalogMediaMatch,
    expected_tracks: &[TrackDigest],
) -> Result<(), ArchiveOpsError> {
    let binding = CatalogBinding {
        source: catalog_match.source.clone(),
        dat_name: catalog_match.game.clone(),
        source_version: catalog_match.source_version.clone(),
        serials: if catalog_match.serial.is_empty() {
            Vec::new()
        } else {
            vec![catalog_match.serial.clone()]
        },
        expected_tracks: expected_tracks.to_vec(),
    };
    let spans_masterings = release_spans_masterings(conn, release, carrier, catalog_match)?;
    retro_junk_archive::bind_carrier_to_catalog(
        &release.directory.join("release.toml"),
        &carrier.directory.join("carrier.toml"),
        &binding,
        spans_masterings,
    )
    .map(|_| ())
    .map_err(ArchiveOpsError::msg)
}

/// Do this archive release's other carriers come from a different catalog
/// release than the one just matched?
///
/// Discs of one set share a catalog release; a boxed set assembled from two
/// pressings does not. Only the catalog can tell those apart, so this asks it:
/// each sibling's recorded track set goes back through the same complete-track
/// rule that identified it, and the release its medium belongs to is compared.
/// A sibling with no recorded track set — a cartridge, or one never identified
/// — says nothing either way and is passed over.
fn release_spans_masterings(
    conn: &retro_junk_db::Connection,
    release: &IndexedRelease,
    binding_carrier: &IndexedCarrier,
    catalog_match: &retro_junk_db::CompleteCatalogMediaMatch,
) -> Result<bool, ArchiveOpsError> {
    for copy in &release.physical_copies {
        for sibling in &copy.carriers {
            if sibling.manifest.carrier_id == binding_carrier.manifest.carrier_id {
                continue;
            }
            let tracks = &sibling.manifest.catalog_binding.expected_tracks;
            if tracks.is_empty() {
                continue;
            }
            let matches = retro_junk_db::match_complete_catalog_media(
                conn,
                &release.manifest.platform_id,
                tracks,
            )
            .map_err(ArchiveOpsError::msg)?;
            if let [sibling_match] = matches.as_slice()
                && sibling_match.release_id != catalog_match.release_id
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn append_reproduction_evidence(
    dump: &IndexedDump,
    audit: &retro_junk_archive::RedumperAudit,
) -> Result<(), ArchiveOpsError> {
    append_evidence(
        dump,
        &VerificationEvidence {
            schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
            verification_id: VerificationId::new(),
            representation_id: dump.manifest.representation_id,
            performed_at: chrono::Utc::now().to_rfc3339(),
            input_manifest_sha256: dump.manifest_sha256.clone(),
            kind: VerificationKind::Reproduction,
            outcome: VerificationOutcome::Verified,
            tool: Some(audit.tool.clone()),
            catalog: None,
            tracks: audit
                .tracks
                .iter()
                .map(|track| TrackVerification {
                    number: track.number,
                    size: track.size,
                    expected_sha1: String::new(),
                    actual_sha1: track.sha1.clone(),
                    matched: false,
                })
                .collect(),
            detail: "Redumper regenerated and hashed a complete track set from the raw master"
                .to_owned(),
        },
    )
}

/// Catalog-verify a master that is already stored as separate tracks.
///
/// A cue/bin master needs no reproduction: its tracks are files, and the dump
/// manifest recorded their digests at ingest. Ordering comes from the cue
/// sheet, which is the only authority on it — sorting by filename puts track
/// 10 before track 2, and a track set in the wrong order simply fails to
/// match, which would read as "not in the catalog".
fn verify_track_set(
    dump: &IndexedDump,
    release: &IndexedRelease,
    carrier: &IndexedCarrier,
    conn: &retro_junk_db::Connection,
    report: &mut IdentifyReport,
) -> Result<(), ArchiveOpsError> {
    let Some(tracks) = ordered_track_digests(dump) else {
        report.unmatched += 1;
        return append_evidence(
            dump,
            &VerificationEvidence {
                schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
                verification_id: VerificationId::new(),
                representation_id: dump.manifest.representation_id,
                performed_at: chrono::Utc::now().to_rfc3339(),
                input_manifest_sha256: dump.manifest_sha256.clone(),
                kind: VerificationKind::Catalog,
                outcome: VerificationOutcome::Unmatched,
                tool: None,
                catalog: None,
                tracks: Vec::new(),
                detail: "Multi-file master has no readable cue sheet to order its tracks"
                    .to_owned(),
            },
        );
    };
    let matches =
        retro_junk_db::match_complete_catalog_media(conn, &release.manifest.platform_id, &tracks)
            .map_err(ArchiveOpsError::msg)?;
    let (outcome, catalog, detail) = match matches.as_slice() {
        [catalog_match] => {
            bind_carrier(conn, release, carrier, catalog_match, &tracks)?;
            report.identified += 1;
            (
                VerificationOutcome::Verified,
                // The whole ordered set matched, so this is a complete match
                // in the sense the evidence flag means.
                Some(catalog_evidence(
                    catalog_match,
                    &release.manifest.platform_id,
                    true,
                )),
                format!(
                    "Complete track set matched catalog media {}",
                    catalog_match.media_id
                ),
            )
        }
        [] => {
            report.unmatched += 1;
            (
                VerificationOutcome::Unmatched,
                None,
                "No catalog medium matched the stored track set".to_owned(),
            )
        }
        _ => {
            report.ambiguous += 1;
            (
                VerificationOutcome::Ambiguous,
                None,
                format!("Stored track set matched {} catalog media", matches.len()),
            )
        }
    };
    let unique = matches.len() == 1;
    append_evidence(
        dump,
        &VerificationEvidence {
            schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
            verification_id: VerificationId::new(),
            representation_id: dump.manifest.representation_id,
            performed_at: chrono::Utc::now().to_rfc3339(),
            input_manifest_sha256: dump.manifest_sha256.clone(),
            kind: VerificationKind::Catalog,
            outcome,
            tool: None,
            catalog,
            tracks: tracks
                .iter()
                .map(|track| TrackVerification {
                    number: track.number,
                    size: track.size,
                    expected_sha1: if unique {
                        track.sha1.clone()
                    } else {
                        String::new()
                    },
                    actual_sha1: track.sha1.clone(),
                    matched: unique,
                })
                .collect(),
            detail,
        },
    )
}

/// The dump's tracks in cue order, with the digests recorded at ingest.
///
/// Returns nothing when the master has no cue sheet, or when the cue names a
/// file the dump does not contain — an unordered guess is worse than an
/// honest "could not tell".
fn ordered_track_digests(dump: &IndexedDump) -> Option<Vec<retro_junk_archive::TrackDigest>> {
    let cue = dump
        .manifest
        .files
        .iter()
        .find(|file| file.path.to_ascii_lowercase().ends_with(".cue"))?;
    let contents = std::fs::read_to_string(dump.directory.join("raw").join(&cue.path)).ok()?;
    let sheet = retro_junk_disc::cue::parse_cue(&contents).ok()?;
    let mut tracks = Vec::new();
    for entry in &sheet.files {
        let wanted = entry.filename.to_ascii_lowercase();
        let file = dump.manifest.files.iter().find(|file| {
            file.path.to_ascii_lowercase().rsplit('/').next() == Some(wanted.as_str())
        })?;
        tracks.push(retro_junk_archive::TrackDigest {
            number: entry
                .tracks
                .first()
                .map_or(0, |track| u32::from(track.number)),
            size: file.size,
            crc32: file.crc32.clone(),
            md5: file.md5.clone(),
            sha1: file.sha1.clone(),
        });
    }
    (!tracks.is_empty()).then_some(tracks)
}

fn append_evidence(
    dump: &IndexedDump,
    evidence: &VerificationEvidence,
) -> Result<(), ArchiveOpsError> {
    let evidence_directory = dump.directory.join("evidence");
    std::fs::create_dir_all(&evidence_directory)?;
    write_json_new(
        &evidence_directory.join(format!("verification-{}.json", evidence.verification_id)),
        evidence,
    )
    .map_err(ArchiveOpsError::msg)
}

#[cfg(test)]
#[path = "tests/archive_ops_tests.rs"]
mod tests;
