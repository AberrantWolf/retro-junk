use std::fs;

use tempfile::TempDir;

use crate::organize::{
    OrganizeEntry, OrganizeJob, collect_cue_companions, execute_organize_job, find_loose_disc_files,
};

#[test]
fn find_loose_disc_files_finds_cue_and_chd() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("SLUS-01235.cue"), "FILE dummy BINARY\n").unwrap();
    fs::write(dir.path().join("SLUS-01235 (Track 1).bin"), "").unwrap();
    fs::write(dir.path().join("SLUS-01236.chd"), "").unwrap();
    fs::write(dir.path().join("readme.txt"), "").unwrap();

    let loose = find_loose_disc_files(dir.path());

    let names: Vec<String> = loose
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    assert!(names.contains(&"SLUS-01235.cue".to_string()));
    assert!(names.contains(&"SLUS-01236.chd".to_string()));
    // BIN and TXT are not entry points
    assert!(!names.contains(&"SLUS-01235 (Track 1).bin".to_string()));
    assert!(!names.contains(&"readme.txt".to_string()));
}

#[test]
fn find_loose_disc_files_skips_m3u_subdirs() {
    let dir = TempDir::new().unwrap();
    // Create an .m3u subdirectory with a CUE inside
    let m3u_dir = dir.path().join("Some Game.m3u");
    fs::create_dir(&m3u_dir).unwrap();
    fs::write(m3u_dir.join("disc.cue"), "FILE dummy BINARY\n").unwrap();

    // A loose file at the top level
    fs::write(dir.path().join("SLUS-99999.cue"), "FILE dummy BINARY\n").unwrap();

    let loose = find_loose_disc_files(dir.path());
    assert_eq!(loose.len(), 1);
    assert!(loose[0].file_name().unwrap().to_string_lossy() == "SLUS-99999.cue");
}

#[test]
fn collect_cue_companions_finds_referenced_bins() {
    let dir = TempDir::new().unwrap();
    let cue_content = r#"FILE "SLUS-01235 (Track 1).bin" BINARY
  TRACK 01 MODE2/2352
    INDEX 01 00:00:00
FILE "SLUS-01235 (Track 2).bin" BINARY
  TRACK 02 AUDIO
    INDEX 01 00:00:00
"#;
    let cue_path = dir.path().join("SLUS-01235.cue");
    fs::write(&cue_path, cue_content).unwrap();
    fs::write(dir.path().join("SLUS-01235 (Track 1).bin"), "").unwrap();
    fs::write(dir.path().join("SLUS-01235 (Track 2).bin"), "").unwrap();

    let companions = collect_cue_companions(&cue_path);
    assert_eq!(companions.len(), 2);
}

#[test]
fn collect_cue_companions_handles_missing_bins() {
    let dir = TempDir::new().unwrap();
    let cue_content = "FILE \"nonexistent.bin\" BINARY\n  TRACK 01 MODE2/2352\n";
    let cue_path = dir.path().join("test.cue");
    fs::write(&cue_path, cue_content).unwrap();

    let companions = collect_cue_companions(&cue_path);
    assert!(companions.is_empty());
}

#[test]
fn execute_organize_job_creates_folder_and_moves_files() {
    let dir = TempDir::new().unwrap();
    let cue_content = "FILE \"disc1.bin\" BINARY\n  TRACK 01 MODE2/2352\n";
    fs::write(dir.path().join("disc1.cue"), cue_content).unwrap();
    fs::write(dir.path().join("disc1.bin"), "bindata").unwrap();
    fs::write(dir.path().join("disc2.chd"), "chddata").unwrap();

    let target_folder = dir.path().join("Final Fantasy VII (USA).m3u");
    let job = OrganizeJob {
        target_folder: target_folder.clone(),
        game_name: "Final Fantasy VII (USA)".to_string(),
        entry_points: vec![
            OrganizeEntry {
                source_path: dir.path().join("disc1.cue"),
                dat_game_name: "Final Fantasy VII (USA) (Disc 1)".to_string(),
                target_filename: "disc1.cue".to_string(),
                disc_number: Some(1),
            },
            OrganizeEntry {
                source_path: dir.path().join("disc2.chd"),
                dat_game_name: "Final Fantasy VII (USA) (Disc 2)".to_string(),
                target_filename: "disc2.chd".to_string(),
                disc_number: Some(2),
            },
        ],
        companion_files: vec![dir.path().join("disc1.bin")],
    };

    let result = execute_organize_job(&job);

    assert!(result.folder_created);
    assert_eq!(result.files_moved, 3); // 2 entry points + 1 companion
    assert!(result.playlist_written);
    assert!(result.errors.is_empty());

    // Verify folder structure
    assert!(target_folder.exists());
    assert!(target_folder.join("disc1.cue").exists());
    assert!(target_folder.join("disc1.bin").exists());
    assert!(target_folder.join("disc2.chd").exists());

    // Verify playlist contents
    let playlist = fs::read_to_string(target_folder.join("Final Fantasy VII (USA).m3u")).unwrap();
    assert_eq!(playlist, "disc1.cue\ndisc2.chd\n");

    // Verify originals are gone
    assert!(!dir.path().join("disc1.cue").exists());
    assert!(!dir.path().join("disc1.bin").exists());
    assert!(!dir.path().join("disc2.chd").exists());
}

#[test]
fn execute_organize_job_handles_existing_target_folder() {
    let dir = TempDir::new().unwrap();
    let target_folder = dir.path().join("Game.m3u");
    fs::create_dir(&target_folder).unwrap(); // Already exists

    let job = OrganizeJob {
        target_folder: target_folder.clone(),
        game_name: "Game".to_string(),
        entry_points: vec![],
        companion_files: vec![],
    };

    let result = execute_organize_job(&job);
    assert!(!result.errors.is_empty());
}

#[test]
fn mixed_directory_only_flags_loose_files() {
    let dir = TempDir::new().unwrap();

    // .m3u folder (already organized)
    let m3u = dir.path().join("Organized Game.m3u");
    fs::create_dir(&m3u).unwrap();
    fs::write(m3u.join("disc.cue"), "").unwrap();

    // Loose disc files
    fs::write(dir.path().join("SCUS-94163.cue"), "FILE dummy BINARY\n").unwrap();
    fs::write(dir.path().join("SCUS-94164.iso"), "").unwrap();

    // Non-disc file
    fs::write(dir.path().join("notes.txt"), "").unwrap();

    let loose = find_loose_disc_files(dir.path());
    assert_eq!(loose.len(), 2);

    let names: Vec<String> = loose
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    assert!(names.contains(&"SCUS-94163.cue".to_string()));
    assert!(names.contains(&"SCUS-94164.iso".to_string()));
}
