//! A person's choice has to outlive a rename and a rebuilt database.

use super::*;

fn content(sha1: &str) -> MarkedContent {
    MarkedContent {
        size: 100,
        crc32: "aabbccdd".to_owned(),
        sha1: sha1.to_owned(),
        md5: String::new(),
    }
}

#[test]
fn a_choice_is_remembered_by_content_and_can_be_replaced_or_cleared() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let subject = content("aaaa");

    assert!(Disambiguations::load(root).unwrap().is_empty());

    choose(root, "ps1", &subject, "media-a", "Game A (Japan)").unwrap();
    let chosen = Disambiguations::load(root).unwrap();
    assert_eq!(chosen.chosen_for(&subject), Some("media-a"));

    // Re-choosing replaces rather than accumulating, so a person can correct
    // themselves without ending up with two live answers.
    choose(root, "ps1", &subject, "media-b", "Game B (USA)").unwrap();
    let chosen = Disambiguations::load(root).unwrap();
    assert_eq!(chosen.chosen_for(&subject), Some("media-b"));

    assert!(clear(root, "ps1", &subject).unwrap());
    assert!(Disambiguations::load(root).unwrap().is_empty());
}

/// The mark names the bytes, not the path, so renaming the file — which the
/// scan sees as a delete plus a create — does not lose the decision.
#[test]
fn a_choice_survives_the_file_being_renamed() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let subject = content("aaaa");
    choose(root, "ps1", &subject, "media-a", "Game A").unwrap();

    // Same bytes, and nothing in the lookup mentions a filename at all.
    let after_rename = content("aaaa");
    assert_eq!(
        Disambiguations::load(root)
            .unwrap()
            .chosen_for(&after_rename),
        Some("media-a")
    );
}

/// Different decisions about one file are independent. Naming marks on
/// platform and digest alone made the second one silently overwrite the first.
#[test]
fn a_disambiguation_does_not_clobber_another_decision_about_the_same_file() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let subject = content("aaaa");

    let region = CollectionMark {
        schema_version: retro_junk_archive::marks::MARK_SCHEMA_VERSION,
        kind: MarkKind::RegionOverride,
        platform_id: "ps1".to_owned(),
        region: "japan".to_owned(),
        name: "Game".to_owned(),
        parent_work_id: String::new(),
        parent_dat_name: String::new(),
        content: subject.clone(),
        chosen_media_id: String::new(),
        chosen_dat_name: String::new(),
        note: String::new(),
    };
    retro_junk_archive::write_mark(root, &region).unwrap();
    choose(root, "ps1", &subject, "media-a", "Game A").unwrap();

    let marks = retro_junk_archive::load_marks(root).unwrap();
    assert_eq!(
        marks.len(),
        2,
        "one decision about the file overwrote the other"
    );
    assert_eq!(
        Disambiguations::from_marks(&marks).chosen_for(&subject),
        Some("media-a")
    );
    assert!(
        marks
            .iter()
            .any(|mark| mark.kind == MarkKind::RegionOverride && mark.region == "japan"),
        "the region correction was lost"
    );
}
