use super::*;
use retro_junk_core::{ChdExtensionRole, ChdMedia};

// -- CHD extension table --

#[test]
fn test_chd_media_for_extension_cue() {
    let analyzer = SegaCdAnalyzer;
    assert_eq!(analyzer.chd_media_for_extension("cue"), Some(ChdMedia::Cd));
    assert_eq!(analyzer.chd_media_for_extension("iso"), None);
    assert_eq!(analyzer.chd_media_for_extension("bin"), None);
}

#[test]
fn test_chd_extensions_declares_a_source_role() {
    // console_supports_chd-equivalent logic: Sega CD is disc-based, so at
    // least one declared extension must be a CHD Source.
    let analyzer = SegaCdAnalyzer;
    assert!(
        analyzer
            .chd_extensions()
            .iter()
            .any(|(_, role)| matches!(role, ChdExtensionRole::Source(_)))
    );
}

// -- Metadata --

#[test]
fn test_platform() {
    let analyzer = SegaCdAnalyzer;
    assert_eq!(analyzer.platform(), Platform::SegaCd);
}

#[test]
fn test_file_extensions() {
    let analyzer = SegaCdAnalyzer;
    assert!(analyzer.file_extensions().contains(&"cue"));
    assert!(analyzer.file_extensions().contains(&"chd"));
}
