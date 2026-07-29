//! Policy load/save: automation-first defaults, round-tripping, and the
//! surgical write that must not clobber unrelated settings.

use super::*;

#[test]
fn defaults_are_automation_first() {
    let policy = AutomationPolicy::default();
    assert!(policy.auto_verify);
    assert!(policy.auto_build);
    assert_eq!(policy.auto_import, AutoImportMode::Suggest);
    assert_eq!(policy.auto_bind_min_confidence, BindConfidence::ExactHash);
    assert!(!policy.verify_published_bytes);
    assert_eq!(policy.deep_rescan_hours, 24);
}

#[test]
fn missing_or_garbage_files_load_as_defaults() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.toml");
    assert_eq!(
        AutomationPolicy::load_from(&path),
        AutomationPolicy::default()
    );
    std::fs::write(&path, "not toml [[[").unwrap();
    assert_eq!(
        AutomationPolicy::load_from(&path),
        AutomationPolicy::default()
    );
    // A present table with one recognized field keeps defaults for the rest.
    std::fs::write(&path, "[automation]\nauto_build = false\n").unwrap();
    let policy = AutomationPolicy::load_from(&path);
    assert!(!policy.auto_build);
    assert!(policy.auto_verify);
}

#[test]
fn save_round_trips_and_preserves_unrelated_settings() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.toml");
    std::fs::write(
        &path,
        "[library]\ncurrent_root = \"/roms\"\n\n[general]\nchdman_path = \"/usr/bin/chdman\"\n",
    )
    .unwrap();
    let policy = AutomationPolicy {
        auto_build: false,
        auto_import: AutoImportMode::On,
        auto_bind_min_confidence: BindConfidence::ExactSerial,
        deep_rescan_hours: 0,
        ..AutomationPolicy::default()
    };
    policy.save_to(&path).unwrap();
    assert_eq!(AutomationPolicy::load_from(&path), policy);
    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(contents.contains("current_root = \"/roms\""));
    assert!(contents.contains("chdman_path = \"/usr/bin/chdman\""));
}
