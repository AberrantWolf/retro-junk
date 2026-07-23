//! Builds disposable playable projections from authoritative archive dumps.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use retro_junk_archive::{
    BuildEvidence, BuildId, CanonicalIntermediateEvidence, CapturedFeature, CatalogEvidence,
    IndexedCarrier, IndexedDump, IndexedRelease, Redumper, RepresentationFormat, RepresentationId,
    ToolRecord, TrackDigest, TrackVerification, VerificationEvidence, VerificationId,
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

#[derive(Debug, Clone)]
pub struct CatalogVerificationRequest {
    pub archive_root: PathBuf,
    pub workspace_root: PathBuf,
    pub dump_id: String,
    pub redumper_path: PathBuf,
    pub expected_tracks: Vec<TrackDigest>,
    pub catalog: CatalogEvidence,
}

/// Regenerate a Redumper dump's ordered track set in disposable local storage
/// and append catalog evidence when every expected digest matches.
#[allow(clippy::too_many_lines)]
pub fn verify_dump_against_catalog(
    request: &CatalogVerificationRequest,
    progress: &dyn Fn(&str, u64, u64),
    cancelled: &AtomicBool,
) -> Result<bool, PlayableBuildError> {
    progress("Reading archive manifests", 0, 0);
    let snapshot = scan_archive(&request.archive_root)
        .map_err(|error| PlayableBuildError::Message(error.to_string()))?;
    let dump = snapshot
        .releases
        .iter()
        .flat_map(|release| &release.physical_copies)
        .flat_map(|copy| &copy.carriers)
        .flat_map(|carrier| &carrier.dumps)
        .find(|dump| dump.manifest.dump_id.to_string() == request.dump_id)
        .ok_or_else(|| {
            PlayableBuildError::Message(format!("archive dump {} was not found", request.dump_id))
        })?;
    if catalog_verified(dump) {
        return Ok(false);
    }
    if request.expected_tracks.is_empty() {
        return Err(PlayableBuildError::Message(
            "The selected catalog medium has no expected track hashes".to_owned(),
        ));
    }
    let (actual_tracks, tool, detail) = if dump.manifest.format == RepresentationFormat::RedumperRaw
    {
        progress(
            "Copying raw dump to local workspace for Redumper verification",
            0,
            0,
        );
        let redumper = Redumper::detect(&request.redumper_path)
            .map_err(|error| PlayableBuildError::Message(error.to_string()))?;
        progress("Regenerating and hashing the complete disc track set", 0, 0);
        let prepared = redumper
            .prepare_with_progress(
                &dump.directory.join("raw"),
                &request.workspace_root,
                cancelled,
                |current, total| {
                    progress(
                        "Copying raw dump to local workspace for Redumper verification",
                        current,
                        total,
                    );
                },
            )
            .map_err(|error| PlayableBuildError::Message(error.to_string()))?;
        let cache = prepared_cache_directory(&request.workspace_root, &dump.manifest_sha256);
        if cache.exists() {
            std::fs::remove_dir_all(&cache)?;
        }
        cache_prepared_intermediate(&prepared, &cache, cancelled)?;
        (
            prepared.audit.tracks.clone(),
            Some(prepared.audit.tool.clone()),
            "Redumper regenerated a complete ordered track set matching the catalog".to_owned(),
        )
    } else {
        let [file] = dump.manifest.files.as_slice() else {
            return Err(PlayableBuildError::Message(format!(
                "Automatic catalog verification is not available for multi-file {:?} preservation masters",
                dump.manifest.format
            )));
        };
        progress(
            "Hashing the preservation master against the catalog",
            0,
            file.size,
        );
        let path = dump.directory.join("raw").join(&file.path);
        let context = crate::create_default_context();
        let analyzer = context.get_by_short_name(&request.catalog.system);
        let actual = if let Some(analyzer) = analyzer {
            let mut input = std::fs::File::open(&path)?;
            let hashes = crate::hasher::compute_all_hashes(
                &mut input,
                analyzer.analyzer.as_ref(),
                Some(&path),
            )
            .map_err(|error| PlayableBuildError::Message(error.to_string()))?;
            TrackDigest {
                number: request.expected_tracks[0].number,
                size: hashes.data_size,
                crc32: hashes.crc32,
                md5: hashes.md5.unwrap_or_default(),
                sha1: hashes.sha1.unwrap_or_default(),
            }
        } else {
            let hashes = retro_junk_archive::hash_file_digests(&path, cancelled)
                .map_err(|error| PlayableBuildError::Message(error.to_string()))?;
            TrackDigest {
                number: request.expected_tracks[0].number,
                size: hashes.size,
                crc32: hashes.crc32,
                md5: hashes.md5,
                sha1: hashes.sha1,
            }
        };
        (
            vec![actual],
            None,
            "The preservation master matched the catalog's normalized file hashes".to_owned(),
        )
    };
    let complete = request.expected_tracks.len() == actual_tracks.len()
        && request
            .expected_tracks
            .iter()
            .zip(&actual_tracks)
            .all(|(expected, actual)| {
                expected.number == actual.number
                    && expected.size == actual.size
                    && expected.sha1.eq_ignore_ascii_case(&actual.sha1)
                    && (expected.crc32.is_empty()
                        || expected.crc32.eq_ignore_ascii_case(&actual.crc32))
                    && (expected.md5.is_empty() || expected.md5.eq_ignore_ascii_case(&actual.md5))
            });
    if !complete {
        return Err(PlayableBuildError::Message(
            "Regenerated track hashes did not match the selected catalog disc".to_owned(),
        ));
    }
    let verification_id = VerificationId::new();
    let evidence = VerificationEvidence {
        schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
        verification_id,
        representation_id: dump.manifest.representation_id,
        performed_at: chrono::Utc::now().to_rfc3339(),
        input_manifest_sha256: dump.manifest_sha256.clone(),
        kind: VerificationKind::Catalog,
        outcome: VerificationOutcome::Verified,
        tool,
        catalog: Some(request.catalog.clone()),
        tracks: request
            .expected_tracks
            .iter()
            .zip(&actual_tracks)
            .map(|(expected, actual)| TrackVerification {
                number: expected.number,
                size: expected.size,
                expected_sha1: expected.sha1.clone(),
                actual_sha1: actual.sha1.clone(),
                matched: true,
            })
            .collect(),
        detail,
    };
    let evidence_directory = dump.directory.join("evidence");
    std::fs::create_dir_all(&evidence_directory)?;
    write_json_new(
        &evidence_directory.join(format!("verification-{verification_id}.json")),
        &evidence,
    )
    .map_err(|error| PlayableBuildError::Message(error.to_string()))?;
    Ok(true)
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
    if let Some(existing) = dump.builds.iter().find(|build| {
        build.evidence.format == request.format
            && retro_junk_archive::playable_presence(
                &request.playable_root,
                &dump.manifest_sha256,
                &build.evidence,
            ) == retro_junk_archive::RepresentationPresence::Present
    }) {
        progress("Preferred playable representation is already present", 1, 1);
        if request.expected_disc_count > 1 {
            project_selected_playlist(request, &request.dump_id)?;
        }
        return Ok(PlayableBuildOutcome {
            output: request
                .playable_root
                .join(&existing.evidence.relative_output_path),
            format: request.format.clone(),
        });
    }
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
    let mut prepared_cache = None;
    let input = match dump.manifest.format {
        RepresentationFormat::RedumperRaw => {
            let cache = prepared_cache_directory(&request.workspace_root, &dump.manifest_sha256);
            if cache.is_dir() {
                let input = find_input(&cache.join("raw"), &["cue", "iso"])?;
                prepared_cache = Some(cache);
                input
            } else {
                progress("Preparing Redumper files in the local workspace", 0, 0);
                let redumper = Redumper::detect(Path::new(""))
                    .map_err(|error| PlayableBuildError::Message(error.to_string()))?;
                let prepared = redumper
                    .prepare_with_progress(
                        &dump.directory.join("raw"),
                        &request.workspace_root,
                        cancelled,
                        |current, total| {
                            progress(
                                "Preparing Redumper files in the local workspace",
                                current,
                                total,
                            );
                        },
                    )
                    .map_err(|error| PlayableBuildError::Message(error.to_string()))?;
                let entrypoint = prepared.entrypoint.clone();
                redumper_workspace = Some(prepared);
                entrypoint
            }
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
        } else if let Some(cache) = prepared_cache.as_ref() {
            let files = retain_cached_intermediate(cache, &retained_path, cancelled)?;
            let format = if input
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
    if let Some(cache) = prepared_cache
        && let Err(error) = std::fs::remove_dir_all(&cache)
    {
        log::warn!(
            "could not remove prepared Redumper cache {}: {error}",
            cache.display()
        );
    }
    Ok(PlayableBuildOutcome {
        output: outcome.output,
        format: RepresentationFormat::Chd,
    })
}

fn prepared_cache_directory(workspace_root: &Path, manifest_sha256: &str) -> PathBuf {
    workspace_root
        .join("prepared-redumper")
        .join(manifest_sha256)
}

fn cache_prepared_intermediate(
    prepared: &retro_junk_archive::RedumperWorkspace,
    destination: &Path,
    cancelled: &AtomicBool,
) -> Result<(), PlayableBuildError> {
    let parent = destination.parent().ok_or_else(|| {
        PlayableBuildError::Message("prepared cache has no parent directory".to_owned())
    })?;
    std::fs::create_dir_all(parent)?;
    let staging = parent.join(format!(".prepared-staging-{}", BuildId::new()));
    let raw = staging.join("raw");
    std::fs::create_dir_all(&raw)?;
    let result = (|| {
        let mut sources =
            std::fs::read_dir(prepared.entrypoint.parent().ok_or_else(|| {
                PlayableBuildError::Message("invalid Redumper output".to_owned())
            })?)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| {
                        matches!(value.to_ascii_lowercase().as_str(), "cue" | "bin" | "iso")
                    })
            })
            .collect::<Vec<_>>();
        sources.sort();
        for source in sources {
            if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(PlayableBuildError::Message(
                    "operation cancelled".to_owned(),
                ));
            }
            let name = source.file_name().ok_or_else(|| {
                PlayableBuildError::Message(format!(
                    "prepared file has no filename: {}",
                    source.display()
                ))
            })?;
            std::fs::rename(&source, raw.join(name))?;
        }
        std::fs::rename(&staging, destination)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result
}

fn retain_cached_intermediate(
    cache: &Path,
    destination: &Path,
    cancelled: &AtomicBool,
) -> Result<Vec<retro_junk_archive::ArchivedFile>, PlayableBuildError> {
    let parent = destination.parent().ok_or_else(|| {
        PlayableBuildError::Message("canonical intermediate has no parent directory".to_owned())
    })?;
    std::fs::create_dir_all(parent)?;
    let staging = parent.join(format!(".intermediate-staging-{}", BuildId::new()));
    let staging_raw = staging.join("raw");
    std::fs::create_dir_all(&staging_raw)?;
    let result = (|| {
        let mut sources = std::fs::read_dir(cache.join("raw"))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_file())
            .collect::<Vec<_>>();
        sources.sort();
        let mut files = Vec::new();
        for source in sources {
            if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(PlayableBuildError::Message(
                    "operation cancelled".to_owned(),
                ));
            }
            let name = source.file_name().ok_or_else(|| {
                PlayableBuildError::Message(format!(
                    "prepared file has no filename: {}",
                    source.display()
                ))
            })?;
            let target = staging_raw.join(name);
            std::fs::copy(&source, &target)?;
            let digests = retro_junk_archive::hash_file_digests(&target, cancelled)
                .map_err(|error| PlayableBuildError::Message(error.to_string()))?;
            files.push(retro_junk_archive::ArchivedFile {
                path: name.to_string_lossy().into_owned(),
                size: digests.size,
                crc32: digests.crc32,
                md5: digests.md5,
                sha1: digests.sha1,
                sha256: digests.sha256,
            });
        }
        std::fs::rename(&staging, destination)?;
        Ok(files)
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result
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
            && verification
                .evidence
                .catalog
                .as_ref()
                .is_some_and(|catalog| catalog.complete_track_set)
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
        let request = PlayableBuildRequest {
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
        };
        let outcome = build_playable(&request, &|_, _, _| {}, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            std::fs::read(&outcome.output).unwrap(),
            b"preservation bytes"
        );
        build_playable(&request, &|_, _, _| {}, &AtomicBool::new(false)).unwrap();
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

    #[cfg(unix)]
    #[test]
    fn catalog_verification_stages_redumper_once_and_records_current_evidence() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("archive");
        let raw = temp.path().join("raw-source");
        let workspace = temp.path().join("work");
        std::fs::create_dir(&raw).unwrap();
        std::fs::write(raw.join("disc.scram"), b"raw master").unwrap();
        std::fs::write(raw.join("disc.state"), b"state").unwrap();
        retro_junk_archive::initialize_archive(
            &archive,
            &retro_junk_archive::ArchiveRootManifest::new("Test"),
        )
        .unwrap();
        let ingested = retro_junk_archive::ingest_new_carrier_dump(
            &archive,
            &raw,
            retro_junk_archive::NewCarrierDump {
                platform_id: "ps1".to_owned(),
                title: "Disc Game".to_owned(),
                region: "usa".to_owned(),
                revision: String::new(),
                variant: String::new(),
                owner_id: "default".to_owned(),
                physical_copy_label: String::new(),
                serial: "DISC-1".to_owned(),
                sequence_number: 1,
                carrier_label: String::new(),
                carrier_kind: retro_junk_archive::CarrierKind::OpticalDisc,
                format: RepresentationFormat::RedumperRaw,
                catalog_binding: retro_junk_archive::CatalogBinding::default(),
                source_package: retro_junk_archive::SourcePackageRecord::default(),
                expected_files: Vec::new(),
                physical_copy_id: None,
            },
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        let tool = temp.path().join("redumper");
        std::fs::write(
            &tool,
            r#"#!/bin/sh
if [ "$1" = "--help" ]; then echo "redumper build test"; exit 0; fi
if [ "$1" = "split" ]; then
  printf 'FILE "disc (Track 01).bin" BINARY\n' > disc.cue
  printf 'track' > 'disc (Track 01).bin'
fi
echo '<rom name="disc (Track 01).bin" size="5" crc="AABBCCDD" md5="0011" sha1="11223344" />'
"#,
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&tool).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&tool, permissions).unwrap();
        let request = CatalogVerificationRequest {
            archive_root: archive.clone(),
            workspace_root: workspace.clone(),
            dump_id: ingested.dump.dump_id.to_string(),
            redumper_path: tool,
            expected_tracks: vec![TrackDigest {
                number: 1,
                size: 5,
                crc32: "aabbccdd".to_owned(),
                md5: "0011".to_owned(),
                sha1: "11223344".to_owned(),
            }],
            catalog: CatalogEvidence {
                source: "redump".to_owned(),
                system: "ps1".to_owned(),
                version: "test".to_owned(),
                game: "Disc Game (USA)".to_owned(),
                complete_track_set: true,
            },
        };
        assert!(
            verify_dump_against_catalog(&request, &|_, _, _| {}, &AtomicBool::new(false)).unwrap()
        );
        let snapshot = scan_archive(&archive).unwrap();
        let dump = &snapshot.releases[0].physical_copies[0].carriers[0].dumps[0];
        assert!(catalog_verified(dump));
        assert!(prepared_cache_directory(&workspace, &dump.manifest_sha256).is_dir());
        assert!(
            !verify_dump_against_catalog(&request, &|_, _, _| {}, &AtomicBool::new(false)).unwrap()
        );
    }
}
