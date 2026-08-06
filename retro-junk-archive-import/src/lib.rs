//! Catalog-driven discovery and transactional import of preservation dumps.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use retro_junk_archive::{
    ArchivedFile, CarrierKind, CatalogBinding, DumpId, FileDigests, NewCarrierDump, PhysicalCopyId,
    RepresentationFormat, SourcePackageRecord, TrackDigest,
};
use retro_junk_core::AnalysisOptions;
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("import source is not a regular file or directory: {0}")]
    InvalidSource(String),
    #[error("invalid dump package: {0}")]
    InvalidPackage(String),
    #[error("symbolic links are not accepted in dump packages: {0}")]
    SymbolicLink(String),
    #[error("CUE sheet references a file outside its dump package: {0}")]
    InvalidCueReference(String),
    #[error("import was cancelled")]
    Cancelled,
    #[error("catalog database is unavailable: {0}")]
    Catalog(String),
    #[error("archive error: {0}")]
    Archive(String),
    #[error(transparent)]
    Stage(#[from] retro_junk_io::StageError),
    #[error("I/O error for {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
}

#[derive(Debug, Clone)]
pub struct DumpImportRequest {
    pub source: PathBuf,
    pub archive_root: PathBuf,
    pub platform_hint: Option<String>,
    pub owner_id: String,
    pub new_physical_copy: bool,
    pub redumper_path: Option<PathBuf>,
    pub workspace_root: Option<PathBuf>,
    /// Copy packages to the device-local workspace before analysis. When
    /// false, calculate inventory hashes and perform analysis against the
    /// original source paths.
    pub stage_packages_locally: bool,
    /// When set, imported byte-identical files are also recorded as existing
    /// playable representations relative to this root.
    pub playable_root: Option<PathBuf>,
    /// Build a verified CHD immediately after publishing each preservation master.
    pub make_playable: bool,
    pub chdman_path: Option<PathBuf>,
    /// User authorization to omit only a verified CUE and its referenced tracks.
    pub discard_redundant_bin_cue: bool,
}

#[derive(Debug, Clone)]
pub struct DumpImportPlan {
    pub request: DumpImportRequest,
    pub candidates: Vec<DumpImportCandidate>,
    pub total_source_bytes: u64,
    _staging_leases: Vec<retro_junk_io::StagingLease>,
}

#[derive(Debug, Clone)]
pub struct DumpImportCandidate {
    pub source: PathBuf,
    /// Device-local package used for repeated identification and archive-copy
    /// reads. `source` remains the provenance and optional consume target.
    pub staged_source: PathBuf,
    pub package: SourcePackageInventory,
    pub format: RepresentationFormat,
    pub carrier_kind: CarrierKind,
    /// Physical platform used for archive layout. This may be more specific
    /// than the shared catalog platform (for example `famicom` vs `nes`).
    pub archive_platform_id: String,
    pub identification: IdentificationResolution,
    pub disposition: ImportDisposition,
    pub selected_match: Option<CatalogCandidate>,
    pub physical_copy_id: Option<PhysicalCopyId>,
    /// The archive release this dump joins, when planning found one it belongs
    /// under. Deciding that two pressings of a game are one owned thing needs
    /// the catalog, which only planning can consult.
    pub join_release: Option<retro_junk_archive::ArchiveReleaseId>,
    pub verification_tracks: Vec<TrackDigest>,
    pub verification_tool: Option<retro_junk_archive::ToolRecord>,
    pub verification_detail: String,
    pub warnings: Vec<String>,
    pub intermediate_source: Option<PlayableIntermediateSource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayableIntermediateSource {
    SuppliedCueBin,
    RedumperSplit,
}

#[derive(Debug, Clone)]
pub struct SourcePackageInventory {
    pub files: Vec<InventoryFile>,
    pub total_bytes: u64,
    pub package_sha256: String,
    pub observed_captured_at: String,
    pub timestamp_source: String,
}

#[derive(Debug, Clone)]
pub struct InventoryFile {
    pub relative_path: String,
    pub digests: FileDigests,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentificationResolution {
    CatalogVerified { method: IdentificationMethod },
    Identified { method: IdentificationMethod },
    Ambiguous,
    Unresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentificationMethod {
    CompleteTrackSet,
    ExactFileHash,
    FormatAwareFileHash,
    HeaderSerial,
    FolderSerial,
    UserSelection,
    RedumperLog,
}

#[derive(Debug, Clone)]
pub enum ImportDisposition {
    Ready,
    ReadyUnbound { title: String, platform_id: String },
    AlreadyArchived { dump_id: DumpId, directory: PathBuf },
    NeedsCatalogChoice { candidates: Vec<CatalogCandidate> },
    NeedsPhysicalCopyChoice { copies: Vec<PhysicalCopyCandidate> },
    Unresolved { reason: String },
    Invalid { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogCandidate {
    pub media_id: String,
    pub release_id: String,
    pub work_id: String,
    pub title: String,
    pub platform_id: String,
    pub region: String,
    pub revision: String,
    pub variant: String,
    pub serial: String,
    pub sequence_number: u32,
    /// The release's total numbered-disc count per the catalog, at least 1.
    /// Zero until plan finalization fills it in; consumers clamp with
    /// `.max(1)` so an unfilled value degrades to single-disc behavior.
    pub release_disc_count: u32,
    pub source: String,
    pub source_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhysicalCopyCandidate {
    pub physical_copy_id: PhysicalCopyId,
    pub copy_number: u32,
    pub label: String,
}

#[derive(Debug, Clone, Copy)]
pub struct ImportProgress {
    pub completed_candidates: u64,
    pub total_candidates: u64,
    pub copied_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningProgressKind {
    Indeterminate,
    Bytes,
    Items,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningProgress {
    pub description: String,
    pub kind: PlanningProgressKind,
    pub current: u64,
    pub total: u64,
}

/// Translate a shared progress report into this module's display kind.
///
/// The reporting operation says what its numbers count; a total of zero still
/// means "no measurable extent", whatever the unit, so it shows as a busy
/// indicator rather than a proportion of nothing.
#[must_use]
pub fn progress_kind(unit: retro_junk_io::ProgressUnit, total: u64) -> PlanningProgressKind {
    match (unit, total) {
        (_, 0) => PlanningProgressKind::Indeterminate,
        (retro_junk_io::ProgressUnit::Bytes, _) => PlanningProgressKind::Bytes,
        (retro_junk_io::ProgressUnit::Items, _) => PlanningProgressKind::Items,
    }
}

#[derive(Debug, Clone)]
pub struct DumpImportBatchResult {
    pub results: Vec<CandidateImportResult>,
}

#[derive(Debug, Clone)]
pub struct CandidateImportResult {
    pub source: PathBuf,
    pub outcome: CandidateImportOutcome,
    pub source_removed: bool,
    pub detail: String,
    pub warnings: Vec<String>,
    pub playable_build: Option<PlayableBuildResult>,
}

#[derive(Debug, Clone)]
pub struct PlayableBuildResult {
    pub outcome: PlayableBuildOutcome,
    pub output: Option<PathBuf>,
    pub detail: String,
    pub intermediate_source: Option<PlayableIntermediateSource>,
    pub authorized_exclusions: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayableBuildOutcome {
    Created,
    Failed,
    NotRequested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateImportOutcome {
    Imported,
    AlreadyArchived,
    Skipped,
    Failed,
    Cancelled,
}

#[allow(clippy::too_many_lines)]
pub fn plan_import(
    request: DumpImportRequest,
    context: &retro_junk_lib::AnalysisContext,
    catalog: &retro_junk_db::Connection,
    cancel: &AtomicBool,
    on_progress: impl Fn(u64, u64),
    on_phase: impl Fn(PlanningProgress),
) -> Result<DumpImportPlan, ImportError> {
    on_phase(PlanningProgress {
        description: format!("Discovering dump packages in {}", request.source.display()),
        kind: PlanningProgressKind::Indeterminate,
        current: 0,
        total: 0,
    });
    let sources = discover_packages(&request.source, context)?;
    on_phase(PlanningProgress {
        description: "Reading the existing archive index".to_owned(),
        kind: PlanningProgressKind::Indeterminate,
        current: 0,
        total: 0,
    });
    let archive = retro_junk_archive::scan_archive(&request.archive_root)
        .map_err(|error| ImportError::Archive(error.to_string()))?;
    let source_count = sources.len() as u64;
    let mut staged_plans = Vec::with_capacity(sources.len());
    for (index, source) in sources.into_iter().enumerate() {
        check_cancel(cancel)?;
        on_phase(PlanningProgress {
            description: format!("Enumerating package files: {}", source.display()),
            kind: PlanningProgressKind::Items,
            current: index as u64,
            total: source_count,
        });
        let plan = retro_junk_io::plan_package(&source)?;
        staged_plans.push((source, plan));
    }
    let total_hint = staged_plans.iter().fold(0_u64, |total, (_, plan)| {
        total.saturating_add(plan.total_bytes)
    });
    let mut hashed = 0_u64;
    on_progress(0, total_hint);
    let mut candidates = Vec::with_capacity(staged_plans.len());
    let mut staging_leases = Vec::with_capacity(staged_plans.len());
    let workspace_root = request
        .workspace_root
        .clone()
        .unwrap_or_else(retro_junk_io::default_transient_workspace);
    on_phase(PlanningProgress {
        description: if request.stage_packages_locally {
            format!(
                "Copying packages to local workspace {} while calculating hashes",
                workspace_root.display()
            )
        } else {
            "Calculating package hashes in place".to_owned()
        },
        kind: PlanningProgressKind::Bytes,
        current: 0,
        total: total_hint,
    });
    let analysis_total = staged_plans.len() as u64;
    let report_analysis_complete = |completed| {
        on_phase(PlanningProgress {
            description: "Resolving package identities against the local catalog".to_owned(),
            kind: PlanningProgressKind::Items,
            current: completed,
            total: analysis_total,
        });
    };
    for (analysis_index, (source, staging_plan)) in staged_plans.into_iter().enumerate() {
        check_cancel(cancel)?;
        on_phase(PlanningProgress {
            description: if request.stage_packages_locally {
                format!("Copying package to local workspace: {}", source.display())
            } else {
                format!("Hashing package in place: {}", source.display())
            },
            kind: PlanningProgressKind::Bytes,
            current: hashed,
            total: total_hint,
        });
        let mut report_bytes = |bytes| {
            hashed = hashed.saturating_add(bytes);
            on_progress(hashed, total_hint);
        };
        let prepared = if request.stage_packages_locally {
            retro_junk_io::stage_planned_package(
                &staging_plan,
                &workspace_root,
                cancel,
                &mut report_bytes,
            )?
        } else {
            retro_junk_io::hash_planned_package_in_place(&staging_plan, cancel, &mut report_bytes)?
        };
        let staged_source = prepared.local_source.clone();
        let package = inventory_prepared(&prepared);
        staging_leases.push(prepared.lease().clone());
        on_phase(PlanningProgress {
            description: if request.stage_packages_locally {
                format!("Analyzing staged package: {}", source.display())
            } else {
                format!("Analyzing package in place: {}", source.display())
            },
            kind: PlanningProgressKind::Items,
            current: analysis_index as u64,
            total: analysis_total,
        });
        let format = detect_format(&source, &package);
        let carrier_kind = carrier_kind_for_format(&format);
        let inferred_platform = (!request.make_playable)
            .then(|| request.playable_root.as_deref())
            .flatten()
            .and_then(|root| infer_playable_platform(root, &source, context));
        let normalized_platform_hint = request.platform_hint.as_deref().map(catalog_platform_hint);
        let effective_platform_hint = normalized_platform_hint
            .as_deref()
            .or(inferred_platform.as_deref());
        let is_cartridge_platform = effective_platform_hint
            .and_then(|platform| context.get_by_short_name(platform))
            .is_some_and(|console| console.analyzer.chd_extensions().is_empty())
            || effective_platform_hint.is_some_and(is_known_cartridge_catalog_platform);
        if request.playable_root.is_some()
            && !request.make_playable
            && (!source.is_file()
                || !matches!(format, RepresentationFormat::Rom)
                || !is_cartridge_platform)
        {
            candidates.push(DumpImportCandidate {
                source,
                staged_source,
                package,
                format,
                carrier_kind,
                archive_platform_id: String::new(),
                identification: IdentificationResolution::Unresolved,
                join_release: None,
                disposition: ImportDisposition::Invalid {
                    reason: "playable promotion currently accepts only loose, archival-equivalent cartridge ROM files".to_owned(),
                },
                selected_match: None,
                physical_copy_id: None,
                verification_tracks: Vec::new(),
                verification_tool: None,
                verification_detail: String::new(),
                warnings: Vec::new(),
                intermediate_source: None,
            });
            report_analysis_complete((analysis_index + 1) as u64);
            continue;
        }
        if let Some((dump_id, directory)) = find_exact_duplicate(&archive, &package) {
            candidates.push(DumpImportCandidate {
                source,
                staged_source,
                package,
                format,
                carrier_kind,
                archive_platform_id: String::new(),
                identification: IdentificationResolution::Unresolved,
                join_release: None,
                disposition: ImportDisposition::AlreadyArchived { dump_id, directory },
                selected_match: None,
                physical_copy_id: None,
                verification_tracks: Vec::new(),
                verification_tool: None,
                verification_detail: String::new(),
                warnings: Vec::new(),
                intermediate_source: None,
            });
            report_analysis_complete((analysis_index + 1) as u64);
            continue;
        }

        let entrypoint = analysis_entrypoint(&staged_source, &package);
        let header = entrypoint
            .as_deref()
            .and_then(|path| analyze_header(path, context, effective_platform_hint));
        let exact = match exact_catalog_matches(
            &staged_source,
            &package,
            &format,
            &request,
            effective_platform_hint,
            context,
            catalog,
            cancel,
            &on_phase,
        ) {
            Ok(exact) => exact,
            Err(ImportError::Cancelled) => return Err(ImportError::Cancelled),
            Err(error) => {
                candidates.push(DumpImportCandidate {
                    source,
                    staged_source,
                    package,
                    format,
                    carrier_kind,
                    archive_platform_id: effective_platform_hint.unwrap_or_default().to_owned(),
                    identification: IdentificationResolution::Unresolved,
                    join_release: None,
                    disposition: ImportDisposition::Invalid {
                        reason: error.to_string(),
                    },
                    selected_match: None,
                    physical_copy_id: None,
                    verification_tracks: Vec::new(),
                    verification_tool: None,
                    verification_detail: String::new(),
                    warnings: vec![
                        "This package could not be analyzed; other packages were still planned"
                            .to_owned(),
                    ],
                    intermediate_source: None,
                });
                report_analysis_complete((analysis_index + 1) as u64);
                continue;
            }
        };
        let ExactCatalogMatch {
            matches: exact_matches,
            tracks: verification_tracks,
            tool: verification_tool,
            method: exact_method,
            detail: verification_detail,
            byte_verified,
        } = exact;
        let (mut matches, resolution) = if !exact_matches.is_empty() {
            (
                exact_matches,
                if byte_verified {
                    IdentificationResolution::CatalogVerified {
                        method: exact_method,
                    }
                } else {
                    IdentificationResolution::Identified {
                        method: exact_method,
                    }
                },
            )
        } else if let Some((_, serial)) = &header {
            (
                catalog_serial_matches(catalog, serial)?,
                IdentificationResolution::Identified {
                    method: IdentificationMethod::HeaderSerial,
                },
            )
        } else {
            let folder_serial = source
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            (
                catalog_serial_matches(catalog, folder_serial)?,
                IdentificationResolution::Identified {
                    method: IdentificationMethod::FolderSerial,
                },
            )
        };
        if let Some(hint) = effective_platform_hint {
            matches.retain(|candidate| candidate.platform_id.eq_ignore_ascii_case(hint));
        }
        if let Some((platform, _)) = header.as_ref() {
            let platform_matches = matches
                .iter()
                .filter(|candidate| candidate.platform_id.eq_ignore_ascii_case(platform))
                .cloned()
                .collect::<Vec<_>>();
            if !platform_matches.is_empty() {
                matches = platform_matches;
            }
        }
        deduplicate_matches(&mut matches);
        let catalog_cleanup_recommended =
            if matches!(resolution, IdentificationResolution::CatalogVerified { .. }) {
                group_equivalent_release_matches(&mut matches)
            } else {
                false
            };
        let (
            selected_match,
            disposition,
            identification,
            physical_copy_id,
            archive_platform_id,
            join_release,
        ) = match matches.as_slice() {
            [] => (
                None,
                ImportDisposition::Unresolved {
                    reason: "no catalog hash or serial match".to_owned(),
                },
                IdentificationResolution::Unresolved,
                None,
                String::new(),
                None,
            ),
            [selected] => {
                let archive_platform_id = physical_archive_platform(&request, &source, selected);
                let compatible_catalog_releases = if selected.work_id.is_empty() {
                    BTreeSet::new()
                } else {
                    retro_junk_db::releases_for_work(catalog, &selected.work_id)
                        .map_err(|error| ImportError::Catalog(error.to_string()))?
                        .into_iter()
                        .filter(|release| {
                            release.platform_id == selected.platform_id
                                && release.region == selected.region
                        })
                        .map(|release| release.id)
                        .collect::<BTreeSet<_>>()
                };
                let joined = archive_release_for(
                    &archive,
                    catalog,
                    &archive_platform_id,
                    selected,
                    &compatible_catalog_releases,
                )?;
                let join_release = joined.map(|release| release.manifest.archive_release_id);
                let copies = joined
                    .map(|release| {
                        release
                            .physical_copies
                            .iter()
                            .map(|copy| PhysicalCopyCandidate {
                                physical_copy_id: copy.manifest.physical_copy_id,
                                copy_number: copy.manifest.copy_number,
                                label: copy.manifest.label.clone(),
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                if !request.new_physical_copy && copies.len() > 1 {
                    (
                        Some(selected.clone()),
                        ImportDisposition::NeedsPhysicalCopyChoice { copies },
                        resolution,
                        None,
                        archive_platform_id,
                        join_release,
                    )
                } else {
                    (
                        Some(selected.clone()),
                        ImportDisposition::Ready,
                        resolution,
                        if request.new_physical_copy {
                            None
                        } else {
                            copies.first().map(|copy| copy.physical_copy_id)
                        },
                        archive_platform_id,
                        join_release,
                    )
                }
            }
            _ => (
                None,
                ImportDisposition::NeedsCatalogChoice {
                    candidates: matches,
                },
                IdentificationResolution::Ambiguous,
                None,
                String::new(),
                None,
            ),
        };
        let intermediate_source = if matches!(format, RepresentationFormat::RedumperRaw)
            && matches!(
                identification,
                IdentificationResolution::CatalogVerified { .. }
            ) {
            Some(if verification_tool.is_some() {
                PlayableIntermediateSource::RedumperSplit
            } else {
                PlayableIntermediateSource::SuppliedCueBin
            })
        } else {
            None
        };
        candidates.push(DumpImportCandidate {
            source,
            staged_source,
            package,
            format,
            carrier_kind,
            archive_platform_id,
            identification,
            disposition,
            selected_match,
            physical_copy_id,
            join_release,
            verification_tracks,
            verification_tool,
            verification_detail,
            warnings: catalog_cleanup_recommended
                .then(|| {
                    "Equivalent duplicate catalog rows resolved to one release; catalog cleanup recommended"
                        .to_owned()
                })
                .into_iter()
                .collect(),
            intermediate_source,
        });
        report_analysis_complete((analysis_index + 1) as u64);
    }
    // The plan is all `execute_import` gets — no catalog connection crosses
    // that boundary — so the release's total disc count is captured here.
    // Playable builds need the total (playlist layout, "(Disc N)" naming);
    // the medium's own sequence number only says which disc this one is.
    for candidate in &mut candidates {
        if let Some(selected) = candidate.selected_match.as_mut() {
            selected.release_disc_count =
                retro_junk_db::release_disc_count(catalog, &selected.release_id)
                    .map_err(|error| ImportError::Catalog(error.to_string()))?;
        }
    }
    let total_source_bytes = candidates
        .iter()
        .map(|candidate| candidate.package.total_bytes)
        .sum();
    Ok(DumpImportPlan {
        request,
        candidates,
        total_source_bytes,
        _staging_leases: staging_leases,
    })
}

#[allow(clippy::too_many_lines)]
pub fn execute_import(
    plan: DumpImportPlan,
    consume: bool,
    cancel: &AtomicBool,
    on_progress: impl Fn(ImportProgress),
    on_phase: impl Fn(PlanningProgress),
) -> Result<DumpImportBatchResult, ImportError> {
    on_phase(PlanningProgress {
        description: "Preparing the archive transaction".to_owned(),
        kind: PlanningProgressKind::Indeterminate,
        current: 0,
        total: 0,
    });
    let _lock = retro_junk_archive::ArchiveLock::acquire(&plan.request.archive_root)
        .map_err(|error| ImportError::Archive(error.to_string()))?;
    retro_junk_archive::upgrade_legacy_regional_physical_platforms(&plan.request.archive_root)
        .map_err(|error| ImportError::Archive(error.to_string()))?;
    let total_candidates = plan.candidates.len() as u64;
    let mut copied_bytes = 0_u64;
    let mut results = Vec::with_capacity(plan.candidates.len());
    let mut created_copies = BTreeMap::<String, Vec<(PhysicalCopyId, BTreeSet<u32>)>>::new();
    // The archive release each logical release (work, platform, region) landed
    // in during this run. Discs of one boxed set can come from different
    // masterings — different revisions, different catalog releases — and
    // nothing in a manifest says they belong together, so the run remembers.
    let mut created_releases = BTreeMap::<String, retro_junk_archive::ArchiveReleaseId>::new();
    let mut imported_packages =
        BTreeMap::<String, (PathBuf, retro_junk_archive::DumpManifest)>::new();
    for (index, candidate) in plan.candidates.into_iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            results.push(result(
                &candidate,
                CandidateImportOutcome::Cancelled,
                false,
                "cancelled",
            ));
            continue;
        }
        on_phase(PlanningProgress {
            description: format!(
                "Publishing archival package: {}",
                candidate.source.display()
            ),
            kind: PlanningProgressKind::Bytes,
            current: copied_bytes,
            total: plan.total_source_bytes,
        });
        let batch_key = batch_duplicate_key(&candidate);
        if let Some((directory, manifest)) = imported_packages.get(&batch_key) {
            append_playable_adoption(
                &plan.request,
                &candidate,
                directory,
                manifest,
                has_current_catalog_evidence(directory, manifest),
            )?;
            if consume {
                report_consume_verification(&on_phase, &candidate);
            }
            let removed = consume && verify_and_consume(&candidate, directory, manifest, cancel)?;
            results.push(result(
                &candidate,
                CandidateImportOutcome::AlreadyArchived,
                removed,
                "exact package was already imported earlier in this batch",
            ));
            on_progress(ImportProgress {
                completed_candidates: (index + 1) as u64,
                total_candidates,
                copied_bytes,
                total_bytes: plan.total_source_bytes,
            });
            on_phase(PlanningProgress {
                description: "Finalizing manifests and evidence".to_owned(),
                kind: PlanningProgressKind::Items,
                current: (index + 1) as u64,
                total: total_candidates,
            });
            continue;
        }
        match &candidate.disposition {
            ImportDisposition::Ready => {
                let selected = candidate
                    .selected_match
                    .as_ref()
                    .expect("ready import has match");
                let logical_release_key = catalog_candidate_release_key(selected);
                let physical_copy_id = candidate.physical_copy_id.or_else(|| {
                    (selected.sequence_number > 0)
                        .then(|| {
                            created_copies.get(&logical_release_key).and_then(|copies| {
                                copies
                                    .iter()
                                    .find(|(_, positions)| {
                                        !positions.contains(&selected.sequence_number)
                                    })
                                    .map(|(copy_id, _)| *copy_id)
                            })
                        })
                        .flatten()
                });
                let binding = CatalogBinding {
                    source: selected.source.clone(),
                    dat_name: selected.title.clone(),
                    source_version: selected.source_version.clone(),
                    serials: if selected.serial.is_empty() {
                        Vec::new()
                    } else {
                        vec![selected.serial.clone()]
                    },
                    expected_tracks: candidate.verification_tracks.clone(),
                };
                let source_package = source_record(&candidate.source, &candidate.package);
                let join_release = candidate
                    .join_release
                    .or_else(|| created_releases.get(&logical_release_key).copied());
                let spec = NewCarrierDump {
                    platform_id: if candidate.archive_platform_id.is_empty() {
                        selected.platform_id.clone()
                    } else {
                        candidate.archive_platform_id.clone()
                    },
                    title: selected.title.clone(),
                    region: selected.region.clone(),
                    revision: selected.revision.clone(),
                    variant: selected.variant.clone(),
                    owner_id: plan.request.owner_id.clone(),
                    physical_copy_label: String::new(),
                    serial: selected.serial.clone(),
                    sequence_number: selected.sequence_number,
                    carrier_label: String::new(),
                    carrier_kind: candidate.carrier_kind.clone(),
                    format: candidate.format.clone(),
                    catalog_binding: binding,
                    join_release,
                    source_package,
                    expected_files: candidate
                        .package
                        .files
                        .iter()
                        .map(|file| retro_junk_archive::ExpectedSourceFile {
                            relative_path: file.relative_path.clone(),
                            digests: file.digests.clone(),
                        })
                        .collect(),
                    physical_copy_id,
                };
                let imported = retro_junk_archive::ingest_new_carrier_dump(
                    &plan.request.archive_root,
                    &candidate.staged_source,
                    spec,
                    cancel,
                    |progress| {
                        on_progress(ImportProgress {
                            completed_candidates: index as u64,
                            total_candidates,
                            copied_bytes: copied_bytes.saturating_add(progress.copied_bytes),
                            total_bytes: plan.total_source_bytes,
                        });
                    },
                );
                match imported {
                    Ok(imported) => {
                        copied_bytes = copied_bytes.saturating_add(candidate.package.total_bytes);
                        created_releases
                            .entry(logical_release_key.clone())
                            .or_insert(imported.release.archive_release_id);
                        let copies = created_copies.entry(logical_release_key).or_default();
                        if let Some((_, positions)) = copies.iter_mut().find(|(copy_id, _)| {
                            *copy_id == imported.physical_copy.physical_copy_id
                        }) {
                            positions.insert(selected.sequence_number);
                        } else {
                            copies.push((
                                imported.physical_copy.physical_copy_id,
                                BTreeSet::from([selected.sequence_number]),
                            ));
                        }
                        if matches!(
                            candidate.identification,
                            IdentificationResolution::CatalogVerified { .. }
                        ) {
                            append_catalog_evidence(&candidate, selected, &imported)?;
                        }
                        append_playable_adoption(
                            &plan.request,
                            &candidate,
                            &imported.dump_directory,
                            &imported.dump,
                            matches!(
                                candidate.identification,
                                IdentificationResolution::CatalogVerified { .. }
                            ),
                        )?;
                        imported_packages.insert(
                            batch_key,
                            (imported.dump_directory.clone(), imported.dump.clone()),
                        );
                        if consume {
                            report_consume_verification(&on_phase, &candidate);
                        }
                        let removed = consume
                            && verify_and_consume(
                                &candidate,
                                &imported.dump_directory,
                                &imported.dump,
                                cancel,
                            )?;
                        let mut imported_result = result(
                            &candidate,
                            CandidateImportOutcome::Imported,
                            removed,
                            imported_verification_detail(&candidate.identification),
                        );
                        imported_result.playable_build = build_imported_playable(
                            &plan.request,
                            &candidate,
                            selected,
                            &imported.dump,
                            cancel,
                            &on_phase,
                        );
                        if imported_result
                            .playable_build
                            .as_ref()
                            .is_some_and(|build| build.outcome == PlayableBuildOutcome::Failed)
                        {
                            imported_result.warnings.push(
                                "archive import succeeded; retry playable CHD creation with `archive build-chd`"
                                    .to_owned(),
                            );
                        }
                        results.push(imported_result);
                    }
                    Err(error) => results.push(result(
                        &candidate,
                        CandidateImportOutcome::Failed,
                        false,
                        &error.to_string(),
                    )),
                }
            }
            ImportDisposition::ReadyUnbound { title, platform_id } => {
                if title.trim().is_empty() || platform_id.trim().is_empty() {
                    results.push(result(
                        &candidate,
                        CandidateImportOutcome::Skipped,
                        false,
                        "unbound imports require both a title and platform",
                    ));
                    continue;
                }
                let source_package = source_record(&candidate.source, &candidate.package);
                let spec = NewCarrierDump {
                    platform_id: platform_id.trim().to_owned(),
                    title: title.trim().to_owned(),
                    region: String::new(),
                    revision: String::new(),
                    variant: String::new(),
                    owner_id: plan.request.owner_id.clone(),
                    physical_copy_label: String::new(),
                    serial: String::new(),
                    sequence_number: 0,
                    carrier_label: String::new(),
                    carrier_kind: candidate.carrier_kind.clone(),
                    format: candidate.format.clone(),
                    catalog_binding: CatalogBinding::default(),
                    join_release: candidate.join_release,
                    source_package,
                    expected_files: candidate
                        .package
                        .files
                        .iter()
                        .map(|file| retro_junk_archive::ExpectedSourceFile {
                            relative_path: file.relative_path.clone(),
                            digests: file.digests.clone(),
                        })
                        .collect(),
                    physical_copy_id: None,
                };
                let imported = retro_junk_archive::ingest_new_carrier_dump(
                    &plan.request.archive_root,
                    &candidate.staged_source,
                    spec,
                    cancel,
                    |progress| {
                        on_progress(ImportProgress {
                            completed_candidates: index as u64,
                            total_candidates,
                            copied_bytes: copied_bytes.saturating_add(progress.copied_bytes),
                            total_bytes: plan.total_source_bytes,
                        });
                    },
                );
                match imported {
                    Ok(imported) => {
                        copied_bytes = copied_bytes.saturating_add(candidate.package.total_bytes);
                        append_playable_adoption(
                            &plan.request,
                            &candidate,
                            &imported.dump_directory,
                            &imported.dump,
                            false,
                        )?;
                        imported_packages.insert(
                            batch_key,
                            (imported.dump_directory.clone(), imported.dump.clone()),
                        );
                        if consume {
                            report_consume_verification(&on_phase, &candidate);
                        }
                        let removed = consume
                            && verify_and_consume(
                                &candidate,
                                &imported.dump_directory,
                                &imported.dump,
                                cancel,
                            )?;
                        results.push(result(
                            &candidate,
                            CandidateImportOutcome::Imported,
                            removed,
                            "imported; archive integrity verified, no catalog identity claimed",
                        ));
                    }
                    Err(error) => results.push(result(
                        &candidate,
                        CandidateImportOutcome::Failed,
                        false,
                        &error.to_string(),
                    )),
                }
            }
            ImportDisposition::AlreadyArchived { directory, .. } => {
                let manifest = retro_junk_archive::read_toml::<retro_junk_archive::DumpManifest>(
                    &directory.join("dump.toml"),
                )
                .map_err(|error| ImportError::Archive(error.to_string()))?;
                if consume {
                    report_consume_verification(&on_phase, &candidate);
                }
                let removed =
                    consume && verify_and_consume(&candidate, directory, &manifest, cancel)?;
                append_playable_adoption(
                    &plan.request,
                    &candidate,
                    directory,
                    &manifest,
                    has_current_catalog_evidence(directory, &manifest),
                )?;
                results.push(result(
                    &candidate,
                    CandidateImportOutcome::AlreadyArchived,
                    removed,
                    "exact package already archived",
                ));
            }
            _ => results.push(result(
                &candidate,
                CandidateImportOutcome::Skipped,
                false,
                disposition_reason(&candidate.disposition),
            )),
        }
        on_progress(ImportProgress {
            completed_candidates: (index + 1) as u64,
            total_candidates,
            copied_bytes,
            total_bytes: plan.total_source_bytes,
        });
        on_phase(PlanningProgress {
            description: "Finalizing manifests and evidence".to_owned(),
            kind: PlanningProgressKind::Items,
            current: (index + 1) as u64,
            total: total_candidates,
        });
    }
    Ok(DumpImportBatchResult { results })
}

fn report_consume_verification(
    on_phase: &impl Fn(PlanningProgress),
    candidate: &DumpImportCandidate,
) {
    on_phase(PlanningProgress {
        description: format!(
            "Verifying source and archived copy before removal: {}",
            candidate.source.display()
        ),
        kind: PlanningProgressKind::Indeterminate,
        current: 0,
        total: 0,
    });
}

fn discover_packages(
    source: &Path,
    context: &retro_junk_lib::AnalysisContext,
) -> Result<Vec<PathBuf>, ImportError> {
    let metadata = std::fs::symlink_metadata(source).map_err(|error| ImportError::Io {
        path: source.display().to_string(),
        source: error,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(ImportError::SymbolicLink(source.display().to_string()));
    }
    if metadata.is_file() {
        return Ok(vec![source.to_path_buf()]);
    }
    if !metadata.is_dir() {
        return Err(ImportError::InvalidSource(source.display().to_string()));
    }
    let source_is_platform_directory = source
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            context.matches_any_console(name) || catalog_platform_name(name).is_some()
        });
    if looks_like_package(source) && !source_is_platform_directory {
        validate_package_layout(source)?;
        return Ok(vec![source.to_path_buf()]);
    }
    let mut packages = Vec::new();
    for child in sorted_children(source)? {
        if child
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with('.'))
        {
            continue;
        }
        if child.is_file() && recognized_file(&child) {
            packages.push(child);
        } else if child.is_dir() {
            let name = child
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            if context.matches_any_console(name) || catalog_platform_name(name).is_some() {
                for grandchild in sorted_children(&child)? {
                    if (grandchild.is_dir() && looks_like_package(&grandchild))
                        || (grandchild.is_file() && recognized_file(&grandchild))
                    {
                        packages.push(grandchild);
                    }
                }
            } else if looks_like_package(&child) {
                packages.push(child);
            }
        }
    }
    if packages.is_empty() {
        return Err(ImportError::InvalidSource(source.display().to_string()));
    }
    packages.sort();
    for package in &packages {
        validate_package_layout(package)?;
    }
    Ok(packages)
}

fn validate_package_layout(package: &Path) -> Result<(), ImportError> {
    retro_junk_archive::validate_redumper_package(package)
        .map_err(|error| ImportError::InvalidPackage(error.to_string()))
}

fn infer_playable_platform(
    playable_root: &Path,
    source: &Path,
    context: &retro_junk_lib::AnalysisContext,
) -> Option<String> {
    let root_name = playable_root.file_name().and_then(|name| name.to_str());
    let folder = root_name
        .filter(|name| context.matches_any_console(name) || catalog_platform_name(name).is_some())
        .or_else(|| {
            source
                .strip_prefix(playable_root)
                .ok()
                .and_then(|relative| relative.components().next())
                .and_then(|component| component.as_os_str().to_str())
        })?;
    context
        .find_by_folder(folder)
        .first()
        .map(|console| console.metadata.short_name.to_owned())
        .or_else(|| catalog_platform_name(folder).map(str::to_owned))
}

fn catalog_platform_hint(platform: &str) -> String {
    catalog_platform_name(platform)
        .unwrap_or(platform)
        .to_owned()
}

/// Choose the physical platform used by the archive without changing the
/// shared catalog namespace used to verify the ROM.
#[must_use]
pub fn physical_archive_platform(
    request: &DumpImportRequest,
    source: &Path,
    selected: &CatalogCandidate,
) -> String {
    let catalog_platform = selected.platform_id.trim().to_ascii_lowercase();
    if !matches!(
        catalog_platform.as_str(),
        "nes" | "snes" | "genesis" | "pce" | "pcecd" | "saturn"
    ) {
        return selected.platform_id.clone();
    }
    if let Some(explicit) = request.platform_hint.as_deref()
        && let Some(platform) = named_physical_platform(&catalog_platform, explicit)
    {
        return platform.to_owned();
    }
    if let Some(platform) = source_platform_folder(request, source)
        .and_then(|folder| named_physical_platform(&catalog_platform, &folder))
        .filter(|platform| *platform != catalog_platform)
    {
        return platform.to_owned();
    }
    regional_physical_platform(&catalog_platform, &selected.region)
        .unwrap_or(catalog_platform.as_str())
        .to_owned()
}

fn source_platform_folder(request: &DumpImportRequest, source: &Path) -> Option<String> {
    let playable_root = request.playable_root.as_deref()?;
    let root_name = playable_root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if catalog_platform_name(root_name).is_some() {
        return Some(root_name.to_owned());
    }
    source
        .strip_prefix(playable_root)
        .ok()
        .and_then(|relative| relative.components().next())
        .and_then(|component| component.as_os_str().to_str())
        .map(str::to_owned)
}

fn catalog_platform_name(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "nes" | "famicom" | "fc" | "family computer" | "nintendo family computer" => Some("nes"),
        "snes"
        | "super nintendo"
        | "super nintendo entertainment system"
        | "sfc"
        | "super famicom"
        | "super-famicom" => Some("snes"),
        "genesis" | "sega genesis" | "md" | "mega drive" | "mega-drive" | "megadrive" => {
            Some("genesis")
        }
        "pce" | "pc engine" | "pc-engine" | "pcengine" | "tg16" | "tg-16" | "turbografx"
        | "turbografx-16" | "turbo grafx 16" => Some("pce"),
        "pcecd" | "pc engine cd" | "pcenginecd" | "tg-cd" | "turbografx-cd" => Some("pcecd"),
        "saturn" | "saturnjp" | "sega saturn" => Some("saturn"),
        _ => None,
    }
}

fn named_physical_platform(catalog_platform: &str, value: &str) -> Option<&'static str> {
    let value = value.trim().to_ascii_lowercase();
    match (catalog_platform, value.as_str()) {
        ("nes", "nes") => Some("nes"),
        ("nes", "famicom" | "fc" | "family computer" | "nintendo family computer") => {
            Some("famicom")
        }
        ("snes", "snes" | "super nintendo" | "super nintendo entertainment system") => Some("snes"),
        ("snes", "snesna") => Some("snesna"),
        ("snes", "sfc" | "super famicom" | "super-famicom") => Some("super-famicom"),
        ("pcecd", "pcecd" | "pc engine cd" | "pcenginecd") => Some("pcenginecd"),
        ("pcecd", "tg-cd" | "turbografx-cd") => Some("tg-cd"),
        ("genesis", "genesis" | "sega genesis") => Some("genesis"),
        ("genesis", "md" | "mega drive" | "mega-drive" | "megadrive") => Some("megadrive"),
        ("pce", "pce" | "pc engine" | "pc-engine" | "pcengine") => Some("pce"),
        ("pce", "tg16" | "tg-16" | "turbografx" | "turbografx-16" | "turbo grafx 16") => {
            Some("tg16")
        }
        ("saturn", "saturn" | "sega saturn") => Some("saturn"),
        ("saturn", "saturnjp") => Some("saturnjp"),
        _ => None,
    }
}

fn regional_physical_platform(catalog_platform: &str, region: &str) -> Option<&'static str> {
    // The regional mapping itself is the archive crate's, shared with the
    // legacy-directory migration so the two can never drift. This wrapper
    // adds only the import-time question: a platform whose family is known
    // but whose region needs no regional directory keeps its own name.
    retro_junk_archive::regional_physical_platform(catalog_platform, region).or(
        match catalog_platform {
            "nes" => Some("nes"),
            "snes" => Some("snes"),
            "genesis" => Some("genesis"),
            "pce" => Some("pce"),
            "saturn" => Some("saturn"),
            _ => None,
        },
    )
}

fn is_known_cartridge_catalog_platform(platform: &str) -> bool {
    matches!(platform, "nes" | "snes" | "genesis" | "pce")
}

fn looks_like_package(path: &Path) -> bool {
    sorted_children(path).is_ok_and(|children| {
        let recognized = children
            .iter()
            .filter(|child| child.is_file() && recognized_file(child))
            .collect::<Vec<_>>();
        recognized.len() == 1
            || recognized.iter().any(|child| {
                matches!(
                    child
                        .extension()
                        .and_then(|extension| extension.to_str())
                        .unwrap_or_default()
                        .to_ascii_lowercase()
                        .as_str(),
                    "cue" | "gdi" | "scram"
                )
            })
    })
}

fn recognized_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "cue"
            | "bin"
            | "iso"
            | "img"
            | "scram"
            | "sub"
            | "gdi"
            | "chd"
            | "rvz"
            | "nes"
            | "unf"
            | "unif"
            | "fds"
            | "sfc"
            | "smc"
            | "swc"
            | "fig"
            | "n64"
            | "z64"
            | "v64"
            | "gb"
            | "gbc"
            | "sgb"
            | "gba"
            | "mb"
            | "nds"
            | "dsi"
            | "sg"
            | "sc"
            | "sms"
            | "md"
            | "gen"
            | "smd"
            | "32x"
            | "pce"
            | "gg"
    )
}

fn sorted_children(path: &Path) -> Result<Vec<PathBuf>, ImportError> {
    let mut children = std::fs::read_dir(path)
        .map_err(|error| ImportError::Io {
            path: path.display().to_string(),
            source: error,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| ImportError::Io {
            path: path.display().to_string(),
            source: error,
        })?
        .into_iter()
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    children.sort();
    Ok(children)
}

fn inventory_package(
    source: &Path,
    cancel: &AtomicBool,
    mut on_bytes: impl FnMut(u64),
) -> Result<SourcePackageInventory, ImportError> {
    let paths = package_files(source)?;
    let mut files = Vec::with_capacity(paths.len());
    let mut total_bytes = 0_u64;
    for (path, relative_path) in paths {
        check_cancel(cancel)?;
        let digests = retro_junk_archive::hash_file_digests(&path, cancel)
            .map_err(|error| ImportError::Archive(error.to_string()))?;
        total_bytes = total_bytes.saturating_add(digests.size);
        on_bytes(digests.size);
        files.push(InventoryFile {
            relative_path,
            digests,
        });
    }
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    let package_sha256 = fingerprint_inventory(&files);
    let (observed_captured_at, timestamp_source) = find_observed_timestamp(source, &files);
    Ok(SourcePackageInventory {
        files,
        total_bytes,
        package_sha256,
        observed_captured_at,
        timestamp_source,
    })
}

fn inventory_prepared(prepared: &retro_junk_io::PreparedPackage) -> SourcePackageInventory {
    let files = prepared
        .files
        .iter()
        .map(|file| InventoryFile {
            relative_path: file.relative_path.clone(),
            digests: FileDigests {
                size: file.digests.size,
                crc32: file.digests.crc32.clone(),
                md5: file.digests.md5.clone(),
                sha1: file.digests.sha1.clone(),
                sha256: file.digests.sha256.clone(),
            },
        })
        .collect::<Vec<_>>();
    let package_sha256 = fingerprint_inventory(&files);
    let (observed_captured_at, timestamp_source) =
        find_observed_timestamp(&prepared.local_source, &files);
    SourcePackageInventory {
        files,
        total_bytes: prepared.total_bytes,
        package_sha256,
        observed_captured_at,
        timestamp_source,
    }
}

fn package_files(source: &Path) -> Result<Vec<(PathBuf, String)>, ImportError> {
    if source.is_file() {
        return Ok(vec![(
            source.to_path_buf(),
            source
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
        )]);
    }
    let mut output = Vec::new();
    walk_package_files(source, source, &mut output)?;
    Ok(output)
}

fn walk_package_files(
    root: &Path,
    directory: &Path,
    output: &mut Vec<(PathBuf, String)>,
) -> Result<(), ImportError> {
    for path in sorted_children(directory)? {
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| ImportError::Io {
            path: path.display().to_string(),
            source: error,
        })?;
        if metadata.file_type().is_symlink() {
            return Err(ImportError::SymbolicLink(path.display().to_string()));
        }
        if metadata.is_dir() {
            walk_package_files(root, &path, output)?;
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            output.push((path, relative));
        }
    }
    Ok(())
}

fn fingerprint_inventory(files: &[InventoryFile]) -> String {
    let mut hash = Sha256::new();
    for file in files {
        hash.update(file.relative_path.as_bytes());
        hash.update([0]);
        hash.update(file.digests.size.to_be_bytes());
        hash.update(file.digests.sha256.as_bytes());
        hash.update([0]);
    }
    format!("{:x}", hash.finalize())
}

fn fingerprint_archived(files: &[ArchivedFile]) -> String {
    let inventory = files
        .iter()
        .map(|file| InventoryFile {
            relative_path: file.path.clone(),
            digests: FileDigests {
                size: file.size,
                crc32: file.crc32.clone(),
                md5: file.md5.clone(),
                sha1: file.sha1.clone(),
                sha256: file.sha256.clone(),
            },
        })
        .collect::<Vec<_>>();
    fingerprint_inventory(&inventory)
}

fn find_exact_duplicate(
    archive: &retro_junk_archive::ArchiveIndexSnapshot,
    package: &SourcePackageInventory,
) -> Option<(DumpId, PathBuf)> {
    archive
        .releases
        .iter()
        .flat_map(|release| &release.physical_copies)
        .flat_map(|copy| &copy.carriers)
        .flat_map(|carrier| &carrier.dumps)
        .find(|dump| {
            let fingerprint = if dump.manifest.source_package.package_sha256.is_empty() {
                fingerprint_archived(&dump.manifest.files)
            } else {
                dump.manifest.source_package.package_sha256.clone()
            };
            fingerprint == package.package_sha256
                && (package.observed_captured_at.is_empty()
                    || dump.manifest.source_package.observed_captured_at
                        == package.observed_captured_at)
        })
        .map(|dump| (dump.manifest.dump_id, dump.directory.clone()))
}

fn detect_format(source: &Path, package: &SourcePackageInventory) -> RepresentationFormat {
    let extensions = package
        .files
        .iter()
        .filter_map(|file| {
            Path::new(&file.relative_path)
                .extension()
                .and_then(|value| value.to_str())
        })
        .map(str::to_ascii_lowercase)
        .collect::<BTreeSet<_>>();
    if extensions.contains("scram") {
        RepresentationFormat::RedumperRaw
    } else if extensions.contains("cue") {
        RepresentationFormat::CueBin
    } else if extensions.contains("iso") {
        RepresentationFormat::Iso
    } else if extensions.contains("chd") {
        RepresentationFormat::Chd
    } else if extensions.contains("rvz") {
        RepresentationFormat::Rvz
    } else if source.is_file() || package.files.len() == 1 {
        RepresentationFormat::Rom
    } else {
        RepresentationFormat::Other("raw-set".to_owned())
    }
}

fn carrier_kind_for_format(format: &RepresentationFormat) -> CarrierKind {
    match format {
        RepresentationFormat::RedumperRaw
        | RepresentationFormat::CueBin
        | RepresentationFormat::Iso
        | RepresentationFormat::Chd
        | RepresentationFormat::Rvz => CarrierKind::OpticalDisc,
        RepresentationFormat::Rom => CarrierKind::Cartridge,
        RepresentationFormat::Other(_) => CarrierKind::Unknown,
    }
}

fn analysis_entrypoint(source: &Path, package: &SourcePackageInventory) -> Option<PathBuf> {
    if source.is_file() {
        return Some(source.to_path_buf());
    }
    for extension in ["cue", "iso", "bin", "img", "chd", "rvz"] {
        if let Some(file) = package.files.iter().find(|file| {
            Path::new(&file.relative_path)
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case(extension))
        }) {
            return Some(source.join(&file.relative_path));
        }
    }
    None
}

fn analyze_header(
    path: &Path,
    context: &retro_junk_lib::AnalysisContext,
    platform_hint: Option<&str>,
) -> Option<(String, String)> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    for console in context.consoles() {
        if platform_hint.is_some_and(|hint| !console.metadata.short_name.eq_ignore_ascii_case(hint))
        {
            continue;
        }
        if !console
            .metadata
            .extensions
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(extension))
        {
            continue;
        }
        let mut file = File::open(path).ok()?;
        let options = AnalysisOptions::new().quick(true).file_path(path);
        if let Ok(identification) = console.analyzer.analyze(&mut file, &options)
            && !identification.serial_number.is_empty()
        {
            return Some((
                console.metadata.short_name.to_owned(),
                identification.serial_number,
            ));
        }
    }
    None
}

struct ExactCatalogMatch {
    matches: Vec<CatalogCandidate>,
    tracks: Vec<TrackDigest>,
    tool: Option<retro_junk_archive::ToolRecord>,
    method: IdentificationMethod,
    detail: String,
    byte_verified: bool,
}

impl ExactCatalogMatch {
    fn empty() -> Self {
        Self {
            matches: Vec::new(),
            tracks: Vec::new(),
            tool: None,
            method: IdentificationMethod::ExactFileHash,
            detail: String::new(),
            byte_verified: false,
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn exact_catalog_matches(
    source: &Path,
    package: &SourcePackageInventory,
    format: &RepresentationFormat,
    request: &DumpImportRequest,
    platform_hint: Option<&str>,
    context: &retro_junk_lib::AnalysisContext,
    catalog: &retro_junk_db::Connection,
    cancel: &AtomicBool,
    on_phase: &impl Fn(PlanningProgress),
) -> Result<ExactCatalogMatch, ImportError> {
    let supplied_cue = source
        .is_dir()
        .then(|| {
            package.files.iter().find_map(|file| {
                Path::new(&file.relative_path)
                    .extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("cue"))
                    .then(|| source.join(&file.relative_path))
            })
        })
        .flatten();
    if matches!(format, RepresentationFormat::CueBin)
        || (matches!(format, RepresentationFormat::RedumperRaw) && supplied_cue.is_some())
    {
        let Some(cue) = supplied_cue.or_else(|| analysis_entrypoint(source, package)) else {
            return Ok(ExactCatalogMatch::empty());
        };
        validate_cue_references(source, package, &cue)?;
        on_phase(PlanningProgress {
            description: format!("Hashing staged disc tracks: {}", cue.display()),
            kind: PlanningProgressKind::Bytes,
            current: 0,
            total: package.total_bytes,
        });
        let last_reported = std::cell::Cell::new(0_u64);
        let hashes = retro_junk_lib::disc_hash::hash_cue_disc(&cue, &|current, total| {
            if current == total || current.saturating_sub(last_reported.get()) >= 4 * 1024 * 1024 {
                last_reported.set(current);
                on_phase(PlanningProgress {
                    description: format!("Hashing staged disc tracks: {}", cue.display()),
                    kind: PlanningProgressKind::Bytes,
                    current,
                    total,
                });
            }
        })
        .map_err(|error| ImportError::Archive(error.to_string()))?;
        let tracks = hashes
            .tracks
            .into_iter()
            .map(|track| TrackDigest {
                number: u32::from(track.track_number),
                size: track.hashes.data_size,
                crc32: track.hashes.crc32,
                md5: track.hashes.md5.unwrap_or_default(),
                sha1: track.hashes.sha1.unwrap_or_default(),
            })
            .collect::<Vec<_>>();
        let matches = retro_junk_db::match_complete_catalog_media_any_platform(catalog, &tracks)
            .map_err(|error| ImportError::Catalog(error.to_string()))?;
        let result = ExactCatalogMatch {
            matches: matches.into_iter().map(CatalogCandidate::from).collect(),
            tracks,
            tool: None,
            method: IdentificationMethod::CompleteTrackSet,
            detail: "Complete ordered supplied CUE track set matched the catalog".to_owned(),
            byte_verified: true,
        };
        if !result.matches.is_empty() || matches!(format, RepresentationFormat::CueBin) {
            return Ok(result);
        }
    }
    if matches!(format, RepresentationFormat::RedumperRaw) && source.is_dir() {
        if let Some(log_match) = redumper_log_catalog_match(source, package, catalog)? {
            if !request.make_playable {
                return Ok(log_match);
            }
        }
        let executable = request
            .redumper_path
            .as_deref()
            .unwrap_or_else(|| Path::new(""));
        let redumper = retro_junk_archive::Redumper::detect(executable)
            .map_err(|error| ImportError::InvalidPackage(error.to_string()))?;
        let workspace = request.workspace_root.clone().unwrap_or_else(|| {
            request
                .archive_root
                .join(".retro-junk/work/import-identification")
        });
        on_phase(PlanningProgress {
            description: format!("Running Redumper analysis for {}", source.display()),
            kind: PlanningProgressKind::Indeterminate,
            current: 0,
            total: 0,
        });
        let audit = redumper
            .audit(source, &workspace, cancel)
            .map_err(|error| ImportError::InvalidPackage(error.to_string()))?;
        let matches =
            retro_junk_db::match_complete_catalog_media_any_platform(catalog, &audit.tracks)
                .map_err(|error| ImportError::Catalog(error.to_string()))?;
        return Ok(ExactCatalogMatch {
            matches: matches.into_iter().map(CatalogCandidate::from).collect(),
            tracks: audit.tracks,
            tool: Some(audit.tool),
            method: IdentificationMethod::CompleteTrackSet,
            detail: "Redumper regenerated a complete ordered track set that matched the catalog"
                .to_owned(),
            byte_verified: true,
        });
    }
    check_cancel(cancel)?;
    let primary = if source.is_file() {
        package.files.first()
    } else {
        package
            .files
            .iter()
            .find(|file| recognized_file(Path::new(&file.relative_path)))
    };
    let Some(primary) = primary else {
        return Ok(ExactCatalogMatch::empty());
    };
    let primary_path = if source.is_file() {
        source.to_path_buf()
    } else {
        source.join(&primary.relative_path)
    };
    if let Some(stored) = stored_catalog_match(catalog, &primary.digests)? {
        return Ok(stored);
    }
    if matches!(format, RepresentationFormat::Rom) {
        let normalized =
            format_aware_catalog_matches(&primary_path, platform_hint, context, catalog, cancel)?;
        if !normalized.is_empty() {
            return Ok(ExactCatalogMatch {
                matches: normalized,
                tracks: Vec::new(),
                tool: None,
                method: IdentificationMethod::FormatAwareFileHash,
                detail: "Format-aware ROM payload hash matched the catalog; archived source bytes were preserved unchanged".to_owned(),
                byte_verified: true,
            });
        }
    }
    Ok(ExactCatalogMatch::empty())
}

fn redumper_log_catalog_match(
    source: &Path,
    package: &SourcePackageInventory,
    catalog: &retro_junk_db::Connection,
) -> Result<Option<ExactCatalogMatch>, ImportError> {
    let logs = package.files.iter().filter(|file| {
        Path::new(&file.relative_path)
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("log"))
    });
    for log in logs {
        let path = source.join(&log.relative_path);
        let text = std::fs::read_to_string(&path).map_err(|source| ImportError::Io {
            path: path.display().to_string(),
            source,
        })?;
        if !text
            .lines()
            .any(|line| line.trim_start().starts_with("<rom "))
        {
            continue;
        }
        let records = retro_junk_dat::parse_logiqx_rom_lines(&text).map_err(|error| {
            ImportError::InvalidPackage(format!("invalid Redumper log: {error}"))
        })?;
        let tracks = records
            .into_iter()
            .enumerate()
            .map(|(index, record)| TrackDigest {
                number: u32::try_from(index + 1).unwrap_or(u32::MAX),
                size: record.size,
                crc32: record.crc,
                md5: record.md5.unwrap_or_default(),
                sha1: record.sha1.unwrap_or_default(),
            })
            .collect::<Vec<_>>();
        let matches = retro_junk_db::match_complete_catalog_media_any_platform(catalog, &tracks)
            .map_err(|error| ImportError::Catalog(error.to_string()))?;
        if !matches.is_empty() {
            return Ok(Some(ExactCatalogMatch {
                matches: matches.into_iter().map(CatalogCandidate::from).collect(),
                tracks,
                tool: None,
                method: IdentificationMethod::RedumperLog,
                detail:
                    "Redumper log hashes identified the catalog release but did not verify retained bytes"
                        .to_owned(),
                byte_verified: false,
            }));
        }
    }
    Ok(None)
}

fn stored_catalog_match(
    catalog: &retro_junk_db::Connection,
    digests: &FileDigests,
) -> Result<Option<ExactCatalogMatch>, ImportError> {
    let matches = retro_junk_db::match_catalog_file_any_platform(catalog, digests)
        .map_err(|error| ImportError::Catalog(error.to_string()))?;
    if matches.is_empty() {
        return Ok(None);
    }
    Ok(Some(ExactCatalogMatch {
        matches: matches.into_iter().map(CatalogCandidate::from).collect(),
        tracks: Vec::new(),
        tool: None,
        method: IdentificationMethod::ExactFileHash,
        detail: "Exact stored-file hash matched the catalog".to_owned(),
        byte_verified: true,
    }))
}

fn format_aware_catalog_matches(
    path: &Path,
    platform_hint: Option<&str>,
    context: &retro_junk_lib::AnalysisContext,
    catalog: &retro_junk_db::Connection,
    cancel: &AtomicBool,
) -> Result<Vec<CatalogCandidate>, ImportError> {
    check_cancel(cancel)?;
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    let mut matches = Vec::new();
    for console in context.consoles() {
        if platform_hint.is_some_and(|hint| !console.metadata.short_name.eq_ignore_ascii_case(hint))
            || !console
                .metadata
                .extensions
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(extension))
        {
            continue;
        }
        let mut input = File::open(path).map_err(|source| ImportError::Io {
            path: path.display().to_string(),
            source,
        })?;
        let Ok(hashes) = retro_junk_lib::hasher::compute_all_hashes(
            &mut input,
            console.analyzer.as_ref(),
            Some(path),
        ) else {
            continue;
        };
        let digests = FileDigests {
            size: hashes.data_size,
            crc32: hashes.crc32,
            md5: hashes.md5.unwrap_or_default(),
            sha1: hashes.sha1.unwrap_or_default(),
            sha256: String::new(),
        };
        let found =
            retro_junk_db::match_catalog_file(catalog, console.metadata.short_name, &digests)
                .map_err(|error| ImportError::Catalog(error.to_string()))?;
        matches.extend(found.into_iter().map(CatalogCandidate::from));
    }
    deduplicate_matches(&mut matches);
    Ok(matches)
}

fn validate_cue_references(
    source: &Path,
    package: &SourcePackageInventory,
    cue: &Path,
) -> Result<(), ImportError> {
    let cue_text = std::fs::read_to_string(cue).map_err(|source_error| ImportError::Io {
        path: cue.display().to_string(),
        source: source_error,
    })?;
    let sheet = retro_junk_disc::cue::parse_cue(&cue_text)
        .map_err(|error| ImportError::Archive(error.to_string()))?;
    let cue_relative = if source.is_dir() {
        cue.strip_prefix(source).unwrap_or(cue)
    } else {
        Path::new(cue.file_name().unwrap_or_default())
    };
    let cue_parent = cue_relative.parent().unwrap_or_else(|| Path::new(""));
    for file in sheet.files {
        let portable = file.filename.replace('\\', "/");
        let referenced = Path::new(&portable);
        if referenced.is_absolute()
            || referenced.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
        {
            return Err(ImportError::InvalidCueReference(file.filename));
        }
        let expected = cue_parent
            .join(referenced)
            .to_string_lossy()
            .replace('\\', "/");
        if !package
            .files
            .iter()
            .any(|inventory| inventory.relative_path == expected)
        {
            return Err(ImportError::InvalidCueReference(file.filename));
        }
    }
    Ok(())
}

fn append_catalog_evidence(
    candidate: &DumpImportCandidate,
    selected: &CatalogCandidate,
    imported: &retro_junk_archive::IngestedCarrierDump,
) -> Result<(), ImportError> {
    let (_, input_manifest_sha256) = retro_junk_archive::sha256_file(
        &imported.dump_directory.join("dump.toml"),
        &AtomicBool::new(false),
    )
    .map_err(|error| ImportError::Archive(error.to_string()))?;
    let verification_id = retro_junk_archive::VerificationId::new();
    let evidence = retro_junk_archive::VerificationEvidence {
        schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
        verification_id,
        representation_id: imported.dump.representation_id,
        performed_at: chrono::Utc::now().to_rfc3339(),
        input_manifest_sha256,
        kind: retro_junk_archive::VerificationKind::Catalog,
        outcome: retro_junk_archive::VerificationOutcome::Verified,
        tool: candidate.verification_tool.clone(),
        catalog: Some(retro_junk_archive::CatalogEvidence {
            source: selected.source.clone(),
            system: selected.platform_id.clone(),
            version: selected.source_version.clone(),
            game: selected.title.clone(),
            complete_track_set: !candidate.verification_tracks.is_empty(),
        }),
        tracks: candidate
            .verification_tracks
            .iter()
            .map(|track| retro_junk_archive::TrackVerification {
                number: track.number,
                size: track.size,
                expected_sha1: track.sha1.clone(),
                actual_sha1: track.sha1.clone(),
                matched: true,
            })
            .collect(),
        detail: candidate.verification_detail.clone(),
    };
    let evidence_directory = imported.dump_directory.join("evidence");
    std::fs::create_dir_all(&evidence_directory).map_err(|source| ImportError::Io {
        path: evidence_directory.display().to_string(),
        source,
    })?;
    retro_junk_archive::write_json_new(
        &evidence_directory.join(format!("verification-{verification_id}.json")),
        &evidence,
    )
    .map_err(|error| ImportError::Archive(error.to_string()))
}

fn append_playable_adoption(
    request: &DumpImportRequest,
    candidate: &DumpImportCandidate,
    dump_directory: &Path,
    manifest: &retro_junk_archive::DumpManifest,
    catalog_verified: bool,
) -> Result<(), ImportError> {
    if request.make_playable {
        return Ok(());
    }
    let Some(playable_root) = request.playable_root.as_deref() else {
        return Ok(());
    };
    if !candidate.source.is_file() || !candidate.source.starts_with(playable_root) {
        return Ok(());
    }
    let relative = retro_junk_archive::normalize_relative_path(
        candidate
            .source
            .strip_prefix(playable_root)
            .unwrap_or(&candidate.source),
    )
    .map_err(|error| ImportError::Archive(error.to_string()))?;
    let Some(file) = candidate.package.files.first() else {
        return Ok(());
    };
    let evidence_directory = dump_directory.join("evidence");
    if evidence_directory.is_dir() {
        let entries = std::fs::read_dir(&evidence_directory).map_err(|source| ImportError::Io {
            path: evidence_directory.display().to_string(),
            source,
        })?;
        for entry in entries {
            let path = entry
                .map_err(|source| ImportError::Io {
                    path: evidence_directory.display().to_string(),
                    source,
                })?
                .path();
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("build-"))
                && let Ok(existing) = retro_junk_archive::read_build_json(&path)
                && existing.relative_output_path == relative
                && existing.output_sha256 == file.digests.sha256
            {
                return Ok(());
            }
        }
    }
    let (_, input_manifest_sha256) =
        retro_junk_archive::sha256_file(&dump_directory.join("dump.toml"), &AtomicBool::new(false))
            .map_err(|error| ImportError::Archive(error.to_string()))?;
    retro_junk_archive::write_build_evidence(
        dump_directory,
        &retro_junk_archive::BuildEvidence {
            schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
            build_id: retro_junk_archive::BuildId::new(),
            parent_representation_id: manifest.representation_id,
            child_representation_id: retro_junk_archive::RepresentationId::new(),
            performed_at: chrono::Utc::now().to_rfc3339(),
            input_manifest_sha256,
            recipe_version: 1,
            format: manifest.format.clone(),
            relative_output_path: relative,
            output_sha256: file.digests.sha256.clone(),
            output_size: file.digests.size,
            catalog_verified,
            round_trip_verified: true,
            tool: None,
            omitted_features: Vec::new(),
            canonical_intermediate: None,
        },
    )
    .map(|_| ())
    .map_err(|error| ImportError::Archive(error.to_string()))
}

fn has_current_catalog_evidence(
    dump_directory: &Path,
    manifest: &retro_junk_archive::DumpManifest,
) -> bool {
    let Ok((_, manifest_sha256)) =
        retro_junk_archive::sha256_file(&dump_directory.join("dump.toml"), &AtomicBool::new(false))
    else {
        return false;
    };
    let Ok(entries) = std::fs::read_dir(dump_directory.join("evidence")) else {
        return false;
    };
    entries.filter_map(Result::ok).any(|entry| {
        retro_junk_archive::read_verification_json(&entry.path()).is_ok_and(|evidence| {
            evidence.representation_id == manifest.representation_id
                && evidence.input_manifest_sha256 == manifest_sha256
                && evidence.kind == retro_junk_archive::VerificationKind::Catalog
                && evidence.outcome == retro_junk_archive::VerificationOutcome::Verified
        })
    })
}

fn catalog_serial_matches(
    catalog: &retro_junk_db::Connection,
    serial: &str,
) -> Result<Vec<CatalogCandidate>, ImportError> {
    retro_junk_db::match_catalog_serial_any_platform(catalog, serial)
        .map(|matches| matches.into_iter().map(CatalogCandidate::from).collect())
        .map_err(|error| ImportError::Catalog(error.to_string()))
}

impl From<retro_junk_db::CompleteCatalogMediaMatch> for CatalogCandidate {
    fn from(value: retro_junk_db::CompleteCatalogMediaMatch) -> Self {
        Self {
            media_id: value.media_id,
            release_id: value.release_id,
            work_id: value.work_id,
            title: value.game,
            platform_id: value.platform_id,
            region: value.region,
            revision: value.revision,
            variant: value.variant,
            serial: value.serial,
            sequence_number: value.sequence_number,
            release_disc_count: 0,
            source: value.source,
            source_version: value.source_version,
        }
    }
}

fn deduplicate_matches(matches: &mut Vec<CatalogCandidate>) {
    matches.sort_by(|a, b| a.media_id.cmp(&b.media_id));
    matches.dedup_by(|a, b| a.media_id == b.media_id);
}

fn group_equivalent_release_matches(matches: &mut Vec<CatalogCandidate>) -> bool {
    let before = matches.len();
    matches.sort_by(|a, b| (&a.release_id, &a.media_id).cmp(&(&b.release_id, &b.media_id)));
    matches.dedup_by(|a, b| a.release_id == b.release_id);
    before != matches.len()
}

/// The archive release a newly identified dump belongs with, if the archive
/// already holds one.
///
/// Two questions, cheapest first. Does an archive release *describe* the same
/// thing — same platform, title, region, revision, variant? Those all came from
/// the catalog when the release was first archived, so a match is a match.
/// Failing that, does one of its carriers *contain* something from the same
/// catalog release, or from a compatible mastering of the same work? That
/// question is answered from each carrier's recorded track set, run back
/// through the catalog — the same evidence that identified it in the first
/// place.
///
/// The manifests hold no catalog row ids to compare, and deliberately so: an id
/// derived from a title moved whenever a title was corrected, which is how one
/// game came to have two archive releases.
fn archive_release_for<'a>(
    archive: &'a retro_junk_archive::ArchiveIndexSnapshot,
    catalog: &retro_junk_db::Connection,
    archive_platform_id: &str,
    selected: &CatalogCandidate,
    compatible_catalog_releases: &BTreeSet<String>,
) -> Result<Option<&'a retro_junk_archive::IndexedRelease>, ImportError> {
    let described = archive.releases.iter().find(|release| {
        release.manifest.platform_id == archive_platform_id
            && release.manifest.title == selected.title
            && release.manifest.region == selected.region
            && release.manifest.revision == selected.revision
            && release.manifest.variant == selected.variant
    });
    if described.is_some() {
        return Ok(described);
    }
    for release in &archive.releases {
        for copy in &release.physical_copies {
            for carrier in &copy.carriers {
                let tracks = &carrier.manifest.catalog_binding.expected_tracks;
                if tracks.is_empty() {
                    continue;
                }
                let matches = retro_junk_db::match_complete_catalog_media(
                    catalog,
                    &release.manifest.platform_id,
                    tracks,
                )
                .map_err(|error| ImportError::Catalog(error.to_string()))?;
                if matches.iter().any(|found| {
                    found.release_id == selected.release_id
                        || compatible_catalog_releases.contains(&found.release_id)
                }) {
                    return Ok(Some(release));
                }
            }
        }
    }
    Ok(None)
}

fn catalog_candidate_release_key(candidate: &CatalogCandidate) -> String {
    if candidate.work_id.is_empty() {
        return candidate.release_id.clone();
    }
    format!(
        "{}\u{1f}{}\u{1f}{}",
        candidate.work_id,
        candidate.platform_id.to_ascii_lowercase(),
        candidate.region.to_ascii_lowercase()
    )
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

    #[test]
    fn ps1_playables_use_the_es_de_psx_directory() {
        assert_eq!(playable_projection_platform("ps1", "Japan"), "psx");
        assert_eq!(playable_projection_platform("ps1", "USA"), "psx");
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn compatible_carrier_masterings_are_grouped_into_one_physical_copy() {
        let temp = tempfile::tempdir().unwrap();
        let inbox = temp.path().join("inbox");
        let archive = temp.path().join("archive");
        std::fs::create_dir_all(inbox.join("SLUS-00001")).unwrap();
        std::fs::create_dir_all(inbox.join("SLUS-00002")).unwrap();
        std::fs::create_dir_all(inbox.join("SLUS-00003")).unwrap();
        std::fs::write(inbox.join("SLUS-00001/disc.bin"), b"disc one").unwrap();
        std::fs::write(inbox.join("SLUS-00002/disc.bin"), b"disc two").unwrap();
        std::fs::write(inbox.join("SLUS-00003/disc.bin"), b"disc one b").unwrap();
        retro_junk_archive::initialize_archive(
            &archive,
            &retro_junk_archive::ArchiveRootManifest::new("Import test"),
        )
        .unwrap();

        let mut catalog = retro_junk_db::open_memory().unwrap();
        // The catalog names the console `ps1`; the archive lays PlayStation
        // discs out under `psx` for the frontend. Both are the same platform,
        // and the projection normalises between them.
        catalog.execute("INSERT INTO platforms(id,display_name,short_name,manufacturer,generation,media_type,release_year,description,core_platform) VALUES('ps1','PlayStation','PS1','Sony',5,'cd',1994,'','Ps1')", []).unwrap();
        catalog
            .execute(
                "INSERT INTO works(id,canonical_name) VALUES('work','Two Disc Game')",
                [],
            )
            .unwrap();
        catalog.execute("INSERT INTO releases(id,work_id,platform_id,region,revision,title) VALUES('release-a','work','ps1','usa','mastering-a','Two Disc Game')", []).unwrap();
        catalog.execute("INSERT INTO releases(id,work_id,platform_id,region,revision,title) VALUES('release-b','work','ps1','usa','mastering-b','Two Disc Game')", []).unwrap();
        catalog.execute("INSERT INTO releases(id,work_id,platform_id,region,revision,title) VALUES('release-c','work','ps1','usa','mastering-c','Two Disc Game')", []).unwrap();
        // Each catalog medium carries the digests of the disc it describes,
        // which is the only thing that binds an archived carrier to it.
        for (media_id, release_id, serial, disc, file) in [
            (
                "disc-1",
                "release-a",
                "SLUS-00001",
                1,
                "SLUS-00001/disc.bin",
            ),
            (
                "disc-2",
                "release-b",
                "SLUS-00002",
                2,
                "SLUS-00002/disc.bin",
            ),
            (
                "disc-1b",
                "release-c",
                "SLUS-00003",
                1,
                "SLUS-00003/disc.bin",
            ),
        ] {
            let digests =
                retro_junk_archive::hash_file_digests(&inbox.join(file), &AtomicBool::new(false))
                    .unwrap();
            catalog.execute(
                "INSERT INTO media(id,release_id,media_serial,disc_number,dat_source,file_size,crc32,sha1,md5)
                 VALUES(?1,?2,?3,?4,'redump',?5,?6,?7,?8)",
                rusqlite::params![
                    media_id,
                    release_id,
                    serial,
                    disc,
                    digests.size,
                    digests.crc32,
                    digests.sha1,
                    digests.md5
                ],
            ).unwrap();
        }
        catalog
            .execute(
                "INSERT INTO media_serial_keys(media_id,serial_key) VALUES('disc-1','SLUS00001')",
                [],
            )
            .unwrap();
        catalog
            .execute(
                "INSERT INTO media_serial_keys(media_id,serial_key) VALUES('disc-2','SLUS00002')",
                [],
            )
            .unwrap();
        catalog
            .execute(
                "INSERT INTO media_serial_keys(media_id,serial_key) VALUES('disc-1b','SLUS00003')",
                [],
            )
            .unwrap();

        let cancel = AtomicBool::new(false);
        let progress = std::cell::RefCell::new(Vec::new());
        let phases = std::cell::RefCell::new(Vec::new());
        let plan = plan_import(
            DumpImportRequest {
                source: inbox,
                archive_root: archive.clone(),
                platform_hint: None,
                owner_id: "default".to_owned(),
                new_physical_copy: false,
                redumper_path: None,
                workspace_root: Some(temp.path().join("work")),
                stage_packages_locally: true,
                playable_root: None,
                make_playable: false,
                chdman_path: None,
                discard_redundant_bin_cue: false,
            },
            &retro_junk_lib::AnalysisContext::new(),
            &catalog,
            &cancel,
            |current, total| progress.borrow_mut().push((current, total)),
            |phase| phases.borrow_mut().push(phase),
        )
        .unwrap();
        let progress = progress.into_inner();
        assert_eq!(progress.first(), Some(&(0, 26)));
        assert_eq!(progress.last(), Some(&(26, 26)));
        assert!(progress.iter().all(|(_, total)| *total == 26));
        let phases = phases.into_inner();
        assert!(phases.iter().any(|phase| {
            phase.kind == PlanningProgressKind::Indeterminate
                && phase.description.contains("archive index")
        }));
        assert!(
            phases
                .iter()
                .any(|phase| { phase.kind == PlanningProgressKind::Bytes && phase.total == 26 })
        );
        assert!(phases.iter().any(|phase| {
            phase.kind == PlanningProgressKind::Items && phase.current == 3 && phase.total == 3
        }));
        assert_eq!(plan.candidates.len(), 3);
        // Each disc matched the catalog on its own bytes, which is what later
        // binds it — the folder serials agree, but they are not the evidence.
        assert!(plan.candidates.iter().all(|candidate| {
            matches!(candidate.disposition, ImportDisposition::Ready)
                && matches!(
                    candidate.identification,
                    IdentificationResolution::CatalogVerified { .. }
                        | IdentificationResolution::Identified { .. }
                )
        }));

        let execution_phases = std::cell::RefCell::new(Vec::new());
        let result = execute_import(
            plan,
            false,
            &cancel,
            |_| {},
            |phase| execution_phases.borrow_mut().push(phase),
        )
        .unwrap();
        let execution_phases = execution_phases.into_inner();
        assert!(execution_phases.iter().any(|phase| {
            phase.kind == PlanningProgressKind::Bytes
                && phase.description.contains("Publishing archival package")
        }));
        assert!(execution_phases.iter().any(|phase| {
            phase.kind == PlanningProgressKind::Items && phase.current == 3 && phase.total == 3
        }));
        assert!(result.results.iter().all(|candidate| {
            candidate.outcome == CandidateImportOutcome::Imported && !candidate.source_removed
        }));
        let snapshot = retro_junk_archive::scan_archive(&archive).unwrap();
        assert_eq!(snapshot.releases.len(), 1);
        assert_eq!(snapshot.releases[0].physical_copies.len(), 2);
        assert_eq!(snapshot.releases[0].physical_copies[0].carriers.len(), 2);
        assert_eq!(snapshot.releases[0].physical_copies[1].carriers.len(), 1);
        // Three discs from three different masterings land under one archive
        // release, each keeping the exact catalog entry its own match named.
        // Nothing here is a catalog row id: the manifests describe what was
        // matched, and which catalog release the set belongs to is worked out
        // from the carriers when the projection is built.
        let carrier_serials = snapshot.releases[0]
            .physical_copies
            .iter()
            .flat_map(|copy| &copy.carriers)
            .flat_map(|carrier| carrier.manifest.catalog_binding.serials.clone())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            carrier_serials,
            [
                "SLUS-00001".to_owned(),
                "SLUS-00002".to_owned(),
                "SLUS-00003".to_owned()
            ]
            .into()
        );
        assert!(
            snapshot.releases[0]
                .directory
                .join("physical-copies/copy-01")
                .is_dir()
        );

        for carrier in snapshot.releases[0]
            .physical_copies
            .iter()
            .flat_map(|copy| &copy.carriers)
        {
            let dump = &carrier.dumps[0];
            let verification_id = retro_junk_archive::VerificationId::new();
            let evidence = retro_junk_archive::VerificationEvidence {
                schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
                verification_id,
                representation_id: dump.manifest.representation_id,
                performed_at: chrono::Utc::now().to_rfc3339(),
                input_manifest_sha256: dump.manifest_sha256.clone(),
                kind: retro_junk_archive::VerificationKind::Catalog,
                outcome: retro_junk_archive::VerificationOutcome::Verified,
                tool: None,
                catalog: Some(retro_junk_archive::CatalogEvidence {
                    source: "redump".to_owned(),
                    system: "psx".to_owned(),
                    version: String::new(),
                    game: "Two Disc Game".to_owned(),
                    complete_track_set: true,
                }),
                tracks: Vec::new(),
                detail: "test catalog evidence".to_owned(),
            };
            let evidence_directory = dump.directory.join("evidence");
            std::fs::create_dir_all(&evidence_directory).unwrap();
            retro_junk_archive::write_json_new(
                &evidence_directory.join(format!("verification-{verification_id}.json")),
                &evidence,
            )
            .unwrap();
        }
        let verified_snapshot = retro_junk_archive::scan_archive(&archive).unwrap();
        retro_junk_db::reconcile_archive_snapshot(
            &mut catalog,
            &verified_snapshot,
            &temp.path().join("playable"),
            &temp.path().join("work"),
        )
        .unwrap();
        let summaries = retro_junk_db::list_archive_release_summaries(
            &catalog,
            &verified_snapshot.manifest.profile_id.to_string(),
        )
        .unwrap();
        let facts = retro_junk_db::facts::release_facts_by_id(
            &catalog,
            &retro_junk_db::facts::FactsScope::profile(
                &verified_snapshot.manifest.profile_id.to_string(),
            ),
        )
        .unwrap();
        let release_facts = &facts[&summaries[0].archive_release_id];
        assert_eq!(release_facts.expected_discs.unwrap().count, 2);
        assert_eq!(retro_junk_db::facts::verified_disc_count(release_facts), 2);
    }

    #[test]
    fn cue_identification_rejects_references_outside_the_package() {
        let temp = tempfile::tempdir().unwrap();
        let package_root = temp.path().join("package");
        std::fs::create_dir(&package_root).unwrap();
        let cue = package_root.join("disc.cue");
        std::fs::write(
            &cue,
            "FILE \"../outside.bin\" BINARY\n  TRACK 01 MODE1/2352\n    INDEX 01 00:00:00\n",
        )
        .unwrap();
        let package = SourcePackageInventory {
            files: Vec::new(),
            total_bytes: 0,
            package_sha256: String::new(),
            observed_captured_at: String::new(),
            timestamp_source: String::new(),
        };
        assert!(matches!(
            validate_cue_references(&package_root, &package, &cue),
            Err(ImportError::InvalidCueReference(_))
        ));
    }

    #[test]
    fn one_invalid_disc_does_not_abort_batch_planning() {
        let temp = tempfile::tempdir().unwrap();
        let inbox = temp.path().join("inbox");
        let bad = inbox.join("bad-disc");
        let good = inbox.join("good-disc");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::create_dir_all(&good).unwrap();
        std::fs::write(
            bad.join("disc.cue"),
            "FILE \"../outside.bin\" BINARY\n  TRACK 01 MODE1/2352\n    INDEX 01 00:00:00\n",
        )
        .unwrap();
        std::fs::write(good.join("track.bin"), vec![0_u8; 2352]).unwrap();
        std::fs::write(
            good.join("disc.cue"),
            "FILE \"track.bin\" BINARY\n  TRACK 01 MODE1/2352\n    INDEX 01 00:00:00\n",
        )
        .unwrap();
        let archive = temp.path().join("archive");
        retro_junk_archive::initialize_archive(
            &archive,
            &retro_junk_archive::ArchiveRootManifest::new("Batch planning"),
        )
        .unwrap();

        let plan = plan_import(
            DumpImportRequest {
                source: inbox,
                archive_root: archive,
                platform_hint: Some("ps1".to_owned()),
                owner_id: "default".to_owned(),
                new_physical_copy: false,
                redumper_path: None,
                workspace_root: Some(temp.path().join("work")),
                stage_packages_locally: true,
                playable_root: None,
                make_playable: false,
                chdman_path: None,
                discard_redundant_bin_cue: false,
            },
            &retro_junk_lib::create_default_context(),
            &retro_junk_db::open_memory().unwrap(),
            &AtomicBool::new(false),
            |_, _| {},
            |_| {},
        )
        .unwrap();

        assert_eq!(plan.candidates.len(), 2);
        assert!(plan.candidates.iter().any(|candidate| {
            candidate.source.ends_with("bad-disc")
                && matches!(candidate.disposition, ImportDisposition::Invalid { .. })
        }));
        assert!(plan.candidates.iter().any(|candidate| {
            candidate.source.ends_with("good-disc")
                && !matches!(candidate.disposition, ImportDisposition::Invalid { .. })
        }));
    }

    #[test]
    fn a_directory_of_loose_roms_is_discovered_as_separate_packages() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("one.nes"), b"one").unwrap();
        std::fs::write(temp.path().join("two.nes"), b"two").unwrap();
        let packages =
            discover_packages(temp.path(), &retro_junk_lib::AnalysisContext::new()).unwrap();
        assert_eq!(packages.len(), 2);
        assert!(packages.iter().all(|package| package.is_file()));
    }

    #[test]
    fn import_planning_can_hash_and_analyze_a_package_in_place() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("game.nes");
        let archive = temp.path().join("archive");
        let workspace = temp.path().join("workspace");
        std::fs::write(&source, b"in-place rom bytes").unwrap();
        retro_junk_archive::initialize_archive(
            &archive,
            &retro_junk_archive::ArchiveRootManifest::new("In-place import"),
        )
        .unwrap();

        let plan = plan_import(
            DumpImportRequest {
                source: source.clone(),
                archive_root: archive,
                platform_hint: None,
                owner_id: "default".to_owned(),
                new_physical_copy: false,
                redumper_path: None,
                workspace_root: Some(workspace.clone()),
                stage_packages_locally: false,
                playable_root: None,
                make_playable: false,
                chdman_path: None,
                discard_redundant_bin_cue: false,
            },
            &retro_junk_lib::AnalysisContext::new(),
            &retro_junk_db::open_memory().unwrap(),
            &AtomicBool::new(false),
            |_, _| {},
            |_| {},
        )
        .unwrap();

        assert_eq!(plan.candidates[0].staged_source, source);
        assert!(!workspace.exists());
    }

    #[test]
    fn combined_catalogs_keep_separate_regional_physical_platforms() {
        let request = DumpImportRequest {
            source: PathBuf::from("/roms"),
            archive_root: PathBuf::from("/archive"),
            platform_hint: None,
            owner_id: "default".to_owned(),
            new_physical_copy: false,
            redumper_path: None,
            workspace_root: None,
            stage_packages_locally: true,
            playable_root: Some(PathBuf::from("/roms")),
            make_playable: false,
            chdman_path: None,
            discard_redundant_bin_cue: false,
        };
        let mut selected = CatalogCandidate {
            media_id: "media".to_owned(),
            release_id: "release".to_owned(),
            work_id: "work".to_owned(),
            title: "Game".to_owned(),
            platform_id: "nes".to_owned(),
            region: "Japan".to_owned(),
            revision: String::new(),
            variant: String::new(),
            serial: String::new(),
            sequence_number: 0,
            release_disc_count: 1,
            source: "no-intro".to_owned(),
            source_version: String::new(),
        };
        assert_eq!(
            physical_archive_platform(&request, Path::new("/roms/nes/game.nes"), &selected),
            "famicom"
        );
        selected.region = "USA".to_owned();
        assert_eq!(
            physical_archive_platform(&request, Path::new("/roms/famicom/game.nes"), &selected),
            "famicom"
        );
        assert_eq!(
            physical_archive_platform(&request, Path::new("/roms/nes/game.nes"), &selected),
            "nes"
        );

        selected.platform_id = "snes".to_owned();
        selected.region = "Japan".to_owned();
        assert_eq!(
            physical_archive_platform(&request, Path::new("/roms/game.sfc"), &selected),
            "super-famicom"
        );
        selected.region = "USA".to_owned();
        assert_eq!(
            physical_archive_platform(
                &request,
                Path::new("/roms/super-famicom/game.sfc"),
                &selected,
            ),
            "super-famicom"
        );
        assert_eq!(
            physical_archive_platform(&request, Path::new("/roms/game.sfc"), &selected),
            "snesna"
        );

        selected.platform_id = "genesis".to_owned();
        selected.region = "Japan".to_owned();
        assert_eq!(
            physical_archive_platform(&request, Path::new("/roms/game.md"), &selected),
            "megadrivejp"
        );
        selected.region = "Europe".to_owned();
        assert_eq!(
            physical_archive_platform(&request, Path::new("/roms/game.md"), &selected),
            "megadrive"
        );
        selected.region = "USA".to_owned();
        assert_eq!(
            physical_archive_platform(&request, Path::new("/roms/game.md"), &selected),
            "genesis"
        );

        selected.platform_id = "pce".to_owned();
        selected.region = "USA".to_owned();
        assert_eq!(
            physical_archive_platform(&request, Path::new("/roms/game.pce"), &selected),
            "tg16"
        );
        selected.region = "Japan".to_owned();
        assert_eq!(
            physical_archive_platform(&request, Path::new("/roms/game.pce"), &selected),
            "pce"
        );
        // Europe too: there is no European PC Engine card library to file
        // separately, so a European release joins the Japanese shelf rather
        // than being relabeled a TurboGrafx-16.
        selected.region = "Europe".to_owned();
        assert_eq!(
            physical_archive_platform(&request, Path::new("/roms/game.pce"), &selected),
            "pce"
        );

        selected.platform_id = "saturn".to_owned();
        selected.region = "Japan".to_owned();
        assert_eq!(
            physical_archive_platform(&request, Path::new("/roms/game.chd"), &selected),
            "saturnjp"
        );
        selected.region = "USA".to_owned();
        assert_eq!(
            physical_archive_platform(&request, Path::new("/roms/game.chd"), &selected),
            "saturn"
        );
        selected.region = "Japan".to_owned();
        assert_eq!(
            physical_archive_platform(&request, Path::new("/roms/saturn/game.chd"), &selected),
            "saturnjp"
        );
        assert_eq!(
            physical_archive_platform(&request, Path::new("/roms/saturnjp/game.chd"), &selected),
            "saturnjp"
        );

        selected.platform_id = "pcecd".to_owned();
        selected.region = "Japan".to_owned();
        assert_eq!(
            physical_archive_platform(&request, Path::new("/roms/game.chd"), &selected),
            "pcenginecd"
        );
        selected.region = "USA".to_owned();
        assert_eq!(
            physical_archive_platform(&request, Path::new("/roms/game.chd"), &selected),
            "tg-cd"
        );

        let mut explicit_nes = request;
        explicit_nes.platform_hint = Some("nes".to_owned());
        selected.platform_id = "nes".to_owned();
        selected.region = "Japan".to_owned();
        assert_eq!(
            physical_archive_platform(
                &explicit_nes,
                Path::new("/roms/famicom/game.nes"),
                &selected,
            ),
            "nes"
        );
    }

    #[test]
    fn playable_nes_rom_is_matched_by_headerless_payload_and_adopted() {
        let temp = tempfile::tempdir().unwrap();
        let playable = temp.path().join("roms");
        let archive = temp.path().join("archive");
        std::fs::create_dir_all(playable.join("nes")).unwrap();
        let rom = playable.join("nes/game.nes");
        let mut bytes = b"NES\x1a\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00".to_vec();
        bytes.extend((0..16 * 1024).map(|index| (index % 251) as u8));
        std::fs::write(&rom, &bytes).unwrap();
        retro_junk_archive::initialize_archive(
            &archive,
            &retro_junk_archive::ArchiveRootManifest::new("Playable import"),
        )
        .unwrap();

        let context = retro_junk_lib::create_default_context();
        let nes = context.get_by_short_name("nes").unwrap();
        let mut input = File::open(&rom).unwrap();
        let hashes = retro_junk_lib::hasher::compute_all_hashes(
            &mut input,
            nes.analyzer.as_ref(),
            Some(&rom),
        )
        .unwrap();
        assert_ne!(hashes.data_size, bytes.len() as u64);

        let catalog = retro_junk_db::open_memory().unwrap();
        catalog.execute("INSERT INTO platforms(id,display_name,short_name,manufacturer,generation,media_type,release_year,description,core_platform) VALUES('nes','NES','NES','Nintendo',3,'cartridge',1983,'','Nes')", []).unwrap();
        catalog
            .execute(
                "INSERT INTO works(id,canonical_name) VALUES('work','Headered Game')",
                [],
            )
            .unwrap();
        catalog.execute("INSERT INTO releases(id,work_id,platform_id,region,title) VALUES('release','work','nes','usa','Headered Game')", []).unwrap();
        catalog.execute(
            "INSERT INTO media(id,release_id,dat_source,file_size,crc32,sha1,md5) VALUES(?1,?2,'no-intro',?3,?4,?5,?6)",
            rusqlite::params![
                "media",
                "release",
                hashes.data_size,
                hashes.crc32,
                hashes.sha1.unwrap(),
                hashes.md5.unwrap()
            ],
        ).unwrap();

        let cancel = AtomicBool::new(false);
        let mut plan = plan_import(
            DumpImportRequest {
                source: playable.clone(),
                archive_root: archive.clone(),
                platform_hint: None,
                owner_id: "default".to_owned(),
                new_physical_copy: false,
                redumper_path: None,
                workspace_root: Some(temp.path().join("work")),
                stage_packages_locally: true,
                playable_root: Some(playable.clone()),
                make_playable: false,
                chdman_path: None,
                discard_redundant_bin_cue: false,
            },
            &context,
            &catalog,
            &cancel,
            |_, _| {},
            |_| {},
        )
        .unwrap();
        assert_eq!(plan.candidates.len(), 1);
        assert!(matches!(
            plan.candidates[0].identification,
            IdentificationResolution::CatalogVerified {
                method: IdentificationMethod::FormatAwareFileHash
            }
        ));
        plan.candidates.push(plan.candidates[0].clone());

        let result = execute_import(plan, false, &cancel, |_| {}, |_| {}).unwrap();
        assert_eq!(result.results[0].outcome, CandidateImportOutcome::Imported);
        assert_eq!(
            result.results[1].outcome,
            CandidateImportOutcome::AlreadyArchived
        );
        let snapshot = retro_junk_archive::scan_archive(&archive).unwrap();
        let dump = &snapshot.releases[0].physical_copies[0].carriers[0].dumps[0];
        assert_eq!(dump.builds.len(), 1);
        assert_eq!(dump.builds[0].evidence.relative_output_path, "nes/game.nes");
        assert!(dump.builds[0].evidence.catalog_verified);
        assert_eq!(
            std::fs::read(dump.directory.join("raw/game.nes")).unwrap(),
            bytes
        );
        assert!(rom.is_file());
    }

    #[test]
    fn unresolved_dump_can_be_imported_without_claiming_catalog_identity() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("homebrew.nes");
        let archive = temp.path().join("archive");
        std::fs::write(&source, b"uncatalogued homebrew").unwrap();
        retro_junk_archive::initialize_archive(
            &archive,
            &retro_junk_archive::ArchiveRootManifest::new("Import test"),
        )
        .unwrap();
        let catalog = retro_junk_db::open_memory().unwrap();
        let cancel = AtomicBool::new(false);
        let mut plan = plan_import(
            DumpImportRequest {
                source,
                archive_root: archive.clone(),
                platform_hint: Some("nes".to_owned()),
                owner_id: "default".to_owned(),
                new_physical_copy: false,
                redumper_path: None,
                workspace_root: Some(temp.path().join("workspace")),
                stage_packages_locally: true,
                playable_root: None,
                make_playable: false,
                chdman_path: None,
                discard_redundant_bin_cue: false,
            },
            &retro_junk_lib::create_default_context(),
            &catalog,
            &cancel,
            |_, _| {},
            |_| {},
        )
        .unwrap();
        assert!(matches!(
            plan.candidates[0].disposition,
            ImportDisposition::Unresolved { .. }
        ));
        plan.candidates[0].disposition = ImportDisposition::ReadyUnbound {
            title: "Homebrew Game".to_owned(),
            platform_id: "nes".to_owned(),
        };

        let result = execute_import(plan, false, &cancel, |_| {}, |_| {}).unwrap();
        assert_eq!(result.results[0].outcome, CandidateImportOutcome::Imported);
        let snapshot = retro_junk_archive::scan_archive(&archive).unwrap();
        assert_eq!(snapshot.releases[0].manifest.title, "Homebrew Game");
        assert_eq!(snapshot.releases[0].manifest.platform_id, "nes");
        assert!(
            snapshot.releases[0]
                .manifest
                .catalog_binding
                .dat_name
                .is_empty(),
            "an unbound import names no catalog entry"
        );
    }

    #[test]
    fn multiple_redumper_images_require_separate_subdirectories() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("multi-disc");
        let archive = temp.path().join("archive");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("disc1.scram"), b"disc one").unwrap();
        std::fs::write(source.join("disc2.scram"), b"disc two").unwrap();
        retro_junk_archive::initialize_archive(
            &archive,
            &retro_junk_archive::ArchiveRootManifest::new("Import test"),
        )
        .unwrap();

        let error = plan_import(
            DumpImportRequest {
                source,
                archive_root: archive,
                platform_hint: Some("saturn".to_owned()),
                owner_id: "default".to_owned(),
                new_physical_copy: false,
                redumper_path: None,
                workspace_root: Some(temp.path().join("workspace")),
                stage_packages_locally: true,
                playable_root: None,
                make_playable: false,
                chdman_path: None,
                discard_redundant_bin_cue: false,
            },
            &retro_junk_lib::create_default_context(),
            &retro_junk_db::open_memory().unwrap(),
            &AtomicBool::new(false),
            |_, _| {},
            |_| {},
        )
        .unwrap_err();

        let message = error.to_string();
        assert!(message.contains("disc1, disc2"));
        assert!(message.contains("separate subdirectory"));
    }

    #[test]
    fn n64_byte_order_is_normalized_for_catalog_matching() {
        let temp = tempfile::tempdir().unwrap();
        let z64 = temp.path().join("game.z64");
        let v64 = temp.path().join("game.v64");
        let mut canonical = vec![0x80, 0x37, 0x12, 0x40];
        canonical.extend((4..4096).map(|index| (index % 239) as u8));
        let mut byte_swapped = canonical.clone();
        for pair in byte_swapped.chunks_exact_mut(2) {
            pair.swap(0, 1);
        }
        std::fs::write(&z64, &canonical).unwrap();
        std::fs::write(&v64, &byte_swapped).unwrap();

        let context = retro_junk_lib::create_default_context();
        let n64 = context.get_by_short_name("n64").unwrap();
        let mut input = File::open(&z64).unwrap();
        let hashes = retro_junk_lib::hasher::compute_all_hashes(
            &mut input,
            n64.analyzer.as_ref(),
            Some(&z64),
        )
        .unwrap();
        let catalog = retro_junk_db::open_memory().unwrap();
        catalog.execute("INSERT INTO platforms(id,display_name,short_name,manufacturer,generation,media_type,release_year,description,core_platform) VALUES('n64','Nintendo 64','N64','Nintendo',5,'cartridge',1996,'','N64')", []).unwrap();
        catalog
            .execute(
                "INSERT INTO works(id,canonical_name) VALUES('work','N64 Game')",
                [],
            )
            .unwrap();
        catalog.execute("INSERT INTO releases(id,work_id,platform_id,region,title) VALUES('release','work','n64','usa','N64 Game')", []).unwrap();
        catalog.execute(
            "INSERT INTO media(id,release_id,dat_source,file_size,crc32,sha1,md5) VALUES(?1,?2,'no-intro',?3,?4,?5,?6)",
            rusqlite::params!["media", "release", hashes.data_size, hashes.crc32, hashes.sha1.unwrap(), hashes.md5.unwrap()],
        ).unwrap();

        let found = format_aware_catalog_matches(
            &v64,
            Some("n64"),
            &context,
            &catalog,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].media_id, "media");
    }
}

fn find_observed_timestamp(source: &Path, files: &[InventoryFile]) -> (String, String) {
    let Some(log) = files.iter().find(|file| {
        Path::new(&file.relative_path)
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("log"))
    }) else {
        return (String::new(), String::new());
    };
    let path = if source.is_file() {
        source.to_path_buf()
    } else {
        source.join(&log.relative_path)
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return (String::new(), String::new());
    };
    for token in text.split_whitespace().map(|token| {
        token.trim_matches(|c: char| {
            !c.is_ascii_alphanumeric() && !matches!(c, '-' | ':' | '+' | '.' | 'T' | 'Z')
        })
    }) {
        if let Ok(value) = chrono::DateTime::parse_from_rfc3339(token) {
            return (value.to_rfc3339(), log.relative_path.clone());
        }
    }
    (String::new(), String::new())
}

fn source_record(source: &Path, package: &SourcePackageInventory) -> SourcePackageRecord {
    SourcePackageRecord {
        original_name: source
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        package_sha256: package.package_sha256.clone(),
        observed_captured_at: package.observed_captured_at.clone(),
        timestamp_source: package.timestamp_source.clone(),
    }
}

fn verify_and_consume(
    candidate: &DumpImportCandidate,
    dump_directory: &Path,
    manifest: &retro_junk_archive::DumpManifest,
    cancel: &AtomicBool,
) -> Result<bool, ImportError> {
    let current = inventory_package(&candidate.source, cancel, |_| {})?;
    if current.package_sha256 != candidate.package.package_sha256
        || current.observed_captured_at != candidate.package.observed_captured_at
    {
        return Ok(false);
    }
    if fingerprint_archived(&manifest.files) != current.package_sha256 {
        return Ok(false);
    }
    let report = retro_junk_archive::verify_dump_integrity(dump_directory, manifest, cancel)
        .map_err(|error| ImportError::Archive(error.to_string()))?;
    if !report.is_verified() {
        return Ok(false);
    }
    if candidate.source.is_dir() {
        std::fs::remove_dir_all(&candidate.source)
    } else {
        std::fs::remove_file(&candidate.source)
    }
    .map_err(|source| ImportError::Io {
        path: candidate.source.display().to_string(),
        source,
    })?;
    Ok(true)
}

fn result(
    candidate: &DumpImportCandidate,
    outcome: CandidateImportOutcome,
    source_removed: bool,
    detail: &str,
) -> CandidateImportResult {
    CandidateImportResult {
        source: candidate.source.clone(),
        outcome,
        source_removed,
        detail: detail.to_owned(),
        warnings: candidate.warnings.clone(),
        playable_build: None,
    }
}

fn build_imported_playable(
    request: &DumpImportRequest,
    candidate: &DumpImportCandidate,
    selected: &CatalogCandidate,
    manifest: &retro_junk_archive::DumpManifest,
    cancel: &AtomicBool,
    on_phase: &impl Fn(PlanningProgress),
) -> Option<PlayableBuildResult> {
    if !request.make_playable {
        return None;
    }
    let Some(playable_root) = request.playable_root.as_ref() else {
        return Some(PlayableBuildResult {
            outcome: PlayableBuildOutcome::Failed,
            output: None,
            detail: "--playable-root is required when playable creation is requested".to_owned(),
            intermediate_source: candidate.intermediate_source,
            authorized_exclusions: Vec::new(),
        });
    };
    on_phase(PlanningProgress {
        description: format!(
            "Creating verified playable CHD for {}",
            candidate.source.display()
        ),
        kind: PlanningProgressKind::Indeterminate,
        current: 0,
        total: 0,
    });
    let workspace_root = request
        .workspace_root
        .clone()
        .unwrap_or_else(retro_junk_io::default_transient_workspace);
    let build = retro_junk_lib::playable_build::build_playable(
        &retro_junk_lib::playable_build::PlayableBuildRequest {
            archive_root: request.archive_root.clone(),
            playable_root: playable_root.clone(),
            workspace_root,
            dump_id: manifest.dump_id.to_string(),
            format: RepresentationFormat::Chd,
            chdman_path: request.chdman_path.clone().unwrap_or_default(),
            redumper_path: request.redumper_path.clone().unwrap_or_default(),
            dolphin_tool_path: PathBuf::new(),
            allow_unverified: false,
            retain_intermediate: false,
            options: BTreeMap::new(),
            playable_platform_id: playable_projection_platform(
                &candidate.archive_platform_id,
                &selected.region,
            ),
            expected_disc_count: selected.release_disc_count.max(1),
            canonical_output_stem: selected.title.clone(),
            canonical_release_name: selected.title.clone(),
        },
        &|description, unit, current, total| {
            on_phase(PlanningProgress {
                description: description.to_owned(),
                kind: progress_kind(unit, total),
                current,
                total,
            });
        },
        cancel,
    );
    match build {
        Ok(outcome) => Some(PlayableBuildResult {
            outcome: PlayableBuildOutcome::Created,
            output: Some(outcome.output),
            detail: if request.discard_redundant_bin_cue {
                "verified CHD created; BIN/CUE exclusion was authorized but no files were removed because safe archive rewriting is unavailable"
                    .to_owned()
            } else {
                "verified CHD created".to_owned()
            },
            intermediate_source: candidate.intermediate_source,
            authorized_exclusions: Vec::new(),
        }),
        Err(error) => Some(PlayableBuildResult {
            outcome: PlayableBuildOutcome::Failed,
            output: None,
            detail: error.to_string(),
            intermediate_source: candidate.intermediate_source,
            authorized_exclusions: Vec::new(),
        }),
    }
}

fn playable_projection_platform(archive_platform: &str, region: &str) -> String {
    retro_junk_frontend::esde::system_directory(archive_platform, Some(region))
}

fn imported_verification_detail(identification: &IdentificationResolution) -> &'static str {
    match identification {
        IdentificationResolution::CatalogVerified { .. } => {
            "imported; archive integrity and catalog hashes verified"
        }
        IdentificationResolution::Identified { .. } => {
            "imported; archive integrity verified, catalog identity inferred but not hash verified"
        }
        IdentificationResolution::Ambiguous | IdentificationResolution::Unresolved => {
            "imported; archive integrity verified, catalog identity not verified"
        }
    }
}

fn batch_duplicate_key(candidate: &DumpImportCandidate) -> String {
    format!(
        "{}\0{}\0{}",
        candidate.package.package_sha256,
        candidate.package.observed_captured_at,
        candidate
            .selected_match
            .as_ref()
            .map_or("", |selected| selected.media_id.as_str())
    )
}

fn disposition_reason(disposition: &ImportDisposition) -> &str {
    match disposition {
        ImportDisposition::NeedsCatalogChoice { .. } => "catalog match is ambiguous",
        ImportDisposition::NeedsPhysicalCopyChoice { .. } => "several physical copies exist",
        ImportDisposition::Unresolved { reason } | ImportDisposition::Invalid { reason } => reason,
        ImportDisposition::Ready
        | ImportDisposition::ReadyUnbound { .. }
        | ImportDisposition::AlreadyArchived { .. } => "",
    }
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), ImportError> {
    if cancel.load(Ordering::Relaxed) {
        Err(ImportError::Cancelled)
    } else {
        Ok(())
    }
}
