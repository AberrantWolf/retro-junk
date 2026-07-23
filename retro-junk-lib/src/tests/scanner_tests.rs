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
