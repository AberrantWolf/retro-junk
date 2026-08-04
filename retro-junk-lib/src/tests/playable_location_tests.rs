//! Finding a release's built playable when the evidence names one folder and
//! the file lives in another.

use super::*;
use crate::archive_fixture::{archive_with_playable, archive_with_playable_at};

/// Read the release and the evidence back out of a scanned archive.
fn release_and_evidence(
    snapshot: &retro_junk_archive::ArchiveIndexSnapshot,
) -> (&IndexedRelease, &IndexedDump, &BuildEvidence) {
    let release = &snapshot.releases[0];
    let dump = &release.physical_copies[0].carriers[0].dumps[0];
    let evidence = retro_junk_archive::current_build_evidence(dump)[0];
    (release, dump, evidence)
}

/// The bug this module exists to prevent, at its smallest.
///
/// A PS1 release is archived under `ps1` and built into the frontend's `psx`
/// folder, so its evidence and its file name different directories. Everything
/// that read the recorded path literally reported the playable missing.
#[test]
fn an_output_written_under_the_archive_folder_is_found_in_the_frontend_folder() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = archive_with_playable_at(temp.path(), "Game (USA).chd", "ps1", "psx");
    let snapshot = retro_junk_archive::scan_archive(&fixture.archive).unwrap();
    let (release, _, evidence) = release_and_evidence(&snapshot);

    assert_eq!(evidence.relative_output_path, "ps1/Game (USA).chd");
    assert_eq!(
        release_output_relative(release, &fixture.playable_root, evidence),
        "psx/Game (USA).chd"
    );
    assert!(release_output_path(release, &fixture.playable_root, evidence).is_file());
}

/// Older evidence records a bare file name, from before playables were filed
/// by system folder at all. The folder is prepended rather than replaced.
#[test]
fn an_output_recorded_with_no_folder_is_found_under_the_system_folder() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = archive_with_playable_at(temp.path(), "Game (USA).chd", "", "psx");
    let snapshot = retro_junk_archive::scan_archive(&fixture.archive).unwrap();
    let (release, _, evidence) = release_and_evidence(&snapshot);

    assert_eq!(evidence.relative_output_path, "Game (USA).chd");
    assert_eq!(
        release_output_relative(release, &fixture.playable_root, evidence),
        "psx/Game (USA).chd"
    );
}

/// Resolution must not go looking when there is nothing to look for: a file
/// that is where its evidence says stays put, or a rename would move the
/// wrong file the moment two folders held the same name.
#[test]
fn an_output_that_is_where_it_says_is_left_alone() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = archive_with_playable(temp.path(), "Game (USA).chd");
    let snapshot = retro_junk_archive::scan_archive(&fixture.archive).unwrap();
    let (release, _, evidence) = release_and_evidence(&snapshot);

    assert_eq!(
        release_output_relative(release, &fixture.playable_root, evidence),
        "psx/Game (USA).chd"
    );
}

/// A same-named file of a different size is somebody else's, so it is not
/// adopted as this release's output.
#[test]
fn a_different_file_under_the_system_folder_is_not_mistaken_for_the_output() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = archive_with_playable_at(temp.path(), "Game (USA).chd", "ps1", "psx");
    std::fs::write(
        fixture.playable_root.join("psx").join("Game (USA).chd"),
        b"a different game entirely",
    )
    .unwrap();
    let snapshot = retro_junk_archive::scan_archive(&fixture.archive).unwrap();
    let (release, _, evidence) = release_and_evidence(&snapshot);

    assert_eq!(
        release_output_relative(release, &fixture.playable_root, evidence),
        "ps1/Game (USA).chd",
        "a file of the wrong size was accepted as this release's output"
    );
}

/// Locating a file more cleverly must not quietly answer the *other*
/// question. An output built from an earlier dump of the carrier is stale
/// however findable it is, because what is on disk no longer reproduces what
/// the archive preserves.
#[test]
fn a_findable_output_built_from_an_older_dump_is_still_reported_stale() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = archive_with_playable_at(temp.path(), "Game (USA).chd", "ps1", "psx");
    let snapshot = retro_junk_archive::scan_archive(&fixture.archive).unwrap();
    let (release, dump, evidence) = release_and_evidence(&snapshot);

    let (_, presence) = release_output_presence(release, &fixture.playable_root, dump, evidence);
    assert_eq!(presence, RepresentationPresence::Present);

    // The same evidence, now claiming a dump the archive no longer holds.
    let mut from_an_older_dump = evidence.clone();
    from_an_older_dump.input_manifest_sha256 = "a fingerprint from an earlier dump".to_owned();
    let (_, presence) =
        release_output_presence(release, &fixture.playable_root, dump, &from_an_older_dump);
    assert_eq!(
        presence,
        RepresentationPresence::Stale,
        "resolving the location swallowed the freshness check"
    );
}
