use super::*;

fn mark(kind: MarkKind, name: &str, sha1: &str) -> CollectionMark {
    CollectionMark {
        schema_version: MARK_SCHEMA_VERSION,
        kind,
        platform_id: "nes".to_owned(),
        region: "usa".to_owned(),
        name: name.to_owned(),
        parent_media_id: String::new(),
        parent_dat_name: String::new(),
        content: MarkedContent {
            size: 262_144,
            crc32: "deadbeef".to_owned(),
            sha1: sha1.to_owned(),
            md5: String::new(),
        },
        chosen_media_id: String::new(),
        chosen_dat_name: String::new(),
        note: String::new(),
    }
}

#[test]
fn a_mark_round_trips_through_its_file() {
    let temp = tempfile::tempdir().unwrap();
    let original = mark(MarkKind::Homebrew, "Finchy Quest", "aaaa1111");
    write_mark(temp.path(), &original).unwrap();
    assert_eq!(load_marks(temp.path()).unwrap(), vec![original]);
}

/// The property the whole design rests on: the same decision made twice, or on
/// two machines, produces the same path and the same bytes. Syncthing has
/// nothing to raise a conflict over and rsync has nothing to clobber — which
/// is why no timestamp is recorded.
#[test]
fn the_same_decision_is_byte_identical_wherever_it_is_made() {
    let one = tempfile::tempdir().unwrap();
    let two = tempfile::tempdir().unwrap();
    let decision = mark(MarkKind::Homebrew, "Finchy Quest", "aaaa1111");

    let first = write_mark(one.path(), &decision).unwrap();
    let second = write_mark(two.path(), &decision).unwrap();
    assert_eq!(first.file_name(), second.file_name());
    assert_eq!(
        std::fs::read(&first).unwrap(),
        std::fs::read(&second).unwrap()
    );

    // Re-writing an identical mark must not touch the file, or every sync
    // would see a modification that changes nothing.
    let before = std::fs::metadata(&first).unwrap().modified().unwrap();
    write_mark(one.path(), &decision).unwrap();
    assert_eq!(
        std::fs::metadata(&first).unwrap().modified().unwrap(),
        before
    );
}

/// Distinct files never share a mark file, so two machines marking different
/// games cannot collide; re-deciding one game replaces only that decision.
#[test]
fn marks_are_keyed_by_content_not_by_name() {
    let temp = tempfile::tempdir().unwrap();
    write_mark(temp.path(), &mark(MarkKind::Homebrew, "One", "aaaa1111")).unwrap();
    write_mark(temp.path(), &mark(MarkKind::Homebrew, "Two", "bbbb2222")).unwrap();
    assert_eq!(load_marks(temp.path()).unwrap().len(), 2);

    // Same content, changed mind: one file, the newer decision.
    let revised = mark(MarkKind::Modded, "One, actually a mod", "aaaa1111");
    write_mark(temp.path(), &revised).unwrap();
    let marks = load_marks(temp.path()).unwrap();
    assert_eq!(marks.len(), 2);
    assert!(marks.contains(&revised));
}

/// A collection that has never been marked is not an error, and neither is one
/// bad file — losing every other decision to a single unparseable mark would
/// be the worst possible failure for a store whose whole job is durability.
#[test]
fn an_absent_store_or_one_bad_mark_costs_nothing_else() {
    let temp = tempfile::tempdir().unwrap();
    assert!(load_marks(temp.path()).unwrap().is_empty());

    let good = mark(MarkKind::Homebrew, "Finchy Quest", "aaaa1111");
    write_mark(temp.path(), &good).unwrap();
    std::fs::write(marks_directory(temp.path()).join("nes-corrupt.toml"), "{{{").unwrap();
    // A mark from a machine running a newer schema is skipped, not fatal.
    let mut future = mark(MarkKind::Homebrew, "From The Future", "cccc3333");
    future.schema_version = MARK_SCHEMA_VERSION + 1;
    write_mark(temp.path(), &future).unwrap();

    assert_eq!(load_marks(temp.path()).unwrap(), vec![good]);
}

#[test]
fn a_mark_without_any_digest_has_no_identity() {
    let temp = tempfile::tempdir().unwrap();
    let mut orphan = mark(MarkKind::Homebrew, "Nameless", "");
    orphan.content.crc32 = String::new();
    assert!(matches!(
        write_mark(temp.path(), &orphan),
        Err(MarkError::NoDigest)
    ));
}

#[test]
fn forgetting_a_decision_removes_only_its_own_file() {
    let temp = tempfile::tempdir().unwrap();
    let kept = mark(MarkKind::Homebrew, "Kept", "aaaa1111");
    let dropped = mark(MarkKind::Homebrew, "Dropped", "bbbb2222");
    write_mark(temp.path(), &kept).unwrap();
    write_mark(temp.path(), &dropped).unwrap();

    assert!(remove_mark(temp.path(), &dropped).unwrap());
    assert!(!remove_mark(temp.path(), &dropped).unwrap());
    assert_eq!(load_marks(temp.path()).unwrap(), vec![kept]);
}

/// The path users actually get: an archive and a playable library that are
/// siblings put the marks store beside both, not inside either.
#[test]
fn marks_live_beside_the_archive_and_the_playable_library() {
    let profile = crate::CollectionProfile::for_roots(
        PathBuf::from("/Volumes/fatretro/RetroLibrary/archive"),
        PathBuf::from("/Volumes/fatretro/RetroLibrary/roms"),
    );
    assert_eq!(
        marks_directory(&profile.collection_root()),
        PathBuf::from("/Volumes/fatretro/RetroLibrary/.retro-junk/marks")
    );

    // Unrelated roots have no shared parent; the archive is the durable one.
    let split = crate::CollectionProfile::for_roots(
        PathBuf::from("/nas/archive"),
        PathBuf::from("/local/roms"),
    );
    assert_eq!(split.collection_root(), PathBuf::from("/nas/archive"));
}
