//! Builds disposable playable projections from authoritative archive dumps.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use retro_junk_archive::{
    BuildEvidence, BuildId, CanonicalIntermediateEvidence, CapturedFeature, IndexedCarrier,
    IndexedDump, IndexedRelease, Redumper, RepresentationFormat, RepresentationId, ToolRecord,
    VerificationKind, VerificationOutcome, scan_archive, sha256_file, write_json_new,
};

#[derive(Debug, Clone)]
pub struct PlayableBuildRequest {
    pub archive_root: PathBuf,
    pub playable_root: PathBuf,
    pub workspace_root: PathBuf,
    pub dump_id: String,
    pub format: RepresentationFormat,
    pub chdman_path: PathBuf,
    pub allow_unverified: bool,
    pub retain_intermediate: bool,
    /// Concrete console folder in the playable library (for example `psx`
    /// even when the archive/catalog canonical identifier is `ps1`).
    pub playable_platform_id: String,
    /// Logical catalog disc count. Values greater than one enable playlist
    /// projection only after every disc in this physical set has been built.
    pub expected_disc_count: u32,
}

#[derive(Debug, Clone)]
pub struct PlayableBuildOutcome {
    pub output: PathBuf,
    pub format: RepresentationFormat,
}

#[derive(Debug, thiserror::Error)]
pub enum PlayableBuildError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Build one selected preservation dump into the requested playable format.
/// The callback reports a human-readable phase plus completed/total bytes when
/// the underlying converter exposes byte progress.
pub fn build_playable(
    request: &PlayableBuildRequest,
    progress: &dyn Fn(&str, u64, u64),
    cancelled: &AtomicBool,
) -> Result<PlayableBuildOutcome, PlayableBuildError> {
    progress("Reading archive manifests", 0, 0);
    let snapshot = scan_archive(&request.archive_root)
        .map_err(|error| PlayableBuildError::Message(error.to_string()))?;
    let selected = snapshot.releases.iter().find_map(|release| {
        release.physical_copies.iter().find_map(|copy| {
            copy.carriers.iter().find_map(|carrier| {
                carrier
                    .dumps
                    .iter()
                    .find(|dump| dump.manifest.dump_id.to_string() == request.dump_id)
                    .map(|dump| (release, carrier, dump))
            })
        })
    });
    let Some((release, carrier, dump)) = selected else {
        return Err(PlayableBuildError::Message(format!(
            "archive dump {} was not found",
            request.dump_id
        )));
    };
    let outcome = match request.format {
        RepresentationFormat::Chd => {
            build_chd(request, release, carrier, dump, progress, cancelled)
        }
        ref format if *format == dump.manifest.format && dump.manifest.files.len() == 1 => {
            mirror(request, release, carrier, dump, progress, cancelled)
        }
        _ => Err(PlayableBuildError::Message(format!(
            "No builder is available from {:?} to {:?}",
            dump.manifest.format, request.format
        ))),
    }?;
    if request.expected_disc_count > 1 {
        progress("Updating multi-disc playlist", 0, 0);
        project_selected_playlist(request, &request.dump_id)?;
    }
    Ok(outcome)
}

#[allow(clippy::too_many_lines)]
fn build_chd(
    request: &PlayableBuildRequest,
    release: &IndexedRelease,
    carrier: &IndexedCarrier,
    dump: &IndexedDump,
    progress: &dyn Fn(&str, u64, u64),
    cancelled: &AtomicBool,
) -> Result<PlayableBuildOutcome, PlayableBuildError> {
    let catalog_verified = catalog_verified(dump);
    if !catalog_verified && !request.allow_unverified {
        return Err(PlayableBuildError::Message(
            "The current dump has no complete-track catalog verification. Verify it first, or enable unverified builds for this policy.".to_owned(),
        ));
    }
    let context = crate::create_default_context();
    let analyzer = context
        .get_by_short_name(&release.manifest.platform_id)
        .ok_or_else(|| {
            PlayableBuildError::Message(format!(
                "No analyzer is registered for {}",
                release.manifest.platform_id
            ))
        })?;
    let mut redumper_workspace = None;
    let input = match dump.manifest.format {
        RepresentationFormat::RedumperRaw => {
            progress("Preparing Redumper files in the local workspace", 0, 0);
            let redumper = Redumper::detect(Path::new(""))
                .map_err(|error| PlayableBuildError::Message(error.to_string()))?;
            let prepared = redumper
                .prepare(
                    &dump.directory.join("raw"),
                    &request.workspace_root,
                    cancelled,
                )
                .map_err(|error| PlayableBuildError::Message(error.to_string()))?;
            let entrypoint = prepared.entrypoint.clone();
            redumper_workspace = Some(prepared);
            entrypoint
        }
        RepresentationFormat::CueBin => find_input(&dump.directory.join("raw"), &["cue"])?,
        RepresentationFormat::Iso => find_input(&dump.directory.join("raw"), &["iso"])?,
        _ => {
            return Err(PlayableBuildError::Message(format!(
                "{:?} cannot be converted to CHD",
                dump.manifest.format
            )));
        }
    };
    let mut job = crate::chd_convert::plan_compression(&input, analyzer.analyzer.as_ref())
        .map_err(|error| PlayableBuildError::Message(error.to_string()))?;
    let output_directory = playable_output_directory(request, release, carrier);
    std::fs::create_dir_all(&output_directory)?;
    job.output = output_directory.join(format!(
        "{}.chd",
        playable_output_stem(release, carrier, request.expected_disc_count > 1)
    ));
    if job.output.exists() {
        return Err(PlayableBuildError::Message(format!(
            "Playable output already exists: {}",
            job.output.display()
        )));
    }
    progress(
        "Compressing and round-trip verifying CHD",
        0,
        job.input_bytes,
    );
    let chdman = crate::chd_convert::Chdman::detect(&request.chdman_path).map_err(|error| {
        PlayableBuildError::Message(format!(
            "{error} {}",
            crate::chd_convert::ChdmanUnavailable::install_hint()
        ))
    })?;
    let outcome = crate::chd_convert::compress_to_chd(
        &chdman,
        &job,
        &|phase, fraction| {
            let total = job.input_bytes;
            let done = (crate::chd_convert::job_fraction(phase, fraction) * total as f64) as u64;
            progress("Compressing and round-trip verifying CHD", done, total);
        },
        cancelled,
    )
    .map_err(|error| PlayableBuildError::Message(error.to_string()))?;
    if !outcome.is_verified() {
        return Err(PlayableBuildError::Message(
            "CHD round-trip verification failed".to_owned(),
        ));
    }
    progress("Recording build evidence", 0, 0);
    let (output_size, output_sha256) = sha256_file(&outcome.output, cancelled)
        .map_err(|error| PlayableBuildError::Message(error.to_string()))?;
    let build_id = BuildId::new();
    let retained_path = dump
        .directory
        .join("intermediates")
        .join(build_id.to_string());
    let canonical_intermediate = if request.retain_intermediate {
        if let Some(workspace) = redumper_workspace.as_ref() {
            let files = workspace
                .retain_intermediate(&retained_path, cancelled)
                .map_err(|error| PlayableBuildError::Message(error.to_string()))?;
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
            Some(CanonicalIntermediateEvidence {
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
    let evidence = BuildEvidence {
        schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
        build_id,
        parent_representation_id: dump.manifest.representation_id,
        child_representation_id: RepresentationId::new(),
        performed_at: chrono::Utc::now().to_rfc3339(),
        input_manifest_sha256: dump.manifest_sha256.clone(),
        recipe_version: 1,
        format: RepresentationFormat::Chd,
        relative_output_path: relative(&request.playable_root, &outcome.output),
        output_sha256,
        output_size,
        catalog_verified,
        round_trip_verified: true,
        tool: Some(ToolRecord {
            name: "chdman".to_owned(),
            version: chdman.version,
            build: String::new(),
        }),
        omitted_features: dump.manifest.captured_features.clone(),
        canonical_intermediate,
    };
    write_evidence(dump, &evidence, &outcome.output)?;
    Ok(PlayableBuildOutcome {
        output: outcome.output,
        format: RepresentationFormat::Chd,
    })
}

fn mirror(
    request: &PlayableBuildRequest,
    release: &IndexedRelease,
    carrier: &IndexedCarrier,
    dump: &IndexedDump,
    progress: &dyn Fn(&str, u64, u64),
    cancelled: &AtomicBool,
) -> Result<PlayableBuildOutcome, PlayableBuildError> {
    let file = &dump.manifest.files[0];
    let source = dump.directory.join("raw").join(&file.path);
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("rom");
    let output_directory = playable_output_directory(request, release, carrier);
    std::fs::create_dir_all(&output_directory)?;
    let output = output_directory.join(format!(
        "{}.{}",
        playable_output_stem(release, carrier, request.expected_disc_count > 1),
        extension
    ));
    if output.exists() {
        return Err(PlayableBuildError::Message(format!(
            "Playable output already exists: {}",
            output.display()
        )));
    }
    progress(
        "Mirroring preservation bytes to the playable library",
        0,
        file.size,
    );
    let temporary = output_directory.join(format!(".{}.mirror.tmp", BuildId::new()));
    std::fs::copy(&source, &temporary)?;
    let (output_size, output_sha256) = sha256_file(&temporary, cancelled)
        .map_err(|error| PlayableBuildError::Message(error.to_string()))?;
    if output_size != file.size || output_sha256 != file.sha256 {
        let _ = std::fs::remove_file(&temporary);
        return Err(PlayableBuildError::Message(
            "Mirrored bytes did not match the preservation manifest".to_owned(),
        ));
    }
    std::fs::rename(&temporary, &output)?;
    let evidence = BuildEvidence {
        schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
        build_id: BuildId::new(),
        parent_representation_id: dump.manifest.representation_id,
        child_representation_id: RepresentationId::new(),
        performed_at: chrono::Utc::now().to_rfc3339(),
        input_manifest_sha256: dump.manifest_sha256.clone(),
        recipe_version: 1,
        format: dump.manifest.format.clone(),
        relative_output_path: relative(&request.playable_root, &output),
        output_sha256,
        output_size,
        catalog_verified: catalog_verified(dump),
        round_trip_verified: true,
        tool: None,
        omitted_features: Vec::<CapturedFeature>::new(),
        canonical_intermediate: None,
    };
    write_evidence(dump, &evidence, &output)?;
    Ok(PlayableBuildOutcome {
        output,
        format: dump.manifest.format.clone(),
    })
}

fn write_evidence(
    dump: &IndexedDump,
    evidence: &BuildEvidence,
    output: &Path,
) -> Result<(), PlayableBuildError> {
    let directory = dump.directory.join("evidence");
    std::fs::create_dir_all(&directory)?;
    if let Err(error) = write_json_new(
        &directory.join(format!("build-{}.json", evidence.build_id)),
        evidence,
    ) {
        let _ = std::fs::remove_file(output);
        return Err(PlayableBuildError::Message(error.to_string()));
    }
    Ok(())
}

fn catalog_verified(dump: &IndexedDump) -> bool {
    dump.verifications.iter().any(|verification| {
        verification.evidence.input_manifest_sha256 == dump.manifest_sha256
            && verification.evidence.kind == VerificationKind::Catalog
            && verification.evidence.outcome == VerificationOutcome::Verified
    })
}

fn find_input(directory: &Path, extensions: &[&str]) -> Result<PathBuf, PlayableBuildError> {
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
        PlayableBuildError::Message(format!(
            "No supported input was found in {}",
            directory.display()
        ))
    })
}

fn playable_output_stem(
    release: &IndexedRelease,
    carrier: &IndexedCarrier,
    multi_disc: bool,
) -> String {
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
    if multi_disc && carrier.manifest.sequence_number > 0 {
        let _ = write!(name, " (Disc {})", carrier.manifest.sequence_number);
    }
    retro_junk_archive::slugify(&name)
}

fn playable_output_directory(
    request: &PlayableBuildRequest,
    release: &IndexedRelease,
    carrier: &IndexedCarrier,
) -> PathBuf {
    let platform = if request.playable_platform_id.trim().is_empty() {
        &release.manifest.platform_id
    } else {
        &request.playable_platform_id
    };
    let base = request
        .playable_root
        .join(retro_junk_archive::slugify(platform));
    if request.expected_disc_count > 1 && carrier.manifest.sequence_number > 0 {
        base.join(format!(
            "{}.m3u",
            retro_junk_archive::slugify(&release_output_name(release))
        ))
    } else {
        base
    }
}

fn release_output_name(release: &IndexedRelease) -> String {
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

fn project_selected_playlist(
    request: &PlayableBuildRequest,
    selected_dump_id: &str,
) -> Result<Option<PathBuf>, PlayableBuildError> {
    let snapshot = scan_archive(&request.archive_root)
        .map_err(|error| PlayableBuildError::Message(error.to_string()))?;
    let selected = snapshot.releases.iter().find_map(|release| {
        release.physical_copies.iter().find_map(|copy| {
            copy.carriers
                .iter()
                .flat_map(|carrier| &carrier.dumps)
                .any(|dump| dump.manifest.dump_id.to_string() == selected_dump_id)
                .then_some((release, copy))
        })
    });
    let Some((release, copy)) = selected else {
        return Ok(None);
    };
    let mut discs = Vec::new();
    for carrier in &copy.carriers {
        let sequence = carrier.manifest.sequence_number;
        if sequence == 0 || sequence > request.expected_disc_count {
            continue;
        }
        let build = carrier
            .dumps
            .iter()
            .flat_map(|dump| {
                dump.builds
                    .iter()
                    .map(move |build| (dump.manifest_sha256.as_str(), build))
            })
            .find(|(manifest_sha, build)| {
                build.evidence.format == request.format
                    && retro_junk_archive::playable_presence(
                        &request.playable_root,
                        manifest_sha,
                        &build.evidence,
                    ) == retro_junk_archive::RepresentationPresence::Present
            });
        if let Some((_, build)) = build {
            discs.push((sequence, build.evidence.relative_output_path.clone()));
        }
    }
    discs.sort_by_key(|(sequence, _)| *sequence);
    discs.dedup_by_key(|(sequence, _)| *sequence);
    if discs.len() != request.expected_disc_count as usize
        || discs
            .iter()
            .enumerate()
            .any(|(index, (sequence, _))| *sequence != index as u32 + 1)
    {
        return Ok(None);
    }
    let platform = if request.playable_platform_id.trim().is_empty() {
        &release.manifest.platform_id
    } else {
        &request.playable_platform_id
    };
    let directory = request
        .playable_root
        .join(retro_junk_archive::slugify(platform))
        .join(format!(
            "{}.m3u",
            retro_junk_archive::slugify(&release_output_name(release))
        ));
    std::fs::create_dir_all(&directory)?;
    let playlist = directory.join(format!(
        "{}.m3u",
        retro_junk_archive::slugify(&release_output_name(release))
    ));
    let contents = discs
        .iter()
        .map(|(_, relative_path)| {
            let output = request.playable_root.join(relative_path);
            pathdiff::diff_paths(&output, &directory)
                .unwrap_or(output)
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let temporary = directory.join(format!(".playlist-{}.tmp", BuildId::new()));
    std::fs::write(&temporary, contents)?;
    std::fs::rename(&temporary, &playlist)?;
    Ok(Some(playlist))
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_rom_build_is_a_verified_mirror_with_evidence() {
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("archive");
        let playable = temp.path().join("playable");
        let source = temp.path().join("game.nes");
        std::fs::write(&source, b"preservation bytes").unwrap();
        retro_junk_archive::initialize_archive(
            &archive,
            &retro_junk_archive::ArchiveRootManifest::new("Test"),
        )
        .unwrap();
        let ingested = retro_junk_archive::ingest_new_carrier_dump(
            &archive,
            &source,
            retro_junk_archive::NewCarrierDump {
                platform_id: "nes".to_owned(),
                title: "Game".to_owned(),
                region: "usa".to_owned(),
                revision: String::new(),
                variant: String::new(),
                owner_id: "default".to_owned(),
                physical_copy_label: String::new(),
                serial: String::new(),
                sequence_number: 0,
                carrier_label: String::new(),
                carrier_kind: retro_junk_archive::CarrierKind::Cartridge,
                format: RepresentationFormat::Rom,
                catalog_binding: retro_junk_archive::CatalogBinding::default(),
                source_package: retro_junk_archive::SourcePackageRecord::default(),
                expected_files: Vec::new(),
                physical_copy_id: None,
            },
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        let outcome = build_playable(
            &PlayableBuildRequest {
                archive_root: archive.clone(),
                playable_root: playable.clone(),
                workspace_root: temp.path().join("work"),
                dump_id: ingested.dump.dump_id.to_string(),
                format: RepresentationFormat::Rom,
                chdman_path: PathBuf::new(),
                allow_unverified: false,
                retain_intermediate: false,
                playable_platform_id: "nes".to_owned(),
                expected_disc_count: 1,
            },
            &|_, _, _| {},
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            std::fs::read(&outcome.output).unwrap(),
            b"preservation bytes"
        );
        let snapshot = scan_archive(&archive).unwrap();
        assert_eq!(
            snapshot.releases[0].physical_copies[0].carriers[0].dumps[0]
                .builds
                .len(),
            1
        );
    }

    #[test]
    fn playlist_is_written_only_after_every_expected_disc_is_playable() {
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("archive");
        let playable = temp.path().join("playable");
        retro_junk_archive::initialize_archive(
            &archive,
            &retro_junk_archive::ArchiveRootManifest::new("Test"),
        )
        .unwrap();
        let mut ingested = Vec::new();
        let mut physical_copy_id = None;
        for sequence in 1..=2 {
            let source = temp.path().join(format!("disc{sequence}.bin"));
            std::fs::write(&source, format!("disc {sequence}")).unwrap();
            let result = retro_junk_archive::ingest_new_carrier_dump(
                &archive,
                &source,
                retro_junk_archive::NewCarrierDump {
                    platform_id: "ps1".to_owned(),
                    title: "Two Disc Game".to_owned(),
                    region: "usa".to_owned(),
                    revision: String::new(),
                    variant: String::new(),
                    owner_id: "default".to_owned(),
                    physical_copy_label: String::new(),
                    serial: format!("DISC-{sequence}"),
                    sequence_number: sequence,
                    carrier_label: String::new(),
                    carrier_kind: retro_junk_archive::CarrierKind::OpticalDisc,
                    format: RepresentationFormat::Rom,
                    catalog_binding: retro_junk_archive::CatalogBinding::default(),
                    source_package: retro_junk_archive::SourcePackageRecord::default(),
                    expected_files: Vec::new(),
                    physical_copy_id,
                },
                &AtomicBool::new(false),
                |_| {},
            )
            .unwrap();
            physical_copy_id = Some(result.physical_copy.physical_copy_id);
            ingested.push(result.dump.dump_id.to_string());
        }
        let request = |dump_id: String| PlayableBuildRequest {
            archive_root: archive.clone(),
            playable_root: playable.clone(),
            workspace_root: temp.path().join("work"),
            dump_id,
            format: RepresentationFormat::Rom,
            chdman_path: PathBuf::new(),
            allow_unverified: false,
            retain_intermediate: false,
            playable_platform_id: "psx".to_owned(),
            expected_disc_count: 2,
        };
        build_playable(
            &request(ingested[0].clone()),
            &|_, _, _| {},
            &AtomicBool::new(false),
        )
        .unwrap();
        let set_dir = playable.join("psx/two-disc-game-usa.m3u");
        assert!(!set_dir.join("two-disc-game-usa.m3u").exists());
        build_playable(
            &request(ingested[1].clone()),
            &|_, _, _| {},
            &AtomicBool::new(false),
        )
        .unwrap();
        let playlist = std::fs::read_to_string(set_dir.join("two-disc-game-usa.m3u")).unwrap();
        assert_eq!(
            playlist,
            "two-disc-game-usa-disc-1.bin\ntwo-disc-game-usa-disc-2.bin\n"
        );
    }
}
