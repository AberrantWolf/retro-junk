use super::*;
use retro_junk_core::{ChdExtensionRole, ChdMedia};

// -- CHD extension table --

#[test]
fn test_chd_media_for_extension_gdi() {
    let analyzer = DreamcastAnalyzer;
    assert_eq!(analyzer.chd_media_for_extension("gdi"), Some(ChdMedia::Cd));
    assert_eq!(analyzer.chd_media_for_extension("cdi"), None);
    assert_eq!(analyzer.chd_media_for_extension("chd"), None);
}

#[test]
fn test_chd_extensions_cdi_is_unconvertible() {
    let analyzer = DreamcastAnalyzer;
    assert_eq!(
        analyzer
            .chd_extensions()
            .iter()
            .find(|(ext, _)| *ext == "cdi")
            .map(|(_, role)| *role),
        Some(ChdExtensionRole::Unconvertible)
    );
}

#[test]
fn test_chd_extensions_declares_a_source_role() {
    // console_supports_chd-equivalent logic: Dreamcast is disc-based, so at
    // least one declared extension must be a CHD Source.
    let analyzer = DreamcastAnalyzer;
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
    let analyzer = DreamcastAnalyzer;
    assert_eq!(analyzer.platform(), Platform::Dreamcast);
}

#[test]
fn test_file_extensions() {
    let analyzer = DreamcastAnalyzer;
    assert!(analyzer.file_extensions().contains(&"gdi"));
    assert!(analyzer.file_extensions().contains(&"cdi"));
    assert!(analyzer.file_extensions().contains(&"chd"));
}
