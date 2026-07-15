use super::*;
use retro_junk_core::{ChdExtensionRole, ChdMedia};

// -- CHD extension table --

#[test]
fn test_chd_media_for_extension_iso() {
    let analyzer = PspAnalyzer;
    assert_eq!(analyzer.chd_media_for_extension("iso"), Some(ChdMedia::Dvd));
    assert_eq!(analyzer.chd_media_for_extension("cso"), None);
    assert_eq!(analyzer.chd_media_for_extension("dax"), None);
}

#[test]
fn test_chd_extensions_cso_and_dax_are_unconvertible() {
    let analyzer = PspAnalyzer;
    let table = analyzer.chd_extensions();
    assert_eq!(
        table.iter().find(|(ext, _)| *ext == "cso").map(|(_, r)| *r),
        Some(ChdExtensionRole::Unconvertible)
    );
    assert_eq!(
        table.iter().find(|(ext, _)| *ext == "dax").map(|(_, r)| *r),
        Some(ChdExtensionRole::Unconvertible)
    );
}

#[test]
fn test_chd_extensions_declares_a_source_role() {
    // console_supports_chd-equivalent logic: PSP UMDs are disc-based, so at
    // least one declared extension must be a CHD Source.
    let analyzer = PspAnalyzer;
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
    let analyzer = PspAnalyzer;
    assert_eq!(analyzer.platform(), Platform::Psp);
}

#[test]
fn test_file_extensions() {
    let analyzer = PspAnalyzer;
    assert!(analyzer.file_extensions().contains(&"iso"));
    assert!(analyzer.file_extensions().contains(&"cso"));
    assert!(analyzer.file_extensions().contains(&"dax"));
}
