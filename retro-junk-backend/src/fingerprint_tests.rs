//! The bug these guard: a name-only fingerprint reported "unchanged" for a
//! file replaced in place, so the row kept the hashes, DAT match, and
//! verification verdict belonging to bytes that were no longer there.

use super::compute_fingerprint;

fn write(path: &std::path::Path, contents: &[u8]) {
    std::fs::write(path, contents).unwrap();
}

#[test]
fn replacing_a_file_in_place_changes_the_fingerprint() {
    let dir = tempfile::tempdir().unwrap();
    let rom = dir.path().join("game.chd");
    write(&rom, b"first dump");
    let before = compute_fingerprint(dir.path()).name_hash;

    // Same name, different bytes — a re-dump or a recompress.
    write(&rom, b"a different dump entirely");
    let after = compute_fingerprint(dir.path()).name_hash;

    assert_ne!(
        before, after,
        "a file replaced in place must not read as unchanged"
    );
}

#[test]
fn a_truncated_file_changes_the_fingerprint() {
    let dir = tempfile::tempdir().unwrap();
    let rom = dir.path().join("game.iso");
    write(&rom, b"complete contents here");
    let before = compute_fingerprint(dir.path()).name_hash;

    write(&rom, b"trunc");
    assert_ne!(before, compute_fingerprint(dir.path()).name_hash);
}

#[test]
fn a_replaced_file_one_level_down_changes_the_fingerprint() {
    // Multi-disc sets live in a subdirectory, and their tracks are exactly
    // the files most likely to be re-dumped on their own.
    let dir = tempfile::tempdir().unwrap();
    let set = dir.path().join("Game.m3u");
    std::fs::create_dir(&set).unwrap();
    let track = set.join("Game (Disc 1).bin");
    write(&track, b"original track");
    let before = compute_fingerprint(dir.path()).name_hash;

    write(&track, b"re-dumped track data");
    assert_ne!(before, compute_fingerprint(dir.path()).name_hash);
}

#[test]
fn an_untouched_folder_keeps_its_fingerprint() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("game.nes"), b"rom");
    write(&dir.path().join("other.nes"), b"another");
    let first = compute_fingerprint(dir.path()).name_hash;
    assert_eq!(first, compute_fingerprint(dir.path()).name_hash);
}

#[test]
fn adding_and_removing_files_still_changes_the_fingerprint() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("game.nes"), b"rom");
    let one = compute_fingerprint(dir.path()).name_hash;

    write(&dir.path().join("second.nes"), b"rom");
    let two = compute_fingerprint(dir.path()).name_hash;
    assert_ne!(one, two);

    std::fs::remove_file(dir.path().join("second.nes")).unwrap();
    assert_eq!(one, compute_fingerprint(dir.path()).name_hash);
}
