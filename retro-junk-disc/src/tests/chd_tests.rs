use crate::chd::*;

#[test]
fn test_parse_meta_field_basic() {
    let text = "TRACK:1 TYPE:MODE2_RAW SUBTYPE:NONE FRAMES:229020 PREFRAMES:150";
    assert_eq!(parse_meta_field(text, "TRACK"), Some("1"));
    assert_eq!(parse_meta_field(text, "TYPE"), Some("MODE2_RAW"));
    assert_eq!(parse_meta_field(text, "FRAMES"), Some("229020"));
    assert_eq!(parse_meta_field(text, "PREFRAMES"), Some("150"));
    assert_eq!(parse_meta_field(text, "SUBTYPE"), Some("NONE"));
}

#[test]
fn test_parse_meta_field_missing() {
    let text = "TRACK:1 TYPE:AUDIO SUBTYPE:NONE FRAMES:18995";
    assert_eq!(parse_meta_field(text, "POSTGAP"), None);
    assert_eq!(parse_meta_field(text, "PREGAP"), None);
}

#[test]
fn test_parse_meta_field_audio_track() {
    let text = "TRACK:2 TYPE:AUDIO SUBTYPE:NONE FRAMES:18995 PREFRAMES:150";
    assert_eq!(parse_meta_field(text, "TRACK"), Some("2"));
    assert_eq!(parse_meta_field(text, "TYPE"), Some("AUDIO"));
    assert_eq!(parse_meta_field(text, "FRAMES"), Some("18995"));
}

#[test]
fn test_chd_track_info_is_data() {
    let mode1 = ChdTrackInfo {
        track_number: 1,
        track_type: "MODE1_RAW".to_string(),
        frames: 19560,
        start_sector: 0,
    };
    let mode2 = ChdTrackInfo {
        track_number: 2,
        track_type: "MODE2_RAW".to_string(),
        frames: 78407,
        start_sector: 19560,
    };
    let audio = ChdTrackInfo {
        track_number: 3,
        track_type: "AUDIO".to_string(),
        frames: 906,
        start_sector: 97967,
    };
    assert!(mode1.is_data());
    assert!(mode2.is_data());
    assert!(!audio.is_data());
}

#[test]
fn test_select_largest_data_track_multi_data() {
    // Saturn-style: MODE1 boot + MODE2 main data
    let tracks = vec![
        ChdTrackInfo {
            track_number: 1,
            track_type: "MODE1_RAW".to_string(),
            frames: 19560,
            start_sector: 0,
        },
        ChdTrackInfo {
            track_number: 2,
            track_type: "MODE2_RAW".to_string(),
            frames: 78407,
            start_sector: 19560,
        },
        ChdTrackInfo {
            track_number: 3,
            track_type: "AUDIO".to_string(),
            frames: 906,
            start_sector: 97967,
        },
    ];
    let selected = select_largest_data_track(&tracks).unwrap();
    assert_eq!(selected.track_number, 2);
    assert_eq!(selected.frames, 78407);
    assert_eq!(selected.start_sector, 19560);
}

#[test]
fn test_select_largest_data_track_single_data() {
    // PS1-style: single MODE2 data track + audio
    let tracks = vec![
        ChdTrackInfo {
            track_number: 1,
            track_type: "MODE2_RAW".to_string(),
            frames: 229020,
            start_sector: 0,
        },
        ChdTrackInfo {
            track_number: 2,
            track_type: "AUDIO".to_string(),
            frames: 18995,
            start_sector: 229020,
        },
    ];
    let selected = select_largest_data_track(&tracks).unwrap();
    assert_eq!(selected.track_number, 1);
    assert_eq!(selected.frames, 229020);
    assert_eq!(selected.start_sector, 0);
}

#[test]
fn test_select_largest_data_track_no_data() {
    let tracks = vec![ChdTrackInfo {
        track_number: 1,
        track_type: "AUDIO".to_string(),
        frames: 5000,
        start_sector: 0,
    }];
    assert!(select_largest_data_track(&tracks).is_none());
}

#[test]
fn test_select_largest_data_track_empty() {
    let tracks: Vec<ChdTrackInfo> = vec![];
    assert!(select_largest_data_track(&tracks).is_none());
}
