use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use retro_junk_core::{
    AnalysisError, AnalysisOptions, ChdExtensionRole, ChdMedia, Platform, ReadSeek, RomAnalyzer,
    RomIdentification,
};
use retro_junk_sony::{Ps1Analyzer, Ps2Analyzer};

use super::*;

// -- parsing --

#[test]
fn parses_compress_and_extract_progress_lines() {
    assert_eq!(
        parse_chdman_percent("Compressing, 45.6% complete... (ratio=41.2%)"),
        Some(0.456)
    );
    assert_eq!(
        parse_chdman_percent("Extracting, 0.0% complete..."),
        Some(0.0)
    );
    assert_eq!(
        parse_chdman_percent("Compression complete ... final ratio = 1.1%"),
        None
    );
    assert_eq!(parse_chdman_percent("Output CHD:   game.chd"), None);
}

#[test]
fn parses_version_from_banner() {
    let banner =
        "chdman - MAME Compressed Hunks of Data (CHD) manager 0.288 (mame0288-dirty)\nUsage:";
    assert_eq!(parse_chdman_version(banner), "0.288");
    assert_eq!(parse_chdman_version("gibberish"), "");
}

// -- planning --

fn write_track(dir: &Path, name: &str, frames: usize, seed: usize) {
    let mut data = Vec::with_capacity(frames * 2352);
    for s in 0..frames {
        for i in 0..2352 {
            data.push(((s * seed + i) % 256) as u8);
        }
    }
    fs::write(dir.join(name), data).unwrap();
}

fn write_redump_style_disc(dir: &Path) -> std::path::PathBuf {
    write_track(dir, "game (Track 1).bin", 100, 7);
    // Track 2 carries a 150-frame pregap (INDEX 00) inside the file.
    write_track(dir, "game (Track 2).bin", 200, 13);
    write_track(dir, "game (Track 3).bin", 30, 17);
    let cue = r#"FILE "game (Track 1).bin" BINARY
  TRACK 01 MODE2/2352
    INDEX 01 00:00:00
FILE "game (Track 2).bin" BINARY
  TRACK 02 AUDIO
    INDEX 00 00:00:00
    INDEX 01 00:02:00
FILE "game (Track 3).bin" BINARY
  TRACK 03 AUDIO
    INDEX 01 00:00:00
"#;
    let cue_path = dir.join("game.cue");
    fs::write(&cue_path, cue).unwrap();
    cue_path
}

#[test]
fn plan_rejects_unsupported_extension_with_hint() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("game.bin");
    fs::write(&bin, [0u8; 16]).unwrap();
    let err = plan_compression(&bin, &Ps1Analyzer).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("PlayStation"), "got: {msg}");
    assert!(msg.contains(".cue"), "hint should point at the cue: {msg}");
}

#[test]
fn plan_rejects_already_chd() {
    let dir = tempfile::tempdir().unwrap();
    let chd = dir.path().join("game.chd");
    fs::write(&chd, [0u8; 16]).unwrap();
    let msg = plan_compression(&chd, &Ps1Analyzer)
        .unwrap_err()
        .to_string();
    assert!(msg.contains("already a CHD"), "got: {msg}");
}

#[test]
fn plan_rejects_cue_with_missing_tracks() {
    let dir = tempfile::tempdir().unwrap();
    let cue = dir.path().join("game.cue");
    fs::write(
        &cue,
        "FILE \"missing.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n",
    )
    .unwrap();
    match plan_compression(&cue, &Ps1Analyzer) {
        Err(ChdConvertError::BrokenSource(missing)) => {
            assert_eq!(missing, vec!["missing.bin".to_string()])
        }
        other => panic!("expected BrokenSource, got {other:?}"),
    }
}

#[test]
fn plan_rejects_existing_output() {
    let dir = tempfile::tempdir().unwrap();
    let cue_path = write_redump_style_disc(dir.path());
    fs::write(dir.path().join("game.chd"), [0u8; 4]).unwrap();
    assert!(matches!(
        plan_compression(&cue_path, &Ps1Analyzer),
        Err(ChdConvertError::OutputExists(_))
    ));
}

#[test]
fn plan_rejects_cue_declaring_pregap() {
    let dir = tempfile::tempdir().unwrap();
    write_track(dir.path(), "game (Track 1).bin", 100, 7);
    write_track(dir.path(), "game (Track 2).bin", 200, 13);
    let cue = r#"FILE "game (Track 1).bin" BINARY
  TRACK 01 MODE2/2352
    INDEX 01 00:00:00
FILE "game (Track 2).bin" BINARY
  TRACK 02 AUDIO
    PREGAP 00:02:00
    INDEX 01 00:00:00
"#;
    let cue_path = dir.path().join("game.cue");
    fs::write(&cue_path, cue).unwrap();

    match plan_compression(&cue_path, &Ps1Analyzer) {
        Err(ChdConvertError::UnsupportedLayout { detail }) => {
            assert!(detail.contains("PREGAP"), "got: {detail}");
        }
        other => panic!("expected UnsupportedLayout, got {other:?}"),
    }
}

#[test]
fn plan_collects_all_source_files_and_sizes() {
    let dir = tempfile::tempdir().unwrap();
    let cue_path = write_redump_style_disc(dir.path());
    let job = plan_compression(&cue_path, &Ps1Analyzer).unwrap();
    assert_eq!(job.media, retro_junk_core::ChdMedia::Cd);
    assert_eq!(job.output, dir.path().join("game.chd"));
    // cue + 3 tracks
    assert_eq!(job.source_files.len(), 4);
    let track_bytes = (100 + 200 + 30) * 2352;
    let cue_bytes = fs::metadata(&cue_path).unwrap().len();
    assert_eq!(job.input_bytes, track_bytes as u64 + cue_bytes);
}

/// Minimal stand-in analyzer for tests that need a specific
/// `chd_extensions()` table without pulling in a real console's other
/// behavior.
struct StubAnalyzer {
    extensions: &'static [(&'static str, ChdExtensionRole)],
}

impl RomAnalyzer for StubAnalyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Ok(RomIdentification::new())
    }

    fn platform(&self) -> Platform {
        Platform::Ps1
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["cue", "iso", "cso", "bin", "toc", "chd"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        true
    }

    fn chd_extensions(&self) -> &'static [(&'static str, ChdExtensionRole)] {
        self.extensions
    }
}

const PS2_STYLE_EXTENSIONS: &[(&str, ChdExtensionRole)] = &[
    ("cue", ChdExtensionRole::Source(ChdMedia::Cd)),
    ("iso", ChdExtensionRole::Source(ChdMedia::Dvd)),
];
const CSO_UNCONVERTIBLE_EXTENSIONS: &[(&str, ChdExtensionRole)] =
    &[("cso", ChdExtensionRole::Unconvertible)];
const UNKNOWN_LAYOUT_EXTENSIONS: &[(&str, ChdExtensionRole)] =
    &[("toc", ChdExtensionRole::Source(ChdMedia::Cd))];

// -- C1 step 5: exhaustive-by-table extension dispatch --

#[test]
fn plan_rejects_source_declared_extension_without_layout_handling() {
    let dir = tempfile::tempdir().unwrap();
    let toc_path = dir.path().join("game.toc");
    fs::write(&toc_path, [0u8; 16]).unwrap();

    let analyzer = StubAnalyzer {
        extensions: UNKNOWN_LAYOUT_EXTENSIONS,
    };
    match plan_compression(&toc_path, &analyzer) {
        Err(ChdConvertError::UnsupportedLayout { detail }) => {
            assert!(detail.contains("toc"), "got: {detail}");
        }
        other => panic!("expected UnsupportedLayout, got {other:?}"),
    }
}

// -- B6: typed skip classes + plan_batch --

#[test]
fn plan_batch_rejects_duplicate_output_first_wins() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("Game.bin"), [0u8; 2352 * 4]).unwrap();
    let cue = "FILE \"Game.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n";
    let cue_path = dir.path().join("Game.cue");
    fs::write(&cue_path, cue).unwrap();
    let iso_path = dir.path().join("Game.iso");
    fs::write(&iso_path, [0u8; 2048 * 4]).unwrap();

    let analyzer = StubAnalyzer {
        extensions: PS2_STYLE_EXTENSIONS,
    };
    let inputs = vec![cue_path.clone(), iso_path.clone()];
    let batch = plan_batch(&inputs, &analyzer);

    assert_eq!(batch.jobs.len(), 1);
    assert_eq!(batch.jobs[0].input, cue_path);
    assert_eq!(batch.skips.len(), 1);
    assert_eq!(batch.skips[0].input, iso_path);
    assert!(
        matches!(batch.skips[0].error, ChdConvertError::DuplicateOutput(_)),
        "{}",
        batch.skips[0].error
    );
}

#[test]
fn plan_batch_classifies_unreadable_container() {
    let dir = tempfile::tempdir().unwrap();
    let cso_path = dir.path().join("game.cso");
    fs::write(&cso_path, [0u8; 16]).unwrap();

    let analyzer = StubAnalyzer {
        extensions: CSO_UNCONVERTIBLE_EXTENSIONS,
    };
    let batch = plan_batch(&[cso_path], &analyzer);
    assert!(batch.jobs.is_empty());
    assert_eq!(batch.skips.len(), 1);
    match &batch.skips[0].error {
        ChdConvertError::UnsupportedSource { class, .. } => {
            assert_eq!(*class, SourceSkipClass::UnreadableContainer);
        }
        other => panic!("expected UnsupportedSource, got {other:?}"),
    }
}

#[test]
fn plan_batch_drops_companion_track_files_silently() {
    let dir = tempfile::tempdir().unwrap();
    let bin_path = dir.path().join("game (Track 2).bin");
    fs::write(&bin_path, [0u8; 16]).unwrap();

    let analyzer = StubAnalyzer {
        extensions: PS2_STYLE_EXTENSIONS,
    };
    let batch = plan_batch(&[bin_path], &analyzer);
    assert!(batch.jobs.is_empty());
    assert!(
        batch.skips.is_empty(),
        "companion data should be silent: {:?}",
        batch
            .skips
            .iter()
            .map(|s| s.error.to_string())
            .collect::<Vec<_>>()
    );
    assert_eq!(batch.already_chd, 0);
}

#[test]
fn plan_batch_counts_already_chd() {
    let dir = tempfile::tempdir().unwrap();
    let chd_path = dir.path().join("game.chd");
    fs::write(&chd_path, [0u8; 16]).unwrap();

    let analyzer = StubAnalyzer {
        extensions: PS2_STYLE_EXTENSIONS,
    };
    let batch = plan_batch(&[chd_path], &analyzer);
    assert!(batch.jobs.is_empty());
    assert!(batch.skips.is_empty());
    assert_eq!(batch.already_chd, 1);
}

// -- B6: finalize_verified --

#[test]
fn finalize_verified_deletes_sources_and_updates_m3u_when_requested() {
    let dir = tempfile::tempdir().unwrap();
    let cue_path = write_redump_style_disc(dir.path());
    fs::write(dir.path().join("game.m3u"), "game.cue\n").unwrap();
    let job = plan_compression(&cue_path, &Ps1Analyzer).unwrap();
    // finalize_verified is only ever called after compress_to_chd has
    // published the .chd; the m3u fix-up (rename.rs machinery) matches
    // playlist entries against files that actually exist on disk.
    fs::write(&job.output, [0u8; 4]).unwrap();

    let report = finalize_verified(&job, true);
    assert!(report.sources_deleted);
    assert!(report.delete_failures.is_empty());
    assert_eq!(report.m3u_lines_updated, 1);
    assert!(report.m3u_errors.is_empty());
    assert!(!cue_path.exists());
    assert_eq!(
        fs::read_to_string(dir.path().join("game.m3u")).unwrap(),
        "game.chd\n"
    );
}

#[test]
fn finalize_verified_no_op_when_delete_sources_false() {
    let dir = tempfile::tempdir().unwrap();
    let cue_path = write_redump_style_disc(dir.path());
    let job = plan_compression(&cue_path, &Ps1Analyzer).unwrap();

    let report = finalize_verified(&job, false);
    assert!(!report.sources_deleted);
    assert!(report.delete_failures.is_empty());
    assert_eq!(report.m3u_lines_updated, 0);
    assert!(cue_path.exists());
}

// -- B4: pairing verification spans by track number --

#[test]
fn sort_and_check_track_numbers_pairs_out_of_order_tracks() {
    let mut source = vec![
        TrackSpan {
            track_number: 2,
            file: PathBuf::from("s2"),
            byte_offset: 0,
            byte_len: 10,
        },
        TrackSpan {
            track_number: 1,
            file: PathBuf::from("s1"),
            byte_offset: 0,
            byte_len: 5,
        },
    ];
    let mut extracted = vec![
        TrackSpan {
            track_number: 1,
            file: PathBuf::from("e1"),
            byte_offset: 0,
            byte_len: 5,
        },
        TrackSpan {
            track_number: 2,
            file: PathBuf::from("e2"),
            byte_offset: 0,
            byte_len: 10,
        },
    ];
    assert!(sort_and_check_track_numbers(&mut source, &mut extracted));
    assert_eq!(source[0].track_number, 1);
    assert_eq!(extracted[0].track_number, 1);
    assert_eq!(source[1].track_number, 2);
    assert_eq!(extracted[1].track_number, 2);
}

#[test]
fn sort_and_check_track_numbers_detects_mismatched_sets() {
    let mut source = vec![
        TrackSpan {
            track_number: 1,
            file: PathBuf::from("s1"),
            byte_offset: 0,
            byte_len: 5,
        },
        TrackSpan {
            track_number: 3,
            file: PathBuf::from("s3"),
            byte_offset: 0,
            byte_len: 5,
        },
    ];
    let mut extracted = vec![
        TrackSpan {
            track_number: 1,
            file: PathBuf::from("e1"),
            byte_offset: 0,
            byte_len: 5,
        },
        TrackSpan {
            track_number: 2,
            file: PathBuf::from("e2"),
            byte_offset: 0,
            byte_len: 5,
        },
    ];
    assert!(!sort_and_check_track_numbers(&mut source, &mut extracted));
}

// -- B3: spans_equal streaming byte comparison --

#[test]
fn spans_equal_true_for_identical_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let len = 2 * (1 << 20); // > 1 MiB, exercises the chunk loop
    let data = vec![7u8; len];
    let path_a = dir.path().join("a.bin");
    let path_b = dir.path().join("b.bin");
    fs::write(&path_a, &data).unwrap();
    fs::write(&path_b, &data).unwrap();
    let span_a = TrackSpan {
        track_number: 1,
        file: path_a,
        byte_offset: 0,
        byte_len: len as u64,
    };
    let span_b = TrackSpan {
        track_number: 1,
        file: path_b,
        byte_offset: 0,
        byte_len: len as u64,
    };

    let mut compared = 0u64;
    let cancel = AtomicBool::new(false);
    let equal = spans_equal(
        &span_a,
        &span_b,
        &mut compared,
        len as u64,
        &|_, _| {},
        &cancel,
    )
    .unwrap();
    assert!(equal);
    assert_eq!(compared, len as u64);
}

#[test]
fn spans_equal_false_and_early_exits_on_mid_span_difference() {
    let dir = tempfile::tempdir().unwrap();
    let len = 2 * (1 << 20);
    let data_a = vec![7u8; len];
    let mut data_b = data_a.clone();
    data_b[len / 2] = 8; // differs partway through the second 1 MiB chunk
    let path_a = dir.path().join("a.bin");
    let path_b = dir.path().join("b.bin");
    fs::write(&path_a, &data_a).unwrap();
    fs::write(&path_b, &data_b).unwrap();
    let span_a = TrackSpan {
        track_number: 1,
        file: path_a,
        byte_offset: 0,
        byte_len: len as u64,
    };
    let span_b = TrackSpan {
        track_number: 1,
        file: path_b,
        byte_offset: 0,
        byte_len: len as u64,
    };

    let mut compared = 0u64;
    let cancel = AtomicBool::new(false);
    let equal = spans_equal(
        &span_a,
        &span_b,
        &mut compared,
        len as u64,
        &|_, _| {},
        &cancel,
    )
    .unwrap();
    assert!(!equal);
    assert!(
        compared < len as u64,
        "should early-exit before comparing every byte: compared {compared}"
    );
}

// -- B7: Chdman::detect_from_setting --

#[test]
fn detect_from_setting_uses_explicit_override_path() {
    let bogus = "/nonexistent/path/to/chdman-xyz";
    let err = Chdman::detect_from_setting(bogus).unwrap_err();
    assert!(err.reason.contains(bogus), "got: {}", err.reason);
}

#[test]
fn detect_from_setting_blank_falls_back_to_path_lookup() {
    // Blank/whitespace must not be treated as an explicit override path
    // (which would try to spawn an empty-string binary and fail with a
    // different, misleading message).
    if let Err(e) = Chdman::detect_from_setting("   ") {
        assert!(e.reason.contains("PATH"), "got: {}", e.reason);
    }
}

// -- B1/B2: fake-chdman-script based tests (no real chdman required) --

#[cfg(unix)]
fn write_fake_chdman(dir: &Path, body: &str) -> Chdman {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("fake-chdman.sh");
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
    Chdman {
        path,
        version: String::new(),
    }
}

/// A fake chdman that just copies its `-i` input to its `-o` output for any
/// subcommand — good enough to exercise the create→extract→verify pipeline
/// with byte-identical "round trip" data without a real chdman.
#[cfg(unix)]
fn fake_chdman_copy(dir: &Path) -> Chdman {
    write_fake_chdman(dir, "cp \"$3\" \"$5\"")
}

#[cfg(unix)]
fn fake_chdman_failing(dir: &Path) -> Chdman {
    write_fake_chdman(dir, "echo 'ERROR: boom' >&2\nexit 1")
}

#[cfg(unix)]
fn fake_chdman_hanging(dir: &Path) -> Chdman {
    // `exec` replaces the shell process with `sleep` instead of forking a
    // child that would keep its own copy of the stderr pipe's write end
    // open — without it, killing the shell wouldn't close the pipe (the
    // grandchild still holds it open) and the reader thread would block
    // for the full sleep duration regardless of cancellation.
    write_fake_chdman(dir, "exec sleep 30")
}

#[test]
#[cfg(unix)]
fn compress_failure_never_removes_a_preexisting_chd() {
    let dir = tempfile::tempdir().unwrap();
    let cue_path = write_redump_style_disc(dir.path());
    let job = plan_compression(&cue_path, &Ps1Analyzer).unwrap();
    fs::write(&job.output, b"do-not-touch").unwrap();

    let chdman = fake_chdman_failing(dir.path());
    let cancel = AtomicBool::new(false);
    let err = compress_to_chd(&chdman, &job, &|_, _| {}, &cancel).unwrap_err();
    assert!(matches!(err, ChdConvertError::ChdmanFailed { .. }), "{err}");
    assert_eq!(fs::read(&job.output).unwrap(), b"do-not-touch");
    assert!(!dir.path().join(".game.chd.tmp").exists());
}

#[test]
#[cfg(unix)]
fn successful_compression_leaves_no_temp_file() {
    let dir = tempfile::tempdir().unwrap();
    let iso_path = dir.path().join("game.iso");
    fs::write(&iso_path, vec![42u8; 4096]).unwrap();
    let job = plan_compression(&iso_path, &Ps2Analyzer).unwrap();

    let chdman = fake_chdman_copy(dir.path());
    let cancel = AtomicBool::new(false);
    let outcome = compress_to_chd(&chdman, &job, &|_, _| {}, &cancel).unwrap();
    assert!(outcome.is_verified(), "{:?}", outcome.verification);
    assert!(job.output.is_file());
    assert!(!dir.path().join(".game.chd.tmp").exists());
}

#[test]
#[cfg(unix)]
fn publish_never_clobbers_output_that_appeared_since_planning() {
    let dir = tempfile::tempdir().unwrap();
    let iso_path = dir.path().join("game.iso");
    fs::write(&iso_path, vec![42u8; 4096]).unwrap();
    let job = plan_compression(&iso_path, &Ps2Analyzer).unwrap();
    // Simulate a .chd that appeared at the output path after planning but
    // before this job could publish (another tool, another run).
    fs::write(&job.output, b"someone-elses-chd").unwrap();

    let chdman = fake_chdman_copy(dir.path());
    let cancel = AtomicBool::new(false);
    let err = compress_to_chd(&chdman, &job, &|_, _| {}, &cancel).unwrap_err();
    assert!(matches!(err, ChdConvertError::OutputExists(_)), "{err}");
    assert_eq!(fs::read(&job.output).unwrap(), b"someone-elses-chd");
    assert!(!dir.path().join(".game.chd.tmp").exists());
}

#[test]
#[cfg(unix)]
fn cancel_returns_quickly_when_chdman_hangs() {
    let dir = tempfile::tempdir().unwrap();
    let cue_path = write_redump_style_disc(dir.path());
    let job = plan_compression(&cue_path, &Ps1Analyzer).unwrap();
    let chdman = fake_chdman_hanging(dir.path());

    let cancel = AtomicBool::new(false);
    let start = std::time::Instant::now();
    let result = std::thread::scope(|scope| {
        scope.spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(200));
            cancel.store(true, Ordering::Relaxed);
        });
        compress_to_chd(&chdman, &job, &|_, _| {}, &cancel)
    });
    let elapsed = start.elapsed();
    assert!(
        matches!(result, Err(ChdConvertError::Cancelled)),
        "{result:?}"
    );
    assert!(elapsed < std::time::Duration::from_secs(2), "{elapsed:?}");
}

// -- end-to-end with a real chdman (skipped when not installed) --

#[test]
fn round_trip_compress_verify_delete_with_real_chdman() {
    let Ok(chdman) = Chdman::detect(Path::new("")) else {
        eprintln!("chdman not installed; skipping integration test");
        return;
    };

    let dir = tempfile::tempdir().unwrap();
    let cue_path = write_redump_style_disc(dir.path());
    let job = plan_compression(&cue_path, &Ps1Analyzer).unwrap();

    let cancel = AtomicBool::new(false);
    let outcome = compress_to_chd(&chdman, &job, &|_, _| {}, &cancel).unwrap();

    assert!(
        outcome.is_verified(),
        "verification: {:?}",
        outcome.verification
    );
    match outcome.verification {
        VerificationOutcome::Verified { tracks } => assert_eq!(tracks, 3),
        VerificationOutcome::Mismatch { detail } => panic!("mismatch: {detail}"),
    }
    assert!(job.output.is_file());
    assert!(outcome.output_bytes > 0);

    // Temp verification dir and temp output file must both be gone.
    assert!(
        !dir.path().join(".game.chd-verify").exists(),
        "verify temp dir leaked"
    );
    assert!(
        !dir.path().join(".game.chd.tmp").exists(),
        "temp output file leaked"
    );

    let failures = delete_job_sources(&job);
    assert!(failures.is_empty(), "{failures:?}");
    assert!(!cue_path.exists());
    assert!(!dir.path().join("game (Track 2).bin").exists());
    assert!(job.output.is_file(), "the CHD must survive source deletion");
}

#[test]
fn compression_failure_leaves_no_partial_output() {
    let Ok(chdman) = Chdman::detect(Path::new("")) else {
        eprintln!("chdman not installed; skipping integration test");
        return;
    };

    let dir = tempfile::tempdir().unwrap();
    // A cue whose bin vanishes between planning and compression.
    let cue_path = write_redump_style_disc(dir.path());
    let job = plan_compression(&cue_path, &Ps1Analyzer).unwrap();
    fs::remove_file(dir.path().join("game (Track 2).bin")).unwrap();

    let cancel = AtomicBool::new(false);
    let err = compress_to_chd(&chdman, &job, &|_, _| {}, &cancel).unwrap_err();
    assert!(matches!(err, ChdConvertError::ChdmanFailed { .. }), "{err}");
    assert!(!job.output.exists(), "partial .chd must be cleaned up");
    assert!(
        !dir.path().join(".game.chd.tmp").exists(),
        "partial temp file must be cleaned up"
    );
}

#[test]
fn m3u_references_follow_the_compressed_disc() {
    let dir = tempfile::tempdir().unwrap();
    let cue_path = write_redump_style_disc(dir.path());
    // A sibling disc in the same playlist that this job did not touch —
    // still a real file, so it must be left alone.
    fs::write(
        dir.path().join("other disc.cue"),
        "FILE \"other.bin\" BINARY\n",
    )
    .unwrap();
    fs::write(dir.path().join("game.m3u"), "game.cue\nother disc.cue\n").unwrap();
    let job = plan_compression(&cue_path, &Ps1Analyzer).unwrap();
    // Simulate the state after a verified compress + delete_job_sources:
    // the .chd exists on disk and the cue (the broken reference) is gone —
    // the rename.rs machinery only fixes references that are actually
    // broken, matching against real files in the directory.
    fs::write(&job.output, [0u8; 4]).unwrap();
    for f in &job.source_files {
        let _ = fs::remove_file(f);
    }

    let (updated, errors) = update_m3u_references(&job);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(updated, 1);
    assert_eq!(
        fs::read_to_string(dir.path().join("game.m3u")).unwrap(),
        "game.chd\nother disc.cue\n"
    );
}

#[test]
fn detect_rejects_non_chdman_binary() {
    // /bin/true runs fine but prints no CHD banner.
    let true_path = Path::new("/bin/true");
    if !true_path.exists() {
        return;
    }
    assert!(Chdman::detect(true_path).is_err());
}
