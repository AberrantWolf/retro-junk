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
//! `progress: &dyn Fn(&str, u64, u64)` + `cancelled: &AtomicBool`.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use retro_junk_archive::{
    ArchiveIndexSnapshot, CatalogBinding, CatalogEvidence, IndexedCarrier, IndexedDump,
    IndexedRelease, RepresentationFormat, TrackDigest, TrackVerification, VerificationEvidence,
    VerificationId, VerificationKind, VerificationOutcome, write_json_new,
};

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
    progress: &dyn Fn(&str, u64, u64),
    cancelled: &AtomicBool,
) -> Result<IntegrityRunReport, ArchiveOpsError> {
    let dumps = all_dumps(snapshot)
        .filter(|(_, _, dump)| only_dump.is_none_or(|id| dump.manifest.dump_id.to_string() == id))
        .map(|(_, _, dump)| dump)
        .collect::<Vec<_>>();
    let total = dumps.len() as u64;
    let mut report = IntegrityRunReport::default();
    progress("Verifying stored bytes", 0, total);
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
        progress("Verifying stored bytes", (index + 1) as u64, total);
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
    progress: &dyn Fn(&str, u64, u64),
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
                                    IdentifySelection::StaleOnly => {
                                        carrier.manifest.catalog_binding.catalog_media_id.is_empty()
                                            || !retro_junk_archive::dump_catalog_verified(dump)
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
    let redumper =
        retro_junk_archive::Redumper::detect(redumper_path).map_err(ArchiveOpsError::msg)?;
    let total = candidates.len() as u64;
    progress("Identifying archived carriers", 0, total);
    for (index, (release, carrier, dump)) in candidates.into_iter().enumerate() {
        if cancelled.load(Ordering::Relaxed) {
            return Err(ArchiveOpsError::Cancelled);
        }
        match redumper.audit(&dump.directory.join("raw"), workspace_root, cancelled) {
            Ok(audit) => {
                append_reproduction_evidence(dump, &audit)?;
                let matches = retro_junk_db::match_complete_catalog_media(
                    conn,
                    &release.manifest.platform_id,
                    &audit.tracks,
                )
                .map_err(ArchiveOpsError::msg)?;
                let unique = matches.len() == 1;
                let (outcome, catalog, detail) = match matches.as_slice() {
                    [catalog_match] => {
                        bind_carrier(release, carrier, catalog_match, &audit.tracks)?;
                        report.identified += 1;
                        (
                            VerificationOutcome::Verified,
                            Some(catalog_evidence(
                                catalog_match,
                                &release.manifest.platform_id,
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
        progress("Identifying archived carriers", (index + 1) as u64, total);
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
    progress: &dyn Fn(&str, u64, u64),
    cancelled: &AtomicBool,
) -> Result<IdentifyReport, ArchiveOpsError> {
    let candidates = all_dumps(snapshot)
        .filter(|(_, _, dump)| only_dump.is_none_or(|id| dump.manifest.dump_id.to_string() == id))
        .filter(|(_, _, dump)| dump.manifest.files.len() == 1)
        .collect::<Vec<_>>();
    let mut report = IdentifyReport {
        selected: candidates.len(),
        ..IdentifyReport::default()
    };
    let total = candidates.len() as u64;
    progress("Catalog-verifying file masters", 0, total);
    for (index, (release, carrier, dump)) in candidates.into_iter().enumerate() {
        if cancelled.load(Ordering::Relaxed) {
            return Err(ArchiveOpsError::Cancelled);
        }
        let [file] = dump.manifest.files.as_slice() else {
            unreachable!("filtered to single-file dumps");
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
                bind_carrier(release, carrier, catalog_match, &[])?;
                report.identified += 1;
                (
                    VerificationOutcome::Verified,
                    Some(catalog_evidence(
                        catalog_match,
                        &release.manifest.platform_id,
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
        progress("Catalog-verifying file masters", (index + 1) as u64, total);
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
    progress: &dyn Fn(&str, u64, u64),
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
            &|description, current, total| {
                progress(&format!("{disc_label}: {description}"), current, total);
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
    let mut catalog_game_names = Vec::new();
    for carrier in &gap.carriers {
        let Some(media_id) = carrier.catalog_media_id.as_deref() else {
            continue;
        };
        let Some(media) =
            retro_junk_db::get_media_by_id(conn, media_id).map_err(ArchiveOpsError::msg)?
        else {
            continue;
        };
        catalog_game_names.push(media.dat_name.clone());
        let canonical_source = if media.rom_name.trim().is_empty() {
            media.dat_name.as_str()
        } else {
            media.rom_name.as_str()
        };
        let stem = Path::new(canonical_source)
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or(canonical_source)
            .to_owned();
        canonical_names.insert(carrier.carrier_id.clone(), stem);
    }
    let game_name_refs = catalog_game_names
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let canonical_release_name = retro_junk_core::disc::derive_base_game_name(&game_name_refs);

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
                expected_disc_count: gap.expected_disc_count,
                canonical_output_stem: canonical_names
                    .get(&carrier.carrier_id)
                    .cloned()
                    .unwrap_or_default(),
                canonical_release_name: canonical_release_name.clone(),
            },
            &|description, current, total| {
                progress(&format!("{disc_label}: {description}"), current, total);
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

fn catalog_evidence(
    catalog_match: &retro_junk_db::CompleteCatalogMediaMatch,
    platform_id: &str,
) -> CatalogEvidence {
    CatalogEvidence {
        source: catalog_match.source.clone(),
        system: platform_id.to_owned(),
        version: catalog_match.source_version.clone(),
        game: catalog_match.game.clone(),
        complete_track_set: true,
    }
}

fn bind_carrier(
    release: &IndexedRelease,
    carrier: &IndexedCarrier,
    catalog_match: &retro_junk_db::CompleteCatalogMediaMatch,
    expected_tracks: &[TrackDigest],
) -> Result<(), ArchiveOpsError> {
    let binding = CatalogBinding {
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
        expected_tracks: expected_tracks.to_vec(),
    };
    retro_junk_archive::bind_carrier_to_catalog(
        &release.directory.join("release.toml"),
        &carrier.directory.join("carrier.toml"),
        &binding,
    )
    .map(|_| ())
    .map_err(ArchiveOpsError::msg)
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
