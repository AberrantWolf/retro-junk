use std::path::Path;

use retro_junk_db::*;

fn row(path: &str, display: &str) -> LibraryEntryRow {
    LibraryEntryRow {
        display_name: display.into(),
        game_entry_json: format!(r#"{{"SingleFile":"{path}"}}"#),
        status: "unknown".into(),
        tag: String::new(),
        crc32: String::new(),
        sha1: String::new(),
        md5: String::new(),
        data_size: 0,
        dat_game_name: String::new(),
        dat_rom_name: String::new(),
        dat_match_method: String::new(),
        region_override: String::new(),
        cover_title: String::new(),
        screen_title: String::new(),
        identification_json: None,
        disc_identifications_json: None,
        broken_references_json: None,
        ambiguous_candidates_json: None,
        cue_compat_issues_json: None,
    }
}

fn setup(entries: &[LibraryEntryRow]) -> (Connection, LibraryRootId, LibraryConsoleId) {
    let conn = open_memory().unwrap();
    let root = upsert_library_root(&conn, "/roms").unwrap();
    let console = save_console_bulk(
        &conn,
        &ConsoleRecord {
            root_id: root,
            platform: "NES",
            folder_name: "nes",
            folder_path: "/roms/nes",
            fingerprint_hash: "folder-1",
            dat_game_count: 0,
        },
        entries,
    )
    .unwrap();
    (conn, root, console)
}

fn scanned(path: &str, display: &str, fingerprint: &str) -> ScannedLibraryEntry {
    ScannedLibraryEntry {
        entry_key: file_source_key(Path::new(path)).unwrap(),
        source_fingerprint: fingerprint.into(),
        row: row(path, display),
    }
}

#[test]
fn source_keys_are_normalized_safe_and_platform_independent() {
    assert_eq!(
        file_source_key(Path::new("a/./b.rom")).unwrap().as_str(),
        "file:a/b.rom"
    );
    assert_eq!(
        set_source_key(Path::new("Game.m3u")).unwrap().as_str(),
        "set:Game.m3u"
    );
    assert_eq!(
        file_source_key(Path::new(r"dir\game.rom"))
            .unwrap()
            .as_str(),
        "file:dir/game.rom"
    );
    assert!(file_source_key(Path::new("../escape.rom")).is_err());
    assert!(file_source_key(Path::new("/absolute.rom")).is_err());
    assert!(file_source_key(Path::new(r"C:\absolute.rom")).is_err());
}

#[test]
fn fingerprint_is_order_independent_and_descriptor_content_sensitive() {
    let descriptor = |path: &str, contents: &[u8]| SourceFileDescriptor {
        relative_path: path.into(),
        kind: SourceFileKind::Cue,
        size: 12,
        modified_seconds: 9,
        modified_nanos: 3,
        descriptor_contents: Some(contents.to_vec()),
    };
    let a = descriptor("disc/a.cue", b"FILE a.bin");
    let b = descriptor("disc/b.cue", b"FILE b.bin");
    assert_eq!(
        source_fingerprint(&[a.clone(), b.clone()]).unwrap(),
        source_fingerprint(&[b.clone(), a.clone()]).unwrap()
    );
    assert_ne!(
        source_fingerprint(&[a, b]).unwrap(),
        source_fingerprint(&[descriptor("disc/a.cue", b"FILE changed.bin")]).unwrap()
    );
}

#[test]
fn cue_fingerprint_tracks_referenced_files_and_descriptor_contents() {
    let dir = tempfile::tempdir().unwrap();
    let cue = dir.path().join("game.cue");
    let bin = dir.path().join("track.bin");
    std::fs::write(&cue, "FILE \"track.bin\" BINARY\n  TRACK 01 MODE1/2352\n").unwrap();
    std::fs::write(&bin, [1_u8, 2, 3]).unwrap();
    let json = serde_json::json!({ "SingleFile": cue }).to_string();
    let first = source_fingerprint_from_game_entry_json(&json, dir.path()).unwrap();
    std::fs::write(&bin, [1_u8, 2, 3, 4]).unwrap();
    let second = source_fingerprint_from_game_entry_json(&json, dir.path()).unwrap();
    assert_ne!(first, second);
    std::fs::write(&cue, "FILE \"track.bin\" BINARY\nREM changed\n").unwrap();
    let third = source_fingerprint_from_game_entry_json(&json, dir.path()).unwrap();
    assert_ne!(second, third);
}

#[test]
fn duplicate_display_names_have_distinct_durable_ids_and_deterministic_pages() {
    let (conn, _, console) = setup(&[row("a/game.nes", "game.nes"), row("b/game.nes", "game.nes")]);
    let query = LibraryEntryListQuery {
        console_id: console,
        search: String::new(),
        filter: LibraryEntryFilter::All,
        sort: LibraryEntrySortField::DisplayName,
        direction: SortDirection::Ascending,
        offset: 0,
        limit: 1,
    };
    let first = query_entry_list(&conn, &query).unwrap();
    let second = query_entry_list(&conn, &LibraryEntryListQuery { offset: 1, ..query }).unwrap();
    assert_eq!(first.total_count, 2);
    assert_eq!(first.rows[0].display_name, second.rows[0].display_name);
    assert_ne!(first.rows[0].id, second.rows[0].id);
    assert!(first.rows[0].id < second.rows[0].id);
}

#[test]
fn console_summaries_report_complete_effective_status_aggregates() {
    let mut matched = row("matched.nes", "matched.nes");
    matched.status = "matched".into();
    let mut unknown = row("unknown.nes", "unknown.nes");
    unknown.status = "unknown".into();
    let mut unrecognized = row("bad.nes", "bad.nes");
    unrecognized.status = "unrecognized".into();
    let mut ambiguous = row("maybe.nes", "maybe.nes");
    ambiguous.status = "ambiguous".into();
    let mut tagged = row("homebrew.nes", "homebrew.nes");
    tagged.status = "unrecognized".into();
    tagged.tag = "homebrew".into();

    let (conn, root, _) = setup(&[matched, unknown, unrecognized, ambiguous, tagged]);
    let summaries = list_console_summaries(&conn, root).unwrap();
    let summary = &summaries[0];

    assert_eq!(summary.entry_count, 5);
    assert_eq!(summary.matched_count, 1);
    assert_eq!(summary.unknown_count, 1);
    assert_eq!(summary.unrecognized_count, 1);
    assert_eq!(summary.ambiguous_count, 1);
    assert_eq!(summary.tagged_count, 1);

    let page = query_entry_list(
        &conn,
        &LibraryEntryListQuery {
            console_id: summary.id,
            search: String::new(),
            filter: LibraryEntryFilter::All,
            sort: LibraryEntrySortField::DisplayName,
            direction: SortDirection::Ascending,
            offset: 0,
            limit: 300,
        },
    )
    .unwrap();
    assert_eq!(page.counts.matched, 1);
    assert_eq!(page.counts.unknown, 1);
    assert_eq!(page.counts.unrecognized, 1);
    assert_eq!(page.counts.ambiguous, 1);
    assert_eq!(page.counts.tagged, 1);

    let unrecognized = query_entry_list(
        &conn,
        &LibraryEntryListQuery {
            filter: LibraryEntryFilter::Error,
            ..LibraryEntryListQuery {
                console_id: summary.id,
                search: String::new(),
                filter: LibraryEntryFilter::All,
                sort: LibraryEntrySortField::DisplayName,
                direction: SortDirection::Ascending,
                offset: 0,
                limit: 300,
            }
        },
    )
    .unwrap();
    assert_eq!(unrecognized.rows.len(), 1);
}

#[test]
fn unchanged_and_changed_scans_preserve_identity_but_invalidate_only_when_needed() {
    let (mut conn, _, console) = setup(&[row("game.nes", "game.nes")]);
    let original = query_entry_list(
        &conn,
        &LibraryEntryListQuery {
            console_id: console,
            search: String::new(),
            filter: LibraryEntryFilter::All,
            sort: LibraryEntrySortField::DisplayName,
            direction: SortDirection::Ascending,
            offset: 0,
            limit: 300,
        },
    )
    .unwrap()
    .rows[0]
        .clone();
    set_entry_tag(&mut conn, original.id, Some("homebrew")).unwrap();
    let token = begin_console_scan(&conn, console).unwrap();
    reconcile_console_scan(
        &mut conn,
        token,
        "folder-2",
        &[scanned("game.nes", "renamed display", "")],
    )
    .unwrap();
    let unchanged = load_entry_detail(&conn, original.id).unwrap().unwrap();
    assert_eq!(unchanged.row.tag, "homebrew");

    let token = begin_console_scan(&conn, console).unwrap();
    reconcile_console_scan(
        &mut conn,
        token,
        "folder-3",
        &[scanned("game.nes", "renamed display", "new-source")],
    )
    .unwrap();
    let changed = load_entry_detail(&conn, original.id).unwrap().unwrap();
    assert_eq!(changed.id, original.id);
    assert_eq!(changed.row.tag, "homebrew");
    assert_eq!(changed.row.status, "unknown");
    assert_eq!(changed.row.crc32, "");
    assert_eq!(changed.source_revision, unchanged.source_revision + 1);
}

#[test]
fn stale_analysis_and_stale_scan_are_atomic_noops() {
    let (mut conn, _, console) = setup(&[row("game.nes", "game.nes")]);
    let id = query_entry_list(
        &conn,
        &LibraryEntryListQuery {
            console_id: console,
            search: String::new(),
            filter: LibraryEntryFilter::All,
            sort: LibraryEntrySortField::DisplayName,
            direction: SortDirection::Ascending,
            offset: 0,
            limit: 300,
        },
    )
    .unwrap()
    .rows[0]
        .id;
    let old_token = begin_console_scan(&conn, console).unwrap();
    let _new_token = begin_console_scan(&conn, console).unwrap();
    assert!(matches!(
        reconcile_console_scan(&mut conn, old_token, "bad", &[]),
        Err(LibraryError::StaleCommand)
    ));
    assert!(load_entry_detail(&conn, id).unwrap().is_some());

    let token = begin_console_scan(&conn, console).unwrap();
    reconcile_console_scan(
        &mut conn,
        token,
        "changed",
        &[scanned("game.nes", "game.nes", "changed")],
    )
    .unwrap();
    let update = EntryAnalysisUpdate {
        status: "matched".into(),
        crc32: "abc".into(),
        sha1: String::new(),
        md5: String::new(),
        data_size: 1,
        dat_game_name: String::new(),
        dat_rom_name: String::new(),
        dat_match_method: String::new(),
        cover_title: String::new(),
        screen_title: String::new(),
        identification_json: None,
        disc_identifications_json: None,
        broken_references_json: None,
        ambiguous_candidates_json: None,
        cue_compat_issues_json: None,
    };
    assert!(matches!(
        apply_entry_analysis(&mut conn, id, 0, &update),
        Err(LibraryError::StaleCommand)
    ));
    assert_eq!(load_entry_detail(&conn, id).unwrap().unwrap().row.crc32, "");
}

#[test]
fn beginning_or_abandoning_a_scan_never_deletes_entries() {
    let (conn, _, console) = setup(&[row("game.nes", "game.nes")]);
    let before = load_consoles_for_root(&conn, LibraryRootId(1)).unwrap()[0].revision;
    begin_console_scan(&conn, console).unwrap();
    assert_eq!(load_entries_for_console(&conn, console).unwrap().len(), 1);
    assert_eq!(
        load_consoles_for_root(&conn, LibraryRootId(1)).unwrap()[0].revision,
        before
    );
}

#[test]
fn v9_migration_preserves_rows_and_ids_without_merging_collisions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "CREATE TABLE schema_version(version INTEGER NOT NULL, applied_at TEXT);
         INSERT INTO schema_version VALUES(9, datetime('now'));
         CREATE TABLE library_roots(id INTEGER PRIMARY KEY,root_path TEXT UNIQUE,created_at TEXT);
         CREATE TABLE library_consoles(id INTEGER PRIMARY KEY,root_id INTEGER,platform TEXT,folder_name TEXT,folder_path TEXT,fingerprint_hash TEXT,dat_game_count INTEGER,UNIQUE(root_id,folder_name));
         CREATE TABLE library_entries(id INTEGER PRIMARY KEY,console_id INTEGER,display_name TEXT,game_entry_json TEXT,status TEXT,tag TEXT,crc32 TEXT,sha1 TEXT,md5 TEXT,data_size INTEGER,dat_game_name TEXT,dat_rom_name TEXT,dat_match_method TEXT,region_override TEXT,cover_title TEXT,screen_title TEXT,identification_json TEXT,disc_identifications_json TEXT,broken_references_json TEXT,ambiguous_candidates_json TEXT,cue_compat_issues_json TEXT,UNIQUE(console_id,display_name));
         INSERT INTO library_roots VALUES(1,'/roms',datetime('now'));
         INSERT INTO library_consoles VALUES(2,1,'NES','nes','/roms/nes','old',0);
         INSERT INTO library_entries(id,console_id,display_name,game_entry_json,status,tag,crc32,region_override) VALUES
           (10,2,'first','{\"SingleFile\":\"/roms/nes/same.nes\"}','matched','homebrew','abc','USA'),
           (11,2,'second','{\"SingleFile\":\"/roms/nes/same.nes\"}','matched','','def',''),
           (12,2,'broken','not json','matched','','ghi','');",
    )
    .unwrap();
    drop(conn);

    let conn = open_database(&path).unwrap();
    let rows: Vec<(u64, String, String, String, String)> = conn
        .prepare("SELECT id,entry_key,status,tag,crc32 FROM library_entries ORDER BY id")
        .unwrap()
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows.iter().map(|r| r.0).collect::<Vec<_>>(), [10, 11, 12]);
    assert!(rows.iter().all(|r| r.1 == format!("invalid:{}", r.0)));
    assert!(rows.iter().all(|r| r.2 == "unknown" && r.4.is_empty()));
    assert_eq!(rows[0].3, "homebrew");
    let state: String = conn
        .query_row(
            "SELECT scan_state FROM library_consoles WHERE id=2",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "stale");
}
