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

/// Scraping spends a metered external quota and adds durable files to the
/// archive, unlike the local idempotent work the other switches cover. It
/// stays opt-in even though the rest of this policy is automation-first.
#[test]
fn scraping_is_the_one_automation_that_is_off_by_default() {
    let policy = AutomationPolicy::default();

    assert!(policy.auto_verify, "local verification stays automatic");
    assert!(policy.auto_build, "local builds stay automatic");
    assert!(!policy.auto_scrape);
    assert!(policy.scrape_only_when_unambiguous);
}

/// The expected artwork set round-trips through the settings file. It is
/// written as directory slugs and read back as types; if those two
/// vocabularies disagree, every release reads as missing artwork it holds.
#[test]
fn the_expected_artwork_set_survives_a_save_and_load() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.toml");
    let policy = AutomationPolicy {
        scrape_asset_types: retro_junk_frontend::AssetSelection {
            types: vec![
                retro_junk_frontend::AssetType::Cover3D,
                retro_junk_frontend::AssetType::PhysicalMedia,
            ],
        }
        .names(),
        ..AutomationPolicy::default()
    };
    policy.save_to(&path).unwrap();

    let loaded = AutomationPolicy::load_from(&path);

    assert_eq!(
        loaded.scrape_selection().types,
        vec![
            retro_junk_frontend::AssetType::Cover3D,
            retro_junk_frontend::AssetType::PhysicalMedia,
        ]
    );
}

/// An unreadable or empty list must not mean "expect nothing" — that would
/// silently report every release as fully scraped and derive no work at all.
#[test]
fn an_unusable_artwork_list_falls_back_to_the_default_set() {
    let policy = AutomationPolicy {
        scrape_asset_types: vec!["not-an-asset-type".to_owned()],
        ..AutomationPolicy::default()
    };

    assert_eq!(
        policy.scrape_selection().types,
        retro_junk_frontend::AssetSelection::default().types
    );
}
