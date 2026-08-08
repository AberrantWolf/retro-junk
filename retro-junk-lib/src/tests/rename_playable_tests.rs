//! The repair for playables built under an older naming rule: the file, its
//! artwork, and the playlist that lists it all move together, and the archive
//! records where it went.

use super::*;
use crate::archive_fixture::{archive_with_playable, archive_with_playable_at};

#[test]
fn renaming_moves_the_file_its_artwork_and_the_playlist_that_lists_it() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = archive_with_playable(temp.path(), "Game (USA) (Track 1).chd");

    // Artwork is named after the playable's stem, so it must follow.
    let media_root = temp.path().join("media");
    let covers = media_root.join("psx").join("covers");
    std::fs::create_dir_all(&covers).unwrap();
    std::fs::write(covers.join("Game (USA) (Track 1).png"), b"art").unwrap();

    // A playlist in the same directory names the old file.
    let playlist = fixture.playable_root.join("psx").join("Game.m3u");
    std::fs::write(&playlist, "Game (USA) (Track 1).chd\n").unwrap();

    let snapshot = retro_junk_archive::scan_archive(&fixture.archive).unwrap();
    let report = rename_playable(&RenamePlayableRequest {
        snapshot: &snapshot,
        playable_root: &fixture.playable_root,
        representation_id: &fixture.representation_id,
        canonical_file_name: "Game (USA).chd",
        media_root: Some(&media_root),
    })
    .unwrap();

    assert_eq!(report.to, "psx/Game (USA).chd");
    assert!(fixture.playable_root.join("psx/Game (USA).chd").is_file());
    assert!(
        !fixture
            .playable_root
            .join("psx/Game (USA) (Track 1).chd")
            .exists()
    );
    assert_eq!(report.media_renamed, 1);
    assert!(covers.join("Game (USA).png").is_file());
    assert_eq!(report.playlists_updated, 1);
    assert_eq!(
        std::fs::read_to_string(&playlist).unwrap().trim(),
        "Game (USA).chd"
    );
}

#[test]
fn the_archive_records_where_the_file_went_without_losing_where_it_was() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = archive_with_playable(temp.path(), "Game (USA) (Track 1).chd");
    let snapshot = retro_junk_archive::scan_archive(&fixture.archive).unwrap();
    rename_playable(&RenamePlayableRequest {
        snapshot: &snapshot,
        playable_root: &fixture.playable_root,
        representation_id: &fixture.representation_id,
        canonical_file_name: "Game (USA).chd",
        media_root: None,
    })
    .unwrap();

    let rescanned = retro_junk_archive::scan_archive(&fixture.archive).unwrap();
    let dump = &rescanned.releases[0].physical_copies[0].carriers[0].dumps[0];
    // Evidence is append-only history: both records survive, and the newest
    // names the current location.
    assert_eq!(dump.builds.len(), 2);
    let newest = dump
        .builds
        .iter()
        .max_by(|left, right| left.evidence.performed_at.cmp(&right.evidence.performed_at))
        .unwrap();
    assert_eq!(newest.evidence.relative_output_path, "psx/Game (USA).chd");
    // Same file, same identity — so the projection updates the representation
    // rather than inventing a second one beside it.
    assert_eq!(
        newest.evidence.child_representation_id.to_string(),
        fixture.representation_id
    );
}

/// A rename must stay inside the build lineage it found.
///
/// Lineage is `(parent representation, format)`, and a representation row is
/// the current state of one file. Writing the *dump's* format instead of the
/// playable's started a second lineage, so both the old and the new record
/// stayed live and each projected a representation row under the same id —
/// which the projection rejects with a unique-constraint failure, taking the
/// whole reconcile down with it.
#[test]
fn renaming_supersedes_the_old_record_instead_of_starting_a_second_lineage() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = archive_with_playable(temp.path(), "Game (USA) (Track 1).chd");
    let snapshot = retro_junk_archive::scan_archive(&fixture.archive).unwrap();
    let before = retro_junk_archive::current_build_evidence(
        &snapshot.releases[0].physical_copies[0].carriers[0].dumps[0],
    );
    assert_eq!(before.len(), 1);
    let original_format = before[0].format.clone();

    rename_playable(&RenamePlayableRequest {
        snapshot: &snapshot,
        playable_root: &fixture.playable_root,
        representation_id: &fixture.representation_id,
        canonical_file_name: "Game (USA).chd",
        media_root: None,
    })
    .unwrap();

    let rescanned = retro_junk_archive::scan_archive(&fixture.archive).unwrap();
    let dump = &rescanned.releases[0].physical_copies[0].carriers[0].dumps[0];
    let current = retro_junk_archive::current_build_evidence(dump);
    assert_eq!(
        current.len(),
        1,
        "the rename left two live records, which project two rows under one id"
    );
    assert_eq!(current[0].relative_output_path, "psx/Game (USA).chd");
    // The playable's own format, not the preservation master's.
    assert_eq!(current[0].format, original_format);
}

/// A rename says nothing about the bytes, so what was verified about them
/// must survive it. Re-deriving these downgraded a playable that had been
/// proven to reproduce its master.
#[test]
fn renaming_preserves_what_was_verified_about_the_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = archive_with_playable(temp.path(), "Game (USA) (Track 1).chd");
    // Mark the existing build as fully verified, as a real one would be.
    let snapshot = retro_junk_archive::scan_archive(&fixture.archive).unwrap();
    let dump = &snapshot.releases[0].physical_copies[0].carriers[0].dumps[0];
    let mut verified = dump.builds[0].evidence.clone();
    verified.catalog_verified = true;
    verified.round_trip_verified = true;
    verified.build_id = retro_junk_archive::BuildId::new();
    verified.performed_at = "2026-02-01T00:00:00Z".to_owned();
    retro_junk_archive::write_build_evidence(&dump.directory, &verified).unwrap();

    let snapshot = retro_junk_archive::scan_archive(&fixture.archive).unwrap();
    rename_playable(&RenamePlayableRequest {
        snapshot: &snapshot,
        playable_root: &fixture.playable_root,
        representation_id: &fixture.representation_id,
        canonical_file_name: "Game (USA).chd",
        media_root: None,
    })
    .unwrap();

    let rescanned = retro_junk_archive::scan_archive(&fixture.archive).unwrap();
    let current = retro_junk_archive::current_build_evidence(
        &rescanned.releases[0].physical_copies[0].carriers[0].dumps[0],
    );
    assert_eq!(current.len(), 1);
    assert!(current[0].catalog_verified);
    assert!(
        current[0].round_trip_verified,
        "the rename dropped a round-trip verification it had no business touching"
    );
}

/// The repair has to look where the file is, not where it was written.
///
/// A PS1 release is archived under `ps1` and its playable is built
/// into the frontend's `psx` folder, so the evidence says `ps1/Game.chd` while
/// the file sits at `psx/Game.chd`. The projection follows that trail and
/// reports the playable present and misnamed — so the UI offers a rename — but
/// the repair read the recorded path literally and gave up, telling the user
/// the file was not where its evidence said.
#[test]
fn renaming_finds_a_playable_filed_under_the_frontends_system_directory() {
    let temp = tempfile::tempdir().unwrap();
    let fixture =
        archive_with_playable_at(temp.path(), "Biohazard 3 - Last Escape.chd", "ps1", "psx");

    let snapshot = retro_junk_archive::scan_archive(&fixture.archive).unwrap();
    let report = rename_playable(&RenamePlayableRequest {
        snapshot: &snapshot,
        playable_root: &fixture.playable_root,
        representation_id: &fixture.representation_id,
        canonical_file_name: "Biohazard 3 - Last Escape (Japan).chd",
        media_root: None,
    })
    .unwrap();

    assert_eq!(report.from, "psx/Biohazard 3 - Last Escape.chd");
    assert_eq!(report.to, "psx/Biohazard 3 - Last Escape (Japan).chd");
    assert!(
        fixture
            .playable_root
            .join("psx/Biohazard 3 - Last Escape (Japan).chd")
            .is_file()
    );
    assert!(
        !fixture
            .playable_root
            .join("psx/Biohazard 3 - Last Escape.chd")
            .exists()
    );

    // The new record names the file's real folder, so nothing downstream has
    // to keep following the old trail to find it.
    let rescanned = retro_junk_archive::scan_archive(&fixture.archive).unwrap();
    let current = retro_junk_archive::current_build_evidence(
        &rescanned.releases[0].physical_copies[0].carriers[0].dumps[0],
    );
    assert_eq!(current.len(), 1);
    assert_eq!(
        current[0].relative_output_path,
        "psx/Biohazard 3 - Last Escape (Japan).chd"
    );
}

#[test]
fn a_name_collision_stops_rather_than_destroying_the_other_file() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = archive_with_playable(temp.path(), "Game (USA) (Track 1).chd");
    let occupied = fixture.playable_root.join("psx").join("Game (USA).chd");
    std::fs::write(&occupied, b"someone else's bytes").unwrap();

    let snapshot = retro_junk_archive::scan_archive(&fixture.archive).unwrap();
    let result = rename_playable(&RenamePlayableRequest {
        snapshot: &snapshot,
        playable_root: &fixture.playable_root,
        representation_id: &fixture.representation_id,
        canonical_file_name: "Game (USA).chd",
        media_root: None,
    });
    assert!(matches!(result, Err(RenamePlayableError::TargetExists(_))));
    assert_eq!(
        std::fs::read(&occupied).unwrap(),
        b"someone else's bytes",
        "the existing file was overwritten"
    );
}

#[test]
fn an_equivalent_canonical_copy_wins_and_the_old_name_is_backed_up() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = archive_with_playable(temp.path(), "Game (USA) (Track 1).chd");
    let old = fixture.playable_root.join("psx/Game (USA) (Track 1).chd");
    let canonical = fixture.playable_root.join("psx/Game (USA).chd");
    std::fs::copy(&old, &canonical).unwrap();

    let snapshot = retro_junk_archive::scan_archive(&fixture.archive).unwrap();
    let report = rename_playable(&RenamePlayableRequest {
        snapshot: &snapshot,
        playable_root: &fixture.playable_root,
        representation_id: &fixture.representation_id,
        canonical_file_name: "Game (USA).chd",
        media_root: None,
    })
    .unwrap();

    assert_eq!(report.to, "psx/Game (USA).chd");
    assert!(canonical.is_file());
    assert!(!old.exists());
    let backups = fixture.playable_root.join(".retro-junk-backups");
    let backed_up = std::fs::read_dir(&backups)
        .unwrap()
        .filter_map(Result::ok)
        .any(|entry| entry.path().join("psx/Game (USA) (Track 1).chd").is_file());
    assert!(backed_up, "the displaced playable was not retained");

    // The newest evidence points at the canonical copy, so convergence is a
    // no-op when the same representation is inspected again.
    let rescanned = retro_junk_archive::scan_archive(&fixture.archive).unwrap();
    let second = rename_playable(&RenamePlayableRequest {
        snapshot: &rescanned,
        playable_root: &fixture.playable_root,
        representation_id: &fixture.representation_id,
        canonical_file_name: "Game (USA).chd",
        media_root: None,
    })
    .unwrap();
    assert_eq!(second.from, second.to);
}
