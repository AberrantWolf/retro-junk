#![allow(clippy::needless_pass_by_value, clippy::too_many_lines)]

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use retro_junk_archive::{
    ArchiveRootManifest, BuildEvidence, BuildId, NewCarrierDump, Redumper, RepresentationFormat,
    RepresentationId, TrackVerification, VerificationEvidence, VerificationId, VerificationKind,
    VerificationOutcome, ingest_new_carrier_dump, initialize_archive, scan_archive, sha256_file,
    slugify, verify_dump_integrity, write_json_new, write_toml_atomic,
};

use crate::CliError;
use crate::cli_types::ArchiveAction;

#[derive(Debug, serde::Serialize)]
struct PlayableInbox {
    generated_at: String,
    playable_root: String,
    entries: Vec<PlayableInboxEntry>,
}

#[derive(Debug, serde::Serialize)]
struct PlayableInboxEntry {
    relative_path: String,
    status: String,
    detail: String,
}

pub(crate) fn run_archive(
    action: ArchiveAction,
    ctx: &retro_junk_lib::AnalysisContext,
) -> Result<(), CliError> {
    let _archive_lock = archive_mutation_root(&action)
        .map(retro_junk_archive::ArchiveLock::acquire)
        .transpose()
        .map_err(|error| CliError::other(error.to_string()))?;
    match action {
        ArchiveAction::Init { archive_root, name } => run_init(archive_root, name),
        ArchiveAction::Import {
            source,
            archive_root,
            db,
            platform,
            owner,
            new_physical_copy,
            redumper,
            workspace_root,
            consume,
            dry_run,
            yes,
        } => run_import(
            ctx,
            source,
            archive_root,
            db,
            platform,
            owner,
            new_physical_copy,
            redumper,
            workspace_root,
            None,
            consume,
            dry_run,
            yes,
        ),
        ArchiveAction::ImportPlayable {
            playable_root,
            archive_root,
            db,
            platform,
            owner,
            new_physical_copy,
            redumper,
            workspace_root,
            dry_run,
            yes,
        } => run_import(
            ctx,
            playable_root.clone(),
            archive_root,
            db,
            platform,
            owner,
            new_physical_copy,
            redumper,
            workspace_root,
            Some(playable_root),
            false,
            dry_run,
            yes,
        ),
        ArchiveAction::Ingest {
            source,
            archive_root,
            platform,
            title,
            format,
            region,
            revision,
            variant,
            serial,
            sequence_number,
            owner,
            physical_copy_label,
            carrier_label,
        } => run_ingest(
            source,
            archive_root,
            NewCarrierDump {
                platform_id: platform,
                title,
                region,
                revision,
                variant,
                owner_id: owner,
                physical_copy_label,
                serial,
                sequence_number,
                carrier_label,
                carrier_kind: carrier_kind_for_format(&parse_format(&format)?),
                format: parse_format(&format)?,
                catalog_binding: retro_junk_archive::CatalogBinding::default(),
                source_package: retro_junk_archive::SourcePackageRecord::default(),
                physical_copy_id: None,
            },
        ),
        ArchiveAction::AddReleaseFile {
            source_file,
            archive_root,
            release_id,
            category,
            asset_type,
            source,
            source_url,
            caption,
        } => {
            let destination = retro_junk_archive::add_release_file(
                &archive_root,
                retro_junk_archive::NewReleaseFile {
                    release_id,
                    source_file: &source_file,
                    category: parse_release_file_category(&category)?,
                    asset_type: &asset_type,
                    source: &source,
                    source_url: &source_url,
                    caption: &caption,
                },
                &AtomicBool::new(false),
            )
            .map_err(|error| CliError::other(error.to_string()))?;
            log::info!(
                "Archived release supporting file at {}",
                destination.display()
            );
            Ok(())
        }
        ArchiveAction::AddPhysicalCopyFile {
            source_file,
            archive_root,
            physical_copy_id,
            category,
            asset_type,
            source,
            caption,
        } => {
            let destination = retro_junk_archive::add_physical_copy_file(
                &archive_root,
                retro_junk_archive::NewPhysicalCopyFile {
                    physical_copy_id,
                    source_file: &source_file,
                    category: parse_physical_copy_file_category(&category)?,
                    asset_type: &asset_type,
                    source: &source,
                    caption: &caption,
                },
                &AtomicBool::new(false),
            )
            .map_err(|error| CliError::other(error.to_string()))?;
            log::info!(
                "Archived physical-copy supporting file at {}",
                destination.display()
            );
            Ok(())
        }
        ArchiveAction::Status { archive_root } => run_status(archive_root),
        ArchiveAction::Verify { archive_root } => run_verify(archive_root),
        ArchiveAction::VerifyCatalog {
            archive_root,
            db,
            dump_id,
        } => run_catalog_verify(ctx, archive_root, db, dump_id.as_deref()),
        ArchiveAction::AuditRedumper {
            archive_root,
            dump_id,
            workspace_root,
            redumper,
            db,
        } => run_redumper_audit(
            archive_root,
            dump_id.as_deref(),
            workspace_root,
            redumper,
            db,
        ),
        ArchiveAction::BuildChd {
            archive_root,
            playable_root,
            dump_id,
            workspace_root,
            chdman,
            redumper,
            allow_unverified,
        } => run_build_chd(
            ctx,
            archive_root,
            playable_root,
            &dump_id,
            workspace_root,
            chdman,
            redumper,
            allow_unverified,
            false,
        ),
        ArchiveAction::BuildRvz {
            archive_root,
            playable_root,
            dump_id,
            workspace_root,
            dolphin_tool,
            allow_unverified,
        } => run_build_rvz(
            archive_root,
            playable_root,
            &dump_id,
            workspace_root,
            dolphin_tool,
            allow_unverified,
            &std::collections::BTreeMap::new(),
        ),
        ArchiveAction::Mirror {
            archive_root,
            playable_root,
            dump_id,
        } => run_mirror(archive_root, playable_root, &dump_id),
        ArchiveAction::Policy {
            archive_root,
            carrier_id,
            format,
            clear,
            retain_intermediate,
            allow_unverified,
        } => run_policy(
            archive_root,
            carrier_id,
            format.as_deref(),
            clear,
            retain_intermediate,
            allow_unverified,
        ),
        ArchiveAction::PolicyDefault {
            archive_root,
            platform,
            format,
            clear,
            retain_intermediate,
            allow_unverified,
        } => run_default_policy(
            archive_root,
            &platform,
            format.as_deref(),
            clear,
            retain_intermediate,
            allow_unverified,
        ),
        ArchiveAction::Build {
            archive_root,
            playable_root,
            workspace_root,
            chdman,
            redumper,
            dolphin_tool,
            dry_run,
            limit,
        } => run_build_queue(
            ctx,
            archive_root,
            playable_root,
            workspace_root,
            chdman,
            redumper,
            dolphin_tool,
            dry_run,
            limit,
        ),
        ArchiveAction::ProjectFrontendFiles {
            archive_root,
            media_root,
        } => run_project_assets(archive_root, media_root),
        ArchiveAction::AdoptPlayable {
            archive_root,
            playable_root,
            db,
        } => run_adopt_playable(ctx, archive_root, playable_root, db),
        ArchiveAction::Recover { archive_root } => run_recover(archive_root),
        ArchiveAction::Reindex {
            archive_root,
            playable_root,
            workspace_root,
            db,
        } => run_reindex(archive_root, playable_root, workspace_root, db),
    }
}

fn archive_mutation_root(action: &ArchiveAction) -> Option<&std::path::Path> {
    match action {
        ArchiveAction::Init { .. }
        | ArchiveAction::Import { .. }
        | ArchiveAction::ImportPlayable { .. }
        | ArchiveAction::Status { .. }
        | ArchiveAction::Reindex { .. }
        | ArchiveAction::ProjectFrontendFiles { .. } => None,
        ArchiveAction::Ingest { archive_root, .. }
        | ArchiveAction::AddReleaseFile { archive_root, .. }
        | ArchiveAction::AddPhysicalCopyFile { archive_root, .. }
        | ArchiveAction::Verify { archive_root }
        | ArchiveAction::VerifyCatalog { archive_root, .. }
        | ArchiveAction::AuditRedumper { archive_root, .. }
        | ArchiveAction::BuildChd { archive_root, .. }
        | ArchiveAction::BuildRvz { archive_root, .. }
        | ArchiveAction::Mirror { archive_root, .. }
        | ArchiveAction::Policy { archive_root, .. }
        | ArchiveAction::PolicyDefault { archive_root, .. }
        | ArchiveAction::Build { archive_root, .. }
        | ArchiveAction::AdoptPlayable { archive_root, .. }
        | ArchiveAction::Recover { archive_root } => Some(archive_root),
    }
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::fn_params_excessive_bools)]
fn run_import(
    context: &retro_junk_lib::AnalysisContext,
    source: PathBuf,
    archive_root: PathBuf,
    database_path: Option<PathBuf>,
    platform_hint: Option<String>,
    owner_id: String,
    new_physical_copy: bool,
    redumper_path: Option<PathBuf>,
    workspace_root: Option<PathBuf>,
    playable_root: Option<PathBuf>,
    consume: bool,
    dry_run: bool,
    yes: bool,
) -> Result<(), CliError> {
    use std::io::{IsTerminal, Write};

    let database_path =
        database_path.unwrap_or_else(retro_junk_lib::settings::catalog_database_path);
    let mut connection = retro_junk_db::open_database(&database_path)
        .map_err(|error| CliError::database(error.to_string()))?;
    let cancelled = AtomicBool::new(false);
    let reconciliation_playable_root = playable_root.clone();
    let plan = retro_junk_archive_import::plan_import(
        retro_junk_archive_import::DumpImportRequest {
            source,
            archive_root: archive_root.clone(),
            platform_hint,
            owner_id,
            new_physical_copy,
            redumper_path,
            workspace_root,
            playable_root,
        },
        context,
        &connection,
        &cancelled,
        |_, _| {},
    )
    .map_err(|error| CliError::other(error.to_string()))?;

    let ready = plan
        .candidates
        .iter()
        .filter(|candidate| {
            matches!(
                candidate.disposition,
                retro_junk_archive_import::ImportDisposition::Ready
                    | retro_junk_archive_import::ImportDisposition::AlreadyArchived { .. }
            )
        })
        .count();
    for candidate in &plan.candidates {
        let identity = candidate.selected_match.as_ref().map_or_else(
            || "unresolved".to_owned(),
            |selected| {
                format!(
                    "{} / {} ({})",
                    selected.platform_id, selected.title, selected.serial
                )
            },
        );
        log::info!(
            "{} -> {} [{:?}; {}]",
            candidate.source.display(),
            identity,
            candidate.format,
            import_disposition_label(&candidate.disposition),
        );
    }
    log::info!(
        "Planned {ready} actionable package(s), {} total package(s), {} byte(s)",
        plan.candidates.len(),
        plan.total_source_bytes
    );
    if dry_run || ready == 0 {
        return Ok(());
    }
    if !yes {
        if !std::io::stdin().is_terminal() {
            return Err(CliError::other(
                "archive import requires --yes when standard input is not interactive",
            ));
        }
        print!(
            "Import {ready} package(s){}? [y/N] ",
            if consume {
                " and consume verified sources"
            } else {
                ""
            }
        );
        std::io::stdout().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            log::info!("Import cancelled");
            return Ok(());
        }
    }
    let result = retro_junk_archive_import::execute_import(plan, consume, &cancelled, |_| {})
        .map_err(|error| CliError::other(error.to_string()))?;
    for item in &result.results {
        log::info!(
            "{:?}: {}{} ({})",
            item.outcome,
            item.source.display(),
            if item.source_removed {
                " [source removed]"
            } else {
                ""
            },
            item.detail
        );
    }
    let snapshot =
        scan_archive(&archive_root).map_err(|error| CliError::other(error.to_string()))?;
    retro_junk_db::reconcile_archive_snapshot(
        &mut connection,
        &snapshot,
        reconciliation_playable_root
            .as_deref()
            .unwrap_or_else(|| std::path::Path::new("")),
        &archive_root.join(".retro-junk/work"),
    )
    .map_err(|error| CliError::database(error.to_string()))?;
    Ok(())
}

fn import_disposition_label(
    disposition: &retro_junk_archive_import::ImportDisposition,
) -> &'static str {
    match disposition {
        retro_junk_archive_import::ImportDisposition::Ready => "ready",
        retro_junk_archive_import::ImportDisposition::AlreadyArchived { .. } => "already archived",
        retro_junk_archive_import::ImportDisposition::NeedsCatalogChoice { .. } => {
            "ambiguous catalog match"
        }
        retro_junk_archive_import::ImportDisposition::NeedsPhysicalCopyChoice { .. } => {
            "physical-copy choice required"
        }
        retro_junk_archive_import::ImportDisposition::Unresolved { .. } => "unresolved",
        retro_junk_archive_import::ImportDisposition::Invalid { .. } => "invalid",
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn run_build_queue(
    ctx: &retro_junk_lib::AnalysisContext,
    archive_root: PathBuf,
    playable_root: PathBuf,
    workspace_root: Option<PathBuf>,
    chdman: Option<PathBuf>,
    redumper: Option<PathBuf>,
    dolphin_tool: Option<PathBuf>,
    dry_run: bool,
    limit: Option<usize>,
) -> Result<(), CliError> {
    let snapshot =
        scan_archive(&archive_root).map_err(|error| CliError::other(error.to_string()))?;
    let defaults = snapshot
        .manifest
        .platform_defaults
        .iter()
        .map(|default| (default.platform_id.as_str(), &default.policy))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut pending = Vec::new();
    let mut planning_failures = Vec::new();
    let mut desired = 0_usize;
    let mut satisfied = 0_usize;
    for release in &snapshot.releases {
        for medium in release
            .physical_copies
            .iter()
            .flat_map(|item| &item.carriers)
        {
            let Some(policy) = medium
                .manifest
                .playable_policy
                .as_ref()
                .or_else(|| defaults.get(release.manifest.platform_id.as_str()).copied())
            else {
                continue;
            };
            desired += 1;
            let Some(selected) = medium.dumps.iter().max_by(|a, b| {
                a.manifest
                    .captured_at
                    .cmp(&b.manifest.captured_at)
                    .then_with(|| a.manifest.dump_id.cmp(&b.manifest.dump_id))
            }) else {
                planning_failures.push(format!(
                    "{} (no preservation dump)",
                    medium.manifest.carrier_id
                ));
                continue;
            };
            let is_satisfied = selected.builds.iter().any(|build| {
                build.evidence.format == policy.format
                    && retro_junk_archive::playable_presence(
                        &playable_root,
                        &selected.manifest_sha256,
                        &build.evidence,
                    ) == retro_junk_archive::RepresentationPresence::Present
            });
            if is_satisfied {
                satisfied += 1;
                continue;
            }
            pending.push((
                medium.manifest.carrier_id.to_string(),
                selected.manifest.dump_id.to_string(),
                policy.clone(),
                selected.manifest.format.clone(),
                selected.manifest.files.len(),
            ));
        }
    }
    let pending_total = pending.len();
    log::info!("Playable queue: {desired} desired, {satisfied} satisfied, {pending_total} pending");
    if dry_run {
        for (media_id, dump_id, policy, _, _) in pending.iter().take(limit.unwrap_or(usize::MAX)) {
            log::info!(
                "pending {media_id}: {:?} from dump {dump_id}",
                policy.format
            );
        }
        for failure in &planning_failures {
            log::error!("pending policy cannot be built: {failure}");
        }
        return if planning_failures.is_empty() {
            Ok(())
        } else {
            Err(CliError::other(format!(
                "{} pending policy item(s) cannot be planned",
                planning_failures.len()
            )))
        };
    }
    let mut built = 0_usize;
    let mut failed = planning_failures;
    for (media_id, dump_id, policy, source_format, file_count) in
        pending.into_iter().take(limit.unwrap_or(usize::MAX))
    {
        let result = if policy.format == RepresentationFormat::Chd {
            run_build_chd(
                ctx,
                archive_root.clone(),
                playable_root.clone(),
                &dump_id,
                workspace_root.clone(),
                chdman.clone(),
                redumper.clone(),
                policy.allow_unverified,
                policy.retain_canonical_intermediate,
            )
        } else if policy.format == RepresentationFormat::Rvz {
            run_build_rvz(
                archive_root.clone(),
                playable_root.clone(),
                &dump_id,
                workspace_root.clone(),
                dolphin_tool.clone(),
                policy.allow_unverified,
                &policy.options,
            )
        } else if policy.format == source_format && file_count == 1 {
            run_mirror(archive_root.clone(), playable_root.clone(), &dump_id)
        } else {
            Err(CliError::other(format!(
                "no builder is available from {source_format:?} to {:?}",
                policy.format
            )))
        };
        match result {
            Ok(()) => built += 1,
            Err(error) => {
                log::error!("{media_id}: {error}");
                failed.push(media_id);
            }
        }
    }
    log::info!("Built {built} playable representation(s)");
    let playlists = project_playlists(&archive_root, &playable_root)?;
    if playlists > 0 {
        log::info!("Projected {playlists} multi-disc playlist(s)");
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(CliError::other(format!(
            "{} queued build(s) failed: {}",
            failed.len(),
            failed.join(", ")
        )))
    }
}

fn desired_policy(
    format: Option<&str>,
    retain_intermediate: bool,
    allow_unverified: bool,
) -> Result<retro_junk_archive::DesiredPlayablePolicy, CliError> {
    Ok(retro_junk_archive::DesiredPlayablePolicy {
        format: parse_format(format.unwrap_or_default())?,
        retain_canonical_intermediate: retain_intermediate,
        allow_unverified,
        options: std::collections::BTreeMap::new(),
    })
}

fn run_project_assets(archive_root: PathBuf, media_root: PathBuf) -> Result<(), CliError> {
    let snapshot =
        scan_archive(&archive_root).map_err(|error| CliError::other(error.to_string()))?;
    let cancelled = AtomicBool::new(false);
    let mut projected = 0_usize;
    for release in &snapshot.releases {
        let mut stems = release
            .physical_copies
            .iter()
            .flat_map(|item| &item.carriers)
            .flat_map(|medium| &medium.dumps)
            .flat_map(|dump| &dump.builds)
            .filter_map(|build| {
                std::path::Path::new(&build.evidence.relative_output_path)
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .map(str::to_owned)
            })
            .collect::<std::collections::BTreeSet<_>>();
        if stems.is_empty() {
            stems.insert(slugify(&release.manifest.title));
        }
        for asset in &release.supporting_files {
            let Some(subdirectory) = frontend_asset_subdirectory(&asset.manifest.asset_type) else {
                log::debug!(
                    "Skipping non-frontend archive asset type {}",
                    asset.manifest.asset_type
                );
                continue;
            };
            let source = asset.directory.join(&asset.manifest.file.path);
            let extension = source
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("bin");
            for stem in &stems {
                let directory = media_root
                    .join(&release.manifest.platform_id)
                    .join(subdirectory);
                std::fs::create_dir_all(&directory)?;
                let destination = directory.join(format!("{stem}.{extension}"));
                if destination.is_file() {
                    let (_, existing_hash) = sha256_file(&destination, &cancelled)
                        .map_err(|error| CliError::other(error.to_string()))?;
                    if existing_hash == asset.manifest.file.sha256 {
                        continue;
                    }
                }
                let token = BuildId::new();
                let temporary = directory.join(format!(".{stem}.{token}.tmp"));
                std::fs::copy(&source, &temporary)?;
                let (size, sha256) = sha256_file(&temporary, &cancelled)
                    .map_err(|error| CliError::other(error.to_string()))?;
                if size != asset.manifest.file.size || sha256 != asset.manifest.file.sha256 {
                    let _ = std::fs::remove_file(&temporary);
                    return Err(CliError::other(format!(
                        "projected asset did not match archive evidence: {}",
                        source.display()
                    )));
                }
                if destination.exists() {
                    let backup = directory.join(format!(".{stem}.{token}.backup"));
                    std::fs::rename(&destination, &backup)?;
                    if let Err(error) = std::fs::rename(&temporary, &destination) {
                        let _ = std::fs::rename(&backup, &destination);
                        return Err(error.into());
                    }
                    std::fs::remove_file(backup)?;
                } else {
                    std::fs::rename(&temporary, &destination)?;
                }
                projected += 1;
            }
        }
    }
    log::info!("Projected {projected} frontend asset file(s) from the archive");
    Ok(())
}

fn run_adopt_playable(
    ctx: &retro_junk_lib::AnalysisContext,
    archive_root: PathBuf,
    playable_root: PathBuf,
    db: Option<PathBuf>,
) -> Result<(), CliError> {
    let snapshot =
        scan_archive(&archive_root).map_err(|error| CliError::other(error.to_string()))?;
    let database_path = match db {
        Some(path) => path,
        None => retro_junk_lib::settings::ensure_catalog_database_location()?,
    };
    let mut connection = retro_junk_db::open_database(&database_path)
        .map_err(|error| CliError::database(error.to_string()))?;
    let mut files = Vec::new();
    collect_playable_files(&playable_root, &playable_root, &mut files)?;
    let known_outputs = snapshot
        .releases
        .iter()
        .flat_map(|release| &release.physical_copies)
        .flat_map(|item| &item.carriers)
        .flat_map(|medium| &medium.dumps)
        .flat_map(|dump| &dump.builds)
        .map(|build| build.evidence.relative_output_path.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let cancelled = AtomicBool::new(false);
    let mut inbox = Vec::new();
    let mut adopted = 0_usize;
    for path in files {
        let relative = retro_junk_archive::normalize_relative_path(
            path.strip_prefix(&playable_root).unwrap_or(&path),
        )
        .map_err(|error| CliError::other(error.to_string()))?;
        if known_outputs.contains(relative.as_str()) {
            continue;
        }
        let digests = retro_junk_archive::hash_file_digests(&path, &cancelled)
            .map_err(|error| CliError::other(error.to_string()))?;
        let masters = snapshot
            .releases
            .iter()
            .flat_map(|release| {
                release.physical_copies.iter().flat_map(move |item| {
                    item.carriers.iter().flat_map(move |medium| {
                        medium.dumps.iter().map(move |dump| (release, medium, dump))
                    })
                })
            })
            .filter(|(_, _, dump)| {
                dump.manifest.files.len() == 1
                    && dump.manifest.files[0].size == digests.size
                    && dump.manifest.files[0].sha256 == digests.sha256
            })
            .collect::<Vec<_>>();
        if let [(release, medium, dump)] = masters.as_slice() {
            let catalog_verified = dump.verifications.iter().any(|verification| {
                verification.evidence.input_manifest_sha256 == dump.manifest_sha256
                    && verification.evidence.kind == VerificationKind::Catalog
                    && verification.evidence.outcome == VerificationOutcome::Verified
            });
            let build_id = BuildId::new();
            let child_representation_id = RepresentationId::new();
            let evidence = BuildEvidence {
                schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
                build_id,
                parent_representation_id: dump.manifest.representation_id,
                child_representation_id,
                performed_at: chrono::Utc::now().to_rfc3339(),
                input_manifest_sha256: dump.manifest_sha256.clone(),
                recipe_version: 1,
                format: dump.manifest.format.clone(),
                relative_output_path: relative,
                output_sha256: digests.sha256.clone(),
                output_size: digests.size,
                catalog_verified,
                round_trip_verified: true,
                tool: None,
                omitted_features: Vec::new(),
                canonical_intermediate: None,
            };
            let evidence_directory = dump.directory.join("evidence");
            std::fs::create_dir_all(&evidence_directory)?;
            write_json_new(
                &evidence_directory.join(format!("build-{build_id}.json")),
                &evidence,
            )
            .map_err(|error| CliError::other(error.to_string()))?;
            retro_junk_db::bind_library_entries_by_hash(
                &connection,
                &release.manifest.platform_id,
                &digests,
                &medium.manifest.catalog_binding.catalog_media_id,
                None,
                "archive_adoption",
            )
            .map_err(|error| CliError::database(error.to_string()))?;
            adopted += 1;
            continue;
        }
        if masters.len() > 1 {
            inbox.push(PlayableInboxEntry {
                relative_path: relative,
                status: "ambiguous_archive_master".to_owned(),
                detail: format!("bytes match {} archived masters", masters.len()),
            });
            continue;
        }
        let platform = std::path::Path::new(&relative)
            .components()
            .next()
            .and_then(|component| component.as_os_str().to_str())
            .unwrap_or_default();
        let catalog_digests = if let Some(console) = ctx.get_by_short_name(platform) {
            let mut input = std::fs::File::open(&path)?;
            let hashes = retro_junk_lib::hasher::compute_all_hashes(
                &mut input,
                console.analyzer.as_ref(),
                Some(&path),
            )
            .map_err(|error| CliError::other(error.to_string()))?;
            retro_junk_archive::FileDigests {
                size: hashes.data_size,
                crc32: hashes.crc32,
                md5: hashes.md5.unwrap_or_default(),
                sha1: hashes.sha1.unwrap_or_default(),
                sha256: digests.sha256.clone(),
            }
        } else {
            digests.clone()
        };
        let catalog = retro_junk_db::match_catalog_file(&connection, platform, &catalog_digests)
            .map_err(|error| CliError::database(error.to_string()))?;
        let (status, detail) = match catalog.as_slice() {
            [matched] => {
                retro_junk_db::bind_library_entries_by_hash(
                    &connection,
                    platform,
                    &catalog_digests,
                    &matched.media_id,
                    None,
                    "catalog_adoption",
                )
                .map_err(|error| CliError::database(error.to_string()))?;
                (
                    "catalog_only",
                    format!(
                        "matches {} ({}) but has no preservation master",
                        matched.game, matched.media_id
                    ),
                )
            }
            [] => ("unmatched", "no archive master or catalog match".to_owned()),
            _ => (
                "ambiguous_catalog",
                format!("matches {} catalog media", catalog.len()),
            ),
        };
        inbox.push(PlayableInboxEntry {
            relative_path: relative,
            status: status.to_owned(),
            detail,
        });
    }
    let state_directory = archive_root.join(".retro-junk");
    std::fs::create_dir_all(&state_directory)?;
    retro_junk_archive::write_toml_atomic(
        &state_directory.join("playable-inbox.toml"),
        &PlayableInbox {
            generated_at: chrono::Utc::now().to_rfc3339(),
            playable_root: playable_root.display().to_string(),
            entries: inbox,
        },
    )
    .map_err(|error| CliError::other(error.to_string()))?;
    let refreshed =
        scan_archive(&archive_root).map_err(|error| CliError::other(error.to_string()))?;
    retro_junk_db::reconcile_archive_snapshot(
        &mut connection,
        &refreshed,
        &playable_root,
        &archive_root.join(".retro-junk/work"),
    )
    .map_err(|error| CliError::database(error.to_string()))?;
    log::info!("Adopted {adopted} byte-identical playable file(s)");
    log::info!(
        "Wrote unresolved playable inventory to {}",
        state_directory.join("playable-inbox.toml").display()
    );
    Ok(())
}

fn collect_playable_files(
    root: &std::path::Path,
    directory: &std::path::Path,
    output: &mut Vec<PathBuf>,
) -> Result<(), CliError> {
    let mut entries = std::fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::path);
    for entry in entries {
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            log::warn!("Skipping symbolic link during adoption: {}", path.display());
        } else if metadata.is_dir() {
            if path
                .strip_prefix(root)
                .ok()
                .and_then(|relative| relative.components().next())
                .is_some_and(|component| component.as_os_str() == ".retro-junk")
            {
                continue;
            }
            collect_playable_files(root, &path, output)?;
        } else if metadata.is_file() {
            output.push(path);
        }
    }
    Ok(())
}

fn run_recover(archive_root: PathBuf) -> Result<(), CliError> {
    let mut abandoned = Vec::new();
    collect_abandoned_work(&archive_root, &archive_root, &mut abandoned)?;
    abandoned.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    let quarantine = archive_root.join(".retro-junk/recovery");
    std::fs::create_dir_all(&quarantine)?;
    let mut moved = 0_usize;
    for source in abandoned {
        if !source.exists() || source.starts_with(&quarantine) {
            continue;
        }
        let name = source
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("abandoned-work");
        let destination = quarantine.join(format!("{}-{name}", BuildId::new()));
        std::fs::rename(&source, &destination)?;
        log::info!(
            "Quarantined {} as {}",
            source.display(),
            destination.display()
        );
        moved += 1;
    }
    log::info!("Quarantined {moved} abandoned staging/work directorie(s); no data was deleted");
    Ok(())
}

fn collect_abandoned_work(
    archive_root: &std::path::Path,
    directory: &std::path::Path,
    output: &mut Vec<PathBuf>,
) -> Result<(), CliError> {
    if directory == archive_root.join(".retro-junk/recovery") {
        return Ok(());
    }
    let mut entries = std::fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::path);
    for entry in entries {
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if name.contains("staging")
            || name.starts_with("redumper-audit-")
            || name.starts_with("rvz-build-")
        {
            output.push(path);
        } else {
            collect_abandoned_work(archive_root, &path, output)?;
        }
    }
    Ok(())
}

fn frontend_asset_subdirectory(asset_type: &str) -> Option<&'static str> {
    match asset_type.trim().to_ascii_lowercase().as_str() {
        "cover" | "box-front" => Some("covers"),
        "3d box" | "cover3d" | "cover-3d" => Some("3dboxes"),
        "screenshot" => Some("screenshots"),
        "title screen" | "titlescreen" => Some("titlescreens"),
        "marquee" => Some("marquees"),
        "video" => Some("videos"),
        "fanart" => Some("fanart"),
        "physical media" | "physicalmedia" => Some("physicalmedia"),
        "miximage" => Some("miximages"),
        _ => None,
    }
}

fn run_default_policy(
    archive_root: PathBuf,
    platform: &str,
    format: Option<&str>,
    clear: bool,
    retain_intermediate: bool,
    allow_unverified: bool,
) -> Result<(), CliError> {
    let path = archive_root.join("retro-junk-archive.toml");
    let mut manifest: ArchiveRootManifest =
        retro_junk_archive::read_toml(&path).map_err(|error| CliError::other(error.to_string()))?;
    manifest
        .platform_defaults
        .retain(|default| default.platform_id != platform);
    if !clear {
        manifest
            .platform_defaults
            .push(retro_junk_archive::PlatformPlayableDefault {
                platform_id: platform.to_owned(),
                policy: desired_policy(format, retain_intermediate, allow_unverified)?,
            });
        manifest
            .platform_defaults
            .sort_by(|a, b| a.platform_id.cmp(&b.platform_id));
    }
    write_toml_atomic(&path, &manifest).map_err(|error| CliError::other(error.to_string()))?;
    log::info!(
        "{} playable default for {platform}",
        if clear { "Cleared" } else { "Set" }
    );
    Ok(())
}

fn run_policy(
    archive_root: PathBuf,
    carrier_id: retro_junk_archive::CarrierId,
    format: Option<&str>,
    clear: bool,
    retain_intermediate: bool,
    allow_unverified: bool,
) -> Result<(), CliError> {
    let snapshot =
        scan_archive(&archive_root).map_err(|error| CliError::other(error.to_string()))?;
    let medium = snapshot
        .releases
        .iter()
        .flat_map(|release| &release.physical_copies)
        .flat_map(|item| &item.carriers)
        .find(|carrier| carrier.manifest.carrier_id == carrier_id)
        .ok_or_else(|| CliError::other(format!("carrier {carrier_id} was not found")))?;
    let mut manifest = medium.manifest.clone();
    manifest.playable_policy = if clear {
        None
    } else {
        Some(desired_policy(
            format,
            retain_intermediate,
            allow_unverified,
        )?)
    };
    write_toml_atomic(&medium.directory.join("carrier.toml"), &manifest)
        .map_err(|error| CliError::other(error.to_string()))?;
    if clear {
        log::info!("Cleared playable policy for {carrier_id}");
    } else {
        log::info!(
            "Flagged {carrier_id} for {:?}",
            manifest.playable_policy.unwrap().format
        );
    }
    Ok(())
}

fn run_mirror(
    archive_root: PathBuf,
    playable_root: PathBuf,
    dump_id: &str,
) -> Result<(), CliError> {
    let snapshot =
        scan_archive(&archive_root).map_err(|error| CliError::other(error.to_string()))?;
    let selected = snapshot.releases.iter().find_map(|release| {
        release.physical_copies.iter().find_map(|item| {
            item.carriers.iter().find_map(|medium| {
                medium
                    .dumps
                    .iter()
                    .find(|dump| dump.manifest.dump_id.to_string() == dump_id)
                    .map(|dump| (release, medium, dump))
            })
        })
    });
    let Some((release, medium, dump)) = selected else {
        return Err(CliError::other(format!(
            "archive dump {dump_id} was not found"
        )));
    };
    let [file] = dump.manifest.files.as_slice() else {
        return Err(CliError::other(
            "direct mirroring requires a preservation master containing exactly one file",
        ));
    };
    let source = dump.directory.join("raw").join(&file.path);
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("rom");
    let output_directory = playable_root.join(slugify(&release.manifest.platform_id));
    std::fs::create_dir_all(&output_directory)?;
    let output = output_directory.join(format!(
        "{}.{}",
        playable_output_stem(release, medium),
        extension
    ));
    if output.exists() {
        return Err(CliError::other(format!(
            "playable output already exists: {}",
            output.display()
        )));
    }
    let build_id = BuildId::new();
    let temporary = output_directory.join(format!(".{build_id}.mirror.tmp"));
    std::fs::copy(&source, &temporary)?;
    let cancelled = AtomicBool::new(false);
    let (output_size, output_sha256) =
        sha256_file(&temporary, &cancelled).map_err(|error| CliError::other(error.to_string()))?;
    if output_size != file.size || output_sha256 != file.sha256 {
        let _ = std::fs::remove_file(&temporary);
        return Err(CliError::other(
            "mirrored bytes did not match the preservation manifest",
        ));
    }
    std::fs::rename(&temporary, &output)?;
    let catalog_verified = dump.verifications.iter().any(|verification| {
        verification.evidence.input_manifest_sha256 == dump.manifest_sha256
            && verification.evidence.kind == VerificationKind::Catalog
            && verification.evidence.outcome == VerificationOutcome::Verified
    });
    let evidence = BuildEvidence {
        schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
        build_id,
        parent_representation_id: dump.manifest.representation_id,
        child_representation_id: RepresentationId::new(),
        performed_at: chrono::Utc::now().to_rfc3339(),
        input_manifest_sha256: dump.manifest_sha256.clone(),
        recipe_version: 1,
        format: dump.manifest.format.clone(),
        relative_output_path: output
            .strip_prefix(&playable_root)
            .unwrap_or(&output)
            .to_string_lossy()
            .replace('\\', "/"),
        output_sha256,
        output_size,
        catalog_verified,
        round_trip_verified: true,
        tool: None,
        omitted_features: Vec::new(),
        canonical_intermediate: None,
    };
    let evidence_directory = dump.directory.join("evidence");
    std::fs::create_dir_all(&evidence_directory)?;
    if let Err(error) = write_json_new(
        &evidence_directory.join(format!("build-{build_id}.json")),
        &evidence,
    ) {
        let _ = std::fs::remove_file(&output);
        return Err(CliError::other(error.to_string()));
    }
    log::info!("Mirrored and byte-verified {}", output.display());
    Ok(())
}

fn run_catalog_verify(
    ctx: &retro_junk_lib::AnalysisContext,
    archive_root: PathBuf,
    db: Option<PathBuf>,
    dump_id: Option<&str>,
) -> Result<(), CliError> {
    let snapshot =
        scan_archive(&archive_root).map_err(|error| CliError::other(error.to_string()))?;
    let database_path = match db {
        Some(path) => path,
        None => retro_junk_lib::settings::ensure_catalog_database_location()?,
    };
    let connection = retro_junk_db::open_database(&database_path)
        .map_err(|error| CliError::database(error.to_string()))?;
    let cancelled = AtomicBool::new(false);
    let mut selected = 0_usize;
    let mut verified = 0_usize;
    for (release, medium, dump) in snapshot
        .releases
        .iter()
        .flat_map(|release| {
            release.physical_copies.iter().flat_map(move |item| {
                item.carriers.iter().flat_map(move |medium| {
                    medium.dumps.iter().map(move |dump| (release, medium, dump))
                })
            })
        })
        .filter(|(_, _, dump)| dump_id.is_none_or(|id| dump.manifest.dump_id.to_string() == id))
    {
        let [file] = dump.manifest.files.as_slice() else {
            if dump_id.is_some() {
                return Err(CliError::other(
                    "general catalog verification requires a single-file dump; use the Redumper audit for raw multi-track discs",
                ));
            }
            continue;
        };
        selected += 1;
        let input_path = dump.directory.join("raw").join(&file.path);
        let raw = retro_junk_archive::hash_file_digests(&input_path, &cancelled)
            .map_err(|error| CliError::other(error.to_string()))?;
        let actual = if let Some(console) = ctx.get_by_short_name(&release.manifest.platform_id) {
            let mut input = std::fs::File::open(&input_path)?;
            let hashes = retro_junk_lib::hasher::compute_all_hashes(
                &mut input,
                console.analyzer.as_ref(),
                Some(&input_path),
            )
            .map_err(|error| CliError::other(error.to_string()))?;
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
            retro_junk_db::match_catalog_file(&connection, &release.manifest.platform_id, &actual)
                .map_err(|error| CliError::database(error.to_string()))?;
        let (outcome, catalog, detail) = match matches.as_slice() {
            [catalog_match] => {
                verified += 1;
                let mut media_manifest = medium.manifest.clone();
                media_manifest
                    .catalog_binding
                    .catalog_media_id
                    .clone_from(&catalog_match.media_id);
                media_manifest
                    .catalog_binding
                    .catalog_release_id
                    .clone_from(&catalog_match.release_id);
                media_manifest
                    .catalog_binding
                    .source
                    .clone_from(&catalog_match.source);
                media_manifest
                    .catalog_binding
                    .dat_name
                    .clone_from(&catalog_match.game);
                media_manifest
                    .catalog_binding
                    .source_version
                    .clone_from(&catalog_match.source_version);
                write_toml_atomic(&medium.directory.join("carrier.toml"), &media_manifest)
                    .map_err(|error| CliError::other(error.to_string()))?;
                let mut release_manifest = release.manifest.clone();
                release_manifest
                    .catalog_binding
                    .catalog_release_id
                    .clone_from(&catalog_match.release_id);
                release_manifest
                    .catalog_binding
                    .source
                    .clone_from(&catalog_match.source);
                release_manifest
                    .catalog_binding
                    .dat_name
                    .clone_from(&catalog_match.game);
                release_manifest
                    .catalog_binding
                    .source_version
                    .clone_from(&catalog_match.source_version);
                write_toml_atomic(&release.directory.join("release.toml"), &release_manifest)
                    .map_err(|error| CliError::other(error.to_string()))?;
                (
                    VerificationOutcome::Verified,
                    Some(retro_junk_archive::CatalogEvidence {
                        source: catalog_match.source.clone(),
                        system: release.manifest.platform_id.clone(),
                        version: catalog_match.source_version.clone(),
                        game: catalog_match.game.clone(),
                        complete_track_set: true,
                    }),
                    format!(
                        "File hashes matched catalog media {}",
                        catalog_match.media_id
                    ),
                )
            }
            [] => (
                VerificationOutcome::Unmatched,
                None,
                "No catalog file matched size and available CRC32/MD5/SHA-1 hashes".to_owned(),
            ),
            _ => (
                VerificationOutcome::Ambiguous,
                None,
                format!("File hashes matched {} catalog media", matches.len()),
            ),
        };
        let verification_id = VerificationId::new();
        let evidence = VerificationEvidence {
            schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
            verification_id,
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
                expected_sha1: matches
                    .first()
                    .map_or_else(String::new, |_| actual.sha1.clone()),
                actual_sha1: actual.sha1,
                matched: matches.len() == 1,
            }],
            detail,
        };
        let evidence_directory = dump.directory.join("evidence");
        std::fs::create_dir_all(&evidence_directory)?;
        write_json_new(
            &evidence_directory.join(format!("verification-{verification_id}.json")),
            &evidence,
        )
        .map_err(|error| CliError::other(error.to_string()))?;
        log::info!("{}: {:?}", dump.manifest.dump_id, evidence.outcome);
    }
    if selected == 0 {
        return Err(CliError::other("no matching single-file dumps found"));
    }
    log::info!("Catalog verified {verified} of {selected} single-file dump(s)");
    Ok(())
}

fn run_redumper_audit(
    archive_root: PathBuf,
    dump_id: Option<&str>,
    workspace_root: Option<PathBuf>,
    redumper_path: Option<PathBuf>,
    db: Option<PathBuf>,
) -> Result<(), CliError> {
    let snapshot =
        scan_archive(&archive_root).map_err(|error| CliError::other(error.to_string()))?;
    let redumper = Redumper::detect(
        redumper_path
            .as_deref()
            .unwrap_or_else(|| std::path::Path::new("")),
    )
    .map_err(|error| CliError::external_tool(error.to_string()))?;
    let workspace_root =
        workspace_root.unwrap_or_else(|| archive_root.join(".retro-junk").join("work"));
    let database_path = match db {
        Some(path) => path,
        None => retro_junk_lib::settings::ensure_catalog_database_location()?,
    };
    let connection = retro_junk_db::open_database(&database_path)
        .map_err(|error| CliError::database(error.to_string()))?;
    let cancelled = AtomicBool::new(false);
    let mut selected = 0_usize;
    let mut failed = 0_usize;
    for (release, medium, dump) in snapshot
        .releases
        .iter()
        .flat_map(|release| {
            release.physical_copies.iter().flat_map(move |item| {
                item.carriers.iter().flat_map(move |medium| {
                    medium.dumps.iter().map(move |dump| (release, medium, dump))
                })
            })
        })
        .filter(|(_, _, dump)| dump.manifest.format == RepresentationFormat::RedumperRaw)
        .filter(|(_, _, dump)| dump_id.is_none_or(|id| dump.manifest.dump_id.to_string() == id))
    {
        selected += 1;
        let verification_id = VerificationId::new();
        let audit = redumper.audit(&dump.directory.join("raw"), &workspace_root, &cancelled);
        let (reproduced, outcome, tool, catalog, tracks, detail) = match audit {
            Ok(audit) => {
                let matches = retro_junk_db::match_complete_catalog_media(
                    &connection,
                    &release.manifest.platform_id,
                    &audit.tracks,
                )
                .map_err(|error| CliError::database(error.to_string()))?;
                let is_unique_match = matches.len() == 1;
                let tracks = audit
                    .tracks
                    .iter()
                    .map(|track| TrackVerification {
                        number: track.number,
                        size: track.size,
                        expected_sha1: if is_unique_match {
                            track.sha1.clone()
                        } else {
                            String::new()
                        },
                        actual_sha1: track.sha1.clone(),
                        matched: is_unique_match,
                    })
                    .collect();
                let (outcome, catalog, detail) = match matches.as_slice() {
                    [catalog_match] => {
                        let mut media_manifest = medium.manifest.clone();
                        media_manifest.catalog_binding.catalog_media_id.clone_from(&catalog_match.media_id);
                        media_manifest.catalog_binding.catalog_release_id.clone_from(&catalog_match.release_id);
                        media_manifest.catalog_binding.source.clone_from(&catalog_match.source);
                        media_manifest.catalog_binding.dat_name.clone_from(&catalog_match.game);
                        media_manifest.catalog_binding.source_version.clone_from(&catalog_match.source_version);
                        media_manifest.catalog_binding.expected_tracks.clone_from(&audit.tracks);
                        write_toml_atomic(&medium.directory.join("carrier.toml"), &media_manifest)
                            .map_err(|error| CliError::other(error.to_string()))?;
                        let mut release_manifest = release.manifest.clone();
                        release_manifest.catalog_binding.catalog_release_id.clone_from(&catalog_match.release_id);
                        release_manifest.catalog_binding.source.clone_from(&catalog_match.source);
                        release_manifest.catalog_binding.dat_name.clone_from(&catalog_match.game);
                        release_manifest.catalog_binding.source_version.clone_from(&catalog_match.source_version);
                        write_toml_atomic(&release.directory.join("release.toml"), &release_manifest)
                            .map_err(|error| CliError::other(error.to_string()))?;
                        (
                            VerificationOutcome::Verified,
                            Some(retro_junk_archive::CatalogEvidence {
                                source: catalog_match.source.clone(),
                                system: release.manifest.platform_id.clone(),
                                version: catalog_match.source_version.clone(),
                                game: catalog_match.game.clone(),
                                complete_track_set: true,
                            }),
                            format!("Complete track set matched catalog media {}", catalog_match.media_id),
                        )
                    }
                    [] => (
                        VerificationOutcome::Unmatched,
                        None,
                        "Raw master reproduced a track set, but no complete catalog match was found".to_owned(),
                    ),
                    _ => (
                        VerificationOutcome::Ambiguous,
                        None,
                        format!("Raw master reproduced a track set matching {} catalog media", matches.len()),
                    ),
                };
                (true, outcome, Some(audit.tool), catalog, tracks, detail)
            }
            Err(error) => {
                failed += 1;
                (
                    false,
                    VerificationOutcome::Failed,
                    None,
                    None,
                    Vec::new(),
                    error.to_string(),
                )
            }
        };
        let evidence_dir = dump.directory.join("evidence");
        std::fs::create_dir_all(&evidence_dir)?;
        if reproduced {
            let reproduction_id = VerificationId::new();
            let reproduction = VerificationEvidence {
                schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
                verification_id: reproduction_id,
                representation_id: dump.manifest.representation_id,
                performed_at: chrono::Utc::now().to_rfc3339(),
                input_manifest_sha256: dump.manifest_sha256.clone(),
                kind: VerificationKind::Reproduction,
                outcome: VerificationOutcome::Verified,
                tool: tool.clone(),
                catalog: None,
                tracks: tracks
                    .iter()
                    .cloned()
                    .map(|mut track| {
                        track.expected_sha1.clear();
                        track.matched = false;
                        track
                    })
                    .collect(),
                detail: "Redumper regenerated and hashed a complete track set from the raw master"
                    .to_owned(),
            };
            write_json_new(
                &evidence_dir.join(format!("verification-{reproduction_id}.json")),
                &reproduction,
            )
            .map_err(|error| CliError::other(error.to_string()))?;
        }
        let evidence = VerificationEvidence {
            schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
            verification_id,
            representation_id: dump.manifest.representation_id,
            performed_at: chrono::Utc::now().to_rfc3339(),
            input_manifest_sha256: dump.manifest_sha256.clone(),
            kind: if reproduced {
                VerificationKind::Catalog
            } else {
                VerificationKind::Reproduction
            },
            outcome,
            tool,
            catalog,
            tracks,
            detail,
        };
        write_json_new(
            &evidence_dir.join(format!("verification-{verification_id}.json")),
            &evidence,
        )
        .map_err(|error| CliError::other(error.to_string()))?;
        log::info!("{}: {:?}", dump.manifest.dump_id, evidence.outcome);
    }
    if selected == 0 {
        return Err(CliError::other("no matching Redumper raw dumps found"));
    }
    if failed > 0 {
        Err(CliError::other(format!(
            "{failed} of {selected} Redumper audit(s) failed; evidence was recorded"
        )))
    } else {
        log::info!("Audited {selected} Redumper raw dump(s)");
        Ok(())
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn run_build_rvz(
    archive_root: PathBuf,
    playable_root: PathBuf,
    dump_id: &str,
    workspace_root: Option<PathBuf>,
    dolphin_tool_path: Option<PathBuf>,
    allow_unverified: bool,
    options: &std::collections::BTreeMap<String, String>,
) -> Result<(), CliError> {
    let snapshot =
        scan_archive(&archive_root).map_err(|error| CliError::other(error.to_string()))?;
    let selected = snapshot.releases.iter().find_map(|release| {
        release.physical_copies.iter().find_map(|item| {
            item.carriers.iter().find_map(|medium| {
                medium
                    .dumps
                    .iter()
                    .find(|dump| dump.manifest.dump_id.to_string() == dump_id)
                    .map(|dump| (release, medium, dump))
            })
        })
    });
    let Some((release, medium, dump)) = selected else {
        return Err(CliError::other(format!(
            "archive dump {dump_id} was not found"
        )));
    };
    if dump.manifest.format != RepresentationFormat::Iso || dump.manifest.files.len() != 1 {
        return Err(CliError::other(
            "RVZ builds require a single-file ISO preservation master",
        ));
    }
    let catalog_verified = dump.verifications.iter().any(|verification| {
        verification.evidence.input_manifest_sha256 == dump.manifest_sha256
            && verification.evidence.kind == VerificationKind::Catalog
            && verification.evidence.outcome == VerificationOutcome::Verified
    });
    if !catalog_verified && !allow_unverified {
        return Err(CliError::other(
            "dump has no current catalog verification; verify it first or allow unverified builds",
        ));
    }
    let dolphin_tool = dolphin_tool_path.unwrap_or_else(|| PathBuf::from("DolphinTool"));
    let help = std::process::Command::new(&dolphin_tool)
        .arg("--help")
        .output()
        .map_err(|error| CliError::external_tool(error.to_string()))?;
    let banner = format!(
        "{}\n{}",
        String::from_utf8_lossy(&help.stdout),
        String::from_utf8_lossy(&help.stderr)
    );
    if !help.status.success()
        || (!banner.to_ascii_lowercase().contains("dolphin")
            && !banner.to_ascii_lowercase().contains("convert"))
    {
        return Err(CliError::external_tool(
            "DolphinTool did not provide a recognized help response",
        ));
    }
    let output_directory = playable_root.join(slugify(&release.manifest.platform_id));
    std::fs::create_dir_all(&output_directory)?;
    let output = output_directory.join(format!("{}.rvz", playable_output_stem(release, medium)));
    if output.exists() {
        return Err(CliError::other(format!(
            "playable output already exists: {}",
            output.display()
        )));
    }
    let build_id = BuildId::new();
    let temporary_output = output_directory.join(format!(".{build_id}.rvz.tmp"));
    let workspace_root = workspace_root.unwrap_or_else(|| archive_root.join(".retro-junk/work"));
    let workspace = workspace_root.join(format!("rvz-build-{build_id}"));
    std::fs::create_dir_all(&workspace)?;
    let round_trip = workspace.join("round-trip.iso");
    let input = dump
        .directory
        .join("raw")
        .join(&dump.manifest.files[0].path);
    let block_size = options.get("block_size").map_or("131072", String::as_str);
    let compression = options.get("compression").map_or("zstd", String::as_str);
    let level = options.get("compression_level").map_or("5", String::as_str);
    let convert = std::process::Command::new(&dolphin_tool)
        .args(["convert", "-i"])
        .arg(&input)
        .arg("-o")
        .arg(&temporary_output)
        .args([
            "-f",
            "rvz",
            "-b",
            block_size,
            "-c",
            compression,
            "-l",
            level,
        ])
        .output()
        .map_err(|error| CliError::external_tool(error.to_string()))?;
    if !convert.status.success() {
        let _ = std::fs::remove_file(&temporary_output);
        let _ = std::fs::remove_dir_all(&workspace);
        return Err(CliError::external_tool(format!(
            "DolphinTool RVZ conversion failed: {}",
            String::from_utf8_lossy(&convert.stderr)
        )));
    }
    let extract = std::process::Command::new(&dolphin_tool)
        .args(["convert", "-i"])
        .arg(&temporary_output)
        .arg("-o")
        .arg(&round_trip)
        .args(["-f", "iso"])
        .output()
        .map_err(|error| CliError::external_tool(error.to_string()))?;
    let cancelled = AtomicBool::new(false);
    let verified = if extract.status.success() {
        let (_, original_sha256) =
            sha256_file(&input, &cancelled).map_err(|error| CliError::other(error.to_string()))?;
        let (_, round_trip_sha256) = sha256_file(&round_trip, &cancelled)
            .map_err(|error| CliError::other(error.to_string()))?;
        original_sha256 == round_trip_sha256
    } else {
        false
    };
    let _ = std::fs::remove_dir_all(&workspace);
    if !verified {
        let _ = std::fs::remove_file(&temporary_output);
        return Err(CliError::other(
            "RVZ round-trip ISO did not match the preservation master",
        ));
    }
    let (output_size, output_sha256) = sha256_file(&temporary_output, &cancelled)
        .map_err(|error| CliError::other(error.to_string()))?;
    std::fs::rename(&temporary_output, &output)?;
    let evidence = BuildEvidence {
        schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
        build_id,
        parent_representation_id: dump.manifest.representation_id,
        child_representation_id: RepresentationId::new(),
        performed_at: chrono::Utc::now().to_rfc3339(),
        input_manifest_sha256: dump.manifest_sha256.clone(),
        recipe_version: 1,
        format: RepresentationFormat::Rvz,
        relative_output_path: output
            .strip_prefix(&playable_root)
            .unwrap_or(&output)
            .to_string_lossy()
            .replace('\\', "/"),
        output_sha256,
        output_size,
        catalog_verified,
        round_trip_verified: true,
        tool: Some(retro_junk_archive::ToolRecord {
            name: "DolphinTool".to_owned(),
            version: banner.lines().next().unwrap_or_default().trim().to_owned(),
            build: String::new(),
        }),
        omitted_features: Vec::new(),
        canonical_intermediate: None,
    };
    let evidence_directory = dump.directory.join("evidence");
    std::fs::create_dir_all(&evidence_directory)?;
    if let Err(error) = write_json_new(
        &evidence_directory.join(format!("build-{build_id}.json")),
        &evidence,
    ) {
        let _ = std::fs::remove_file(&output);
        return Err(CliError::other(error.to_string()));
    }
    log::info!("Built and round-trip verified {}", output.display());
    Ok(())
}

fn playable_output_stem(
    release: &retro_junk_archive::IndexedRelease,
    medium: &retro_junk_archive::IndexedCarrier,
) -> String {
    let mut name = release_output_name(release);
    if medium.manifest.sequence_number > 0 {
        let _ = write!(name, " (Disc {})", medium.manifest.sequence_number);
    }
    slugify(&name)
}

fn release_output_name(release: &retro_junk_archive::IndexedRelease) -> String {
    let mut name = release.manifest.title.clone();
    for value in [
        &release.manifest.region,
        &release.manifest.revision,
        &release.manifest.variant,
    ] {
        if !value.is_empty() {
            let _ = write!(name, " ({value})");
        }
    }
    name
}

fn project_playlists(
    archive_root: &std::path::Path,
    playable_root: &std::path::Path,
) -> Result<usize, CliError> {
    let snapshot =
        scan_archive(archive_root).map_err(|error| CliError::other(error.to_string()))?;
    let defaults = snapshot
        .manifest
        .platform_defaults
        .iter()
        .map(|default| (default.platform_id.as_str(), &default.policy))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut written = 0_usize;
    for release in &snapshot.releases {
        let mut discs = Vec::new();
        for medium in release
            .physical_copies
            .iter()
            .flat_map(|item| &item.carriers)
        {
            if medium.manifest.sequence_number == 0 {
                continue;
            }
            let Some(policy) = medium
                .manifest
                .playable_policy
                .as_ref()
                .or_else(|| defaults.get(release.manifest.platform_id.as_str()).copied())
            else {
                continue;
            };
            let build = medium
                .dumps
                .iter()
                .flat_map(|dump| {
                    dump.builds
                        .iter()
                        .map(move |build| (dump.manifest_sha256.as_str(), build))
                })
                .find(|(manifest_sha, build)| {
                    build.evidence.format == policy.format
                        && retro_junk_archive::playable_presence(
                            playable_root,
                            manifest_sha,
                            &build.evidence,
                        ) == retro_junk_archive::RepresentationPresence::Present
                });
            if let Some((_, build)) = build {
                discs.push((
                    medium.manifest.sequence_number,
                    build.evidence.relative_output_path.clone(),
                ));
            }
        }
        if discs.len() < 2 {
            continue;
        }
        discs.sort_by_key(|(number, _)| *number);
        let directory = playable_root.join(slugify(&release.manifest.platform_id));
        std::fs::create_dir_all(&directory)?;
        let playlist = directory.join(format!("{}.m3u", slugify(&release_output_name(release))));
        let contents = discs
            .iter()
            .map(|(_, path)| {
                std::path::Path::new(path)
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or(path)
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let temporary = directory.join(format!(".playlist-{}.tmp", BuildId::new()));
        std::fs::write(&temporary, contents)?;
        std::fs::rename(&temporary, &playlist)?;
        written += 1;
    }
    Ok(written)
}

#[allow(clippy::too_many_arguments)]
fn run_build_chd(
    ctx: &retro_junk_lib::AnalysisContext,
    archive_root: PathBuf,
    playable_root: PathBuf,
    dump_id: &str,
    workspace_root: Option<PathBuf>,
    chdman_path: Option<PathBuf>,
    redumper_path: Option<PathBuf>,
    allow_unverified: bool,
    retain_intermediate: bool,
) -> Result<(), CliError> {
    let snapshot =
        scan_archive(&archive_root).map_err(|error| CliError::other(error.to_string()))?;
    let selected = snapshot.releases.iter().find_map(|release| {
        release.physical_copies.iter().find_map(|item| {
            item.carriers.iter().find_map(|medium| {
                medium
                    .dumps
                    .iter()
                    .find(|dump| dump.manifest.dump_id.to_string() == dump_id)
                    .map(|dump| (release, medium, dump))
            })
        })
    });
    let Some((release, medium, dump)) = selected else {
        return Err(CliError::other(format!(
            "archive dump {dump_id} was not found"
        )));
    };
    let catalog_verified = dump.verifications.iter().any(|verification| {
        verification.evidence.input_manifest_sha256 == dump.manifest_sha256
            && verification.evidence.kind == VerificationKind::Catalog
            && verification.evidence.outcome == VerificationOutcome::Verified
    });
    if !catalog_verified && !allow_unverified {
        return Err(CliError::other(
            "dump has no current complete-track catalog verification; audit it first or pass --allow-unverified",
        ));
    }

    let analyzer = ctx
        .get_by_short_name(&release.manifest.platform_id)
        .ok_or_else(|| CliError::unknown_system(release.manifest.platform_id.clone()))?;
    let workspace_root =
        workspace_root.unwrap_or_else(|| archive_root.join(".retro-junk").join("work"));
    let cancelled = AtomicBool::new(false);
    let mut redumper_workspace = None;
    let input = match dump.manifest.format {
        RepresentationFormat::RedumperRaw => {
            let redumper = Redumper::detect(
                redumper_path
                    .as_deref()
                    .unwrap_or_else(|| std::path::Path::new("")),
            )
            .map_err(|error| CliError::external_tool(error.to_string()))?;
            let prepared = redumper
                .prepare(&dump.directory.join("raw"), &workspace_root, &cancelled)
                .map_err(|error| CliError::other(error.to_string()))?;
            let entrypoint = prepared.entrypoint.clone();
            redumper_workspace = Some(prepared);
            entrypoint
        }
        RepresentationFormat::CueBin => find_input(&dump.directory.join("raw"), &["cue"])?,
        RepresentationFormat::Iso => find_input(&dump.directory.join("raw"), &["iso"])?,
        _ => {
            return Err(CliError::other(format!(
                "{:?} cannot be converted to CHD by this workflow",
                dump.manifest.format
            )));
        }
    };

    let mut job = retro_junk_lib::chd_convert::plan_compression(&input, analyzer.analyzer.as_ref())
        .map_err(|error| CliError::other(error.to_string()))?;
    let output_directory = playable_root.join(slugify(&release.manifest.platform_id));
    std::fs::create_dir_all(&output_directory)?;
    job.output = output_directory.join(format!("{}.chd", playable_output_stem(release, medium)));
    if job.output.exists() {
        return Err(CliError::other(format!(
            "playable output already exists: {}",
            job.output.display()
        )));
    }
    let chdman = retro_junk_lib::chd_convert::Chdman::detect(
        chdman_path
            .as_deref()
            .unwrap_or_else(|| std::path::Path::new("")),
    )
    .map_err(|error| CliError::external_tool(error.to_string()))?;
    let outcome =
        retro_junk_lib::chd_convert::compress_to_chd(&chdman, &job, &|_, _| {}, &cancelled)
            .map_err(|error| CliError::other(error.to_string()))?;
    if !outcome.is_verified() {
        return Err(CliError::other(
            "CHD round-trip verification failed; temporary output was discarded",
        ));
    }
    let (output_size, output_sha256) = sha256_file(&outcome.output, &cancelled)
        .map_err(|error| CliError::other(error.to_string()))?;
    let child_representation_id = RepresentationId::new();
    let build_id = BuildId::new();
    let retained_path = dump
        .directory
        .join("intermediates")
        .join(build_id.to_string());
    let canonical_intermediate = if retain_intermediate {
        if let Some(workspace) = redumper_workspace.as_ref() {
            let files = match workspace.retain_intermediate(&retained_path, &cancelled) {
                Ok(files) => files,
                Err(error) => {
                    let _ = std::fs::remove_file(&outcome.output);
                    return Err(CliError::other(error.to_string()));
                }
            };
            let format = if workspace
                .entrypoint
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case("iso"))
            {
                RepresentationFormat::Iso
            } else {
                RepresentationFormat::CueBin
            };
            Some(retro_junk_archive::CanonicalIntermediateEvidence {
                representation_id: RepresentationId::new(),
                format,
                relative_path: format!("intermediates/{build_id}"),
                files,
            })
        } else {
            None
        }
    } else {
        None
    };
    let relative_output_path = outcome
        .output
        .strip_prefix(&playable_root)
        .unwrap_or(&outcome.output)
        .to_string_lossy()
        .replace('\\', "/");
    let evidence = BuildEvidence {
        schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
        build_id,
        parent_representation_id: dump.manifest.representation_id,
        child_representation_id,
        performed_at: chrono::Utc::now().to_rfc3339(),
        input_manifest_sha256: dump.manifest_sha256.clone(),
        recipe_version: 1,
        format: RepresentationFormat::Chd,
        relative_output_path,
        output_sha256,
        output_size,
        catalog_verified,
        round_trip_verified: true,
        tool: Some(retro_junk_archive::ToolRecord {
            name: "chdman".to_owned(),
            version: chdman.version,
            build: String::new(),
        }),
        omitted_features: dump.manifest.captured_features.clone(),
        canonical_intermediate,
    };
    let evidence_directory = dump.directory.join("evidence");
    std::fs::create_dir_all(&evidence_directory)?;
    if let Err(error) = write_json_new(
        &evidence_directory.join(format!("build-{build_id}.json")),
        &evidence,
    ) {
        let _ = std::fs::remove_file(&outcome.output);
        if retained_path.exists() {
            let _ = std::fs::remove_dir_all(&retained_path);
        }
        return Err(CliError::other(error.to_string()));
    }
    drop(redumper_workspace);
    log::info!("Built and round-trip verified {}", outcome.output.display());
    if !catalog_verified {
        log::warn!("Playable CHD was explicitly built without catalog verification");
    }
    Ok(())
}

fn find_input(directory: &std::path::Path, extensions: &[&str]) -> Result<PathBuf, CliError> {
    let mut paths = std::fs::read_dir(directory)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| {
                    extensions
                        .iter()
                        .any(|extension| value.eq_ignore_ascii_case(extension))
                })
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths.into_iter().next().ok_or_else(|| {
        CliError::other(format!(
            "no supported entrypoint found in {}",
            directory.display()
        ))
    })
}

fn run_init(archive_root: PathBuf, name: String) -> Result<(), CliError> {
    let manifest = ArchiveRootManifest::new(name);
    initialize_archive(&archive_root, &manifest)
        .map_err(|error| CliError::other(error.to_string()))?;
    log::info!("Initialized archive at {}", archive_root.display());
    log::info!("Profile ID: {}", manifest.profile_id);
    Ok(())
}

fn run_ingest(
    source: PathBuf,
    archive_root: PathBuf,
    spec: NewCarrierDump,
) -> Result<(), CliError> {
    let cancelled = AtomicBool::new(false);
    let result = ingest_new_carrier_dump(&archive_root, &source, spec, &cancelled, |_| {})
        .map_err(|error| CliError::other(error.to_string()))?;
    log::info!("Archived {}", result.release.title);
    log::info!("Dump: {}", result.dump.dump_id);
    log::info!("Stored at: {}", result.dump_directory.display());
    log::info!(
        "Verified staged copy: {} file(s), {} byte(s)",
        result.dump.files.len(),
        result.dump.files.iter().map(|file| file.size).sum::<u64>()
    );
    log::info!("Source was retained at {}", source.display());
    Ok(())
}

fn run_status(archive_root: PathBuf) -> Result<(), CliError> {
    let snapshot =
        scan_archive(&archive_root).map_err(|error| CliError::other(error.to_string()))?;
    let items = snapshot
        .releases
        .iter()
        .map(|release| release.physical_copies.len())
        .sum::<usize>();
    let media = snapshot
        .releases
        .iter()
        .flat_map(|release| &release.physical_copies)
        .map(|item| item.carriers.len())
        .sum::<usize>();
    let dumps = snapshot
        .releases
        .iter()
        .flat_map(|release| &release.physical_copies)
        .flat_map(|item| &item.carriers)
        .map(|medium| medium.dumps.len())
        .sum::<usize>();
    let policies = snapshot
        .releases
        .iter()
        .flat_map(|release| {
            let has_default = snapshot
                .manifest
                .platform_defaults
                .iter()
                .any(|default| default.platform_id == release.manifest.platform_id);
            release
                .physical_copies
                .iter()
                .flat_map(|item| &item.carriers)
                .filter(move |medium| medium.manifest.playable_policy.is_some() || has_default)
        })
        .count();
    log::info!(
        "{} ({})",
        snapshot.manifest.display_name,
        snapshot.manifest.profile_id
    );
    log::info!("Root: {}", archive_root.display());
    log::info!(
        "{} release(s), {} physical copy/copies, {} carrier(s), {} dump(s)",
        snapshot.releases.len(),
        items,
        media,
        dumps
    );
    log::info!("{policies} media item(s) flagged for a playable representation");
    Ok(())
}

fn run_verify(archive_root: PathBuf) -> Result<(), CliError> {
    let snapshot =
        scan_archive(&archive_root).map_err(|error| CliError::other(error.to_string()))?;
    let cancelled = AtomicBool::new(false);
    let mut checked = 0_usize;
    let mut failed = 0_usize;
    for dump in snapshot
        .releases
        .iter()
        .flat_map(|release| &release.physical_copies)
        .flat_map(|item| &item.carriers)
        .flat_map(|medium| &medium.dumps)
    {
        let report = verify_dump_integrity(&dump.directory, &dump.manifest, &cancelled)
            .map_err(|error| CliError::other(error.to_string()))?;
        checked += 1;
        if !report.is_verified() {
            failed += 1;
        }
        let verification_id = VerificationId::new();
        let evidence = VerificationEvidence {
            schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
            verification_id,
            representation_id: dump.manifest.representation_id,
            performed_at: chrono::Utc::now().to_rfc3339(),
            input_manifest_sha256: dump.manifest_sha256.clone(),
            kind: VerificationKind::Integrity,
            outcome: if report.is_verified() {
                VerificationOutcome::Verified
            } else {
                VerificationOutcome::Failed
            },
            tool: None,
            catalog: None,
            tracks: Vec::new(),
            detail: if report.is_verified() {
                format!(
                    "SHA-256 verified {} stored file(s), {} byte(s)",
                    report.checked_files, report.checked_bytes
                )
            } else {
                report
                    .failures
                    .iter()
                    .map(|failure| format!("{}: {}", failure.path, failure.reason))
                    .collect::<Vec<_>>()
                    .join("; ")
            },
        };
        let evidence_dir = dump.directory.join("evidence");
        std::fs::create_dir_all(&evidence_dir)?;
        write_json_new(
            &evidence_dir.join(format!("verification-{verification_id}.json")),
            &evidence,
        )
        .map_err(|error| CliError::other(error.to_string()))?;
        log::info!(
            "{}: {}",
            dump.manifest.dump_id,
            if report.is_verified() {
                "verified"
            } else {
                "FAILED"
            }
        );
    }
    log::info!("Checked {checked} dump(s); {failed} failed");
    if failed > 0 {
        Err(CliError::other("archive integrity verification failed"))
    } else {
        Ok(())
    }
}

fn run_reindex(
    archive_root: PathBuf,
    playable_root: Option<PathBuf>,
    workspace_root: Option<PathBuf>,
    db: Option<PathBuf>,
) -> Result<(), CliError> {
    let snapshot =
        scan_archive(&archive_root).map_err(|error| CliError::other(error.to_string()))?;
    let playable_root = playable_root.unwrap_or_else(|| {
        archive_root
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("playable")
    });
    let workspace_root =
        workspace_root.unwrap_or_else(|| archive_root.join(".retro-junk").join("work"));
    let db = match db {
        Some(path) => path,
        None => retro_junk_lib::settings::ensure_catalog_database_location()?,
    };
    let mut connection =
        retro_junk_db::open_database(&db).map_err(|error| CliError::database(error.to_string()))?;
    retro_junk_db::reconcile_archive_snapshot(
        &mut connection,
        &snapshot,
        &playable_root,
        &workspace_root,
    )
    .map_err(|error| CliError::database(error.to_string()))?;
    log::info!("Rebuilt archive index in {}", db.display());
    Ok(())
}

fn parse_format(value: &str) -> Result<RepresentationFormat, CliError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "redumper" | "redumper-raw" | "redumper_raw" => Ok(RepresentationFormat::RedumperRaw),
        "rom" => Ok(RepresentationFormat::Rom),
        "cue-bin" | "cue_bin" | "bin-cue" => Ok(RepresentationFormat::CueBin),
        "iso" => Ok(RepresentationFormat::Iso),
        "chd" => Ok(RepresentationFormat::Chd),
        "rvz" => Ok(RepresentationFormat::Rvz),
        other if !other.is_empty() => Ok(RepresentationFormat::Other(other.to_owned())),
        _ => Err(CliError::other("representation format cannot be empty")),
    }
}

fn carrier_kind_for_format(format: &RepresentationFormat) -> retro_junk_archive::CarrierKind {
    match format {
        RepresentationFormat::RedumperRaw
        | RepresentationFormat::CueBin
        | RepresentationFormat::Iso
        | RepresentationFormat::Chd
        | RepresentationFormat::Rvz => retro_junk_archive::CarrierKind::OpticalDisc,
        RepresentationFormat::Rom => retro_junk_archive::CarrierKind::Cartridge,
        RepresentationFormat::Other(_) => retro_junk_archive::CarrierKind::Unknown,
    }
}

fn parse_release_file_category(
    value: &str,
) -> Result<retro_junk_archive::ReleaseFileCategory, CliError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "artwork" => Ok(retro_junk_archive::ReleaseFileCategory::Artwork),
        "video" | "videos" => Ok(retro_junk_archive::ReleaseFileCategory::Video),
        "document" | "documents" => Ok(retro_junk_archive::ReleaseFileCategory::Document),
        "metadata" => Ok(retro_junk_archive::ReleaseFileCategory::Metadata),
        other => Err(CliError::other(format!(
            "unsupported release file category: {other}"
        ))),
    }
}

fn parse_physical_copy_file_category(
    value: &str,
) -> Result<retro_junk_archive::PhysicalCopyFileCategory, CliError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "photo" | "photos" => Ok(retro_junk_archive::PhysicalCopyFileCategory::Photo),
        "provenance" => Ok(retro_junk_archive::PhysicalCopyFileCategory::Provenance),
        "document" | "documents" => Ok(retro_junk_archive::PhysicalCopyFileCategory::Document),
        other => Err(CliError::other(format!(
            "unsupported physical-copy file category: {other}"
        ))),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn rvz_builder_round_trips_before_publishing() {
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("archive");
        retro_junk_archive::initialize_archive(
            &archive,
            &retro_junk_archive::ArchiveRootManifest::new("RVZ test"),
        )
        .unwrap();
        let iso = temp.path().join("game.iso");
        std::fs::write(&iso, b"lossless disc bytes").unwrap();
        let ingested = retro_junk_archive::ingest_new_carrier_dump(
            &archive,
            &iso,
            retro_junk_archive::NewCarrierDump {
                platform_id: "gc".to_owned(),
                title: "Game".to_owned(),
                region: "usa".to_owned(),
                revision: String::new(),
                variant: String::new(),
                owner_id: "default".to_owned(),
                physical_copy_label: String::new(),
                serial: String::new(),
                sequence_number: 1,
                carrier_label: String::new(),
                carrier_kind: retro_junk_archive::CarrierKind::OpticalDisc,
                format: RepresentationFormat::Iso,
                catalog_binding: retro_junk_archive::CatalogBinding::default(),
                source_package: retro_junk_archive::SourcePackageRecord::default(),
                physical_copy_id: None,
            },
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        let tool = temp.path().join("DolphinTool");
        std::fs::write(
            &tool,
            r#"#!/bin/sh
if [ "$1" = "--help" ]; then echo "DolphinTool convert"; exit 0; fi
input=""; output=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -i) input="$2"; shift 2 ;;
    -o) output="$2"; shift 2 ;;
    *) shift ;;
  esac
done
cp "$input" "$output"
"#,
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&tool).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&tool, permissions).unwrap();
        let playable = temp.path().join("playable");
        run_build_rvz(
            archive.clone(),
            playable.clone(),
            &ingested.dump.dump_id.to_string(),
            None,
            Some(tool),
            true,
            &std::collections::BTreeMap::new(),
        )
        .unwrap();
        assert!(playable.join("gc/game-usa-disc-1.rvz").is_file());
        assert_eq!(
            retro_junk_archive::scan_archive(&archive).unwrap().releases[0].physical_copies[0]
                .carriers[0]
                .dumps[0]
                .builds
                .len(),
            1
        );
    }
}
