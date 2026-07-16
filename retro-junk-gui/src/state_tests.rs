//! Unit tests for state.rs: entry lookups, the CHD-compression completion
//! handler (D4), and the multi-disc file refresh (D5).

use std::collections::HashSet;

use retro_junk_lib::scanner::GameEntry;

use super::*;
use crate::test_support::{test_console, test_entry};

// -- find_entry_by_file_mut (D4) --

#[test]
fn find_entry_by_file_mut_matches_single_file_entry() {
    let path = PathBuf::from("/roms/psx/Game.cue");
    let mut console = test_console("psx", vec![test_entry(GameEntry::SingleFile(path.clone()))]);

    let found = console.find_entry_by_file_mut(&path);
    assert!(found.is_some());
}

#[test]
fn find_entry_by_file_mut_matches_any_file_of_multidisc_entry() {
    let disc1 = PathBuf::from("/roms/psx/Game (Disc 1).chd");
    let disc2 = PathBuf::from("/roms/psx/Game (Disc 2).chd");
    let mut console = test_console(
        "psx",
        vec![test_entry(GameEntry::MultiDisc {
            name: "Game.m3u".to_string(),
            files: vec![disc1.clone(), disc2.clone()],
        })],
    );

    // Matching the second disc (not just the first) must still find the entry.
    assert!(console.find_entry_by_file_mut(&disc2).is_some());
    assert!(console.find_entry_by_file_mut(&disc1).is_some());
}

#[test]
fn find_entry_by_file_mut_returns_none_for_unknown_file() {
    let mut console = test_console(
        "psx",
        vec![test_entry(GameEntry::SingleFile(PathBuf::from(
            "/roms/psx/Game.cue",
        )))],
    );
    assert!(
        console
            .find_entry_by_file_mut(Path::new("/roms/psx/Other.cue"))
            .is_none()
    );
}

// -- refresh_multidisc_files (D5) --

#[test]
fn refresh_multidisc_files_remaps_via_playlist_with_claim_tracking() {
    let dir = tempfile::TempDir::new().unwrap();
    let folder = dir.path();

    // Two discs, remapped by the playlist to new .chd names.
    std::fs::write(folder.join("D1.chd"), "").unwrap();
    std::fs::write(folder.join("D2.chd"), "").unwrap();
    // Stray leftovers from a failed delete — must not appear in `files`.
    std::fs::write(folder.join("D1.sbi"), "").unwrap();
    std::fs::write(folder.join("D1.bin"), "").unwrap();
    std::fs::write(folder.join("Game.m3u"), "D1.chd\nD2.chd\n").unwrap();

    let mut entry = test_entry(GameEntry::MultiDisc {
        name: "Game.m3u".to_string(),
        files: vec![folder.join("D1.bin"), folder.join("D2.bin")],
    });
    entry.disc_identifications = Some(vec![
        DiscIdentification {
            path: folder.join("D1.bin"),
            identification: RomIdentification::new(),
            hashes: None,
            dat_match: None,
        },
        DiscIdentification {
            path: folder.join("D2.bin"),
            identification: RomIdentification::new(),
            hashes: None,
            dat_match: None,
        },
    ]);

    let extensions: HashSet<String> = ["chd", "bin", "sbi"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    refresh_multidisc_files(&mut entry, folder, &extensions);

    let GameEntry::MultiDisc { files, .. } = &entry.game_entry else {
        panic!("expected MultiDisc");
    };
    // Only the two chds — the .sbi/.bin stragglers are excluded because the
    // playlist (not the fallback scan) drives collection.
    assert_eq!(files.len(), 2);
    assert!(files.iter().all(|p| p.extension().unwrap() == "chd"));

    let discs = entry.disc_identifications.as_ref().unwrap();
    assert_eq!(discs[0].path, folder.join("D1.chd"));
    assert_eq!(discs[1].path, folder.join("D2.chd"));
}

#[test]
fn refresh_multidisc_files_does_not_wipe_entry_on_empty_result() {
    let dir = tempfile::TempDir::new().unwrap();
    let folder = dir.path(); // empty directory, no matching files

    let original_files = vec![folder.join("D1.chd"), folder.join("D2.chd")];
    let mut entry = test_entry(GameEntry::MultiDisc {
        name: "Game.m3u".to_string(),
        files: original_files.clone(),
    });

    let extensions: HashSet<String> = ["chd"].iter().map(|s| s.to_string()).collect();
    refresh_multidisc_files(&mut entry, folder, &extensions);

    let GameEntry::MultiDisc { files, .. } = &entry.game_entry else {
        panic!("expected MultiDisc");
    };
    assert_eq!(
        files, &original_files,
        "an empty collection result must not wipe the entry's file list"
    );
}

#[test]
fn refresh_multidisc_files_claim_tracking_avoids_double_assignment() {
    let dir = tempfile::TempDir::new().unwrap();
    let folder = dir.path();

    // Only one new file, but two discs whose old stems don't match anything
    // in the new set — neither should both grab the same file.
    std::fs::write(folder.join("Only.chd"), "").unwrap();

    let mut entry = test_entry(GameEntry::MultiDisc {
        name: "Game.m3u".to_string(),
        files: vec![folder.join("Old1.bin"), folder.join("Old2.bin")],
    });
    entry.disc_identifications = Some(vec![
        DiscIdentification {
            path: folder.join("Old1.bin"),
            identification: RomIdentification::new(),
            hashes: None,
            dat_match: None,
        },
        DiscIdentification {
            path: folder.join("Old2.bin"),
            identification: RomIdentification::new(),
            hashes: None,
            dat_match: None,
        },
    ]);

    let extensions: HashSet<String> = ["chd"].iter().map(|s| s.to_string()).collect();
    refresh_multidisc_files(&mut entry, folder, &extensions);

    let discs = entry.disc_identifications.as_ref().unwrap();
    // Neither stem ("Old1"/"Old2") matches "Only", so both keep their stale
    // path rather than both collapsing onto "Only.chd".
    assert_eq!(discs[0].path, folder.join("Old1.bin"));
    assert_eq!(discs[1].path, folder.join("Old2.bin"));
}

// -- ChdCompressPromptReady handler (D1) --

#[test]
fn chd_compress_prompt_ready_stores_the_prompt() {
    let mut app = crate::app::RetroJunkApp::with_parts(
        &dummy_ctx(),
        crate::settings::AppSettings::default(),
        None,
        None,
    );
    assert!(app.chd_compress_prompt.is_none());

    let prompt = ChdCompressPrompt {
        folder_name: "psx".to_string(),
        items: Vec::new(),
        skipped: Vec::new(),
        chdman: Err(retro_junk_lib::chd_convert::ChdmanUnavailable {
            reason: "not found".to_string(),
        }),
        delete_sources: false,
    };

    handle_message(
        &mut app,
        AppMessage::ChdCompressPromptReady { prompt },
        &dummy_ctx(),
    );

    let stored = app
        .chd_compress_prompt
        .as_ref()
        .expect("prompt must be stored");
    assert_eq!(stored.folder_name, "psx");
}

// -- ChdCompressComplete handler (D4) --

fn dummy_ctx() -> egui::Context {
    egui::Context::default()
}

#[test]
fn chd_compress_complete_updates_single_file_entry_and_invalidates_checks() {
    let mut app = crate::app::RetroJunkApp::with_parts(
        &dummy_ctx(),
        crate::settings::AppSettings::default(),
        None,
        None,
    );
    let input = PathBuf::from("/roms/psx/Game.cue");
    let output = PathBuf::from("/roms/psx/Game.chd");
    let mut entry = test_entry(GameEntry::SingleFile(input.clone()));
    entry.hashes = Some(retro_junk_dat::FileHashes {
        crc32: "deadbeef".to_string(),
        md5: None,
        sha1: None,
        data_size: 1,
        warnings: Vec::new(),
    });
    entry.broken_references = Some(Vec::new());
    entry.cue_compat_issues = Some(Vec::new());
    app.library.consoles.push(test_console("psx", vec![entry]));

    let job = retro_junk_lib::chd_convert::CompressionJob {
        input: input.clone(),
        media: retro_junk_core::ChdMedia::Cd,
        output: output.clone(),
        source_files: vec![input.clone()],
        input_bytes: 100,
    };

    let results = vec![ChdCompressResult {
        input_name: "Game.cue".to_string(),
        job,
        outcome: ChdCompressOutcome::Compressed {
            input_bytes: 100,
            output_bytes: 40,
            tracks: 1,
            sources_deleted: true,
            delete_failures: Vec::new(),
        },
    }];

    handle_message(
        &mut app,
        AppMessage::ChdCompressComplete {
            folder_name: "psx".to_string(),
            results,
        },
        &dummy_ctx(),
    );

    let console = &app.library.consoles[0];
    assert_eq!(console.entries.len(), 1);
    let GameEntry::SingleFile(ref path) = console.entries[0].game_entry else {
        panic!("expected SingleFile");
    };
    assert_eq!(
        path, &output,
        "entry path must be updated to the .chd output"
    );
    assert!(
        console.entries[0].hashes.is_none(),
        "stale hashes must be invalidated"
    );
    assert!(console.entries[0].broken_references.is_none());
    assert!(console.entries[0].cue_compat_issues.is_none());
    assert!(console.fingerprint.is_none(), "fingerprint must be cleared");
}

#[test]
fn chd_compress_complete_without_source_deletion_leaves_entry_untouched_but_clears_fingerprint() {
    let mut app = crate::app::RetroJunkApp::with_parts(
        &dummy_ctx(),
        crate::settings::AppSettings::default(),
        None,
        None,
    );
    let input = PathBuf::from("/roms/psx/Game.cue");
    let mut entry = test_entry(GameEntry::SingleFile(input.clone()));
    entry.hashes = Some(retro_junk_dat::FileHashes {
        crc32: "deadbeef".to_string(),
        md5: None,
        sha1: None,
        data_size: 1,
        warnings: Vec::new(),
    });
    let mut console = test_console("psx", vec![entry]);
    console.fingerprint = Some(crate::cache::FolderFingerprint {
        name_hash: "stale".to_string(),
    });
    app.library.consoles.push(console);

    let job = retro_junk_lib::chd_convert::CompressionJob {
        input: input.clone(),
        media: retro_junk_core::ChdMedia::Cd,
        output: PathBuf::from("/roms/psx/Game.chd"),
        source_files: vec![input.clone()],
        input_bytes: 100,
    };

    let results = vec![ChdCompressResult {
        input_name: "Game.cue".to_string(),
        job,
        outcome: ChdCompressOutcome::Compressed {
            input_bytes: 100,
            output_bytes: 40,
            tracks: 1,
            sources_deleted: false,
            delete_failures: Vec::new(),
        },
    }];

    handle_message(
        &mut app,
        AppMessage::ChdCompressComplete {
            folder_name: "psx".to_string(),
            results,
        },
        &dummy_ctx(),
    );

    let console = &app.library.consoles[0];
    // Entry untouched: still SingleFile at the original cue path, hashes kept.
    let GameEntry::SingleFile(ref path) = console.entries[0].game_entry else {
        panic!("expected SingleFile");
    };
    assert_eq!(path, &input);
    assert!(console.entries[0].hashes.is_some());
    // But the fingerprint was cleared so the next scan reconciles the new
    // sibling .chd (the `changed == true` path was taken).
    assert!(console.fingerprint.is_none());
}

#[test]
fn chd_compress_complete_drops_ghost_entries_whose_files_were_all_deleted() {
    let mut app = crate::app::RetroJunkApp::with_parts(
        &dummy_ctx(),
        crate::settings::AppSettings::default(),
        None,
        None,
    );
    let cue = PathBuf::from("/roms/psx/Game.cue");
    let track_bin = PathBuf::from("/roms/psx/Game (Track 2).bin");
    let output = PathBuf::from("/roms/psx/Game.chd");

    // The scanner's stem-only dedup can leave a lone SingleFile "ghost"
    // entry for a track bin alongside the real cue entry.
    let cue_entry = test_entry(GameEntry::SingleFile(cue.clone()));
    let ghost_entry = test_entry(GameEntry::SingleFile(track_bin.clone()));
    app.library
        .consoles
        .push(test_console("psx", vec![cue_entry, ghost_entry]));

    let job = retro_junk_lib::chd_convert::CompressionJob {
        input: cue.clone(),
        media: retro_junk_core::ChdMedia::Cd,
        output: output.clone(),
        source_files: vec![cue.clone(), track_bin.clone()],
        input_bytes: 100,
    };

    let results = vec![ChdCompressResult {
        input_name: "Game.cue".to_string(),
        job,
        outcome: ChdCompressOutcome::Compressed {
            input_bytes: 100,
            output_bytes: 40,
            tracks: 2,
            sources_deleted: true,
            delete_failures: Vec::new(),
        },
    }];

    handle_message(
        &mut app,
        AppMessage::ChdCompressComplete {
            folder_name: "psx".to_string(),
            results,
        },
        &dummy_ctx(),
    );

    let console = &app.library.consoles[0];
    assert_eq!(
        console.entries.len(),
        1,
        "the dangling track-bin ghost entry must be removed"
    );
    let GameEntry::SingleFile(ref path) = console.entries[0].game_entry else {
        panic!("expected SingleFile");
    };
    assert_eq!(path, &output);
}
