//! Unit tests for the hash operation: work collection, progress accounting,
//! CUE/BIN hashing edge cases, and complete-disc verification judgment.

use super::*;
use crate::library::EntryStatus;
use retro_junk_lib::scanner::GameEntry;

/// Minimal entry around a `GameEntry` — the other fields are irrelevant here.
fn test_entry(game_entry: GameEntry) -> LibraryEntry {
    static NEXT_ENTRY_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    LibraryEntry {
        id: Some(retro_junk_db::LibraryEntryId(
            NEXT_ENTRY_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        )),
        revision: 0,
        source_revision: 0,
        game_entry,
        identification: None,
        hashes: None,
        disc_verification: Default::default(),
        dat_match: None,
        status: EntryStatus::Unknown,
        ambiguous_candidates: Vec::new(),
        asset_paths: None,
        region_override: None,
        cover_title: String::new(),
        screen_title: String::new(),
        disc_identifications: None,
        broken_references: None,
        cue_compat_issues: None,
        tag: None,
    }
}

#[test]
fn normal_hash_work_reuses_complete_results_but_force_includes_them() {
    let mut cached = test_entry(GameEntry::SingleFile("cached.nes".into()));
    cached.hashes = Some(FileHashes {
        crc32: "12345678".into(),
        sha1: Some("abc".into()),
        md5: None,
        data_size: 3,
        warnings: Vec::new(),
    });
    let missing = test_entry(GameEntry::SingleFile("missing.nes".into()));

    let (normal, _) = collect_hash_work([&cached, &missing].into_iter(), false);
    assert_eq!(normal.len(), 1);
    assert_eq!(normal[0].entry_name, "missing.nes");

    let (forced, _) = collect_hash_work([&cached, &missing].into_iter(), true);
    assert_eq!(forced.len(), 2);
}

fn disc_candidate() -> retro_junk_db::CatalogMediaMatch {
    retro_junk_db::CatalogMediaMatch {
        media: retro_junk_catalog::types::Media {
            id: "game-media".into(),
            release_id: "game-release".into(),
            media_serial: "SLUS-00000".into(),
            disc_number: 0,
            disc_label: String::new(),
            revision: String::new(),
            status: retro_junk_catalog::types::MediaStatus::Verified,
            tag: None,
            dat_name: "Game (USA)".into(),
            rom_name: "Game (USA) (Track 1).bin".into(),
            dat_source: "redump".into(),
            file_size: 100,
            crc32: "11111111".into(),
            sha1: String::new(),
            md5: String::new(),
            created_at: String::new(),
            updated_at: String::new(),
        },
        platform_id: "ps1".into(),
        region: "usa".into(),
        release_revision: String::new(),
        release_title: "Game".into(),
        cover_title: String::new(),
        screen_title: String::new(),
    }
}

fn local_track(number: u8, size: u64, crc32: &str) -> retro_junk_lib::disc_hash::DiscTrackHashes {
    retro_junk_lib::disc_hash::DiscTrackHashes {
        track_number: number,
        is_data: number == 1,
        hashes: FileHashes {
            crc32: crc32.into(),
            sha1: None,
            md5: None,
            data_size: size,
            warnings: Vec::new(),
        },
    }
}

#[test]
fn replacing_a_container_estimate_keeps_the_batch_total_consistent() {
    let initial = 100 + 200;
    let after_cue_expands_to_bin = replace_component_total(initial, 100, 700);
    assert_eq!(after_cue_expands_to_bin, 900);
    assert_eq!(
        replace_component_total(after_cue_expands_to_bin, 200, 150),
        850
    );
}

#[test]
fn cue_hash_progress_reports_referenced_bin_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let cue_path = dir.path().join("Game.cue");
    let bin_path = dir.path().join("Game.bin");
    let bin_size = 128 * 1024;
    std::fs::write(&bin_path, vec![0_u8; bin_size]).unwrap();
    std::fs::write(
        &cue_path,
        "FILE \"Game.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n",
    )
    .unwrap();
    let cue_size = std::fs::metadata(&cue_path).unwrap().len();
    let item = HashWork {
        entry_id: retro_junk_db::LibraryEntryId(1),
        entry_name: "Game".into(),
        path: cue_path,
        file_size: cue_size,
        is_disc: false,
        identification: None,
    };
    let progress = Cell::new((0_u64, 0_u64));

    let hashes = hash_one(
        &item,
        &retro_junk_sony::Ps1Analyzer,
        &|done, total| {
            progress.set((done, total));
        },
        &|_, _, _| {},
        dir.path(),
        true,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();

    assert_eq!(hashes.primary.data_size, bin_size as u64);
    assert_eq!(progress.get(), (bin_size as u64, bin_size as u64));
    assert!(progress.get().1 > cue_size);
}

#[test]
fn malformed_combined_bin_cue_recovers_data_track_but_cannot_verify_disc() {
    let dir = tempfile::tempdir().unwrap();
    let cue_path = dir.path().join("game.cue");
    let bin_path = dir.path().join("game.bin");
    let mut bin = Vec::new();
    for _ in 0..2 {
        let mut sector = vec![0_u8; 2352];
        sector[..12].copy_from_slice(&retro_junk_disc::CD_SYNC_PATTERN);
        sector[24] = 1;
        bin.extend(sector);
    }
    bin.extend(vec![0x55_u8; 3 * 2352]);
    std::fs::write(&bin_path, bin).unwrap();
    std::fs::write(
        &cue_path,
        "FILE \"game.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n  TRACK 02 AUDIO\n    INDEX 01 00:00:00\nFILE \"game.bin\" BINARY\n",
    )
    .unwrap();
    let item = HashWork {
        entry_id: retro_junk_db::LibraryEntryId(1),
        entry_name: "Game".into(),
        path: cue_path,
        file_size: 0,
        is_disc: false,
        identification: None,
    };

    let result = hash_one(
        &item,
        &retro_junk_sony::Ps1Analyzer,
        &|_, _| {},
        &|_, _, _| {},
        dir.path(),
        true,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();

    assert_eq!(result.primary.data_size, 2 * 2352);
    assert_eq!(result.disc_verification, DiscVerification::InvalidLayout);
    assert!(result.cue_tracks.is_none());
    assert!(
        result
            .primary
            .warnings
            .iter()
            .any(|warning| warning.contains("Invalid CUE layout"))
    );
}

#[test]
fn complete_catalog_track_assignment_is_required_for_disc_verification() {
    let candidate = disc_candidate();
    let local = vec![
        local_track(1, 100, "11111111"),
        local_track(2, 50, "22222222"),
    ];
    let tracks = vec![
        retro_junk_db::MediaTrack {
            media_id: candidate.media.id.clone(),
            track_number: 1,
            track_name: "Game (Track 1).bin".into(),
            file_size: 100,
            crc32: "11111111".into(),
            sha1: String::new(),
            md5: String::new(),
        },
        retro_junk_db::MediaTrack {
            media_id: candidate.media.id.clone(),
            track_number: 2,
            track_name: "Game (Track 2).bin".into(),
            file_size: 50,
            crc32: "22222222".into(),
            sha1: String::new(),
            md5: String::new(),
        },
    ];
    let tracks_by_media = HashMap::from([(candidate.media.id.clone(), tracks)]);

    assert_eq!(
        fully_matching_disc_media_ids(&local, std::slice::from_ref(&candidate), &tracks_by_media),
        HashSet::from([candidate.media.id.clone()])
    );

    let missing_audio = &local[..1];
    assert!(
        fully_matching_disc_media_ids(
            missing_audio,
            std::slice::from_ref(&candidate),
            &tracks_by_media
        )
        .is_empty()
    );
    assert!(
        describe_incomplete_disc(
            missing_audio,
            std::slice::from_ref(&candidate),
            &tracks_by_media
        )
        .iter()
        .any(|warning| warning.contains("Track 2 is missing"))
    );
}
