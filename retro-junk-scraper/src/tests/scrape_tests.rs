use super::*;

use retro_junk_archive::{CollectionMark, MarkIndex, MarkKind, MarkedContent};
use retro_junk_core::Platform;

fn mark(name: &str, sha1: &str, crc32: &str, size: u64) -> CollectionMark {
    CollectionMark {
        schema_version: retro_junk_archive::marks::MARK_SCHEMA_VERSION,
        kind: MarkKind::Modded,
        platform_id: "psx".to_owned(),
        region: "usa".to_owned(),
        name: name.to_owned(),
        parent_work_id: String::new(),
        parent_dat_name: "Test Game (USA)".to_owned(),
        content: MarkedContent {
            size,
            crc32: crc32.to_owned(),
            sha1: sha1.to_owned(),
            md5: String::new(),
        },
        chosen_media_id: String::new(),
        chosen_dat_name: String::new(),
        note: String::new(),
    }
}

fn rom(filename: &str, size: u64, hashes: Option<RomHashes>) -> RomInfo {
    RomInfo {
        serial: String::new(),
        scraper_serial: String::new(),
        filename: filename.to_owned(),
        file_size: size,
        hashes,
        platform: Platform::Ps1,
        expects_serial: true,
    }
}

/// Content is the strongest key a mark has, and the only one a rename cannot
/// break — so it is tried first, in either digest the caller happens to hold.
#[test]
fn a_mark_is_found_by_either_digest_the_scan_computed() {
    let index = MarkIndex::from_marks(vec![mark("Hack.chd", "abc123", "DEADBEEF", 700)]);

    let by_sha1 = rom(
        "renamed.chd",
        700,
        Some(RomHashes {
            crc32: String::new(),
            md5: String::new(),
            sha1: "ABC123".to_owned(),
        }),
    );
    assert!(
        file_mark(&index, &by_sha1).is_some(),
        "digest comparison is case-insensitive, and a rename must not lose the decision"
    );

    let by_crc = rom(
        "renamed.chd",
        700,
        Some(RomHashes {
            crc32: "deadbeef".to_owned(),
            md5: String::new(),
            sha1: String::new(),
        }),
    );
    assert!(file_mark(&index, &by_crc).is_some());
}

/// Serial-expecting consoles skip hashing for speed, so on exactly those
/// platforms name and size are the only key available. Without the fallback,
/// a PS1 mod would be looked up by its own bytes on every run.
#[test]
fn an_unhashed_file_still_finds_its_mark_by_name_and_size() {
    let index = MarkIndex::from_marks(vec![mark("Hack.chd", "abc123", "DEADBEEF", 700)]);

    assert!(file_mark(&index, &rom("hack.chd", 700, None)).is_some());
    assert!(
        file_mark(&index, &rom("hack.chd", 701, None)).is_none(),
        "a different file of the same name is a different file"
    );
    assert!(file_mark(&index, &rom("other.chd", 700, None)).is_none());
}

/// The overwhelming majority of collections have no marks at all, and every
/// scrape passes through here.
#[test]
fn an_unmarked_collection_costs_a_lookup_nothing() {
    assert!(file_mark(&MarkIndex::default(), &rom("game.chd", 700, None)).is_none());
}
