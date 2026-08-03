use std::sync::atomic::AtomicBool;

use super::*;

#[test]
fn dump_ingest_adopts_existing_playable_artwork_once() {
    let temp = tempfile::tempdir().unwrap();
    let archive_root = temp.path().join("archive");
    let root_manifest = retro_junk_archive::ArchiveRootManifest::new("Artwork");
    retro_junk_archive::initialize_archive(&archive_root, &root_manifest).unwrap();
    let playable_root = temp.path().join("roms");
    let rom = playable_root.join("nes/game.nes");
    std::fs::create_dir_all(rom.parent().unwrap()).unwrap();
    std::fs::write(&rom, b"rom").unwrap();
    retro_junk_archive::ingest_new_carrier_dump(
        &archive_root,
        &rom,
        retro_junk_archive::NewCarrierDump {
            platform_id: "nes".to_owned(),
            title: "Game".to_owned(),
            region: "usa".to_owned(),
            revision: String::new(),
            variant: String::new(),
            owner_id: "default".to_owned(),
            physical_copy_label: String::new(),
            serial: String::new(),
            sequence_number: 0,
            carrier_label: String::new(),
            carrier_kind: retro_junk_archive::CarrierKind::Cartridge,
            format: retro_junk_archive::RepresentationFormat::Rom,
            catalog_binding: retro_junk_archive::CatalogBinding {
                catalog_release_id: "release-game".to_owned(),
                catalog_media_id: "media-game".to_owned(),
                ..Default::default()
            },
            source_package: retro_junk_archive::SourcePackageRecord::default(),
            expected_files: Vec::new(),
            physical_copy_id: None,
        },
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();

    let media_root = temp.path().join("media");
    let cover = media_root.join("nes/covers/game.png");
    std::fs::create_dir_all(cover.parent().unwrap()).unwrap();
    std::fs::write(&cover, b"existing cover").unwrap();
    let connection = retro_junk_db::open_memory().unwrap();
    connection.execute("INSERT INTO platforms(id,display_name,short_name,manufacturer,generation,media_type,release_year,description,core_platform) VALUES('nes','NES','NES','Nintendo',3,'cartridge',1983,'','Nes')", []).unwrap();
    connection
        .execute(
            "INSERT INTO works(id,canonical_name) VALUES('work-game','Game')",
            [],
        )
        .unwrap();
    connection.execute("INSERT INTO releases(id,work_id,platform_id,region,title) VALUES('release-game','work-game','nes','usa','Game')", []).unwrap();
    connection.execute("INSERT INTO media(id,release_id,dat_source) VALUES('media-game','release-game','no-intro')", []).unwrap();
    connection
        .execute(
            "INSERT INTO library_roots(id,root_path) VALUES(1,?1)",
            [playable_root.to_string_lossy().as_ref()],
        )
        .unwrap();
    connection.execute(
        "INSERT INTO library_consoles(id,root_id,platform,folder_name,folder_path,fingerprint_hash,scan_state) VALUES(1,1,'Nes','nes',?1,'fp','ready')",
        [playable_root.join("nes").to_string_lossy().as_ref()],
    ).unwrap();
    let game_entry_json =
        serde_json::to_string(&retro_junk_lib::scanner::GameEntry::SingleFile(rom)).unwrap();
    connection.execute(
        "INSERT INTO library_entries(id,console_id,entry_key,display_name,game_entry_json) VALUES(1,1,'file:game.nes','game.nes',?1)",
        [&game_entry_json],
    ).unwrap();
    connection.execute(
        "INSERT INTO library_entry_media_bindings(library_entry_id,catalog_media_id,match_method) VALUES(1,'media-game','test')",
        [],
    ).unwrap();

    let profile = retro_junk_archive::CollectionProfile {
        profile_id: root_manifest.profile_id,
        display_name: "Artwork".to_owned(),
        archive_root: archive_root.clone(),
        playable_root,
        workspace_root: temp.path().join("workspace"),
        network_mode: true,
        platform_defaults: Vec::new(),
        incoming_roots: Vec::new(),
        watch_backend: retro_junk_archive::WatchBackend::default(),
    };
    let snapshot = retro_junk_archive::scan_archive(&archive_root).unwrap();
    let cancel = AtomicBool::new(false);
    let media_setting = media_root.to_string_lossy();
    let _lock = retro_junk_archive::ArchiveLock::acquire(&archive_root).unwrap();
    assert_eq!(
        adopt_playable_artwork(&connection, &snapshot, &profile, &media_setting, &cancel).unwrap(),
        1
    );
    let rescanned = retro_junk_archive::scan_archive(&archive_root).unwrap();
    assert_eq!(rescanned.releases[0].supporting_files.len(), 1);
    assert_eq!(
        std::fs::read(
            rescanned.releases[0].supporting_files[0]
                .directory
                .join(&rescanned.releases[0].supporting_files[0].manifest.file.path)
        )
        .unwrap(),
        b"existing cover"
    );
    assert_eq!(
        adopt_playable_artwork(&connection, &rescanned, &profile, &media_setting, &cancel).unwrap(),
        0
    );
}
