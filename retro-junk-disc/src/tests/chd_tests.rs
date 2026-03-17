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
