use super::*;

use retro_junk_archive::{CollectionMark, MarkKind, MarkedContent, marks::MARK_SCHEMA_VERSION};
use retro_junk_core::Platform;

fn own_identity() -> RomInfo {
    RomInfo {
        serial: "SNS-MW-USA".to_owned(),
        scraper_serial: "SNS-MW".to_owned(),
        filename: "Super Mario World (Kaizo Edition).sfc".to_owned(),
        file_size: 1_048_576,
        hashes: Some(RomHashes {
            crc32: "aabbccdd".to_owned(),
            md5: "0123456789abcdef0123456789abcdef".to_owned(),
            sha1: "da39a3ee5e6b4b0d3255bfef95601890afd80709".to_owned(),
        }),
        platform: Platform::Snes,
        expects_serial: true,
    }
}

fn mark(kind: MarkKind, parent_dat_name: &str) -> CollectionMark {
    CollectionMark {
        schema_version: MARK_SCHEMA_VERSION,
        kind,
        platform_id: "snes".to_owned(),
        region: "usa".to_owned(),
        name: "Super Mario World (Kaizo Edition).sfc".to_owned(),
        parent_work_id: String::new(),
        parent_dat_name: parent_dat_name.to_owned(),
        content: MarkedContent {
            size: 1_048_576,
            crc32: "aabbccdd".to_owned(),
            sha1: "da39a3ee5e6b4b0d3255bfef95601890afd80709".to_owned(),
            md5: String::new(),
        },
        note: String::new(),
    }
}

/// The whole point: a mod's bytes have never been in any scraper's database,
/// so offering them spends a request to learn nothing. The parent's identity
/// goes out instead.
#[test]
fn a_mod_is_never_offered_its_own_digests() {
    let parent = ParentIdentity {
        filename: "Super Mario World (USA).sfc".to_owned(),
        file_size: 524_288,
        serial: String::new(),
        hashes: Some(RomHashes {
            crc32: "11223344".to_owned(),
            md5: "ffffffffffffffffffffffffffffffff".to_owned(),
            sha1: "1111111111111111111111111111111111111111".to_owned(),
        }),
    };
    let asked = Derivation::Parent(parent.clone())
        .identify(&own_identity())
        .unwrap();

    assert_eq!(asked.hashes, parent.hashes);
    assert_eq!(asked.filename, "Super Mario World (USA).sfc");
    assert_eq!(asked.file_size, 524_288);
    assert!(!asked.expects_serial);
}

/// A ROM hack normally leaves the original header alone, so the file's own
/// serial identifies the parent. It is kept only when the catalog offers
/// nothing better.
#[test]
fn a_mods_own_serial_stands_in_for_a_parent_the_catalog_cannot_name() {
    let unnamed_parent = ParentIdentity {
        filename: "Super Mario World (USA)".to_owned(),
        ..ParentIdentity::default()
    };
    let asked = Derivation::Parent(unnamed_parent)
        .identify(&own_identity())
        .unwrap();
    assert_eq!(asked.serial, "SNS-MW-USA");
    assert_eq!(asked.scraper_serial, "SNS-MW");

    let catalogued_parent = ParentIdentity {
        filename: "Super Mario World (USA)".to_owned(),
        serial: "SNS-MW-USA-1".to_owned(),
        ..ParentIdentity::default()
    };
    let asked = Derivation::Parent(catalogued_parent)
        .identify(&own_identity())
        .unwrap();
    assert_eq!(asked.serial, "SNS-MW-USA-1");
    // The adapted form is analyzer-derived from the file's own header; it does
    // not describe the serial that replaced it.
    assert!(asked.scraper_serial.is_empty());
}

/// A DAT game name is not a filename, and the tier it feeds matches ROM names.
/// The one place that can know the extension is the file itself.
#[test]
fn a_parent_dat_name_gains_the_derivatives_extension_without_losing_its_dots() {
    let parent = ParentIdentity {
        filename: "Dr. Mario (USA)".to_owned(),
        ..ParentIdentity::default()
    };
    let own = RomInfo {
        filename: "Dr. Mario (Hard Mode).nes".to_owned(),
        ..own_identity()
    };
    assert_eq!(
        Derivation::Parent(parent).identify(&own).unwrap().filename,
        "Dr. Mario (Hard Mode).nes".replace("(Hard Mode)", "(USA)")
    );

    // An already-complete ROM name is left alone rather than doubled.
    let parent = ParentIdentity {
        filename: "Dr. Mario (USA).nes".to_owned(),
        ..ParentIdentity::default()
    };
    assert_eq!(
        Derivation::Parent(parent).identify(&own).unwrap().filename,
        "Dr. Mario (USA).nes"
    );
}

/// Nobody assigned homebrew a serial, so whatever sits where one would be is a
/// placeholder — and a placeholder that collides with a commercial game's
/// serial would publish that game's artwork under a homebrew title.
#[test]
fn homebrew_is_asked_about_by_name_and_never_by_serial() {
    let asked = Derivation::Standalone.identify(&own_identity()).unwrap();
    assert!(asked.serial.is_empty());
    assert!(asked.scraper_serial.is_empty());
    assert!(!asked.expects_serial);
    // Its own digests are its own: a scraper that has this homebrew title
    // holds exactly these bytes.
    assert_eq!(asked.hashes, own_identity().hashes);
    assert_eq!(asked.filename, own_identity().filename);
}

/// "This is a mod" without "of what" is not an identity. Asking anyway is the
/// behavior this module exists to stop.
#[test]
fn a_mod_with_no_parent_has_nothing_to_ask() {
    assert_eq!(
        Derivation::from_mark(&mark(MarkKind::Modded, "")),
        Derivation::UnknownParent
    );
    assert!(
        Derivation::UnknownParent
            .identify(&own_identity())
            .is_none()
    );
    assert!(
        Derivation::Parent(ParentIdentity::default())
            .identify(&own_identity())
            .is_none()
    );
    assert!(Derivation::UnknownParent.note().is_some());
}

/// Without a catalog — a fresh machine, a Syncthing'd ROM tree — the mark
/// alone still names the parent, and a DAT game name is exactly what the
/// filename tier wants.
#[test]
fn a_mark_alone_carries_the_derivation() {
    let derivation = Derivation::from_mark(&mark(MarkKind::Modded, "Super Mario World (USA)"));
    assert_eq!(
        derivation,
        Derivation::Parent(ParentIdentity {
            filename: "Super Mario World (USA)".to_owned(),
            ..ParentIdentity::default()
        })
    );
    let asked = derivation.identify(&own_identity()).unwrap();
    assert_eq!(asked.filename, "Super Mario World (USA).sfc");
    assert!(asked.hashes.is_none());
    assert_eq!(asked.file_size, 0, "the parent's size is not the mod's");

    assert_eq!(
        Derivation::from_mark(&mark(MarkKind::Homebrew, "")),
        Derivation::Standalone
    );
}

/// An unmarked file is untouched. Every scrape runs through this, so the
/// no-decision path has to be an identity function.
#[test]
fn an_unmarked_file_is_asked_about_as_itself() {
    let own = own_identity();
    let asked = Derivation::Own.identify(&own).unwrap();
    assert_eq!(asked.serial, own.serial);
    assert_eq!(asked.filename, own.filename);
    assert_eq!(asked.hashes, own.hashes);
    assert_eq!(asked.file_size, own.file_size);
    assert!(asked.expects_serial);
    assert!(Derivation::Own.note().is_none());
}
