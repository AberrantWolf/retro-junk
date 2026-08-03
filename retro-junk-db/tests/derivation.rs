//! Derivation: what the catalog says a file is, and whose identity a scraper
//! must therefore be asked about.

use retro_junk_db::library::ScrapeIdentityTier;
use retro_junk_db::{CatalogDerivation, LibraryEntryId, open_memory, query_entry_derivations};

/// One SNES work with a DAT-imported medium, plus a library row for a hack of
/// it. Everything below differs only in what is tagged and what exists.
fn catalog_with_parent() -> rusqlite::Connection {
    let conn = open_memory().unwrap();
    conn.execute_batch(
        "INSERT INTO platforms(id,display_name,short_name,manufacturer,generation,media_type,release_year,description,core_platform)
         VALUES('snes','SNES','SNES','Nintendo',4,'cartridge',1990,'','Snes');
         INSERT INTO works(id,canonical_name) VALUES('snes:smw','Super Mario World');
         INSERT INTO releases(id,work_id,platform_id,region,title)
         VALUES('snes:smw:usa','snes:smw','snes','usa','Super Mario World');
         INSERT INTO media(id,release_id,dat_source,dat_name,rom_name,file_size,media_serial,crc32,md5,sha1)
         VALUES('snes:smw:usa:media','snes:smw:usa','no-intro','Super Mario World (USA)',
                'Super Mario World (USA).sfc',524288,'SNS-MW-USA','11223344',
                'ffffffffffffffffffffffffffffffff','1111111111111111111111111111111111111111');
         INSERT INTO library_roots(id,root_path) VALUES(1,'/roms');
         INSERT INTO library_consoles(id,root_id,platform,folder_name,folder_path,fingerprint_hash)
         VALUES(1,1,'Snes','snes','/roms/snes','');
         INSERT INTO library_entries(id,console_id,entry_key,display_name,game_entry_json,
                                     crc32,sha1,md5,data_size,status)
         VALUES(1,1,'file:kaizo.sfc','Super Mario World (Kaizo).sfc','{}',
                'aabbccdd','da39a3ee','0123456789abcdef',1048576,'unrecognized');",
    )
    .unwrap();
    conn
}

/// The point of the whole feature: a mod is asked about as the work it was
/// made from. Its own digests match nothing in any scraper's database, and
/// offering them spends a metered request to be told so.
#[test]
fn a_modded_entry_resolves_its_parents_identity() {
    let conn = catalog_with_parent();
    conn.execute_batch(
        "UPDATE library_entries SET tag='modded' WHERE id=1;
         INSERT INTO media(id,release_id,tag,crc32,sha1,file_size)
         VALUES('snes:smw:usa:modded','snes:smw:usa','modded','aabbccdd','da39a3ee',1048576);
         INSERT INTO library_entry_media_bindings(library_entry_id,catalog_media_id,match_method)
         VALUES(1,'snes:smw:usa:modded','collection_mark');",
    )
    .unwrap();

    let derivations = query_entry_derivations(&conn, &[LibraryEntryId(1)]).unwrap();
    let CatalogDerivation::Modded {
        parent: Some(parent),
    } = derivations.get(&LibraryEntryId(1)).unwrap()
    else {
        panic!("a mod bound to its parent's work must resolve that work");
    };
    assert_eq!(parent.filename, "Super Mario World (USA).sfc");
    assert_eq!(parent.serial, "SNS-MW-USA");
    assert_eq!(parent.crc32, "11223344");
    assert_eq!(
        parent.file_size, 524_288,
        "the parent's size, not the mod's"
    );
}

/// A mod whose parent this catalog does not hold cannot be asked about at all
/// — and saying so is what stops derivation from proposing a scrape that
/// spends requests every pass and can never succeed.
#[test]
fn a_mod_with_no_catalogued_parent_has_no_identity_at_all() {
    let conn = catalog_with_parent();
    conn.execute("UPDATE library_entries SET tag='modded' WHERE id=1", [])
        .unwrap();

    let derivations = query_entry_derivations(&conn, &[LibraryEntryId(1)]).unwrap();
    assert_eq!(
        derivations.get(&LibraryEntryId(1)),
        Some(&CatalogDerivation::Modded { parent: None }),
        "a tag with no work behind it names no parent"
    );

    let identity = retro_junk_db::library::ArchivedScrapeIdentity {
        filename: "Super Mario World (Kaizo).sfc".to_owned(),
        file_size: 1_048_576,
        serial: "SNS-MW-USA".to_owned(),
        crc32: "aabbccdd".to_owned(),
        md5: "0123456789abcdef".to_owned(),
        sha1: "da39a3ee".to_owned(),
        derivation: CatalogDerivation::Modded { parent: None },
    };
    assert_eq!(
        identity.tier(),
        ScrapeIdentityTier::None,
        "its own serial and digests describe bytes no catalog has ever held"
    );
}

/// A mod is only as identifiable as its parent, whatever the file itself
/// carries. Automation gates on this: publishing a name-tier guess into the
/// archive unattended is exactly what the gate exists to prevent.
#[test]
fn a_mods_identity_strength_is_its_parents() {
    let parent = retro_junk_db::library::ArchivedScrapeIdentity {
        filename: "Super Mario World (USA).sfc".to_owned(),
        ..Default::default()
    };
    let by_name = retro_junk_db::library::ArchivedScrapeIdentity {
        serial: "SNS-MW-USA".to_owned(),
        derivation: CatalogDerivation::Modded {
            parent: Some(Box::new(parent.clone())),
        },
        ..Default::default()
    };
    assert_eq!(by_name.tier(), ScrapeIdentityTier::Filename);

    let by_hashes = retro_junk_db::library::ArchivedScrapeIdentity {
        derivation: CatalogDerivation::Modded {
            parent: Some(Box::new(retro_junk_db::library::ArchivedScrapeIdentity {
                crc32: "a".to_owned(),
                md5: "b".to_owned(),
                sha1: "c".to_owned(),
                ..parent
            })),
        },
        ..Default::default()
    };
    assert_eq!(by_hashes.tier(), ScrapeIdentityTier::Hashes);
}

/// Nobody assigned homebrew a serial. Counting one would rank a placeholder
/// above a name — and the serial tier would then match some commercial game
/// that really does own those characters.
#[test]
fn homebrews_serial_never_counts_toward_its_identity() {
    let homebrew = retro_junk_db::library::ArchivedScrapeIdentity {
        filename: "Finchy Quest".to_owned(),
        serial: "GB-0001".to_owned(),
        derivation: CatalogDerivation::Homebrew,
        ..Default::default()
    };
    assert_eq!(homebrew.tier(), ScrapeIdentityTier::Filename);
}

/// Ids are minted per DAT release and do not survive a re-import elsewhere, so
/// what a mark records about a parent is what it is *called*.
#[test]
fn a_parent_is_named_by_its_dat_name_and_falls_back_to_its_canonical_name() {
    let conn = catalog_with_parent();
    assert_eq!(
        retro_junk_db::derivation::work_lookup_name(&conn, "snes:smw", "snes").unwrap(),
        "Super Mario World (USA)"
    );

    // A work synthesized from a mark has no DAT-derived medium at all.
    conn.execute_batch(
        "INSERT INTO works(id,canonical_name,tag) VALUES('snes:homebrew:x','Finchy Quest','homebrew');
         INSERT INTO releases(id,work_id,platform_id,region,title)
         VALUES('snes:homebrew:x:rel','snes:homebrew:x','snes','usa','Finchy Quest');",
    )
    .unwrap();
    assert_eq!(
        retro_junk_db::derivation::work_lookup_name(&conn, "snes:homebrew:x", "snes").unwrap(),
        "Finchy Quest"
    );
}

/// Tagging writes the durable form as well as the row. Without this the
/// decision lives only in a database that is rebuilt from DATs — and "matched
/// by no DAT" is the normal state of every file these decisions are about.
#[test]
fn tagging_a_mod_records_the_decision_beside_the_collection() {
    let collection = tempfile::tempdir().unwrap();
    let mut conn = catalog_with_parent();

    retro_junk_db::create_modded_and_tag_entry(
        &mut conn,
        LibraryEntryId(1),
        &retro_junk_db::ModdedEntry {
            work_id: "snes:smw",
            platform_id: "snes",
            region: "usa",
            disc_number: None,
            hashes: None,
            collection_root: Some(collection.path()),
        },
    )
    .unwrap();

    let marks = retro_junk_archive::load_marks(collection.path()).unwrap();
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0].kind, retro_junk_archive::MarkKind::Modded);
    assert_eq!(
        marks[0].parent_dat_name, "Super Mario World (USA)",
        "the parent travels by name, because ids do not travel at all"
    );
    assert_eq!(marks[0].parent_work_id, "snes:smw");
    assert_eq!(marks[0].content.sha1, "da39a3ee");
    assert_eq!(marks[0].name, "Super Mario World (Kaizo).sfc");

    // Taking the tag off forgets the decision everywhere, not just in the row.
    retro_junk_db::set_entry_tag(&mut conn, LibraryEntryId(1), None, Some(collection.path()))
        .unwrap();
    assert!(
        retro_junk_archive::load_marks(collection.path())
            .unwrap()
            .is_empty()
    );
}

/// Tagging has to leave the decision *usable*, not merely recorded. Everything
/// downstream reads the row's bound medium to learn which work a mod derives
/// from, so without the binding a freshly tagged mod would be scraped by its
/// own bytes — the exact behavior the tag exists to prevent — until some later
/// full reconcile happened to bind it.
#[test]
fn tagging_a_mod_makes_its_parent_resolvable_at_once() {
    let collection = tempfile::tempdir().unwrap();
    let mut conn = catalog_with_parent();

    retro_junk_db::create_modded_and_tag_entry(
        &mut conn,
        LibraryEntryId(1),
        &retro_junk_db::ModdedEntry {
            work_id: "snes:smw",
            platform_id: "snes",
            region: "usa",
            disc_number: None,
            hashes: None,
            collection_root: Some(collection.path()),
        },
    )
    .unwrap();

    let derivations = query_entry_derivations(&conn, &[LibraryEntryId(1)]).unwrap();
    let CatalogDerivation::Modded {
        parent: Some(parent),
    } = derivations.get(&LibraryEntryId(1)).unwrap()
    else {
        panic!("the work the user just picked must be the resolved parent");
    };
    assert_eq!(parent.filename, "Super Mario World (USA).sfc");
}

/// Content is the only identity these files have. A row with no digests yet
/// still gets its tag — it just has nothing portable to be keyed on until it
/// is hashed, and inventing a key would produce a mark nothing can match.
#[test]
fn an_unhashed_row_is_tagged_without_a_mark_that_could_never_match() {
    let collection = tempfile::tempdir().unwrap();
    let mut conn = catalog_with_parent();
    conn.execute(
        "UPDATE library_entries SET crc32='',sha1='',md5='' WHERE id=1",
        [],
    )
    .unwrap();

    retro_junk_db::create_homebrew_and_tag_entry(
        &mut conn,
        LibraryEntryId(1),
        "Finchy Quest",
        "snes",
        "usa",
        Some(collection.path()),
    )
    .unwrap();

    let tag: String = conn
        .query_row("SELECT tag FROM library_entries WHERE id=1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(tag, "homebrew");
    assert!(
        retro_junk_archive::load_marks(collection.path())
            .unwrap()
            .is_empty()
    );
}

/// A multi-track disc's `rom_name` is its largest *member track*, so offering
/// it to a scraper asks about `Some Game (USA) (Track 2).bin` — a file that
/// exists in no collection, and a track rather than the disc. The archive
/// holds the whole medium, and the library's own scrape offers the whole
/// medium's filename for the same disc, so this is also the two surfaces
/// asking one disc two different questions.
#[test]
fn a_multi_track_disc_is_asked_about_as_the_disc_not_a_track() {
    let conn = open_memory().unwrap();
    conn.execute_batch(
        "INSERT INTO platforms(id,display_name,short_name,manufacturer,generation,media_type,release_year,description,core_platform)
         VALUES('psx','PS1','PS1','Sony',5,'optical',1994,'','Ps1');
         INSERT INTO works(id,canonical_name) VALUES('psx:sotn','Castlevania - Symphony of the Night');
         INSERT INTO releases(id,work_id,platform_id,region,title)
         VALUES('psx:sotn:usa','psx:sotn','psx','usa','Castlevania - Symphony of the Night');
         -- The importer stores the largest track's name, size, and hashes on
         -- the medium; per-track rows are what record that it is multi-track.
         INSERT INTO media(id,release_id,dat_source,dat_name,rom_name,file_size,media_serial,crc32,md5,sha1)
         VALUES('psx:sotn:usa:media','psx:sotn:usa','redump',
                'Castlevania - Symphony of the Night (USA)',
                'Castlevania - Symphony of the Night (USA) (Track 2).bin',
                614_000_000,'SLUS-00067','11223344',
                'ffffffffffffffffffffffffffffffff','1111111111111111111111111111111111111111');
         INSERT INTO media_tracks(media_id,track_number,track_name,file_size,crc32,sha1)
         VALUES('psx:sotn:usa:media',1,'Castlevania - Symphony of the Night (USA) (Track 1).bin',300000,'a','b'),
               ('psx:sotn:usa:media',2,'Castlevania - Symphony of the Night (USA) (Track 2).bin',614000000,'c','d');
         INSERT INTO library_roots(id,root_path) VALUES(1,'/roms');
         INSERT INTO library_consoles(id,root_id,platform,folder_name,folder_path,fingerprint_hash)
         VALUES(1,1,'Ps1','psx','/roms/psx','');
         INSERT INTO library_entries(id,console_id,entry_key,display_name,game_entry_json,crc32,sha1,md5,data_size,status,tag)
         VALUES(1,1,'file:sotn.chd','Castlevania (Randomizer).chd','{}','aabbccdd','da39a3ee','0123456789abcdef',600,'unrecognized','modded');
         INSERT INTO media(id,release_id,tag,crc32,sha1,file_size)
         VALUES('psx:sotn:usa:modded','psx:sotn:usa','modded','aabbccdd','da39a3ee',600);
         INSERT INTO library_entry_media_bindings(library_entry_id,catalog_media_id,match_method)
         VALUES(1,'psx:sotn:usa:modded','collection_mark');",
    )
    .unwrap();

    let derivations = query_entry_derivations(&conn, &[LibraryEntryId(1)]).unwrap();
    let CatalogDerivation::Modded {
        parent: Some(parent),
    } = derivations.get(&LibraryEntryId(1)).unwrap()
    else {
        panic!("a mod bound to its parent's work must resolve that work");
    };
    assert_eq!(
        parent.filename, "Castlevania - Symphony of the Night (USA)",
        "the disc's name, never a member track's filename"
    );
    assert_eq!(
        parent.file_size, 0,
        "romtaille narrows a name match to files of that size, so a track's \
         size must not travel with the disc's name"
    );
    // The identification that actually matters is untouched: the track's
    // digests are real digests of a real file, and the serial names the disc.
    assert_eq!(parent.serial, "SLUS-00067");
    assert_eq!(parent.crc32, "11223344");
    assert_eq!(parent.tier(), ScrapeIdentityTier::Serial);
}

/// The rule must not reach single-file media, whose ROM name legitimately
/// distinguishes representations sharing one game name — and whose extension
/// the scraper's filename tier expects.
#[test]
fn a_single_file_medium_keeps_its_rom_filename_and_size() {
    let conn = catalog_with_parent();
    conn.execute_batch(
        "UPDATE library_entries SET tag='modded' WHERE id=1;
         INSERT INTO media(id,release_id,tag,crc32,sha1,file_size)
         VALUES('snes:smw:usa:modded','snes:smw:usa','modded','aabbccdd','da39a3ee',1048576);
         INSERT INTO library_entry_media_bindings(library_entry_id,catalog_media_id,match_method)
         VALUES(1,'snes:smw:usa:modded','collection_mark');",
    )
    .unwrap();

    let derivations = query_entry_derivations(&conn, &[LibraryEntryId(1)]).unwrap();
    let CatalogDerivation::Modded {
        parent: Some(parent),
    } = derivations.get(&LibraryEntryId(1)).unwrap()
    else {
        panic!("a mod bound to its parent's work must resolve that work");
    };
    assert_eq!(parent.filename, "Super Mario World (USA).sfc");
    assert_eq!(parent.file_size, 524_288);
}
