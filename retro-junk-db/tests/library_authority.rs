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
        hash_warnings_json: None,
        disc_verification: "not_applicable".into(),
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
    let mut conn = open_memory().unwrap();
    let root = upsert_library_root(&conn, "/roms").unwrap();
    let console = ensure_library_console(
        &conn,
        &LibraryConsoleDescriptor {
            root_id: root,
            platform: "NES".into(),
            folder_name: "nes".into(),
            folder_path: "/roms/nes".into(),
        },
    )
    .unwrap();
    let scanned: Vec<_> = entries
        .iter()
        .map(|row| ScannedLibraryEntry {
            entry_key: source_key_from_game_entry_json(
                row.game_entry_json.as_str(),
                Path::new("/roms/nes"),
            )
            .unwrap(),
            source_fingerprint: String::new(),
            row: row.clone(),
        })
        .collect();
    let token = begin_console_scan(&conn, console).unwrap();
    reconcile_console_scan(&mut conn, token, "folder-1", &scanned).unwrap();
    for detail in load_entry_details_for_console(&conn, console).unwrap() {
        let source = entries
            .iter()
            .find(|row| row.display_name == detail.row.display_name)
            .unwrap();
        apply_entry_analysis(
            &mut conn,
            detail.id,
            detail.source_revision,
            &EntryAnalysisUpdate {
                status: source.status.clone(),
                crc32: source.crc32.clone(),
                sha1: source.sha1.clone(),
                md5: source.md5.clone(),
                data_size: source.data_size,
                hash_warnings_json: source.hash_warnings_json.clone(),
                disc_verification: source.disc_verification.clone(),
                dat_game_name: source.dat_game_name.clone(),
                dat_rom_name: source.dat_rom_name.clone(),
                dat_match_method: source.dat_match_method.clone(),
                cover_title: source.cover_title.clone(),
                screen_title: source.screen_title.clone(),
                identification_json: source.identification_json.clone(),
                disc_identifications_json: source.disc_identifications_json.clone(),
                broken_references_json: source.broken_references_json.clone(),
                ambiguous_candidates_json: source.ambiguous_candidates_json.clone(),
                cue_compat_issues_json: source.cue_compat_issues_json.clone(),
            },
        )
        .unwrap();
        if !source.tag.is_empty() {
            set_entry_tag(&mut conn, detail.id, Some(&source.tag), None).unwrap();
        }
    }
    (conn, root, console)
}

fn scanned(path: &str, display: &str, fingerprint: &str) -> ScannedLibraryEntry {
    ScannedLibraryEntry {
        entry_key: file_source_key(Path::new(path)).unwrap(),
        source_fingerprint: fingerprint.into(),
        row: row(path, display),
    }
}

fn analyzed_scanned(path: &str, display: &str, fingerprint: &str) -> ScannedLibraryEntry {
    let mut scanned = scanned(path, display, fingerprint);
    scanned.row.status = "matched".into();
    scanned.row.crc32 = "89abcdef".into();
    scanned.row.sha1 = "0123456789abcdef".into();
    scanned.row.md5 = "fedcba9876543210".into();
    scanned.row.data_size = 42;
    scanned.row.dat_game_name = "Catalog Game".into();
    scanned.row.dat_rom_name = "game.nes".into();
    scanned.row.dat_match_method = "sha1".into();
    scanned.row.cover_title = "Cover".into();
    scanned.row.screen_title = "Screen".into();
    scanned.row.identification_json = Some(r#"{"serial_number":"TEST-1"}"#.into());
    scanned.row.broken_references_json = Some("[]".into());
    scanned
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
fn list_search_treats_sql_wildcards_and_escape_characters_literally() {
    let (conn, _, console) = setup(&[
        row("percent.nes", "100% fun"),
        row("underscore.nes", "under_score"),
        row("slash.nes", r"back\slash"),
        row("ordinary.nes", "ordinary"),
    ]);
    let find = |search: &str| {
        query_entry_list(
            &conn,
            &LibraryEntryListQuery {
                console_id: console,
                search: search.into(),
                filter: LibraryEntryFilter::All,
                sort: LibraryEntrySortField::DisplayName,
                direction: SortDirection::Ascending,
                offset: 0,
                limit: 300,
            },
        )
        .unwrap()
        .rows
    };
    assert_eq!(find("%").len(), 1);
    assert_eq!(find("_").len(), 1);
    assert_eq!(find(r"\").len(), 1);
}

#[test]
fn list_projection_contains_all_automatically_known_row_fields() {
    let mut entry = row("game.nes", "game.nes");
    entry.identification_json = Some(
        r#"{"serial_number":"","internal_name":"HEADER TITLE","regions":["Usa","Europe"]}"#.into(),
    );
    entry.hash_warnings_json = Some(r#"["headered dump"]"#.into());
    entry.disc_identifications_json = Some(
        r#"[{"identification":{"serial_number":"DISC-1"},"hashes":{"warnings":["track mismatch"]}}]"#
            .into(),
    );
    let (conn, _, console) = setup(&[entry]);
    let page = query_entry_list(
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
    .unwrap();
    let projected = &page.rows[0];
    assert_eq!(projected.internal_name, "HEADER TITLE");
    assert_eq!(projected.detected_regions, ["Usa", "Europe"]);
    assert!(projected.has_hash_warnings);
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
    let mut likely = row("likely.nes", "likely.nes");
    likely.status = "likely".into();
    let mut tagged = row("homebrew.nes", "homebrew.nes");
    tagged.status = "unrecognized".into();
    tagged.tag = "homebrew".into();

    let (conn, root, _) = setup(&[matched, unknown, unrecognized, ambiguous, likely, tagged]);
    let summaries = list_console_summaries(&conn, root).unwrap();
    let summary = &summaries[0];

    assert_eq!(summary.entry_count, 6);
    assert_eq!(summary.matched_count, 1);
    assert_eq!(summary.unknown_count, 1);
    assert_eq!(summary.unrecognized_count, 1);
    assert_eq!(summary.ambiguous_count, 1);
    assert_eq!(summary.likely_count, 1);
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
    assert_eq!(page.counts.likely, 1);
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
    set_entry_tag(&mut conn, original.id, Some("homebrew"), None).unwrap();
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
fn reconciliation_persists_completed_analysis_for_new_and_changed_sources() {
    let mut conn = open_memory().unwrap();
    let root = upsert_library_root(&conn, "/roms").unwrap();
    let console = ensure_library_console(
        &conn,
        &LibraryConsoleDescriptor {
            root_id: root,
            platform: "NES".into(),
            folder_name: "nes".into(),
            folder_path: "/roms/nes".into(),
        },
    )
    .unwrap();

    let token = begin_console_scan(&conn, console).unwrap();
    reconcile_console_scan(
        &mut conn,
        token,
        "folder-1",
        &[analyzed_scanned("game.nes", "game.nes", "source-1")],
    )
    .unwrap();
    let first = load_entry_details_for_console(&conn, console)
        .unwrap()
        .remove(0);
    assert_eq!(first.row.status, "matched");
    assert_eq!(first.row.crc32, "89abcdef");
    assert_eq!(first.row.data_size, 42);
    assert_eq!(first.row.dat_game_name, "Catalog Game");
    assert!(first.row.identification_json.is_some());

    set_entry_tag(&mut conn, first.id, Some("homebrew"), None).unwrap();
    set_entry_region_override(&mut conn, first.id, Some("US"), None).unwrap();
    let token = begin_console_scan(&conn, console).unwrap();
    let mut changed = analyzed_scanned("game.nes", "renamed.nes", "source-2");
    changed.row.crc32 = "changed".into();
    reconcile_console_scan(&mut conn, token, "folder-2", &[changed]).unwrap();

    let second = load_entry_detail(&conn, first.id).unwrap().unwrap();
    assert_eq!(second.row.status, "matched");
    assert_eq!(second.row.crc32, "changed");
    assert_eq!(second.row.tag, "homebrew");
    assert_eq!(second.row.region_override, "US");
    assert_eq!(second.source_revision, first.source_revision + 1);
}

#[test]
fn identical_reconciliation_does_not_dirty_entry_revisions() {
    let (mut conn, _, console) = setup(&[row("game.nes", "game.nes")]);
    let before = load_entry_details_for_console(&conn, console)
        .unwrap()
        .remove(0);
    let token = begin_console_scan(&conn, console).unwrap();
    let changes = reconcile_console_scan(
        &mut conn,
        token,
        "folder-1",
        &[scanned("game.nes", "game.nes", "")],
    )
    .unwrap();
    let after = load_entry_detail(&conn, before.id).unwrap().unwrap();
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.source_revision, before.source_revision);
    assert!(changes.affected_entries.is_empty());
    assert!(changes.entry_revisions.is_empty());
    assert!(changes.console_revision.is_none());
    assert!(changes.root_revision.is_none());
}

#[test]
fn unchanged_source_reconciliation_refreshes_analysis_and_preserves_user_fields() {
    let (mut conn, _, console) = setup(&[row("game.nes", "game.nes")]);
    let before = load_entry_details_for_console(&conn, console)
        .unwrap()
        .remove(0);
    set_entry_tag(&mut conn, before.id, Some("homebrew"), None).unwrap();
    set_entry_region_override(&mut conn, before.id, Some("JP"), None).unwrap();

    let mut rescanned = scanned("game.nes", "game.nes", "");
    rescanned.row.status = "likely".into();
    rescanned.row.dat_game_name = "Header Match".into();
    rescanned.row.dat_match_method = "serial".into();
    rescanned.row.identification_json = Some(r#"{"serial_number":"TEST-1"}"#.into());
    let token = begin_console_scan(&conn, console).unwrap();
    reconcile_console_scan(&mut conn, token, "folder-2", &[rescanned]).unwrap();

    let after = load_entry_detail(&conn, before.id).unwrap().unwrap();
    assert_eq!(after.row.status, "likely");
    assert_eq!(after.row.dat_game_name, "Header Match");
    assert_eq!(after.row.tag, "homebrew");
    assert_eq!(after.row.region_override, "JP");
    assert_eq!(after.source_revision, before.source_revision);
}

#[test]
fn reconciliation_does_not_overwrite_a_hash_that_committed_after_scan_snapshot() {
    let (mut conn, _, console) = setup(&[row("game.nes", "game.nes")]);
    let snapshot = load_entry_details_for_console(&conn, console)
        .unwrap()
        .remove(0);
    apply_entry_hash_update(
        &mut conn,
        snapshot.id,
        snapshot.source_revision,
        &EntryHashUpdate {
            status: "matched".into(),
            crc32: "12345678".into(),
            sha1: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            md5: String::new(),
            data_size: 64,
            hash_warnings_json: None,
            disc_verification: "not_applicable".into(),
            dat_game_name: "Hash Match".into(),
            dat_rom_name: "game.nes".into(),
            dat_match_method: "sha1".into(),
            cover_title: "Hash Match".into(),
            screen_title: String::new(),
            disc_identifications_json: None,
            ambiguous_candidates_json: Some("[]".into()),
        },
    )
    .unwrap();

    let mut stale_scan = scanned("game.nes", "game.nes", "");
    stale_scan.row.status = "likely".into();
    stale_scan.row.dat_game_name = "Header Match".into();
    stale_scan.row.identification_json = Some(r#"{"serial_number":"TEST-1"}"#.into());
    let token = begin_console_scan(&conn, console).unwrap();
    reconcile_console_scan(&mut conn, token, "folder-2", &[stale_scan]).unwrap();

    let after = load_entry_detail(&conn, snapshot.id).unwrap().unwrap();
    assert_eq!(after.row.status, "matched");
    assert_eq!(after.row.crc32, "12345678");
    assert_eq!(after.row.dat_game_name, "Hash Match");
    assert!(after.row.identification_json.is_some());
    assert_eq!(after.source_revision, snapshot.source_revision);
}

#[test]
fn analysis_batch_commits_valid_entries_and_skips_stale_sources() {
    let (mut conn, _, console) = setup(&[row("a.nes", "a.nes"), row("b.nes", "b.nes")]);
    let details = load_entry_details_for_console(&conn, console).unwrap();
    let a = details
        .iter()
        .find(|detail| detail.row.display_name == "a.nes")
        .unwrap();
    let b = details
        .iter()
        .find(|detail| detail.row.display_name == "b.nes")
        .unwrap();
    let token = begin_console_scan(&conn, console).unwrap();
    reconcile_console_scan(
        &mut conn,
        token,
        "folder-2",
        &[
            scanned("a.nes", "a.nes", ""),
            scanned("b.nes", "b.nes", "changed"),
        ],
    )
    .unwrap();

    let update = |name: &str| EntryAnalysisUpdate {
        status: "likely".into(),
        crc32: String::new(),
        sha1: String::new(),
        md5: String::new(),
        data_size: 0,
        hash_warnings_json: None,
        disc_verification: "not_applicable".into(),
        dat_game_name: name.into(),
        dat_rom_name: String::new(),
        dat_match_method: "serial".into(),
        cover_title: String::new(),
        screen_title: String::new(),
        identification_json: None,
        disc_identifications_json: None,
        broken_references_json: Some("[]".into()),
        ambiguous_candidates_json: Some("[]".into()),
        cue_compat_issues_json: Some("[]".into()),
    };
    let changes = apply_entry_analysis_batch(
        &mut conn,
        &[
            EntryAnalysisCommand {
                entry_id: a.id,
                expected_source_revision: a.source_revision,
                update: update("A match"),
            },
            EntryAnalysisCommand {
                entry_id: b.id,
                expected_source_revision: b.source_revision,
                update: update("stale B match"),
            },
        ],
    )
    .unwrap();

    assert_eq!(changes.affected_entries, vec![a.id]);
    assert_eq!(
        load_entry_detail(&conn, a.id).unwrap().unwrap().row.status,
        "likely"
    );
    assert_eq!(
        load_entry_detail(&conn, b.id).unwrap().unwrap().row.status,
        "unknown"
    );
}

#[test]
fn catalog_creation_and_library_tagging_are_one_transaction() {
    let (mut conn, _, console) = setup(&[row("game.nes", "game.nes")]);
    let entry = load_entry_details_for_console(&conn, console)
        .unwrap()
        .remove(0);
    // The catalog rows are keyed on the file's digests, so a row nobody has
    // hashed never reaches the catalog at all — and there would be nothing to
    // roll back.
    conn.execute(
        "UPDATE library_entries SET sha1='aaaa1111',crc32='deadbeef',data_size=262144 WHERE id=?1",
        [entry.id.0],
    )
    .unwrap();

    // No such platform exists, so the catalog release insert violates its FK.
    // The library tag must roll back with the catalog work/media rows.
    assert!(
        create_homebrew_and_tag_entry(
            &mut conn,
            entry.id,
            "Test Homebrew",
            "missing-platform",
            "us",
            None,
        )
        .is_err()
    );
    let after = load_entry_detail(&conn, entry.id).unwrap().unwrap();
    assert!(after.row.tag.is_empty());
    let work_count: u64 = conn
        .query_row(
            "SELECT COUNT(*) FROM works WHERE canonical_name='Test Homebrew'",
            [],
            |result| result.get(0),
        )
        .unwrap();
    assert_eq!(work_count, 0);
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
        hash_warnings_json: None,
        disc_verification: "not_applicable".into(),
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
fn hash_updates_are_partial_and_reject_changed_sources() {
    let mut initial = row("game.nes", "game.nes");
    initial.identification_json = Some(r#"{"serial_number":"TEST"}"#.into());
    initial.broken_references_json = Some(r#"[{"path":"missing.bin"}]"#.into());
    initial.cue_compat_issues_json = Some(r#"[{"summary":"keep me"}]"#.into());
    let (mut conn, _, console) = setup(&[initial.clone()]);
    let detail = load_entry_details_for_console(&conn, console)
        .unwrap()
        .remove(0);

    let update = EntryHashUpdate {
        status: "matched".into(),
        crc32: "12345678".into(),
        sha1: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        md5: String::new(),
        data_size: 64,
        hash_warnings_json: None,
        disc_verification: "not_applicable".into(),
        dat_game_name: "Game (USA)".into(),
        dat_rom_name: "Game (USA).nes".into(),
        dat_match_method: "crc32".into(),
        cover_title: "Game".into(),
        screen_title: String::new(),
        disc_identifications_json: None,
        ambiguous_candidates_json: Some("[]".into()),
    };
    apply_entry_hash_update(&mut conn, detail.id, detail.source_revision, &update).unwrap();

    let updated = load_entry_detail(&conn, detail.id).unwrap().unwrap();
    assert_eq!(updated.row.status, "matched");
    assert_eq!(updated.row.crc32, "12345678");
    assert_eq!(updated.row.identification_json, initial.identification_json);
    assert_eq!(
        updated.row.broken_references_json,
        initial.broken_references_json
    );
    assert_eq!(
        updated.row.cue_compat_issues_json,
        initial.cue_compat_issues_json
    );

    let token = begin_console_scan(&conn, console).unwrap();
    reconcile_console_scan(
        &mut conn,
        token,
        "changed-again",
        &[scanned("game.nes", "game.nes", "changed-source")],
    )
    .unwrap();
    assert!(matches!(
        apply_entry_hash_update(&mut conn, detail.id, detail.source_revision, &update),
        Err(LibraryError::StaleCommand)
    ));
    assert_eq!(
        load_entry_detail(&conn, detail.id)
            .unwrap()
            .unwrap()
            .row
            .crc32,
        ""
    );
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

/// A region correction is a decision no DAT records, so a copy kept only in
/// this database dies with the row. A rename performed outside the app is
/// indistinguishable from a delete plus a create, so the row *is* deleted —
/// which is how the correction used to be lost. The durable mark is content-
/// keyed, so it re-applies to the same bytes under any name.
#[test]
fn a_region_correction_survives_a_rename_made_outside_the_app() {
    let collection = tempfile::tempdir().unwrap();
    let mut hashed = row("/roms/nes/game.nes", "game.nes");
    hashed.crc32 = "aabbccdd".into();
    hashed.sha1 = "a".repeat(40);
    hashed.data_size = 4096;
    let (mut conn, _root, console) = setup(&[hashed.clone()]);
    let entry = load_entry_details_for_console(&conn, console).unwrap()[0].id;

    set_entry_region_override(&mut conn, entry, Some("Japan"), Some(collection.path())).unwrap();
    // The decision is on disk, keyed by content rather than by filename.
    let marks = retro_junk_archive::load_marks(collection.path()).unwrap();
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0].kind, retro_junk_archive::MarkKind::RegionOverride);
    assert_eq!(marks[0].region, "Japan");

    // The file is renamed outside the app: the old row goes, a new one
    // arrives with nothing carried over.
    let mut renamed = row("/roms/nes/Game (Japan).nes", "Game (Japan).nes");
    renamed.crc32 = hashed.crc32.clone();
    renamed.sha1 = hashed.sha1.clone();
    renamed.data_size = hashed.data_size;
    let scanned = ScannedLibraryEntry {
        entry_key: source_key_from_game_entry_json(
            renamed.game_entry_json.as_str(),
            Path::new("/roms/nes"),
        )
        .unwrap(),
        source_fingerprint: "fp".into(),
        row: renamed,
    };
    let token = begin_console_scan(&conn, console).unwrap();
    reconcile_console_scan(&mut conn, token, "folder-2", &[scanned]).unwrap();
    let after_scan: String = conn
        .query_row(
            "SELECT region_override FROM library_entries WHERE console_id=?1",
            [console.0],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(after_scan, "", "the row genuinely lost the correction");

    // Re-applying the collection's marks restores it, by content.
    retro_junk_db::archive::apply_collection_marks(&conn, collection.path()).unwrap();
    let restored: String = conn
        .query_row(
            "SELECT region_override FROM library_entries WHERE console_id=?1",
            [console.0],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(restored, "Japan");
}

/// Clearing the correction removes the mark, so it does not come back on the
/// next scan.
#[test]
fn clearing_a_region_correction_forgets_it_durably() {
    let collection = tempfile::tempdir().unwrap();
    let mut hashed = row("/roms/nes/game.nes", "game.nes");
    hashed.crc32 = "aabbccdd".into();
    hashed.sha1 = "b".repeat(40);
    hashed.data_size = 2048;
    let (mut conn, _root, console) = setup(&[hashed]);
    let entry = load_entry_details_for_console(&conn, console).unwrap()[0].id;

    set_entry_region_override(&mut conn, entry, Some("Europe"), Some(collection.path())).unwrap();
    assert_eq!(
        retro_junk_archive::load_marks(collection.path())
            .unwrap()
            .len(),
        1
    );
    set_entry_region_override(&mut conn, entry, None, Some(collection.path())).unwrap();
    assert!(
        retro_junk_archive::load_marks(collection.path())
            .unwrap()
            .is_empty()
    );
}
