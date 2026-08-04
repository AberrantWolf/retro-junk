use std::path::Path;

use crate::profiles::resolve_profile_from;

/// Initialize a collection at `<root>/archive` and return its identity.
fn init_collection(root: &Path, name: &str) -> retro_junk_archive::ArchiveProfileId {
    std::fs::create_dir_all(root.join("roms")).unwrap();
    let manifest = retro_junk_archive::ArchiveRootManifest::new(name);
    retro_junk_archive::initialize_archive(&root.join("archive"), &manifest).unwrap();
    manifest.profile_id
}

fn write_settings(path: &Path, current: &str, profile_id: &str, collection: &Path) {
    std::fs::write(
        path,
        format!(
            "[library]\n\
             current_profile = \"{current}\"\n\
             \n\
             [[library.profiles]]\n\
             profile_id = \"{profile_id}\"\n\
             display_name = \"Collection\"\n\
             archive_root = \"{archive}\"\n\
             playable_root = \"{roms}\"\n\
             workspace_root = \"{work}\"\n",
            archive = collection.join("archive").display(),
            roms = collection.join("roms").display(),
            work = collection.join("work").display(),
        ),
    )
    .unwrap();
}

/// A profile written against a copy of an existing archive can carry an id the
/// archive never knew. The projection is keyed on the archive's own id, so the
/// CLI and daemon have to re-adopt it rather than wait for the GUI to rewrite
/// settings — otherwise every archived release is invisible to them too.
#[test]
fn a_resolved_profile_adopts_the_identity_of_the_archive_it_points_at() {
    let temp = tempfile::tempdir().unwrap();
    let archived = init_collection(temp.path(), "Collection");
    let stale = retro_junk_archive::ArchiveProfileId::new();
    let settings = temp.path().join("settings.toml");
    write_settings(
        &settings,
        &stale.to_string(),
        &stale.to_string(),
        temp.path(),
    );

    let resolved = resolve_profile_from(&settings, None).expect("profile resolves");

    assert_eq!(resolved.profile_id, archived);
    assert_eq!(resolved.archive_root, temp.path().join("archive"));
}

/// Selecting by name still works, and the selected profile is rekeyed too.
#[test]
fn selection_by_display_name_also_adopts_archive_identity() {
    let temp = tempfile::tempdir().unwrap();
    let archived = init_collection(temp.path(), "Collection");
    let stale = retro_junk_archive::ArchiveProfileId::new();
    let settings = temp.path().join("settings.toml");
    write_settings(
        &settings,
        &stale.to_string(),
        &stale.to_string(),
        temp.path(),
    );

    let resolved = resolve_profile_from(&settings, Some("collection")).expect("profile resolves");

    assert_eq!(resolved.profile_id, archived);
}

/// A `current_profile` naming no configured profile fell back to the sole
/// entry before adoption existed; that fallback must survive.
#[test]
fn a_dangling_current_profile_still_falls_back_to_the_only_profile() {
    let temp = tempfile::tempdir().unwrap();
    let archived = init_collection(temp.path(), "Collection");
    let settings = temp.path().join("settings.toml");
    write_settings(
        &settings,
        &retro_junk_archive::ArchiveProfileId::new().to_string(),
        &retro_junk_archive::ArchiveProfileId::new().to_string(),
        temp.path(),
    );

    let resolved = resolve_profile_from(&settings, None).expect("profile resolves");

    assert_eq!(resolved.profile_id, archived);
}

#[test]
fn an_unknown_selector_resolves_to_nothing() {
    let temp = tempfile::tempdir().unwrap();
    init_collection(temp.path(), "Collection");
    let id = retro_junk_archive::ArchiveProfileId::new();
    let settings = temp.path().join("settings.toml");
    write_settings(&settings, &id.to_string(), &id.to_string(), temp.path());

    assert!(resolve_profile_from(&settings, Some("no-such-collection")).is_none());
}

/// The gamelist a frontend reads and the gamelist a sync writes have to be the
/// same file.
///
/// The CLI built its roots from empty strings and then substituted its own
/// sibling `-metadata` default, so with `metadata_dir = "."` it maintained
/// `roms-metadata/psx/gamelist.xml` while ES-DE read `roms/psx/gamelist.xml`.
/// Both existed, both parsed, and every projection updated the one nobody
/// read — so a renamed game kept its old, dangling entry in the frontend.
#[test]
fn the_users_metadata_setting_decides_where_gamelists_go() {
    let temp = tempfile::tempdir().unwrap();
    let settings = temp.path().join("settings.toml");
    let roms = temp.path().join("roms");
    std::fs::write(&settings, "[general]\nmetadata_dir = \".\"\n").unwrap();

    let roots = crate::profiles::frontend_roots_from(&settings, &roms, None, None);
    assert_eq!(
        roots.metadata_root, roms,
        "the sync wrote gamelists somewhere the frontend does not read"
    );
    // An unset assets directory still means the sibling media convention.
    assert_eq!(roots.media_root, temp.path().join("roms-media"));
}

/// An explicit `--metadata-root` still wins; silence is what must fall back to
/// the user's settings rather than to a second default.
#[test]
fn an_explicit_root_overrides_the_setting_but_silence_does_not() {
    let temp = tempfile::tempdir().unwrap();
    let settings = temp.path().join("settings.toml");
    let roms = temp.path().join("roms");
    std::fs::write(&settings, "[general]\nmetadata_dir = \".\"\n").unwrap();

    let chosen = temp.path().join("elsewhere");
    let roots = crate::profiles::frontend_roots_from(&settings, &roms, None, Some(chosen.clone()));
    assert_eq!(roots.metadata_root, chosen);
}
