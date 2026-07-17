//! CHD compression via an external `chdman` (MAME tools) process.
//!
//! There is no Rust implementation of CHD *creation* (the `chd` crate is
//! read-only by design), so compression shells out to chdman — the canonical
//! MAME tool every emulator's CHD support is built against. Which platforms
//! and source extensions are eligible is analyzer knowledge, declared via
//! [`RomAnalyzer::chd_media_for_extension`].
//!
//! Reliability model: after `createcd`/`createdvd` succeeds, the CHD is
//! **round-trip verified** — extracted back with chdman into a temp
//! directory, and every track's content compared byte-for-byte against the
//! original source files (track spans via `retro_junk_disc::track_layout`,
//! so multi-bin, single-bin, GDI, and ISO layouts all compare 1:1). Only a
//! verified CHD is reported as safe; source deletion is a separate,
//! caller-driven step gated on that verification.
//!
//! Output ownership: the CHD is built at a hidden temp path and only
//! `rename`d into place after verification succeeds (see [`compress_to_chd`]).
//! Every failure path removes only that temp file — a job never deletes a
//! `.chd` it did not itself create.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use retro_junk_core::{ChdExtensionRole, ChdMedia, RomAnalyzer};
use retro_junk_disc::track_layout::{TrackSpan, cue_track_spans, gdi_track_spans, parse_gdi};

/// chdman could not be found or executed.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{reason}")]
pub struct ChdmanUnavailable {
    /// What went wrong (path tried, spawn error).
    pub reason: String,
}

impl ChdmanUnavailable {
    /// Platform-appropriate installation instructions, shown to users so
    /// they know why disc rips can't be compressed and how to fix it.
    pub fn install_hint() -> &'static str {
        if cfg!(target_os = "macos") {
            "chdman is part of MAME's ROM tools. Install with Homebrew: `brew install rom-tools`."
        } else if cfg!(target_os = "windows") {
            "chdman.exe ships with MAME (https://www.mamedev.org/release.html). \
             Install MAME, or place chdman.exe anywhere and set its location in Settings."
        } else {
            "chdman is part of MAME's tools. Install your distribution's package: \
             `mame-tools` (Arch, Debian/Ubuntu, Fedora)."
        }
    }
}

/// A located, working chdman executable.
#[derive(Debug, Clone)]
pub struct Chdman {
    pub path: PathBuf,
    /// Version string parsed from the banner (e.g. "0.288"). Empty when the
    /// banner carried no recognizable version.
    pub version: String,
}

impl Chdman {
    /// Locate chdman: an explicit override path if given (from settings),
    /// otherwise `chdman` on PATH (pass an empty path for PATH lookup).
    /// Runs it once to confirm it executes and to capture its version banner.
    pub fn detect(override_path: &Path) -> Result<Chdman, ChdmanUnavailable> {
        let use_path_lookup = override_path.as_os_str().is_empty();
        let candidate: PathBuf = if use_path_lookup {
            PathBuf::from("chdman")
        } else {
            override_path.to_path_buf()
        };

        // chdman with no arguments prints its banner + usage and exits
        // non-zero; a failed *spawn* is what distinguishes "not installed".
        let output = Command::new(&candidate)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| ChdmanUnavailable {
                reason: if use_path_lookup {
                    format!("chdman not found on PATH ({e})")
                } else {
                    format!("Cannot run chdman at {}: {e}", override_path.display())
                },
            })?;

        let banner = String::from_utf8_lossy(&output.stdout).to_string()
            + &String::from_utf8_lossy(&output.stderr);
        if !banner.contains("Compressed Hunks of Data") {
            return Err(ChdmanUnavailable {
                reason: format!(
                    "{} does not appear to be chdman (no CHD banner in output)",
                    candidate.display()
                ),
            });
        }

        Ok(Chdman {
            path: candidate,
            version: parse_chdman_version(&banner),
        })
    }

    /// Detect chdman from a settings string: empty/whitespace means "use PATH".
    pub fn detect_from_setting(setting: &str) -> Result<Chdman, ChdmanUnavailable> {
        Self::detect(Path::new(setting.trim()))
    }
}

/// Parse the version from chdman's banner line:
/// `chdman - MAME Compressed Hunks of Data (CHD) manager 0.288 (mame0288-dirty)`
///
/// Returns an empty string when the banner carries no recognizable version.
fn parse_chdman_version(banner: &str) -> String {
    banner
        .lines()
        .find(|l| l.contains("manager"))
        .and_then(|line| line.split("manager").nth(1))
        .and_then(|after| after.trim().split_whitespace().next())
        .filter(|version| version.starts_with(|c: char| c.is_ascii_digit()))
        .unwrap_or_default()
        .to_string()
}

/// Why a file cannot be compressed to CHD. Display text derives from this;
/// frontends branch on it instead of string-matching extensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceSkipClass {
    /// Already a .chd.
    AlreadyChd,
    /// Track/data file referenced by a sheet; compress the sheet instead.
    CompanionData,
    /// A container format chdman cannot read (declared by the analyzer —
    /// e.g. CSO/DAX, CDI). See [`RomAnalyzer::chd_extensions`].
    UnreadableContainer,
    /// No conversion is defined for this platform+extension.
    NoConversion,
}

impl SourceSkipClass {
    fn hint(&self) -> &'static str {
        match self {
            SourceSkipClass::AlreadyChd => " (already a CHD)",
            SourceSkipClass::CompanionData => {
                " (compress the .cue/.gdi that references it instead)"
            }
            SourceSkipClass::UnreadableContainer => " (a container format chdman cannot read)",
            SourceSkipClass::NoConversion => "",
        }
    }
}

/// Errors from planning or running a compression.
#[derive(Debug, thiserror::Error)]
pub enum ChdConvertError {
    #[error("{0}")]
    ChdmanUnavailable(#[from] ChdmanUnavailable),
    #[error("{platform} cannot compress .{extension} files to CHD{}", class.hint())]
    UnsupportedSource {
        platform: String,
        extension: String,
        class: SourceSkipClass,
    },
    #[error("output already exists: {}", .0.display())]
    OutputExists(PathBuf),
    #[error(
        "output {} already planned for another input in this batch",
        .0.display()
    )]
    DuplicateOutput(PathBuf),
    #[error("source is incomplete; missing referenced file(s): {}", .0.join(", "))]
    BrokenSource(Vec<String>),
    #[error("cannot verify layout: {detail}")]
    UnsupportedLayout { detail: String },
    #[error("chdman failed: {detail}")]
    ChdmanFailed { detail: String },
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Analysis(#[from] retro_junk_core::AnalysisError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// A planned single-disc compression: one cue/gdi/iso input → one .chd.
#[derive(Debug, Clone)]
pub struct CompressionJob {
    /// The file handed to chdman (`.cue`, `.gdi`, or `.iso`).
    pub input: PathBuf,
    pub media: ChdMedia,
    /// Target `.chd` path (input with its extension replaced).
    pub output: PathBuf,
    /// Every file the source comprises: the input itself plus all track
    /// files it references. This is the set deleted by
    /// [`delete_job_sources`] after a verified compression.
    pub source_files: Vec<PathBuf>,
    /// Total size of `source_files` in bytes.
    pub input_bytes: u64,
}

/// Build a [`CompressionJob`] for a disc image, validating eligibility.
///
/// `input` is the container the library entry points at (cue/gdi/iso). The
/// analyzer decides whether this platform+extension pair has a supported,
/// lossless CHD conversion.
pub fn plan_compression(
    input: &Path,
    analyzer: &dyn RomAnalyzer,
) -> Result<CompressionJob, ChdConvertError> {
    let extension = input
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let Some(media) = analyzer.chd_media_for_extension(&extension) else {
        let class = if extension == "chd" {
            SourceSkipClass::AlreadyChd
        } else if analyzer
            .chd_extensions()
            .iter()
            .any(|(e, role)| *e == extension && matches!(role, ChdExtensionRole::Unconvertible))
        {
            SourceSkipClass::UnreadableContainer
        } else if matches!(extension.as_str(), "bin" | "img") {
            SourceSkipClass::CompanionData
        } else {
            SourceSkipClass::NoConversion
        };
        return Err(ChdConvertError::UnsupportedSource {
            platform: analyzer.platform_name().to_string(),
            extension,
            class,
        });
    };

    let output = input.with_extension("chd");
    if output.exists() {
        return Err(ChdConvertError::OutputExists(output));
    }

    // Resolve every file the source comprises. Dispatch is exhaustive over
    // the extensions the analyzer table can ever hand us as `media`: `cue`
    // and `gdi` need their track-layout files resolved; `iso` is already a
    // single whole-disc file. Any other extension an analyzer declares as a
    // CHD `Source` would have no track-layout handling here or in
    // `layout_spans`, so it is rejected loudly at plan time instead of
    // silently mishandled.
    let mut source_files = vec![input.to_path_buf()];
    let dir = input.parent().unwrap_or(Path::new("."));
    match extension.as_str() {
        "cue" => {
            let files = crate::disc_set::expand_disc_set(input)?;
            if !files.missing.is_empty() {
                return Err(ChdConvertError::BrokenSource(files.missing));
            }
            source_files.extend(files.tracks);

            // Reject PREGAP/POSTGAP-declaring sheets: chdman synthesizes
            // these gaps into the CHD and materializes them on extraction,
            // so the extracted track is longer than the source span and a
            // byte-exact round-trip comparison can never succeed. Fail here
            // at plan time instead of after a full compress+extract cycle.
            let sheet = retro_junk_disc::cue::parse_cue(&fs::read_to_string(input)?)?;
            if sheet
                .files
                .iter()
                .flat_map(|f| &f.tracks)
                .any(|t| t.pregap_frames > 0 || t.postgap_frames > 0)
            {
                return Err(ChdConvertError::UnsupportedLayout {
                    detail: "CUE declares PREGAP/POSTGAP gaps that are not stored in the \
                             track files; chdman synthesizes them into the CHD, so a \
                             byte-exact round-trip comparison is impossible"
                        .to_string(),
                });
            }
        }
        "gdi" => {
            let tracks = parse_gdi(&fs::read_to_string(input)?)?;
            let mut missing = Vec::new();
            for t in &tracks {
                let path = dir.join(&t.filename);
                if path.is_file() {
                    source_files.push(path);
                } else {
                    missing.push(t.filename.clone());
                }
            }
            if !missing.is_empty() {
                return Err(ChdConvertError::BrokenSource(missing));
            }
        }
        "iso" => {}
        other => {
            return Err(ChdConvertError::UnsupportedLayout {
                detail: format!(
                    ".{other} is declared CHD-convertible but no track-layout handling exists for it"
                ),
            });
        }
    }

    let mut input_bytes = 0;
    for f in &source_files {
        input_bytes += fs::metadata(f)?.len();
    }

    Ok(CompressionJob {
        input: input.to_path_buf(),
        media,
        output,
        source_files,
        input_bytes,
    })
}

/// One input that couldn't become a job, with the reason.
pub struct PlanSkip {
    pub input: PathBuf,
    /// Carries a [`SourceSkipClass`] when the error is `UnsupportedSource`.
    pub error: ChdConvertError,
}

/// Result of planning a whole batch of inputs.
pub struct PlannedBatch {
    pub jobs: Vec<CompressionJob>,
    /// Skips worth telling the user about (everything except `CompanionData`).
    pub skips: Vec<PlanSkip>,
    pub already_chd: usize,
}

/// Plan every input, classify skips, and reject duplicate outputs: if two
/// inputs (e.g. same-stem cue+iso) map to one .chd, the first wins and the
/// second becomes a skip (guards the output-ownership protocol at plan time —
/// see [`compress_to_chd`]).
///
/// This is the single place `SourceSkipClass` policy lives: `AlreadyChd` is
/// only counted, `CompanionData` is dropped silently (noise inside
/// multi-disc/companion folders), everything else is reported as a skip.
pub fn plan_batch(inputs: &[PathBuf], analyzer: &dyn RomAnalyzer) -> PlannedBatch {
    let mut jobs = Vec::new();
    let mut skips = Vec::new();
    let mut already_chd = 0usize;
    let mut planned_outputs: HashSet<PathBuf> = HashSet::new();

    for input in inputs {
        match plan_compression(input, analyzer) {
            Ok(job) => {
                if !planned_outputs.insert(job.output.clone()) {
                    skips.push(PlanSkip {
                        input: input.clone(),
                        error: ChdConvertError::DuplicateOutput(job.output.clone()),
                    });
                    continue;
                }
                jobs.push(job);
            }
            Err(ChdConvertError::UnsupportedSource {
                class: SourceSkipClass::AlreadyChd,
                ..
            }) => {
                already_chd += 1;
            }
            Err(ChdConvertError::UnsupportedSource {
                class: SourceSkipClass::CompanionData,
                ..
            }) => {
                // Track/data files that a cue/gdi in this same batch already
                // covers; both frontends agreed this is noise.
            }
            Err(error) => skips.push(PlanSkip {
                input: input.clone(),
                error,
            }),
        }
    }

    PlannedBatch {
        jobs,
        skips,
        already_chd,
    }
}

/// Progress phases of one compression job, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChdPhase {
    /// chdman createcd/createdvd running.
    Compressing,
    /// chdman extractcd/extractdvd re-expanding the CHD for verification.
    Extracting,
    /// Comparing source and extracted tracks byte-for-byte.
    Verifying,
}

/// Progress callback: `(phase, fraction_within_phase 0.0..=1.0)`.
pub type ChdProgressFn<'a> = &'a dyn Fn(ChdPhase, f64);

/// Fold a per-phase fraction into a single 0.0..=1.0 job fraction.
///
/// Compression dominates wall time; extraction and hashing are lighter.
/// Shared by every frontend so job progress bars behave identically.
pub fn job_fraction(phase: ChdPhase, fraction: f64) -> f64 {
    let fraction = fraction.clamp(0.0, 1.0);
    match phase {
        ChdPhase::Compressing => 0.60 * fraction,
        ChdPhase::Extracting => 0.60 + 0.20 * fraction,
        ChdPhase::Verifying => 0.80 + 0.20 * fraction,
    }
}

/// Result of round-trip verification.
#[derive(Debug, Clone)]
pub enum VerificationOutcome {
    /// Every track's bytes match the original source exactly.
    Verified { tracks: usize },
    /// The re-extracted data does not match the source. The .chd has been
    /// removed; the source files are untouched.
    Mismatch { detail: String },
}

/// A completed (created + verified) compression.
#[derive(Debug, Clone)]
pub struct CompressionOutcome {
    pub output: PathBuf,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub verification: VerificationOutcome,
}

impl CompressionOutcome {
    /// True when the CHD exists and round-trip verification passed.
    pub fn is_verified(&self) -> bool {
        matches!(self.verification, VerificationOutcome::Verified { .. })
    }
}

/// Compress one disc to CHD and round-trip verify the result.
///
/// Output ownership: chdman writes to a hidden temp file
/// (`.{stem}.chd.tmp`) in the same directory as `job.output`, never to
/// `job.output` itself. Every failure or cancellation path removes only that
/// temp file — this job can never delete a `.chd` it did not create, even if
/// another tool or a previous run left one at `job.output`. Only after
/// verification succeeds is the temp file published by renaming it onto
/// `job.output`, and only if nothing has appeared there since planning. On
/// verification mismatch the temp file is removed and the mismatch is
/// reported in the outcome rather than as an error.
pub fn compress_to_chd(
    chdman: &Chdman,
    job: &CompressionJob,
    on_progress: ChdProgressFn<'_>,
    cancel: &AtomicBool,
) -> Result<CompressionOutcome, ChdConvertError> {
    // CD media → createcd/extractcd, DVD media → createdvd/extractdvd.
    let create_cmd = match job.media {
        ChdMedia::Cd => "createcd",
        ChdMedia::Dvd => "createdvd",
    };

    let stem = job
        .output
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("chd");
    let temp_output = job.output.with_file_name(format!(".{stem}.chd.tmp"));
    if temp_output.exists() {
        // Stale leftover from a crashed run; this job is about to recreate it.
        fs::remove_file(&temp_output)?;
    }

    let result = run_chdman(
        &chdman.path,
        &[
            OsStr::new(create_cmd),
            OsStr::new("-i"),
            job.input.as_os_str(),
            OsStr::new("-o"),
            temp_output.as_os_str(),
        ],
        &|pct| on_progress(ChdPhase::Compressing, pct),
        cancel,
    );
    if let Err(e) = result {
        remove_file_best_effort(&temp_output);
        return Err(e);
    }

    let verification = match verify_round_trip(chdman, job, &temp_output, on_progress, cancel) {
        Ok(v) => v,
        Err(e) => {
            remove_file_best_effort(&temp_output);
            return Err(e);
        }
    };

    let output_bytes = match &verification {
        VerificationOutcome::Verified { .. } => {
            // Publish. This check-then-rename has a narrow TOCTOU window in
            // isolation, but it is closed in practice by plan_batch's
            // duplicate-output rejection (no two jobs in one batch plan the
            // same output) and the GUI's overlapping-operation guard (no two
            // batches run concurrently against the same library) — nothing
            // else should be racing to create this exact path.
            if job.output.exists() {
                // Appeared since planning (another tool, another run). Do
                // not clobber it; only this job's own temp file is ours.
                fs::remove_file(&temp_output)?;
                return Err(ChdConvertError::OutputExists(job.output.clone()));
            }
            fs::rename(&temp_output, &job.output)?;
            fs::metadata(&job.output)?.len()
        }
        VerificationOutcome::Mismatch { .. } => {
            remove_file_best_effort(&temp_output);
            0
        }
    };

    Ok(CompressionOutcome {
        output: job.output.clone(),
        input_bytes: job.input_bytes,
        output_bytes,
        verification,
    })
}

/// Delete a verified job's source files. Call only after
/// [`CompressionOutcome::is_verified`] returns true and the user opted in.
///
/// Returns the files that could not be deleted, with reasons.
pub fn delete_job_sources(job: &CompressionJob) -> Vec<(PathBuf, String)> {
    let mut failures = Vec::new();
    for f in &job.source_files {
        if let Err(e) = fs::remove_file(f) {
            failures.push((f.clone(), e.to_string()));
        }
    }
    failures
}

/// Rewrite sibling `.m3u` playlists that referenced the job's input file to
/// point at the new `.chd`. Call after [`delete_job_sources`] so multi-disc
/// playlists don't dangle.
///
/// Delegates to the same playlist-rewriting machinery `rename.rs` uses
/// (rename-map lookup, then same-stem-with-original-extension-first
/// fallback over the entry-point extension list), so `./`-prefixed and
/// stem-only entries are handled identically to a manual rename.
///
/// Returns `(lines updated, per-file error messages)`.
///
/// Known residual gap (tracked in TODO.md, not fixed here): on
/// case-**sensitive** filesystems, a playlist entry whose case differs from
/// the actual file still misses, because the fallback lookup does not probe
/// the directory case-insensitively.
pub fn update_m3u_references(job: &CompressionJob) -> (usize, Vec<String>) {
    let dir = job.input.parent().unwrap_or(Path::new("."));
    let (Some(old_name), Some(new_name)) = (
        job.input.file_name().and_then(|n| n.to_str()),
        job.output.file_name().and_then(|n| n.to_str()),
    ) else {
        return (0, Vec::new());
    };

    let rename_map =
        std::collections::HashMap::from([(old_name.to_string(), new_name.to_string())]);
    let mut errors = Vec::new();
    let updated = crate::rename::fix_m3u_references_in_dir(dir, &rename_map, &mut errors);
    (updated, errors)
}

/// Outcome of the post-verify finalize step ([`finalize_verified`]).
pub struct FinalizeReport {
    pub sources_deleted: bool,
    pub delete_failures: Vec<(PathBuf, String)>,
    pub m3u_lines_updated: usize,
    pub m3u_errors: Vec<String>,
}

/// After a verified compression: optionally delete sources, then repoint
/// sibling .m3u playlists at the new .chd. The single implementation shared
/// by every frontend so "verified success" always means the same sequence
/// of side effects.
pub fn finalize_verified(job: &CompressionJob, delete_sources: bool) -> FinalizeReport {
    if !delete_sources {
        return FinalizeReport {
            sources_deleted: false,
            delete_failures: Vec::new(),
            m3u_lines_updated: 0,
            m3u_errors: Vec::new(),
        };
    }

    let delete_failures = delete_job_sources(job);
    let (m3u_lines_updated, m3u_errors) = update_m3u_references(job);
    FinalizeReport {
        sources_deleted: true,
        delete_failures,
        m3u_lines_updated,
        m3u_errors,
    }
}

// -- round-trip verification --

/// Temp directory removed on drop (best effort).
struct TempDirGuard(PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Extract the freshly-created CHD (at `chd_path`, the not-yet-published temp
/// file) next to it and compare every track's content against the original
/// source files.
fn verify_round_trip(
    chdman: &Chdman,
    job: &CompressionJob,
    chd_path: &Path,
    on_progress: ChdProgressFn<'_>,
    cancel: &AtomicBool,
) -> Result<VerificationOutcome, ChdConvertError> {
    let source_ext = job
        .input
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    // Extract into a hidden temp dir next to the output: same filesystem
    // (no cross-device surprises) and sized for disc images, unlike /tmp
    // which is commonly a RAM-backed tmpfs.
    let stem = job
        .output
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("chd");
    let temp_dir = job
        .output
        .parent()
        .unwrap_or(Path::new("."))
        .join(format!(".{stem}.chd-verify"));
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir)?;
    }
    fs::create_dir(&temp_dir)?;
    let _guard = TempDirGuard(temp_dir.clone());

    let (extract_cmd, extract_name) = match job.media {
        ChdMedia::Cd if source_ext == "gdi" => ("extractcd", "verify.gdi"),
        ChdMedia::Cd => ("extractcd", "verify.cue"),
        ChdMedia::Dvd => ("extractdvd", "verify.iso"),
    };
    let extract_target = temp_dir.join(extract_name);

    run_chdman(
        &chdman.path,
        &[
            OsStr::new(extract_cmd),
            OsStr::new("-i"),
            chd_path.as_os_str(),
            OsStr::new("-o"),
            extract_target.as_os_str(),
        ],
        &|pct| on_progress(ChdPhase::Extracting, pct),
        cancel,
    )?;

    let mut source_spans = layout_spans(&job.input, &source_ext)?;
    let extracted_ext = if job.media == ChdMedia::Dvd {
        "iso"
    } else {
        &source_ext
    };
    let mut extracted_spans = layout_spans(&extract_target, extracted_ext)?;

    if source_spans.len() != extracted_spans.len() {
        return Ok(VerificationOutcome::Mismatch {
            detail: format!(
                "track count differs: source has {}, extracted CHD has {}",
                source_spans.len(),
                extracted_spans.len()
            ),
        });
    }

    // Pair by track number, not listing position: a cue that lists tracks
    // out of ascending order must still compare track N against track N.
    if !sort_and_check_track_numbers(&mut source_spans, &mut extracted_spans) {
        return Ok(VerificationOutcome::Mismatch {
            detail: "track numbering differs between source and extracted CHD".to_string(),
        });
    }

    // Fast reject on lengths before comparing any bytes.
    for (s, e) in source_spans.iter().zip(&extracted_spans) {
        if s.byte_len != e.byte_len {
            return Ok(VerificationOutcome::Mismatch {
                detail: format!(
                    "track {} size differs: source {} bytes, extracted {} bytes",
                    s.track_number, s.byte_len, e.byte_len
                ),
            });
        }
    }

    let total_bytes: u64 = source_spans.iter().map(|s| s.byte_len).sum();
    let mut compared_bytes: u64 = 0;

    for (s, e) in source_spans.iter().zip(&extracted_spans) {
        if !spans_equal(s, e, &mut compared_bytes, total_bytes, on_progress, cancel)? {
            return Ok(VerificationOutcome::Mismatch {
                detail: format!("track {} content differs after round-trip", s.track_number),
            });
        }
    }

    Ok(VerificationOutcome::Verified {
        tracks: source_spans.len(),
    })
}

/// Sort both span lists by track number in place, then report whether the
/// resulting track-number sequences match. `cue_track_spans` emits spans in
/// listing order, so a cue that lists tracks out of ascending order must
/// still be paired track-N-against-track-N rather than position-against-
/// position.
fn sort_and_check_track_numbers(source: &mut [TrackSpan], extracted: &mut [TrackSpan]) -> bool {
    source.sort_by_key(|s| s.track_number);
    extracted.sort_by_key(|e| e.track_number);
    source
        .iter()
        .map(|s| s.track_number)
        .eq(extracted.iter().map(|e| e.track_number))
}

/// Per-track byte spans for any supported source layout.
fn layout_spans(container: &Path, extension: &str) -> Result<Vec<TrackSpan>, ChdConvertError> {
    let dir = container.parent().unwrap_or(Path::new(".")).to_path_buf();
    match extension {
        "cue" => {
            let sheet = retro_junk_disc::cue::parse_cue(&fs::read_to_string(container)?)?;
            Ok(cue_track_spans(&sheet, &dir)?)
        }
        "gdi" => {
            let tracks = parse_gdi(&fs::read_to_string(container)?)?;
            Ok(gdi_track_spans(&tracks, &dir)?)
        }
        // ISO (and any other Source-declared extension): the whole file is
        // the single "track". `plan_compression`'s exhaustive-by-table
        // dispatch guarantees only "cue" | "gdi" | "iso" ever reach a
        // CompressionJob, and thus this function — any other
        // Source(Cd)/Source(Dvd) extension is rejected at plan time with
        // UnsupportedLayout before a job can be created.
        _ => Ok(vec![TrackSpan {
            track_number: 1,
            file: container.to_path_buf(),
            byte_offset: 0,
            byte_len: fs::metadata(container)?.len(),
        }]),
    }
}

/// Read exactly `buf.len()` bytes, looping on short reads. Errors with
/// `UnexpectedEof` naming `path` if the file runs out before `buf` fills.
fn read_exact_looped(
    file: &mut fs::File,
    buf: &mut [u8],
    path: &Path,
) -> Result<(), ChdConvertError> {
    let mut filled = 0;
    while filled < buf.len() {
        let n = file.read(&mut buf[filled..])?;
        if n == 0 {
            return Err(ChdConvertError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                format!("unexpected EOF reading {}", path.display()),
            )));
        }
        filled += n;
    }
    Ok(())
}

/// Stream-compare two equal-length spans. Returns `Ok(true)` when every byte
/// matches; early-exits on the first differing chunk.
fn spans_equal(
    a: &TrackSpan,
    b: &TrackSpan,
    compared_bytes: &mut u64,
    total_bytes: u64,
    on_progress: ChdProgressFn<'_>,
    cancel: &AtomicBool,
) -> Result<bool, ChdConvertError> {
    let mut file_a = fs::File::open(&a.file)?;
    file_a.seek(SeekFrom::Start(a.byte_offset))?;
    let mut file_b = fs::File::open(&b.file)?;
    file_b.seek(SeekFrom::Start(b.byte_offset))?;

    const CHUNK: usize = 1 << 20;
    let mut buf_a = vec![0u8; CHUNK];
    let mut buf_b = vec![0u8; CHUNK];
    let mut remaining = a.byte_len;

    while remaining > 0 {
        if cancel.load(Ordering::Relaxed) {
            return Err(ChdConvertError::Cancelled);
        }
        let want = remaining.min(CHUNK as u64) as usize;
        read_exact_looped(&mut file_a, &mut buf_a[..want], &a.file)?;
        read_exact_looped(&mut file_b, &mut buf_b[..want], &b.file)?;
        if buf_a[..want] != buf_b[..want] {
            return Ok(false);
        }
        remaining -= want as u64;
        *compared_bytes += want as u64;
        if total_bytes > 0 {
            on_progress(
                ChdPhase::Verifying,
                *compared_bytes as f64 / total_bytes as f64,
            );
        }
    }

    Ok(true)
}

// -- chdman process handling --

fn remove_file_best_effort(path: &Path) {
    let _ = fs::remove_file(path);
}

/// Parse a chdman progress line, e.g. `Compressing, 45.6% complete... (ratio=41.2%)`
/// or `Extracting, 12.0% complete...`. Returns the fraction 0.0..=1.0.
fn parse_chdman_percent(line: &str) -> Option<f64> {
    let rest = line.split_once(", ")?.1;
    let percent = rest.split_once("% complete")?.0;
    percent.trim().parse::<f64>().ok().map(|p| p / 100.0)
}

/// Run chdman with the given arguments, streaming `\r`/`\n`-delimited
/// progress from stderr. Kills the process on cancellation.
///
/// The blocking stderr read runs on a dedicated thread; the main loop polls
/// it with a short timeout so a cancel request reaches `child.kill()`
/// promptly even if chdman stalls without emitting any output (hung network
/// mount, spun-down disk) instead of blocking forever in `read()`.
fn run_chdman(
    chdman_path: &Path,
    args: &[&OsStr],
    on_percent: &dyn Fn(f64),
    cancel: &AtomicBool,
) -> Result<(), ChdConvertError> {
    let mut child = Command::new(chdman_path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| ChdmanUnavailable {
            reason: format!("Cannot run chdman at {}: {e}", chdman_path.display()),
        })?;

    let mut pending = Vec::new();
    let mut info_lines: Vec<String> = Vec::new();

    let (chunk_tx, chunk_rx) = mpsc::channel::<Vec<u8>>();
    let mut stderr = child.stderr.take().expect("stderr piped");
    let reader = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = stderr.read(&mut buf) {
            if n == 0 {
                break;
            }
            if chunk_tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    let mut cancelled = false;
    loop {
        if cancel.load(Ordering::Relaxed) && !cancelled {
            cancelled = true;
            let _ = child.kill(); // reader unblocks via EOF after kill
        }
        match chunk_rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(chunk) => {
                for &b in &chunk {
                    if b == b'\r' || b == b'\n' {
                        handle_chdman_line(&pending, on_percent, &mut info_lines);
                        pending.clear();
                    } else {
                        pending.push(b);
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    handle_chdman_line(&pending, on_percent, &mut info_lines);
    let _ = reader.join();

    let status = child.wait()?;
    if cancelled {
        return Err(ChdConvertError::Cancelled);
    }
    if !status.success() {
        // The tail of the non-progress output carries the actual error
        // (e.g. "ERROR: couldn't find bin file [...]").
        let start = info_lines.len().saturating_sub(4);
        let detail = info_lines[start..].join(" | ");
        return Err(ChdConvertError::ChdmanFailed { detail });
    }
    Ok(())
}

fn handle_chdman_line(raw: &[u8], on_percent: &dyn Fn(f64), info_lines: &mut Vec<String>) {
    if raw.is_empty() {
        return;
    }
    let line = String::from_utf8_lossy(raw);
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    if let Some(fraction) = parse_chdman_percent(line) {
        on_percent(fraction);
    } else {
        info_lines.push(line.to_string());
    }
}

#[cfg(test)]
#[path = "tests/chd_convert_tests.rs"]
mod tests;
