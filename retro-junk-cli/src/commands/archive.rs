#![allow(clippy::needless_pass_by_value, clippy::too_many_lines)]

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use retro_junk_archive::{
    ArchiveRootManifest, BuildId, NewCarrierDump, RepresentationFormat, ingest_new_carrier_dump,
    initialize_archive, scan_archive, write_toml_atomic,
};
use retro_junk_backend::adoption::{
    AdoptionCandidate, AdoptionCandidateKind, AdoptionSuggestionPayload,
};

use crate::CliError;
use crate::cli_types::ArchiveAction;

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
            make_playable,
            playable_root,
            chdman,
            discard_redundant_bin_cue,
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
            playable_root,
            None,
            make_playable,
            chdman,
            discard_redundant_bin_cue,
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
            media_root,
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
            media_root,
            false,
            None,
            false,
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
                join_release: None,
                source_package: retro_junk_archive::SourcePackageRecord::default(),
                expected_files: Vec::new(),
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
        } => run_shared_single_build(
            archive_root,
            playable_root,
            &dump_id,
            workspace_root,
            chdman,
            redumper,
            None,
            RepresentationFormat::Chd,
            allow_unverified,
            false,
            std::collections::BTreeMap::new(),
        ),
        ArchiveAction::BuildRvz {
            archive_root,
            playable_root,
            dump_id,
            workspace_root,
            dolphin_tool,
            allow_unverified,
        } => run_shared_single_build(
            archive_root,
            playable_root,
            &dump_id,
            workspace_root,
            None,
            None,
            dolphin_tool,
            RepresentationFormat::Rvz,
            allow_unverified,
            false,
            std::collections::BTreeMap::new(),
        ),
        ArchiveAction::Mirror {
            archive_root,
            playable_root,
            dump_id,
        } => run_shared_single_build(
            archive_root,
            playable_root,
            &dump_id,
            None,
            None,
            None,
            None,
            RepresentationFormat::Rom,
            true,
            false,
            std::collections::BTreeMap::new(),
        ),
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
            db,
            media_root,
            metadata_root,
            no_project_assets,
            no_update_gamelists,
            dry_run,
            limit,
        } => {
            // The queue is the shared convergence path now: derive, then
            // execute through the one executor. --dry-run keeps its
            // non-zero-on-blocked contract.
            let mut only = vec![
                "verify-catalog".to_owned(),
                "audit-redumper".to_owned(),
                "build".to_owned(),
            ];
            if !no_project_assets {
                only.push("project".to_owned());
            }
            if !no_update_gamelists {
                only.push("gamelist".to_owned());
            }
            crate::commands::sync::run_sync(crate::cli_types::SyncArgs {
                profile: None,
                archive_root: Some(archive_root),
                playable_root: Some(playable_root),
                workspace_root,
                platform: None,
                release: None,
                only,
                dry_run,
                limit,
                chdman,
                redumper,
                dolphin_tool,
                media_root,
                metadata_root,
                db,
            })
        }
        ArchiveAction::ProjectFrontendFiles {
            archive_root,
            media_root,
        } => run_project_assets(archive_root, media_root),
        ArchiveAction::GenerateMiximages {
            archive_root,
            playable_root,
            media_root,
            workspace_root,
            release_id,
        } => run_generate_miximages(
            archive_root,
            playable_root,
            media_root,
            workspace_root,
            release_id.as_deref(),
        ),
        ArchiveAction::AdoptPlayable {
            archive_root,
            playable_root,
            db,
            release_id,
            dry_run,
        } => run_adopt_playable(ctx, archive_root, playable_root, db, release_id, dry_run),
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
        // A dry run writes nothing, so it has no business taking the write
        // lock — doing so blocked every other reader for as long as the scan
        // took, which for a whole-library sweep is a long time.
        ArchiveAction::AdoptPlayable { dry_run: true, .. }
        // Build routes through the shared executor, which takes the archive
        // lock per action rather than for the whole invocation.
        | ArchiveAction::Init { .. }
        | ArchiveAction::Import { .. }
        | ArchiveAction::ImportPlayable { .. }
        | ArchiveAction::Status { .. }
        | ArchiveAction::Reindex { .. }
        | ArchiveAction::Build { .. }
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
        | ArchiveAction::GenerateMiximages { archive_root, .. }
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
    media_root: Option<PathBuf>,
    make_playable: bool,
    chdman_path: Option<PathBuf>,
    discard_redundant_bin_cue: bool,
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
    let mut plan = retro_junk_archive_import::plan_import(
        retro_junk_archive_import::DumpImportRequest {
            source,
            archive_root: archive_root.clone(),
            platform_hint,
            owner_id,
            new_physical_copy,
            redumper_path,
            workspace_root,
            stage_packages_locally: true,
            playable_root: playable_root.clone(),
            make_playable,
            chdman_path,
            discard_redundant_bin_cue,
        },
        context,
        &connection,
        &cancelled,
        |_, _| {},
        |_| {},
    )
    .map_err(|error| CliError::other(error.to_string()))?;
    resolve_import_choices(&mut plan, dry_run)?;

    let ready = plan
        .candidates
        .iter()
        .filter(|candidate| {
            matches!(
                candidate.disposition,
                retro_junk_archive_import::ImportDisposition::Ready
                    | retro_junk_archive_import::ImportDisposition::ReadyUnbound { .. }
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
            "{} -> {} [{}; {:?}; {}]",
            candidate.source.display(),
            identity,
            import_identification_label(&candidate.identification),
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
    let frontend_asset_candidates = if let Some(playable_root) = playable_root.as_deref() {
        let media_root =
            media_root.unwrap_or_else(|| retro_junk_lib::util::default_media_dir(playable_root));
        plan.candidates
            .iter()
            .filter(|candidate| {
                matches!(
                    candidate.disposition,
                    retro_junk_archive_import::ImportDisposition::Ready
                        | retro_junk_archive_import::ImportDisposition::ReadyUnbound { .. }
                        | retro_junk_archive_import::ImportDisposition::AlreadyArchived { .. }
                )
            })
            .filter_map(|candidate| {
                let selected = candidate.selected_match.as_ref()?;
                let name = candidate.source.file_name()?.to_str()?;
                let frontend_platform_id = candidate
                    .source
                    .strip_prefix(playable_root)
                    .ok()
                    .and_then(|relative| relative.components().next())
                    .and_then(|component| component.as_os_str().to_str())
                    .unwrap_or(&candidate.archive_platform_id)
                    .to_owned();
                let stem =
                    if candidate.source.is_dir() && name.to_ascii_lowercase().ends_with(".m3u") {
                        name.to_owned()
                    } else {
                        candidate
                            .source
                            .file_stem()
                            .and_then(|value| value.to_str())
                            .unwrap_or(name)
                            .to_owned()
                    };
                Some((
                    selected.release_id.clone(),
                    frontend_platform_id,
                    stem,
                    media_root.clone(),
                ))
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let result =
        retro_junk_archive_import::execute_import(plan, consume, &cancelled, |_| {}, |_| {})
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
        for warning in &item.warnings {
            log::warn!("{}: {warning}", item.source.display());
        }
        if let Some(build) = &item.playable_build {
            log::info!(
                "Playable CHD {:?}: {}{}",
                build.outcome,
                build.detail,
                build
                    .output
                    .as_ref()
                    .map_or_else(String::new, |path| format!(" ({})", path.display()))
            );
        }
    }
    let mut snapshot =
        scan_archive(&archive_root).map_err(|error| CliError::other(error.to_string()))?;
    if !frontend_asset_candidates.is_empty() {
        let adopted = adopt_imported_frontend_assets(
            &archive_root,
            &connection,
            &snapshot,
            &frontend_asset_candidates,
            &cancelled,
        )?;
        if adopted > 0 {
            log::info!("Archived {adopted} existing frontend artwork/video file(s)");
            snapshot =
                scan_archive(&archive_root).map_err(|error| CliError::other(error.to_string()))?;
        }
    }
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

fn resolve_import_choices(
    plan: &mut retro_junk_archive_import::DumpImportPlan,
    dry_run: bool,
) -> Result<(), CliError> {
    use std::io::IsTerminal;

    let interactive = std::io::stdin().is_terminal() && !dry_run;
    let request = plan.request.clone();
    for candidate in &mut plan.candidates {
        match candidate.disposition.clone() {
            retro_junk_archive_import::ImportDisposition::NeedsCatalogChoice { candidates } => {
                log::info!("Catalog choices for {}:", candidate.source.display());
                for (index, choice) in candidates.iter().enumerate() {
                    log::info!(
                        "  {}. {} / {} [{}; {}]",
                        index + 1,
                        choice.platform_id,
                        choice.title,
                        choice.region,
                        choice.serial
                    );
                }
                if !interactive {
                    continue;
                }
                let answer = prompt_line("Choose catalog release (blank to skip): ")?;
                let Some(choice) = answer
                    .parse::<usize>()
                    .ok()
                    .and_then(|index| index.checked_sub(1))
                    .and_then(|index| candidates.get(index))
                    .cloned()
                else {
                    continue;
                };
                candidate.archive_platform_id =
                    retro_junk_archive_import::physical_archive_platform(
                        &request,
                        &candidate.source,
                        &choice,
                    );
                candidate.selected_match = Some(choice);
                candidate.identification =
                    retro_junk_archive_import::IdentificationResolution::Identified {
                        method: retro_junk_archive_import::IdentificationMethod::UserSelection,
                    };
                candidate.disposition = retro_junk_archive_import::ImportDisposition::Ready;
            }
            retro_junk_archive_import::ImportDisposition::NeedsPhysicalCopyChoice { copies } => {
                log::info!("Physical-copy choices for {}:", candidate.source.display());
                for (index, copy) in copies.iter().enumerate() {
                    log::info!(
                        "  {}. copy-{:02} {}",
                        index + 1,
                        copy.copy_number,
                        copy.label
                    );
                }
                if !interactive {
                    continue;
                }
                let answer = prompt_line("Choose physical copy (blank to skip): ")?;
                let Some(copy_id) = answer
                    .parse::<usize>()
                    .ok()
                    .and_then(|index| index.checked_sub(1))
                    .and_then(|index| copies.get(index))
                    .map(|copy| copy.physical_copy_id)
                else {
                    continue;
                };
                candidate.physical_copy_id = Some(copy_id);
                candidate.disposition = retro_junk_archive_import::ImportDisposition::Ready;
            }
            retro_junk_archive_import::ImportDisposition::Unresolved { .. } if interactive => {
                let answer = prompt_line(&format!(
                    "No catalog match for {}. Archive as an unbound release? [y/N] ",
                    candidate.source.display()
                ))?;
                if !matches!(answer.to_ascii_lowercase().as_str(), "y" | "yes") {
                    continue;
                }
                let suggested_title = candidate
                    .source
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or("Unknown release");
                let title = prompt_line(&format!("Release title [{suggested_title}]: "))?;
                let title = if title.is_empty() {
                    suggested_title.to_owned()
                } else {
                    title
                };
                let suggested_platform = request.platform_hint.as_deref().unwrap_or("");
                let platform_id =
                    prompt_line(&format!("Platform identifier [{suggested_platform}]: "))?;
                let platform_id = if platform_id.is_empty() {
                    suggested_platform.to_owned()
                } else {
                    platform_id
                };
                candidate.archive_platform_id.clone_from(&platform_id);
                candidate.disposition =
                    retro_junk_archive_import::ImportDisposition::ReadyUnbound {
                        title,
                        platform_id,
                    };
            }
            _ => {}
        }
    }
    Ok(())
}

fn prompt_line(prompt: &str) -> Result<String, CliError> {
    use std::io::Write;

    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(answer.trim().to_owned())
}

fn adopt_imported_frontend_assets(
    archive_root: &std::path::Path,
    connection: &retro_junk_db::Connection,
    snapshot: &retro_junk_archive::ArchiveIndexSnapshot,
    candidates: &[(String, String, String, PathBuf)],
    cancelled: &AtomicBool,
) -> Result<usize, CliError> {
    use retro_junk_frontend::AssetType;

    const TYPES: [AssetType; 9] = [
        AssetType::Cover,
        AssetType::Cover3D,
        AssetType::Screenshot,
        AssetType::TitleScreen,
        AssetType::Marquee,
        AssetType::Video,
        AssetType::Fanart,
        AssetType::PhysicalMedia,
        AssetType::Miximage,
    ];
    // Which archive release holds which catalog release is the projection's
    // answer, derived by content from each release's carriers. The manifests
    // themselves name no catalog row.
    let releases = retro_junk_db::archive::archive_releases_by_catalog_release(
        connection,
        &snapshot.manifest.profile_id.to_string(),
    )
    .map_err(|error| CliError::other(error.to_string()))?;
    let mut pending = Vec::new();
    for (catalog_release_id, platform_id, stem, media_root) in candidates {
        let Some(release_id) = releases
            .get(catalog_release_id.as_str())
            .and_then(|id| id.parse::<retro_junk_archive::ArchiveReleaseId>().ok())
        else {
            continue;
        };
        for asset_type in TYPES {
            let directory = media_root.join(platform_id).join(asset_type.subdirectory());
            for extension in asset_type.discovery_extensions() {
                let path = directory.join(format!("{stem}.{extension}"));
                if path.is_file() {
                    pending.push((release_id, asset_type, path));
                    break;
                }
            }
        }
    }
    let names = pending
        .iter()
        .map(|(_, asset_type, _)| asset_type.to_string())
        .collect::<Vec<_>>();
    let requests = pending
        .iter()
        .zip(&names)
        .map(
            |((release_id, asset_type, path), asset_name)| retro_junk_archive::NewReleaseFile {
                release_id: *release_id,
                source_file: path,
                category: if *asset_type == AssetType::Video {
                    retro_junk_archive::ReleaseFileCategory::Video
                } else {
                    retro_junk_archive::ReleaseFileCategory::Artwork
                },
                asset_type: asset_name,
                source: "existing playable media",
                source_url: "",
                caption: "",
            },
        )
        .collect::<Vec<_>>();
    let _archive_lock = retro_junk_archive::ArchiveLock::acquire(archive_root)
        .map_err(|error| CliError::other(error.to_string()))?;
    retro_junk_archive::add_release_files(archive_root, &requests, cancelled)
        .map(|results| results.into_iter().filter(|result| result.added).count())
        .map_err(|error| CliError::other(error.to_string()))
}

fn import_disposition_label(
    disposition: &retro_junk_archive_import::ImportDisposition,
) -> &'static str {
    match disposition {
        retro_junk_archive_import::ImportDisposition::Ready => "ready",
        retro_junk_archive_import::ImportDisposition::ReadyUnbound { .. } => "ready (unbound)",
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

fn import_identification_label(
    identification: &retro_junk_archive_import::IdentificationResolution,
) -> &'static str {
    use retro_junk_archive_import::{IdentificationMethod, IdentificationResolution};
    match identification {
        IdentificationResolution::CatalogVerified {
            method: IdentificationMethod::CompleteTrackSet,
        } => "catalog hash verified (complete track set)",
        IdentificationResolution::CatalogVerified {
            method: IdentificationMethod::ExactFileHash,
        } => "catalog hash verified (exact file)",
        IdentificationResolution::CatalogVerified {
            method: IdentificationMethod::FormatAwareFileHash,
        } => "catalog hash verified (normalized payload)",
        IdentificationResolution::CatalogVerified { .. } => "catalog hash verified",
        IdentificationResolution::Identified {
            method: IdentificationMethod::HeaderSerial,
        } => "catalog identity inferred from header serial; not hash verified",
        IdentificationResolution::Identified {
            method: IdentificationMethod::FolderSerial,
        } => "catalog identity inferred from folder serial; not hash verified",
        IdentificationResolution::Identified {
            method: IdentificationMethod::UserSelection,
        } => "catalog identity selected by user; not hash verified",
        IdentificationResolution::Identified { .. } => {
            "catalog identity inferred; not hash verified"
        }
        IdentificationResolution::Ambiguous => "catalog identity ambiguous",
        IdentificationResolution::Unresolved => "catalog identity unresolved",
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
    let cancelled = AtomicBool::new(false);
    let report = retro_junk_lib::archive_assets::project_archive_assets(
        &archive_root,
        &media_root,
        None,
        &cancelled,
    )
    .map_err(|error| CliError::other(error.to_string()))?;
    log::info!(
        "Projected {} frontend asset file(s); {} already current; {} unsupported archive file(s) skipped",
        report.copied,
        report.current,
        report.skipped_unknown
    );
    Ok(())
}

fn run_generate_miximages(
    archive_root: PathBuf,
    playable_root: PathBuf,
    media_root: Option<PathBuf>,
    workspace_root: Option<PathBuf>,
    release_id: Option<&str>,
) -> Result<(), CliError> {
    let cancelled = AtomicBool::new(false);
    let media_root =
        media_root.unwrap_or_else(|| retro_junk_lib::util::default_media_dir(&playable_root));
    let workspace_root =
        workspace_root.unwrap_or_else(|| archive_root.join(".retro-junk").join("work"));
    let layout = retro_junk_frontend::miximage_layout::MiximageLayout::load_or_create()
        .map_err(|error| CliError::other(error.to_string()))?;
    let snapshot =
        scan_archive(&archive_root).map_err(|error| CliError::other(error.to_string()))?;
    let mut generated = 0_usize;
    let mut archived = 0_usize;
    let mut matched = 0_usize;
    for release in snapshot.releases.iter().filter(|release| {
        release_id.is_none_or(|id| release.manifest.archive_release_id.to_string() == id)
    }) {
        matched += 1;
        let has_playable_evidence = release
            .physical_copies
            .iter()
            .flat_map(|copy| &copy.carriers)
            .flat_map(|carrier| &carrier.dumps)
            .any(|dump| !dump.builds.is_empty());
        let scratch = workspace_root
            .join("miximages")
            .join(release.manifest.archive_release_id.to_string());
        let targets = if has_playable_evidence {
            retro_junk_lib::archive_assets::release_media_stems_by_platform(release)
                .into_iter()
                .map(|(platform, stems)| (media_root.join(platform), stems))
                .collect::<Vec<_>>()
        } else {
            vec![(
                scratch.clone(),
                std::collections::BTreeSet::from(["original".to_owned()]),
            )]
        };
        let mut archive_source = None;
        for (target, stems) in targets {
            retro_junk_lib::archive_assets::project_release_assets(
                release, &target, &stems, &cancelled,
            )
            .map_err(|error| CliError::other(error.to_string()))?;
            for stem in stems {
                match retro_junk_lib::archive_assets::generate_frontend_miximage(
                    &target, &stem, &layout,
                )
                .map_err(|error| CliError::other(error.to_string()))?
                {
                    Some(path) => {
                        generated += 1;
                        archive_source.get_or_insert(path);
                    }
                    None => log::warn!(
                        "{} needs an archived screenshot before a miximage can be generated",
                        release.manifest.title
                    ),
                }
            }
        }
        if let Some(source_file) = archive_source.as_ref() {
            let results = retro_junk_archive::add_release_files(
                &archive_root,
                &[retro_junk_archive::NewReleaseFile {
                    release_id: release.manifest.archive_release_id,
                    source_file,
                    category: retro_junk_archive::ReleaseFileCategory::Artwork,
                    asset_type: "miximage",
                    source: "retro-junk miximage",
                    source_url: "",
                    caption: "",
                }],
                &cancelled,
            )
            .map_err(|error| CliError::other(error.to_string()))?;
            if results[0].added {
                archived += 1;
            }
        }
        if scratch.is_dir() {
            std::fs::remove_dir_all(&scratch)
                .map_err(|error| CliError::other(error.to_string()))?;
        }
    }
    if release_id.is_some() && matched == 0 {
        return Err(CliError::other("archive release was not found"));
    }
    log::info!("Generated {generated} miximage projection(s); archived {archived} new original(s)");
    Ok(())
}

/// Account for the playable files that are actually on disk, in two passes.
///
/// A file can be unaccounted for in two different ways, and they need
/// different evidence. Either a build's output moved and its evidence still
/// names the old path — recoverable from the recorded output digest — or no
/// build ever produced the file and it happens to be byte-identical to an
/// archived master. The moved case runs first: it is exact, and resolving it
/// keeps those files out of the second pass's suggestion pile.
fn run_adopt_playable(
    ctx: &retro_junk_lib::AnalysisContext,
    archive_root: PathBuf,
    playable_root: PathBuf,
    db: Option<PathBuf>,
    release_id: Option<String>,
    dry_run: bool,
) -> Result<(), CliError> {
    // The adoption passes append evidence into the archive, and every archive
    // mutation happens under the whole-archive lock; the scan happens under
    // it too, so what the sweep proves is what it writes against. A dry run
    // writes nothing and stays lock-free.
    let _archive_lock = if dry_run {
        None
    } else {
        retro_junk_archive::ArchiveLock::acquire_wait(&archive_root, &AtomicBool::new(false))
            .map_err(|error| CliError::other(error.to_string()))?
    };
    let snapshot =
        scan_archive(&archive_root).map_err(|error| CliError::other(error.to_string()))?;
    let database_path = match db {
        Some(path) => path,
        None => retro_junk_lib::settings::ensure_catalog_database_location()?,
    };
    let mut connection = retro_junk_db::open_database(&database_path)
        .map_err(|error| CliError::database(error.to_string()))?;

    let adoption = retro_junk_lib::archive_ops::AdoptionRequest {
        snapshot: &snapshot,
        playable_root: &playable_root,
        only_release: release_id.as_deref(),
        dry_run,
    };
    let quiet = |description: &str, _: retro_junk_io::ProgressUnit, _: u64, _: u64| {
        log::debug!("{description}");
    };
    let mut moved = retro_junk_lib::archive_ops::adopt_moved_playables(
        &adoption,
        &quiet,
        &AtomicBool::new(false),
    )
    .map_err(|error| CliError::other(error.to_string()))?;
    for (label, from, to) in &moved.adopted {
        log::info!("{label}: moved playable re-adopted, {from} -> {to}");
    }
    // Files the pipeline never built, proven to be a carrier's derivative by
    // its verified track set. Runs before the byte-identical pass below, which
    // can only ever match an uncompressed mirror of a master.
    let unbuilt = retro_junk_lib::archive_ops::adopt_unbuilt_playables(
        &adoption,
        &connection,
        &quiet,
        &AtomicBool::new(false),
    )
    .map_err(|error| CliError::other(error.to_string()))?;
    for (label, _, to) in &unbuilt.adopted {
        log::info!("{label}: adopted existing playable {to} as this carrier's derivative");
    }
    let unbuilt_count = unbuilt.adopted.len();
    moved.adopted.extend(unbuilt.adopted);
    for (label, path) in &moved.unresolved {
        log::warn!("{label}: {path} is missing and no file under the playable root matches it");
    }
    // Later passes must see the evidence just written, or a re-adopted file
    // looks unaccounted for and gets suggested for review.
    let snapshot = if moved.adopted.is_empty() || dry_run {
        snapshot
    } else {
        scan_archive(&archive_root).map_err(|error| CliError::other(error.to_string()))?
    };

    let mut files = Vec::new();
    collect_playable_files(&playable_root, &playable_root, &mut files)?;
    // Every file the archive already accounts for, at the location it is
    // actually in. Comparing against the recorded path instead made each
    // output whose release publishes into a different folder than the one it
    // was built into look unaccounted for, and offered a file the archive
    // already owns for adoption a second time.
    let outputs_root = playable_root.as_path();
    let known_outputs = snapshot
        .releases
        .iter()
        .flat_map(|release| {
            release
                .physical_copies
                .iter()
                .flat_map(|item| &item.carriers)
                .flat_map(|medium| &medium.dumps)
                .flat_map(|dump| &dump.builds)
                .map(move |build| {
                    retro_junk_lib::playable_location::release_output_relative(
                        release,
                        outputs_root,
                        &build.evidence,
                    )
                })
        })
        .collect::<std::collections::BTreeSet<_>>();
    // Decisions the user already made about whole groups of strays. Consulted
    // before anything is hashed, so an ignored file costs a path comparison
    // rather than a full read — which on a library of a thousand unaccounted
    // files is the difference between a sweep and an afternoon.
    let ignored = retro_junk_archive::IgnoreRules::load(&retro_junk_archive::collection_root_for(
        &archive_root,
        &playable_root,
    ))
    .map_err(|error| CliError::other(error.to_string()))?;
    let cancelled = AtomicBool::new(false);
    let mut suggested = 0_usize;
    let mut adopted = 0_usize;
    let mut skipped = 0_usize;
    for path in files {
        let relative = retro_junk_archive::normalize_relative_path(
            path.strip_prefix(&playable_root).unwrap_or(&path),
        )
        .map_err(|error| CliError::other(error.to_string()))?;
        if known_outputs.contains(relative.as_str()) {
            continue;
        }
        if let Some(rule) = ignored.matching(&relative) {
            log::debug!("{relative}: ignored by rule '{}'", rule.pattern);
            skipped += 1;
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
            if dry_run {
                log::info!(
                    "{relative} would be adopted: byte-identical to master {}",
                    dump.manifest.dump_id
                );
                adopted += 1;
                continue;
            }
            retro_junk_lib::archive_ops::adopt_identical_playable(
                &retro_junk_lib::archive_ops::IdenticalAdoption {
                    dump,
                    carrier: medium,
                    platform_id: &release.manifest.platform_id,
                    relative_path: &relative,
                    digests: &digests,
                },
                &connection,
            )
            .map_err(|error| CliError::other(error.to_string()))?;
            adopted += 1;
            continue;
        }
        if masters.len() > 1 {
            // Every master with these bytes, so the review can be resolved by
            // choosing one rather than sending the user back to the files.
            let candidates = masters
                .iter()
                .map(|(release, medium, dump)| AdoptionCandidate {
                    kind: AdoptionCandidateKind::ArchiveMaster,
                    id: dump.manifest.dump_id.to_string(),
                    label: release_label(&release.manifest),
                    archive_release_id: release.manifest.archive_release_id.to_string(),
                    carrier_id: medium.manifest.carrier_id.to_string(),
                    platform_id: release.manifest.platform_id.clone(),
                })
                .collect();
            suggest_adoption_review(
                &mut connection,
                &AdoptionSuggestionPayload {
                    relative_path: relative.clone(),
                    status: "ambiguous_archive_master".to_owned(),
                    detail: format!("bytes match {} archived masters", masters.len()),
                    candidates,
                },
                0.3,
                dry_run,
            )?;
            suggested += 1;
            continue;
        }
        let platform = retro_junk_backend::adoption::platform_of(&relative);
        // Format-aware hashing so a headered cartridge compares against the
        // catalog the way the catalog stores it. A file this platform's
        // analyzer cannot read is not a reason to abandon the run — the whole
        // point of the sweep is that the playable tree contains things nobody
        // has accounted for yet. It falls back to the raw digests, and if
        // those match nothing it is filed for review like any other stranger.
        let catalog_digests = catalog_comparison_digests(ctx, &platform, &path, &digests);
        let catalog = retro_junk_db::match_catalog_file(&connection, &platform, &catalog_digests)
            .map_err(|error| CliError::database(error.to_string()))?;
        let (status, detail, candidates) = match catalog.as_slice() {
            [matched] => {
                if !dry_run {
                    retro_junk_db::bind_library_entries_by_hash(
                        &connection,
                        &platform,
                        &catalog_digests,
                        &retro_junk_db::LibraryEntryBinding {
                            // Catalog identity only: nothing in the archive
                            // holds this file, which is what the report says.
                            catalog_media_id: &matched.media_id,
                            match_method: "catalog_adoption",
                            ..Default::default()
                        },
                    )
                    .map_err(|error| CliError::database(error.to_string()))?;
                }
                (
                    "catalog_only",
                    format!(
                        "matches {} ({}) but has no preservation master",
                        matched.game, matched.media_id
                    ),
                    // Already bound above; there is nothing left to choose.
                    Vec::new(),
                )
            }
            [] => (
                "unmatched",
                "no archive master or catalog match".to_owned(),
                Vec::new(),
            ),
            many => (
                "ambiguous_catalog",
                format!("matches {} catalog media", many.len()),
                many.iter()
                    .map(|matched| AdoptionCandidate {
                        kind: AdoptionCandidateKind::CatalogMedium,
                        id: matched.media_id.clone(),
                        label: catalog_match_label(matched),
                        archive_release_id: String::new(),
                        carrier_id: String::new(),
                        platform_id: matched.platform_id.clone(),
                    })
                    .collect(),
            ),
        };
        let confidence = match status {
            "catalog_only" => 0.6,
            "ambiguous_catalog" => 0.3,
            _ => 0.1,
        };
        suggest_adoption_review(
            &mut connection,
            &AdoptionSuggestionPayload {
                relative_path: relative.clone(),
                status: status.to_owned(),
                detail,
                candidates,
            },
            confidence,
            dry_run,
        )?;
        suggested += 1;
    }
    if dry_run {
        log::info!(
            "Dry run: {} moved playable(s) re-adopted, {unbuilt_count} unbuilt playable(s) adopted, {adopted} byte-identical file(s) adopted, {suggested} filed for review",
            moved.adopted.len() - unbuilt_count
        );
        return Ok(());
    }
    let refreshed =
        scan_archive(&archive_root).map_err(|error| CliError::other(error.to_string()))?;
    retro_junk_db::reconcile_archive_snapshot(
        &mut connection,
        &refreshed,
        &playable_root,
        &archive_root.join(".retro-junk/work"),
    )
    .map_err(|error| CliError::database(error.to_string()))?;
    log::info!(
        "Re-adopted {} moved playable output(s) and {unbuilt_count} unbuilt one(s); {} still missing",
        moved.adopted.len() - unbuilt_count,
        moved.unresolved.len()
    );
    log::info!("Adopted {adopted} byte-identical playable file(s)");
    log::info!(
        "Recorded {suggested} unresolved playable file(s) as suggestions (`retro-junk suggestions list`)"
    );
    if skipped > 0 {
        log::info!(
            "Skipped {skipped} file(s) covered by {} ignore rule(s) (`retro-junk suggestions ignores`)",
            ignored.len()
        );
    }
    Ok(())
}

/// One open review row per unresolved playable file; re-running adoption
/// refreshes rather than piles up.
fn suggest_adoption_review(
    connection: &mut retro_junk_db::Connection,
    payload: &AdoptionSuggestionPayload,
    confidence: f64,
    dry_run: bool,
) -> Result<(), CliError> {
    if dry_run {
        log::info!(
            "{} would be filed for review: {} — {}",
            payload.relative_path,
            payload.status,
            payload.detail
        );
        return Ok(());
    }
    retro_junk_backend::adoption::open_adoption_suggestion(
        connection,
        payload,
        confidence,
        "cli-adopt",
    )
    .map_err(|error| CliError::database(error.to_string()))
}

/// The digests the catalog compares a playable file against.
fn catalog_comparison_digests(
    ctx: &retro_junk_lib::AnalysisContext,
    platform_id: &str,
    path: &std::path::Path,
    raw: &retro_junk_archive::FileDigests,
) -> retro_junk_archive::FileDigests {
    ctx.get_by_short_name(platform_id)
        .and_then(|console| {
            let mut input = std::fs::File::open(path).ok()?;
            let hashes = retro_junk_lib::hasher::compute_all_hashes(
                &mut input,
                console.analyzer.as_ref(),
                Some(path),
            )
            .map_err(|error| {
                log::debug!("{}: {error}; comparing raw digests instead", path.display());
            })
            .ok()?;
            Some(retro_junk_archive::FileDigests {
                size: hashes.data_size,
                crc32: hashes.crc32,
                md5: hashes.md5.unwrap_or_default(),
                sha1: hashes.sha1.unwrap_or_default(),
                sha256: raw.sha256.clone(),
            })
        })
        .unwrap_or_else(|| raw.clone())
}

/// A release as a person would name it, for a review card to show.
fn release_label(manifest: &retro_junk_archive::ReleaseManifest) -> String {
    if manifest.region.is_empty() {
        manifest.title.clone()
    } else {
        format!("{} ({})", manifest.title, manifest.region)
    }
}

/// A catalogued medium as a person would name it.
fn catalog_match_label(matched: &retro_junk_db::CompleteCatalogMediaMatch) -> String {
    let mut label = matched.game.clone();
    if !matched.region.is_empty() {
        label.push_str(&format!(" ({})", matched.region));
    }
    if !matched.source.is_empty() {
        label.push_str(&format!(" · {}", matched.source));
    }
    label
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
        // Host-filesystem bookkeeping is not collection content. A library
        // mirrored onto exFAT or SMB carries an AppleDouble sidecar beside
        // every file, and each one keeps the extension it shadows — so without
        // this every game filed a second, bogus "unmatched" review row.
        if retro_junk_io::is_noise_path(&path) {
            continue;
        }
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

fn run_default_policy(
    archive_root: PathBuf,
    platform: &str,
    format: Option<&str>,
    clear: bool,
    retain_intermediate: bool,
    allow_unverified: bool,
) -> Result<(), CliError> {
    let path = retro_junk_archive::root_manifest_path(&archive_root);
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

#[allow(clippy::too_many_arguments)]
fn run_shared_single_build(
    archive_root: PathBuf,
    playable_root: PathBuf,
    dump_id: &str,
    workspace_root: Option<PathBuf>,
    chdman_path: Option<PathBuf>,
    redumper_path: Option<PathBuf>,
    dolphin_tool_path: Option<PathBuf>,
    format: RepresentationFormat,
    allow_unverified: bool,
    retain_intermediate: bool,
    options: std::collections::BTreeMap<String, String>,
) -> Result<(), CliError> {
    let snapshot =
        scan_archive(&archive_root).map_err(|error| CliError::other(error.to_string()))?;
    let selected = snapshot.releases.iter().find_map(|release| {
        release.physical_copies.iter().find_map(|copy| {
            copy.carriers.iter().find_map(|carrier| {
                carrier
                    .dumps
                    .iter()
                    .find(|dump| dump.manifest.dump_id.to_string() == dump_id)
                    .map(|dump| (release, copy, carrier, dump))
            })
        })
    });
    let Some((release, copy, carrier, dump)) = selected else {
        return Err(CliError::other(format!(
            "archive dump {dump_id} was not found"
        )));
    };
    let expected_disc_count = copy
        .carriers
        .iter()
        .map(|carrier| carrier.manifest.sequence_number)
        .max()
        .unwrap_or(0)
        .max(1);
    let format = if format == RepresentationFormat::Rom {
        dump.manifest.format.clone()
    } else {
        format
    };
    let request = retro_junk_lib::playable_build::PlayableBuildRequest {
        archive_root: archive_root.clone(),
        playable_root,
        workspace_root: workspace_root
            .unwrap_or_else(|| archive_root.join(".retro-junk").join("work")),
        dump_id: dump.manifest.dump_id.to_string(),
        format,
        chdman_path: chdman_path.unwrap_or_default(),
        redumper_path: redumper_path.unwrap_or_default(),
        dolphin_tool_path: dolphin_tool_path.unwrap_or_default(),
        allow_unverified,
        retain_intermediate,
        options,
        playable_platform_id: retro_junk_frontend::esde::system_directory(
            &release.manifest.platform_id,
            Some(&release.manifest.region),
        ),
        expected_disc_count,
        canonical_output_stem: String::new(),
        canonical_release_name: String::new(),
    };
    let outcome = retro_junk_lib::playable_build::build_playable(
        &request,
        &log_progress,
        &AtomicBool::new(false),
    )
    .map_err(|error| CliError::other(error.to_string()))?;
    log::info!(
        "Built and round-trip verified {} for carrier {}",
        outcome.output.display(),
        carrier.manifest.carrier_id
    );
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
    let report = retro_junk_lib::archive_ops::verify_catalog_files(
        &snapshot,
        &connection,
        ctx,
        dump_id,
        &log_progress,
        &cancelled,
    )
    .map_err(|error| CliError::other(error.to_string()))?;
    if report.selected == 0 {
        return Err(CliError::other("no matching single-file dumps found"));
    }
    log::info!(
        "Catalog verified {} of {} single-file dump(s)",
        report.identified,
        report.selected
    );
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
    let workspace_root =
        workspace_root.unwrap_or_else(|| archive_root.join(".retro-junk").join("work"));
    let database_path = match db {
        Some(path) => path,
        None => retro_junk_lib::settings::ensure_catalog_database_location()?,
    };
    let connection = retro_junk_db::open_database(&database_path)
        .map_err(|error| CliError::database(error.to_string()))?;
    let cancelled = AtomicBool::new(false);
    let report = retro_junk_lib::archive_ops::identify_archived_carriers(
        &retro_junk_lib::archive_ops::IdentifyCarriersRequest {
            snapshot: &snapshot,
            selection: retro_junk_lib::archive_ops::IdentifySelection::All,
            only_dump: dump_id,
            redumper_path: redumper_path
                .as_deref()
                .unwrap_or_else(|| std::path::Path::new("")),
            workspace_root: &workspace_root,
        },
        &connection,
        &log_progress,
        &cancelled,
    )
    .map_err(|error| CliError::other(error.to_string()))?;
    if report.selected == 0 {
        return Err(CliError::other("no matching Redumper raw dumps found"));
    }
    if report.failed > 0 {
        Err(CliError::other(format!(
            "{} of {} Redumper audit(s) failed; evidence was recorded",
            report.failed, report.selected
        )))
    } else {
        log::info!("Audited {} Redumper raw dump(s)", report.selected);
        Ok(())
    }
}

pub(crate) fn log_progress(
    phase: &str,
    unit: retro_junk_io::ProgressUnit,
    current: u64,
    total: u64,
) {
    match (total, unit) {
        (0, _) => log::info!("{phase}"),
        (_, retro_junk_io::ProgressUnit::Bytes) => log::info!(
            "{phase}: {} / {}",
            retro_junk_core::util::format_bytes_approx(current),
            retro_junk_core::util::format_bytes_approx(total),
        ),
        (_, retro_junk_io::ProgressUnit::Items) => log::info!("{phase}: {current}/{total}"),
    }
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
    let report = retro_junk_lib::archive_ops::verify_archive_integrity(
        &snapshot,
        None,
        &log_progress,
        &cancelled,
    )
    .map_err(|error| CliError::other(error.to_string()))?;
    for (dump_id, reason) in &report.failures {
        log::error!("{dump_id}: {reason}");
    }
    log::info!(
        "Checked {} dump(s); {} failed",
        report.checked,
        report.failed
    );
    if report.failed > 0 {
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
    let cancel = AtomicBool::new(false);
    let _archive_lock = retro_junk_archive::ArchiveLock::acquire_wait(&archive_root, &cancel)
        .map_err(|error| CliError::other(error.to_string()))?
        .ok_or_else(|| CliError::other("archive reindex cancelled"))?;
    let upgraded = retro_junk_archive::upgrade_legacy_regional_physical_platforms(&archive_root)
        .map_err(|error| CliError::other(error.to_string()))?;
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
    if upgraded > 0 {
        log::info!("Reclassified {upgraded} release(s) under their regional physical platform");
    }
    log::info!("Rebuilt archive index in {}", db.display());
    Ok(())
}

fn parse_format(value: &str) -> Result<RepresentationFormat, CliError> {
    value.parse().map_err(CliError::other)
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
                join_release: None,
                source_package: retro_junk_archive::SourcePackageRecord::default(),
                expected_files: Vec::new(),
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
        run_shared_single_build(
            archive.clone(),
            playable.clone(),
            &ingested.dump.dump_id.to_string(),
            None,
            None,
            None,
            Some(tool),
            RepresentationFormat::Rvz,
            true,
            false,
            std::collections::BTreeMap::new(),
        )
        .unwrap();
        // An unbound carrier is named the way a catalog would have named it —
        // readable, with the region written as a DAT writes it — rather than
        // slugified. A library must not read as two collections depending on
        // whether a carrier happened to resolve to a catalog medium.
        assert!(
            playable.join("gc/Game (USA).rvz").is_file(),
            "unbound carriers use the same readable scheme as bound ones"
        );
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
