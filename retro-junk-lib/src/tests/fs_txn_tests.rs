use super::*;
use std::fs;
use tempfile::TempDir;

fn write(dir: &TempDir, name: &str, content: &str) -> PathBuf {
    let path = dir.path().join(name);
    fs::write(&path, content).unwrap();
    path
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

#[test]
fn commit_renames_and_writes() {
    let dir = TempDir::new().unwrap();
    let a = write(&dir, "a.bin", "aaa");
    let cue = write(&dir, "game.cue", "FILE \"a.bin\" BINARY\n");

    let mut txn = FsTransaction::new();
    txn.rename(&a, dir.path().join("Game (USA).bin"));
    txn.rename(&cue, dir.path().join("Game (USA).cue"));
    txn.write_file(
        dir.path().join("Game (USA).cue"),
        "FILE \"Game (USA).bin\" BINARY\n",
    );

    let summary = txn.commit().unwrap();
    assert_eq!(summary.renames, 2);
    assert_eq!(summary.writes, 1);
    assert!(!a.exists());
    assert!(!cue.exists());
    assert_eq!(read(&dir.path().join("Game (USA).bin")), "aaa");
    assert_eq!(
        read(&dir.path().join("Game (USA).cue")),
        "FILE \"Game (USA).bin\" BINARY\n"
    );
}

#[test]
fn noop_renames_are_skipped() {
    let dir = TempDir::new().unwrap();
    let a = write(&dir, "a.bin", "aaa");

    let mut txn = FsTransaction::new();
    txn.rename(&a, &a);
    assert!(txn.is_empty());
}

#[test]
fn preflight_rejects_missing_source() {
    let dir = TempDir::new().unwrap();
    let mut txn = FsTransaction::new();
    txn.rename(dir.path().join("ghost.bin"), dir.path().join("x.bin"));
    let err = txn.commit().unwrap_err();
    assert!(err.message.contains("does not exist"), "{}", err.message);
    assert!(err.rollback_errors.is_empty());
}

#[test]
fn preflight_rejects_existing_target() {
    let dir = TempDir::new().unwrap();
    let a = write(&dir, "a.bin", "aaa");
    let b = write(&dir, "b.bin", "bbb");

    let mut txn = FsTransaction::new();
    txn.rename(&a, &b);
    let err = txn.commit().unwrap_err();
    assert!(err.message.contains("already exists"), "{}", err.message);
    // Nothing changed
    assert_eq!(read(&a), "aaa");
    assert_eq!(read(&b), "bbb");
}

#[test]
fn preflight_rejects_duplicate_targets() {
    let dir = TempDir::new().unwrap();
    let a = write(&dir, "a.bin", "aaa");
    let b = write(&dir, "b.bin", "bbb");

    let mut txn = FsTransaction::new();
    let target = dir.path().join("same.bin");
    txn.rename(&a, &target);
    txn.rename(&b, &target);
    let err = txn.commit().unwrap_err();
    assert!(
        err.message.contains("Multiple operations"),
        "{}",
        err.message
    );
    assert_eq!(read(&a), "aaa");
    assert_eq!(read(&b), "bbb");
}

#[test]
fn swap_renames_use_two_phase() {
    let dir = TempDir::new().unwrap();
    let one = write(&dir, "Track 1.bin", "one");
    let two = write(&dir, "Track 2.bin", "two");

    let mut txn = FsTransaction::new();
    txn.rename(&one, &two);
    txn.rename(&two, &one);
    txn.commit().unwrap();

    assert_eq!(read(&dir.path().join("Track 1.bin")), "two");
    assert_eq!(read(&dir.path().join("Track 2.bin")), "one");
}

#[test]
fn chain_renames_use_two_phase() {
    let dir = TempDir::new().unwrap();
    let a = write(&dir, "a.bin", "aaa");
    let b = write(&dir, "b.bin", "bbb");

    // a -> b (occupied by a txn source), b -> c
    let mut txn = FsTransaction::new();
    txn.rename(&a, &b);
    txn.rename(&b, dir.path().join("c.bin"));
    txn.commit().unwrap();

    assert!(!a.exists());
    assert_eq!(read(&dir.path().join("b.bin")), "aaa");
    assert_eq!(read(&dir.path().join("c.bin")), "bbb");
}

#[test]
fn failed_rename_rolls_back_completed_ops() {
    let dir = TempDir::new().unwrap();
    let a = write(&dir, "a.bin", "aaa");
    let b = write(&dir, "b.bin", "bbb");

    let mut txn = FsTransaction::new();
    txn.rename(&a, dir.path().join("a2.bin"));
    // Renaming into a directory that doesn't exist passes preflight
    // (target doesn't exist) but fails at execution.
    txn.rename(&b, dir.path().join("no-such-dir").join("b2.bin"));

    let err = txn.commit().unwrap_err();
    assert!(err.rollback_errors.is_empty(), "{:?}", err.rollback_errors);
    // First rename was undone
    assert_eq!(read(&a), "aaa");
    assert_eq!(read(&b), "bbb");
    assert!(!dir.path().join("a2.bin").exists());
}

#[test]
fn failed_write_rolls_back_renames_and_writes() {
    let dir = TempDir::new().unwrap();
    let a = write(&dir, "a.bin", "aaa");
    let existing = write(&dir, "notes.txt", "original");
    // A directory at the write path passes preflight (parent exists) but
    // fails at fs::write time.
    let blocked = dir.path().join("blocked.txt");
    fs::create_dir(&blocked).unwrap();

    let mut txn = FsTransaction::new();
    txn.rename(&a, dir.path().join("a2.bin"));
    txn.write_file(&existing, "modified");
    txn.write_file(&blocked, "will fail");

    let err = txn.commit().unwrap_err();
    assert!(err.rollback_errors.is_empty(), "{:?}", err.rollback_errors);
    assert_eq!(read(&a), "aaa");
    assert!(!dir.path().join("a2.bin").exists());
    assert_eq!(read(&existing), "original");
}

#[test]
fn write_to_new_file_is_deleted_on_rollback() {
    let dir = TempDir::new().unwrap();
    let new_file = dir.path().join("new.txt");
    let blocked = dir.path().join("blocked.txt");
    fs::create_dir(&blocked).unwrap();

    let mut txn = FsTransaction::new();
    txn.write_file(&new_file, "created");
    txn.write_file(&blocked, "will fail");

    let err = txn.commit().unwrap_err();
    assert!(err.rollback_errors.is_empty(), "{:?}", err.rollback_errors);
    assert!(!new_file.exists());
}
