use super::*;
use crate::disc_set::TrackRename;
use tempfile::TempDir;

const GAME: &str = "Cool Game (USA)";

/// Build a DiscSetPlan by hand for executor tests (planning logic is
/// covered by disc_set_tests).
fn make_plan(dir: &Path) -> DiscSetPlan {
    DiscSetPlan {
        cue: dir.join("dump.cue"),
        cue_target_filename: format!("{GAME}.cue"),
        game_name: GAME.to_string(),
        matched_by: MatchMethod::Serial,
        tracks: vec![
            TrackRename {
                source: dir.join("dump (Track 1).bin"),
                target_filename: format!("{GAME} (Track 1).bin"),
            },
            TrackRename {
                source: dir.join("dump (Track 2).bin"),
                target_filename: format!("{GAME} (Track 2).bin"),
            },
        ],
        new_cue_content: Some(format!(
            "FILE \"{GAME} (Track 1).bin\" BINARY\nFILE \"{GAME} (Track 2).bin\" BINARY\n"
        )),
        cue_verified: Some(true),
    }
}

fn write_dump(dir: &Path) {
    std::fs::write(dir.join("dump (Track 1).bin"), "one").unwrap();
    std::fs::write(dir.join("dump (Track 2).bin"), "two").unwrap();
    std::fs::write(
        dir.join("dump.cue"),
        "FILE \"dump (Track 1).bin\" BINARY\nFILE \"dump (Track 2).bin\" BINARY\n",
    )
    .unwrap();
}

#[test]
fn disc_set_renames_all_files_and_rewrites_cue() {
    let tmp = TempDir::new().unwrap();
    write_dump(tmp.path());
    let plan = make_plan(tmp.path());

    let result = execute_disc_set(&plan, &ExecutionContext::default());
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(result.files_renamed, 3);
    assert!(result.cue_rewritten);

    assert!(tmp.path().join(format!("{GAME} (Track 1).bin")).is_file());
    assert!(tmp.path().join(format!("{GAME} (Track 2).bin")).is_file());
    let cue_content = std::fs::read_to_string(tmp.path().join(format!("{GAME}.cue"))).unwrap();
    assert!(cue_content.contains(&format!("FILE \"{GAME} (Track 1).bin\" BINARY")));
    assert!(!tmp.path().join("dump.cue").exists());
    assert!(!tmp.path().join("dump (Track 1).bin").exists());
}

#[test]
fn disc_set_moves_media_and_rewrites_gamelist_in_same_transaction() {
    let tmp = TempDir::new().unwrap();
    let roms = tmp.path().join("roms");
    let media = tmp.path().join("media");
    std::fs::create_dir_all(&roms).unwrap();
    std::fs::create_dir_all(media.join("covers")).unwrap();
    write_dump(&roms);
    std::fs::write(media.join("covers").join("dump.png"), "img").unwrap();
    let gamelist = tmp.path().join("gamelist.xml");
    std::fs::write(&gamelist, "<path>./dump.cue</path>\n").unwrap();

    let plan = make_plan(&roms);
    let gamelist_for_closure = gamelist.clone();
    let rewriter = move |stem_map: &HashMap<String, String>| -> Vec<(PathBuf, String)> {
        // Minimal stand-in for the frontend's gamelist rewriter.
        match stem_map.get("dump") {
            Some(new_stem) => vec![(
                gamelist_for_closure.clone(),
                format!("<path>./{new_stem}.cue</path>\n"),
            )],
            None => Vec::new(),
        }
    };
    let exec = ExecutionContext {
        media_dir: Some(media.clone()),
        gamelist_rewriter: Some(&rewriter),
    };

    let result = execute_disc_set(&plan, &exec);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(result.media_renamed, 1);
    assert_eq!(result.gamelists_updated, 1);
    assert!(media.join("covers").join(format!("{GAME}.png")).is_file());
    assert!(!media.join("covers").join("dump.png").exists());
    assert_eq!(
        std::fs::read_to_string(&gamelist).unwrap(),
        format!("<path>./{GAME}.cue</path>\n")
    );
}

#[test]
fn disc_set_with_occupied_target_changes_nothing() {
    let tmp = TempDir::new().unwrap();
    write_dump(tmp.path());
    // Occupy one of the targets
    std::fs::write(tmp.path().join(format!("{GAME} (Track 2).bin")), "squatter").unwrap();

    let plan = make_plan(tmp.path());
    let result = execute_disc_set(&plan, &ExecutionContext::default());
    assert!(!result.errors.is_empty());
    assert_eq!(result.files_renamed, 0);

    // Everything untouched
    assert!(tmp.path().join("dump.cue").is_file());
    assert!(tmp.path().join("dump (Track 1).bin").is_file());
    assert!(tmp.path().join("dump (Track 2).bin").is_file());
    assert_eq!(
        std::fs::read_to_string(tmp.path().join(format!("{GAME} (Track 2).bin"))).unwrap(),
        "squatter"
    );
    let cue_content = std::fs::read_to_string(tmp.path().join("dump.cue")).unwrap();
    assert!(
        cue_content.contains("dump (Track 1).bin"),
        "cue not rewritten"
    );
}

#[test]
fn single_rename_moves_media_transactionally() {
    let tmp = TempDir::new().unwrap();
    let roms = tmp.path().join("roms");
    let media = tmp.path().join("media");
    std::fs::create_dir_all(&roms).unwrap();
    std::fs::create_dir_all(media.join("screenshots")).unwrap();
    let source = roms.join("old.chd");
    std::fs::write(&source, "chd").unwrap();
    std::fs::write(media.join("screenshots").join("old.png"), "img").unwrap();

    let exec = ExecutionContext {
        media_dir: Some(media.clone()),
        gamelist_rewriter: None,
    };
    let target = roms.join("New Name (USA).chd");
    let result = execute_single_rename(&source, &target, &exec).unwrap();
    assert_eq!(result.media_renamed, 1);
    assert!(target.is_file());
    assert!(
        media
            .join("screenshots")
            .join("New Name (USA).png")
            .is_file()
    );
}
