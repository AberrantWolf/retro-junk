//! One rule, so the builder, the rename planner, and the conformance check
//! cannot disagree about what a file is called.

use super::*;

fn catalog_disc(dat_name: &str, rom_name: &str) -> CanonicalName {
    CanonicalName {
        dat_name: dat_name.to_owned(),
        rom_name: rom_name.to_owned(),
        medium_has_tracks: true,
        ..Default::default()
    }
}

#[test]
fn a_whole_disc_takes_the_game_name_not_its_largest_track() {
    // The catalog's rom_name for a multi-track disc is a track file. Naming a
    // container after it produced "… (Track 1).chd", and the scraped artwork
    // and frontend entry inherited that name too.
    let inputs = catalog_disc(
        "Crash Team Racing (USA)",
        "Crash Team Racing (USA) (Track 1).bin",
    );
    let (stem, source) = canonical_stem(&inputs);
    assert_eq!(stem, "Crash Team Racing (USA)");
    assert_eq!(source, NameSource::Catalog);
    assert_eq!(
        canonical_filename(&inputs, "chd"),
        "Crash Team Racing (USA).chd"
    );
}

#[test]
fn a_single_file_medium_keeps_the_catalog_filename() {
    let inputs = CanonicalName {
        dat_name: "Super Mario Bros. (World)".to_owned(),
        rom_name: "Super Mario Bros. (World).nes".to_owned(),
        medium_has_tracks: false,
        ..Default::default()
    };
    assert_eq!(canonical_stem(&inputs).0, "Super Mario Bros. (World)");
}

#[test]
fn an_unbound_release_is_named_from_its_manifest_and_says_so() {
    let inputs = CanonicalName {
        title: "Some Import".to_owned(),
        region: "jpn".to_owned(),
        revision: "Rev 1".to_owned(),
        ..Default::default()
    };
    let (stem, source) = canonical_stem(&inputs);
    assert_eq!(source, NameSource::ArchiveManifest);
    // Region is stored lowercased and written the way a catalog writes it.
    assert!(stem.starts_with("Some Import ("), "{stem}");
    assert!(stem.contains("Rev 1"), "{stem}");
    assert!(!stem.contains("jpn"), "{stem}");
}

#[test]
fn a_disc_number_is_appended_only_for_a_multi_disc_release() {
    let mut inputs = catalog_disc(
        "Final Fantasy VII (USA)",
        "Final Fantasy VII (USA) (Track 1).bin",
    );
    inputs.disc_number = 2;
    inputs.expected_disc_count = 3;
    assert_eq!(
        canonical_stem(&inputs).0,
        "Final Fantasy VII (USA) (Disc 2)"
    );

    // The set's folder holds every disc, so it carries no disc suffix.
    assert_eq!(canonical_release_stem(&inputs), "Final Fantasy VII (USA)");

    // A single-disc release never gains one.
    inputs.expected_disc_count = 1;
    assert_eq!(canonical_stem(&inputs).0, "Final Fantasy VII (USA)");
}

#[test]
fn a_catalog_name_that_already_states_its_disc_is_not_given_a_second_one() {
    let mut inputs = catalog_disc(
        "Final Fantasy VII (USA) (Disc 2)",
        "Final Fantasy VII (USA) (Disc 2) (Track 1).bin",
    );
    inputs.disc_number = 2;
    inputs.expected_disc_count = 3;
    assert_eq!(
        canonical_stem(&inputs).0,
        "Final Fantasy VII (USA) (Disc 2)"
    );
}

#[test]
fn conformance_compares_stems_so_a_format_conversion_is_not_a_rename() {
    let inputs = catalog_disc(
        "Crash Team Racing (USA)",
        "Crash Team Racing (USA) (Track 1).bin",
    );
    assert!(name_conforms("Crash Team Racing (USA).chd", &inputs));
    assert!(name_conforms("Crash Team Racing (USA).cue", &inputs));
    // The old wrong name, which is exactly what needs finding.
    assert!(!name_conforms(
        "Crash Team Racing (USA) (Track 1).chd",
        &inputs
    ));
}

#[test]
fn a_provisional_name_never_condemns_an_existing_file() {
    // Without a catalog identity there is no authority to say a name is
    // wrong, so an unidentified release must not be reported as misnamed.
    let inputs = CanonicalName {
        title: "Whatever The User Called It".to_owned(),
        region: "usa".to_owned(),
        ..Default::default()
    };
    assert!(name_conforms("Something Else Entirely.chd", &inputs));
}

#[test]
fn release_stem_removes_only_catalog_disc_tag() {
    let input = catalog_disc(
        "Final Fantasy VIII (USA) (Rev 1) (Disc 2)",
        "Final Fantasy VIII (USA) (Rev 1) (Disc 2) (Track 1).bin",
    );
    assert_eq!(
        canonical_release_stem(&input),
        "Final Fantasy VIII (USA) (Rev 1)"
    );

    let input = catalog_disc(
        "Final Fantasy IX (USA, Canada) (Disc 3)",
        "Final Fantasy IX (USA, Canada) (Disc 3) (Track 1).bin",
    );
    assert_eq!(
        canonical_release_stem(&input),
        "Final Fantasy IX (USA, Canada)"
    );
}
