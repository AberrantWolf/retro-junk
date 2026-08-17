use std::fs;

use tempfile::TempDir;

use crate::scanner::{extension_set, scan_game_entries};

/// A standalone `.chd` sharing a stem with a `.cue` is the same game (D4):
/// the folder must scan to a single entry, keyed on the cue, not two.
#[test]
fn chd_sharing_stem_with_cue_dedupes_to_one_entry() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("Game.cue"), "FILE \"Game.bin\" BINARY\n").unwrap();
    fs::write(dir.path().join("Game.bin"), "").unwrap();
    fs::write(dir.path().join("Game.chd"), "").unwrap();

    let extensions = extension_set(&["cue", "bin", "chd"]);
    let entries = scan_game_entries(dir.path(), &extensions).unwrap();

    assert_eq!(
        entries.len(),
        1,
        "expected one deduped entry, got {entries:?}"
    );
    assert_eq!(entries[0].display_name(), "Game.cue");
}

/// Without a same-stem cue, a standalone `.chd` is its own entry.
#[test]
fn standalone_chd_is_its_own_entry() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("Solo.chd"), "").unwrap();

    let extensions = extension_set(&["cue", "bin", "chd"]);
    let entries = scan_game_entries(dir.path(), &extensions).unwrap();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].display_name(), "Solo.chd");
}

#[test]
fn playlist_claims_top_level_discs_as_one_logical_game() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("Disc 1.chd"), "").unwrap();
    fs::write(dir.path().join("Disc 2.chd"), "").unwrap();
    let set = dir.path().join("Game.m3u");
    fs::create_dir(&set).unwrap();
    fs::write(set.join("Game.m3u"), "../Disc 1.chd\n../Disc 2.chd\n").unwrap();

    let entries = scan_game_entries(dir.path(), &extension_set(&["chd"])).unwrap();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].display_name(), "Game.m3u");
    assert_eq!(entries[0].all_files().len(), 2);
}

#[test]
fn partially_broken_playlist_is_not_accepted_as_a_smaller_set() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("Disc 1.chd"), "one").unwrap();
    let set = dir.path().join("Game.m3u");
    fs::create_dir(&set).unwrap();
    fs::write(set.join("Disc 2.chd"), "two").unwrap();
    fs::write(set.join("Game.m3u"), "../missing-disc-1.chd\nDisc 2.chd\n").unwrap();

    let entries = scan_game_entries(dir.path(), &extension_set(&["chd"])).unwrap();

    assert_eq!(
        entries.len(),
        1,
        "the loose disc remains visible: {entries:?}"
    );
    assert_eq!(entries[0].display_name(), "Disc 1.chd");
}

#[test]
fn misnamed_or_duplicate_playlists_do_not_describe_a_valid_set() {
    for playlists in [vec!["Wrong.m3u"], vec!["Game.m3u", "Duplicate.m3u"]] {
        let dir = TempDir::new().unwrap();
        let set = dir.path().join("Game.m3u");
        fs::create_dir(&set).unwrap();
        fs::write(set.join("Disc 1.chd"), "one").unwrap();
        fs::write(set.join("Disc 2.chd"), "two").unwrap();
        for playlist in playlists {
            fs::write(set.join(playlist), "Disc 1.chd\nDisc 2.chd\n").unwrap();
        }

        let entries = scan_game_entries(dir.path(), &extension_set(&["chd"])).unwrap();

        assert!(
            entries.is_empty(),
            "ambiguous playlist was accepted: {entries:?}"
        );
    }
}

/// Copying a library onto exFAT or an SMB share leaves macOS `AppleDouble`
/// sidecars beside every file. They carry the shadowed file's extension, so
/// scanning must reject them by name or each real game gains a phantom twin.
#[test]
fn apple_double_sidecars_do_not_scan_as_games() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("Chrono Trigger.sfc"), "").unwrap();
    fs::write(dir.path().join("._Chrono Trigger.sfc"), "").unwrap();
    fs::write(dir.path().join(".DS_Store"), "").unwrap();

    let extensions = extension_set(&["sfc"]);
    let entries = scan_game_entries(dir.path(), &extensions).unwrap();

    assert_eq!(entries.len(), 1, "expected one entry, got {entries:?}");
    assert_eq!(entries[0].display_name(), "Chrono Trigger.sfc");
}

/// A sidecar shadowing a `.cue` must not claim the cue's stem, or the real
/// `.bin` would be deduped away against a file that holds no track data.
#[test]
fn apple_double_sidecar_of_a_cue_does_not_capture_its_stem() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("._Game.cue"), "").unwrap();
    fs::write(dir.path().join("Game.bin"), "").unwrap();

    let extensions = extension_set(&["cue", "bin"]);
    let entries = scan_game_entries(dir.path(), &extensions).unwrap();

    assert_eq!(entries.len(), 1, "expected one entry, got {entries:?}");
    assert_eq!(entries[0].display_name(), "Game.bin");
}
