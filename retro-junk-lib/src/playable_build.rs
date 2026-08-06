//! Builds disposable playable projections from authoritative archive dumps.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use retro_junk_archive::{
    BuildEvidence, BuildId, CanonicalIntermediateEvidence, CapturedFeature, CatalogEvidence,
    IndexedCarrier, IndexedDump, IndexedRelease, RepresentationFormat, RepresentationId,
    ToolRecord, TrackDigest, TrackVerification, VerificationEvidence, VerificationId,
    VerificationKind, VerificationOutcome, scan_archive, sha256_file, write_json_new,
};
use retro_junk_io::{PhaseProgressFn, ProgressUnit};

#[derive(Debug, Clone)]
pub struct PlayableBuildRequest {
    pub archive_root: PathBuf,
    pub playable_root: PathBuf,
    pub workspace_root: PathBuf,
    pub dump_id: String,
    pub format: RepresentationFormat,
    pub chdman_path: PathBuf,
    pub redumper_path: PathBuf,
    pub dolphin_tool_path: PathBuf,
    pub allow_unverified: bool,
    pub retain_intermediate: bool,
    pub options: std::collections::BTreeMap<String, String>,
    /// Concrete console folder in the playable library (for example `psx`
    /// even when the archive/catalog canonical identifier is `ps1`).
    pub playable_platform_id: String,
    /// Logical catalog disc count. Values greater than one enable playlist
    /// projection only after every disc in this physical set has been built.
    pub expected_disc_count: u32,
    /// DAT-canonical filename stem used by the ordinary Rename operation.
    pub canonical_output_stem: String,
    /// DAT-canonical release name with any per-disc suffix removed.
    pub canonical_release_name: String,
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

/// Regenerate a Redumper dump's ordered track set in disposable workspace storage
/// and append catalog evidence when every expected digest matches.
#[allow(clippy::too_many_lines)]
pub fn verify_dump_against_catalog(
    request: &CatalogVerificationRequest,
    progress: &PhaseProgressFn<'_>,
    cancelled: &AtomicBool,
) -> Result<bool, PlayableBuildError> {
    progress("Reading archive manifests", ProgressUnit::Items, 0, 0);
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
    if retro_junk_archive::dump_catalog_verified(dump) {
        return Ok(false);
    }
    if request.expected_tracks.is_empty() {
        return Err(PlayableBuildError::Message(
            "The selected catalog medium has no expected track hashes".to_owned(),
        ));
    }
    let (actual_tracks, tool, detail) = if dump.manifest.format == RepresentationFormat::RedumperRaw
    {
        // Kept for whatever builds this dump next: the split CUE/BIN files are
        // exactly what a CHD conversion needs, and reproducing them is the
        // expensive part.
        let prepared = crate::redumper_cache::prepare(
            &request.redumper_path,
            &dump.directory.join("raw"),
            &request.workspace_root,
            &dump.manifest_sha256,
            progress,
            cancelled,
        )?;
        let audit = prepared.audit().clone();
        prepared.keep();
        (
            audit.tracks,
            Some(audit.tool),
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
            ProgressUnit::Bytes,
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
    progress: &PhaseProgressFn<'_>,
    cancelled: &AtomicBool,
) -> Result<PlayableBuildOutcome, PlayableBuildError> {
    progress("Reading archive manifests", ProgressUnit::Items, 0, 0);
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
    // Where the output is, not where it was written: looking only at the
    // recorded path re-converted a disc that was already sitting in the
    // frontend's folder, then left two copies of it on disk.
    let existing = dump.builds.iter().find_map(|build| {
        if build.evidence.format != request.format {
            return None;
        }
        let (relative, presence) = crate::playable_location::release_output_presence(
            release,
            &request.playable_root,
            dump,
            &build.evidence,
        );
        (presence == retro_junk_archive::RepresentationPresence::Present).then_some(relative)
    });
    if let Some(relative) = existing {
        progress(
            "Preferred playable representation is already present",
            ProgressUnit::Items,
            1,
            1,
        );
        if request.expected_disc_count > 1 {
            project_selected_playlist(request, &request.dump_id)?;
        }
        return Ok(PlayableBuildOutcome {
            output: request.playable_root.join(relative),
            format: request.format.clone(),
        });
    }
    let outcome = match request.format {
        RepresentationFormat::Chd => {
            build_chd(request, release, carrier, dump, progress, cancelled)
        }
        RepresentationFormat::Rvz => {
            build_rvz(request, release, carrier, dump, progress, cancelled)
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
        progress("Updating multi-disc playlist", ProgressUnit::Items, 0, 0);
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
    progress: &PhaseProgressFn<'_>,
    cancelled: &AtomicBool,
) -> Result<PlayableBuildOutcome, PlayableBuildError> {
    let catalog_verified = retro_junk_archive::dump_catalog_verified(dump);
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
    // Raw dumps reach a converter only through the shared prepared-output
    // cache, so identification and this build never split the same dump twice.
    let mut prepared_cache = None;
    let input = match dump.manifest.format {
        RepresentationFormat::RedumperRaw => {
            let prepared = crate::redumper_cache::prepare(
                &request.redumper_path,
                &dump.directory.join("raw"),
                &request.workspace_root,
                &dump.manifest_sha256,
                progress,
                cancelled,
            )?;
            let input = prepared.entrypoint()?;
            prepared_cache = Some(prepared.directory().to_path_buf());
            prepared.keep();
            input
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
        playable_output_stem(request, release, carrier)
    ));
    if job.output.exists() {
        return Err(PlayableBuildError::Message(format!(
            "Playable output already exists: {}",
            job.output.display()
        )));
    }
    progress(
        "Compressing and round-trip verifying CHD",
        ProgressUnit::Bytes,
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
            progress(
                "Compressing and round-trip verifying CHD",
                ProgressUnit::Bytes,
                done,
                total,
            );
        },
        cancelled,
    )
    .map_err(|error| PlayableBuildError::Message(error.to_string()))?;
    if !outcome.is_verified() {
        return Err(PlayableBuildError::Message(
            "CHD round-trip verification failed".to_owned(),
        ));
    }
    progress("Recording build evidence", ProgressUnit::Items, 0, 0);
    let (output_size, output_sha256) = sha256_file(&outcome.output, cancelled)
        .map_err(|error| PlayableBuildError::Message(error.to_string()))?;
    let build_id = BuildId::new();
    let retained_path = dump
        .directory
        .join("intermediates")
        .join(build_id.to_string());
    // Every raw-dump build now reads its split files from the shared cache, so
    // there is one way to promote them into the archive rather than one per
    // place the files might have come from.
    let canonical_intermediate = match prepared_cache
        .as_ref()
        .filter(|_| request.retain_intermediate)
    {
        Some(cache) => {
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
        }
        None => None,
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

#[allow(clippy::too_many_lines)]
fn build_rvz(
    request: &PlayableBuildRequest,
    release: &IndexedRelease,
    carrier: &IndexedCarrier,
    dump: &IndexedDump,
    progress: &PhaseProgressFn<'_>,
    cancelled: &AtomicBool,
) -> Result<PlayableBuildOutcome, PlayableBuildError> {
    if dump.manifest.format != RepresentationFormat::Iso || dump.manifest.files.len() != 1 {
        return Err(PlayableBuildError::Message(
            "RVZ builds require a single-file ISO preservation master".to_owned(),
        ));
    }
    let catalog_verified = retro_junk_archive::dump_catalog_verified(dump);
    if !catalog_verified && !request.allow_unverified {
        return Err(PlayableBuildError::Message(
            "The current dump has no catalog verification. Verify it first, or enable unverified builds for this policy.".to_owned(),
        ));
    }
    let dolphin_tool = if request.dolphin_tool_path.as_os_str().is_empty() {
        PathBuf::from("DolphinTool")
    } else {
        request.dolphin_tool_path.clone()
    };
    progress("Checking DolphinTool", ProgressUnit::Items, 0, 0);
    let help = std::process::Command::new(&dolphin_tool)
        .arg("--help")
        .output()
        .map_err(|error| {
            PlayableBuildError::Message(format!("Could not run DolphinTool: {error}"))
        })?;
    let banner = format!(
        "{}\n{}",
        String::from_utf8_lossy(&help.stdout),
        String::from_utf8_lossy(&help.stderr)
    );
    if !help.status.success()
        || (!banner.to_ascii_lowercase().contains("dolphin")
            && !banner.to_ascii_lowercase().contains("convert"))
    {
        return Err(PlayableBuildError::Message(
            "DolphinTool did not provide a recognized help response".to_owned(),
        ));
    }
    let output_directory = playable_output_directory(request, release, carrier);
    std::fs::create_dir_all(&output_directory)?;
    let output = output_directory.join(format!(
        "{}.rvz",
        playable_output_stem(request, release, carrier)
    ));
    if output.exists() {
        return Err(PlayableBuildError::Message(format!(
            "Playable output already exists: {}",
            output.display()
        )));
    }
    let build_id = BuildId::new();
    let temporary_output = output_directory.join(format!(".{build_id}.rvz.tmp"));
    let workspace = request.workspace_root.join(format!("rvz-build-{build_id}"));
    std::fs::create_dir_all(&workspace)?;
    let round_trip = workspace.join("round-trip.iso");
    let input = dump
        .directory
        .join("raw")
        .join(&dump.manifest.files[0].path);
    let block_size = request
        .options
        .get("block_size")
        .map_or("131072", String::as_str);
    let compression = request
        .options
        .get("compression")
        .map_or("zstd", String::as_str);
    let level = request
        .options
        .get("compression_level")
        .map_or("5", String::as_str);
    progress(
        "Converting ISO to RVZ",
        ProgressUnit::Bytes,
        0,
        dump.manifest.files[0].size,
    );
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
        .map_err(|error| PlayableBuildError::Message(error.to_string()))?;
    if !convert.status.success() {
        let _ = std::fs::remove_file(&temporary_output);
        let _ = std::fs::remove_dir_all(&workspace);
        return Err(PlayableBuildError::Message(format!(
            "DolphinTool RVZ conversion failed: {}",
            String::from_utf8_lossy(&convert.stderr)
        )));
    }
    if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
        let _ = std::fs::remove_file(&temporary_output);
        let _ = std::fs::remove_dir_all(&workspace);
        return Err(PlayableBuildError::Message(
            "operation cancelled".to_owned(),
        ));
    }
    progress(
        "Round-trip verifying RVZ",
        ProgressUnit::Bytes,
        0,
        dump.manifest.files[0].size,
    );
    let extract = std::process::Command::new(&dolphin_tool)
        .args(["convert", "-i"])
        .arg(&temporary_output)
        .arg("-o")
        .arg(&round_trip)
        .args(["-f", "iso"])
        .output()
        .map_err(|error| PlayableBuildError::Message(error.to_string()))?;
    let verified = if extract.status.success() {
        let (_, original_sha256) = sha256_file(&input, cancelled)
            .map_err(|error| PlayableBuildError::Message(error.to_string()))?;
        let (_, round_trip_sha256) = sha256_file(&round_trip, cancelled)
            .map_err(|error| PlayableBuildError::Message(error.to_string()))?;
        original_sha256 == round_trip_sha256
    } else {
        false
    };
    let _ = std::fs::remove_dir_all(&workspace);
    if !verified {
        let _ = std::fs::remove_file(&temporary_output);
        return Err(PlayableBuildError::Message(
            "RVZ round-trip ISO did not match the preservation master".to_owned(),
        ));
    }
    let (output_size, output_sha256) = sha256_file(&temporary_output, cancelled)
        .map_err(|error| PlayableBuildError::Message(error.to_string()))?;
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
        relative_output_path: relative(&request.playable_root, &output),
        output_sha256,
        output_size,
        catalog_verified,
        round_trip_verified: true,
        tool: Some(ToolRecord {
            name: "DolphinTool".to_owned(),
            version: banner.lines().next().unwrap_or_default().trim().to_owned(),
            build: String::new(),
        }),
        omitted_features: Vec::new(),
        canonical_intermediate: None,
    };
    write_evidence(dump, &evidence, &output)?;
    Ok(PlayableBuildOutcome {
        output,
        format: RepresentationFormat::Rvz,
    })
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
    progress: &PhaseProgressFn<'_>,
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
        playable_output_stem(request, release, carrier),
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
        ProgressUnit::Bytes,
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
        catalog_verified: retro_junk_archive::dump_catalog_verified(dump),
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
    if let Err(error) = retro_junk_archive::write_build_evidence(&dump.directory, evidence) {
        // An output no evidence names is worse than no output: nothing would
        // ever find it again.
        let _ = std::fs::remove_file(output);
        return Err(PlayableBuildError::Message(error.to_string()));
    }
    Ok(())
}

pub(crate) fn find_input(
    directory: &Path,
    extensions: &[&str],
) -> Result<PathBuf, PlayableBuildError> {
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

/// What a carrier's playable file is called.
///
/// One scheme, whether or not the carrier resolved to a catalog medium. It
/// used to be two: a bound carrier took the catalog's name (`Castlevania -
/// Symphony of the Night (USA)`) and an unbound one was slugified
/// (`castlevania-symphony-of-the-night-usa`). A library holding both read as
/// two collections, and — worse — binding a carrier later silently changed
/// what its playable was called, so a rebuild left the old file behind under
/// the old name. The archive's own title is the same title the catalog would
/// have supplied, so there is no reason for the two to look different.
///
/// Both paths go through [`retro_junk_archive::safe_file_stem`] here rather
/// than at their sources, so a catalog name that is not a legal filename
/// cannot reach the filesystem by either route.
/// Name inputs for one carrier's output, in the shape the one rule takes.
///
/// The caller resolved the catalog name already (it has the connection); an
/// empty one leaves the archive manifest to name the file.
fn name_inputs<'a>(
    request: &'a PlayableBuildRequest,
    release: &'a IndexedRelease,
    carrier: &'a IndexedCarrier,
) -> crate::naming::NameInputs<'a> {
    crate::naming::NameInputs {
        // A resolved catalog stem is already whole-medium-corrected, so it is
        // passed as the name itself rather than re-derived from a rom_name.
        dat_name: request.canonical_output_stem.trim(),
        rom_name: "",
        medium_has_tracks: false,
        title: &release.manifest.title,
        region: &release.manifest.region,
        revision: &release.manifest.revision,
        variant: &release.manifest.variant,
        disc_number: carrier.manifest.sequence_number,
        disc_count: request.expected_disc_count,
    }
}

fn playable_output_stem(
    request: &PlayableBuildRequest,
    release: &IndexedRelease,
    carrier: &IndexedCarrier,
) -> String {
    crate::naming::canonical_stem(&name_inputs(request, release, carrier)).0
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
        base.join(format!("{}.m3u", canonical_release_name(request, release)))
    } else {
        base
    }
}

/// What a multi-disc set's `.m3u` directory is called: the release's name
/// with no disc suffix, since the folder holds every disc.
fn canonical_release_name(request: &PlayableBuildRequest, release: &IndexedRelease) -> String {
    crate::naming::canonical_release_stem(&crate::naming::NameInputs {
        dat_name: request.canonical_release_name.trim(),
        rom_name: "",
        medium_has_tracks: false,
        title: &release.manifest.title,
        region: &release.manifest.region,
        revision: &release.manifest.revision,
        variant: &release.manifest.variant,
        disc_number: 0,
        disc_count: 0,
    })
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
        // Each disc is listed at the location it is actually in. A disc whose
        // evidence names the archive's platform folder was invisible here, so
        // the set failed its own "every disc present" check and no playlist
        // was written for a set that was complete on disk.
        let disc = carrier
            .dumps
            .iter()
            .flat_map(|dump| dump.builds.iter().map(move |build| (dump, build)))
            .find_map(|(dump, build)| {
                if build.evidence.format != request.format {
                    return None;
                }
                let (relative, presence) = crate::playable_location::release_output_presence(
                    release,
                    &request.playable_root,
                    dump,
                    &build.evidence,
                );
                (presence == retro_junk_archive::RepresentationPresence::Present)
                    .then_some(relative)
            });
        if let Some(relative) = disc {
            discs.push((sequence, relative));
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
        .join(format!("{}.m3u", canonical_release_name(request, release)));
    std::fs::create_dir_all(&directory)?;
    let playlist = directory.join(format!("{}.m3u", canonical_release_name(request, release)));
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
                join_release: None,
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
            redumper_path: PathBuf::new(),
            dolphin_tool_path: PathBuf::new(),
            allow_unverified: false,
            retain_intermediate: false,
            options: std::collections::BTreeMap::new(),
            playable_platform_id: "nes".to_owned(),
            expected_disc_count: 1,
            canonical_output_stem: String::new(),
            canonical_release_name: String::new(),
        };
        let outcome = build_playable(&request, &|_, _, _, _| {}, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            std::fs::read(&outcome.output).unwrap(),
            b"preservation bytes"
        );
        build_playable(&request, &|_, _, _, _| {}, &AtomicBool::new(false)).unwrap();
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
                    join_release: None,
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
        let request = |dump_id: String, sequence: u32| PlayableBuildRequest {
            archive_root: archive.clone(),
            playable_root: playable.clone(),
            workspace_root: temp.path().join("work"),
            dump_id,
            format: RepresentationFormat::Rom,
            chdman_path: PathBuf::new(),
            redumper_path: PathBuf::new(),
            dolphin_tool_path: PathBuf::new(),
            allow_unverified: false,
            retain_intermediate: false,
            options: std::collections::BTreeMap::new(),
            playable_platform_id: "psx".to_owned(),
            expected_disc_count: 2,
            canonical_output_stem: format!("Two Disc Game (USA) (Disc {sequence})"),
            canonical_release_name: "Two Disc Game (USA)".to_owned(),
        };
        build_playable(
            &request(ingested[0].clone(), 1),
            &|_, _, _, _| {},
            &AtomicBool::new(false),
        )
        .unwrap();
        let set_dir = playable.join("psx/Two Disc Game (USA).m3u");
        assert!(!set_dir.join("Two Disc Game (USA).m3u").exists());
        build_playable(
            &request(ingested[1].clone(), 2),
            &|_, _, _, _| {},
            &AtomicBool::new(false),
        )
        .unwrap();
        let playlist = std::fs::read_to_string(set_dir.join("Two Disc Game (USA).m3u")).unwrap();
        assert_eq!(
            playlist,
            "Two Disc Game (USA) (Disc 1).bin\nTwo Disc Game (USA) (Disc 2).bin\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn rvz_build_round_trips_through_shared_builder() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("archive");
        let playable = temp.path().join("playable");
        let source = temp.path().join("game.iso");
        std::fs::write(&source, b"disc image").unwrap();
        retro_junk_archive::initialize_archive(
            &archive,
            &retro_junk_archive::ArchiveRootManifest::new("Test"),
        )
        .unwrap();
        let ingested = retro_junk_archive::ingest_new_carrier_dump(
            &archive,
            &source,
            retro_junk_archive::NewCarrierDump {
                platform_id: "gamecube".to_owned(),
                title: "Game".to_owned(),
                region: String::new(),
                revision: String::new(),
                variant: String::new(),
                owner_id: "default".to_owned(),
                physical_copy_label: String::new(),
                serial: String::new(),
                sequence_number: 0,
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
if [ "$1" = "--help" ]; then echo "Dolphin convert"; exit 0; fi
input=""
output=""
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
        let request = PlayableBuildRequest {
            archive_root: archive.clone(),
            playable_root: playable,
            workspace_root: temp.path().join("work"),
            dump_id: ingested.dump.dump_id.to_string(),
            format: RepresentationFormat::Rvz,
            chdman_path: PathBuf::new(),
            redumper_path: PathBuf::new(),
            dolphin_tool_path: tool,
            allow_unverified: true,
            retain_intermediate: false,
            options: std::collections::BTreeMap::new(),
            playable_platform_id: "gamecube".to_owned(),
            expected_disc_count: 1,
            canonical_output_stem: "Game".to_owned(),
            canonical_release_name: "Game".to_owned(),
        };
        let outcome = build_playable(&request, &|_, _, _, _| {}, &AtomicBool::new(false)).unwrap();
        assert_eq!(std::fs::read(&outcome.output).unwrap(), b"disc image");
        let snapshot = scan_archive(&archive).unwrap();
        let evidence =
            &snapshot.releases[0].physical_copies[0].carriers[0].dumps[0].builds[0].evidence;
        assert_eq!(evidence.format, RepresentationFormat::Rvz);
        assert!(evidence.round_trip_verified);
        assert!(!evidence.catalog_verified);
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
                join_release: None,
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
            verify_dump_against_catalog(&request, &|_, _, _, _| {}, &AtomicBool::new(false))
                .unwrap()
        );
        let snapshot = scan_archive(&archive).unwrap();
        let dump = &snapshot.releases[0].physical_copies[0].carriers[0].dumps[0];
        assert!(retro_junk_archive::dump_catalog_verified(dump));
        // Verification leaves the split output behind on purpose, so the build
        // that follows it does not copy and split the same raw dump again.
        let cache = crate::redumper_cache::cache_directory(&workspace, &dump.manifest_sha256);
        assert!(cache.join("raw").is_dir());
        assert!(cache.join("audit.json").is_file());
        assert!(
            !verify_dump_against_catalog(&request, &|_, _, _, _| {}, &AtomicBool::new(false))
                .unwrap()
        );
    }
}
