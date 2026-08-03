//! The whole point of the cache is that the expensive part happens once.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use retro_junk_io::ProgressUnit;

use crate::redumper_cache::{cache_directory, prepare};

/// A stand-in for redumper that records every phase it is asked to run.
///
/// Counting `split` invocations is how a test can tell "reused the earlier
/// result" from "quietly did the work again", which no assertion about the
/// returned track digests could distinguish.
#[cfg(unix)]
fn fake_redumper(directory: &Path) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let tool = directory.join("redumper");
    let invocations = directory.join("invocations");
    std::fs::write(
        &tool,
        format!(
            r#"#!/bin/sh
if [ "$1" = "--help" ]; then echo "redumper cache test"; exit 0; fi
echo "$1" >> '{}'
if [ "$1" = "split" ]; then
  printf 'FILE "disc (Track 01).bin" BINARY\n' > disc.cue
  printf 'track' > 'disc (Track 01).bin'
fi
echo '<rom name="disc (Track 01).bin" size="5" crc="AABBCCDD" md5="0011" sha1="11223344" />'
"#,
            invocations.display()
        ),
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&tool).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&tool, permissions).unwrap();
    (tool, invocations)
}

#[cfg(unix)]
fn splits(invocations: &Path) -> usize {
    std::fs::read_to_string(invocations)
        .unwrap_or_default()
        .lines()
        .filter(|line| *line == "split")
        .count()
}

#[cfg(unix)]
fn raw_dump(directory: &Path) -> PathBuf {
    let raw = directory.join("raw");
    std::fs::create_dir_all(&raw).unwrap();
    std::fs::write(raw.join("disc.scram"), b"raw master bytes").unwrap();
    std::fs::write(raw.join("disc.state"), b"state").unwrap();
    raw
}

/// Identification and a later build both need this work done, and each used to
/// do it independently — a full copy plus split of the raw dump, twice.
#[cfg(unix)]
#[test]
fn a_second_caller_reuses_the_first_callers_split_output() {
    let temp = tempfile::tempdir().unwrap();
    let raw = raw_dump(temp.path());
    let workspace = temp.path().join("work");
    let (tool, invocations) = fake_redumper(temp.path());

    let first = prepare(
        &tool,
        &raw,
        &workspace,
        "dump-sha",
        &|_, _, _, _| {},
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(!first.reused());
    assert_eq!(first.audit().tracks.len(), 1);
    assert_eq!(splits(&invocations), 1);
    first.keep();

    let second = prepare(
        &tool,
        &raw,
        &workspace,
        "dump-sha",
        &|_, _, _, _| {},
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(second.reused());
    assert_eq!(splits(&invocations), 1, "the split ran a second time");
    assert_eq!(second.audit().tracks[0].sha1, "11223344");
    assert!(second.entrypoint().unwrap().ends_with("disc.cue"));
}

/// The cache is keyed on the dump's manifest hash, so a repaired or re-ingested
/// dump can never be handed the previous contents' tracks.
#[cfg(unix)]
#[test]
fn different_bytes_never_read_the_earlier_entry() {
    let temp = tempfile::tempdir().unwrap();
    let raw = raw_dump(temp.path());
    let workspace = temp.path().join("work");
    let (tool, invocations) = fake_redumper(temp.path());

    prepare(
        &tool,
        &raw,
        &workspace,
        "first-sha",
        &|_, _, _, _| {},
        &AtomicBool::new(false),
    )
    .unwrap()
    .keep();
    let other = prepare(
        &tool,
        &raw,
        &workspace,
        "second-sha",
        &|_, _, _, _| {},
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(!other.reused());
    assert_eq!(splits(&invocations), 2);
}

/// Split output left behind without its audit record cannot be trusted to
/// describe the dump it sits under, so it is rebuilt rather than believed.
#[cfg(unix)]
#[test]
fn a_half_written_entry_is_redone_not_reused() {
    let temp = tempfile::tempdir().unwrap();
    let raw = raw_dump(temp.path());
    let workspace = temp.path().join("work");
    let (tool, invocations) = fake_redumper(temp.path());

    prepare(
        &tool,
        &raw,
        &workspace,
        "dump-sha",
        &|_, _, _, _| {},
        &AtomicBool::new(false),
    )
    .unwrap()
    .keep();
    std::fs::remove_file(cache_directory(&workspace, "dump-sha").join("audit.json")).unwrap();

    let again = prepare(
        &tool,
        &raw,
        &workspace,
        "dump-sha",
        &|_, _, _, _| {},
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(!again.reused());
    assert_eq!(splits(&invocations), 2);
    assert!(
        cache_directory(&workspace, "dump-sha")
            .join("audit.json")
            .is_file()
    );
}

/// Discarding has to actually reclaim the space — a disc the catalog cannot
/// resolve gets no build, and keeping its split output would grow the workspace
/// on every convergence run.
#[cfg(unix)]
#[test]
fn discarding_frees_the_scratch_space() {
    let temp = tempfile::tempdir().unwrap();
    let raw = raw_dump(temp.path());
    let workspace = temp.path().join("work");
    let (tool, _) = fake_redumper(temp.path());

    let prepared = prepare(
        &tool,
        &raw,
        &workspace,
        "dump-sha",
        &|_, _, _, _| {},
        &AtomicBool::new(false),
    )
    .unwrap();
    prepared.discard();
    assert!(!cache_directory(&workspace, "dump-sha").exists());
}

/// The copy dominates a disc audit's runtime, so it has to report bytes as it
/// goes. Reporting nothing left the caller's progress bar frozen for minutes;
/// reporting without the unit made a one-dump run render "0 B / 1 B".
#[cfg(unix)]
#[test]
fn the_copy_phase_reports_byte_progress() {
    let temp = tempfile::tempdir().unwrap();
    let raw = raw_dump(temp.path());
    let workspace = temp.path().join("work");
    let (tool, _) = fake_redumper(temp.path());

    let reports = std::sync::Mutex::new(Vec::new());
    prepare(
        &tool,
        &raw,
        &workspace,
        "dump-sha",
        &|phase, unit, current, total| {
            reports
                .lock()
                .unwrap()
                .push((phase.to_owned(), unit, current, total));
        },
        &AtomicBool::new(false),
    )
    .unwrap()
    .keep();

    let reports = reports.into_inner().unwrap();
    assert!(
        reports.iter().any(|(phase, unit, current, total)| {
            phase == retro_junk_archive::redumper::COPY_PHASE
                && *unit == ProgressUnit::Bytes
                && *current > 0
                && *total > 0
        }),
        "expected byte progress while copying, got {reports:?}"
    );
}
