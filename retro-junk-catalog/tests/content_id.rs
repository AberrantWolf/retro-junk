//! What a content id must and must not do.
//!
//! These guard the properties the rest of the catalog leans on: the same bytes
//! always name the same row, different bytes never do, and the title has no
//! say in it at all.

use retro_junk_catalog::content_id::{
    self, ContentIdError, ContentPart, MEDIA_PREFIX, RELEASE_PREFIX, WORK_PREFIX,
};

fn track(sha1: &str, size: u64) -> ContentPart {
    ContentPart::new(size, sha1)
}

const DISC_TRACK_1: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DISC_TRACK_2: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[test]
fn same_digests_give_the_same_id_every_time() {
    let first =
        content_id::media_id(&[track(DISC_TRACK_1, 100), track(DISC_TRACK_2, 200)]).unwrap();
    let again =
        content_id::media_id(&[track(DISC_TRACK_1, 100), track(DISC_TRACK_2, 200)]).unwrap();
    assert_eq!(first, again);
}

#[test]
fn digest_case_does_not_change_the_id() {
    let lower = content_id::media_id(&[track(DISC_TRACK_1, 100)]).unwrap();
    let upper = content_id::media_id(&[track(&DISC_TRACK_1.to_uppercase(), 100)]).unwrap();
    assert_eq!(lower, upper);
}

#[test]
fn a_changed_track_hash_changes_the_id() {
    let original = content_id::media_id(&[track(DISC_TRACK_1, 100)]).unwrap();
    let redumped = content_id::media_id(&[track(DISC_TRACK_2, 100)]).unwrap();
    assert_ne!(original, redumped);
}

#[test]
fn a_changed_track_size_changes_the_id() {
    let original = content_id::media_id(&[track(DISC_TRACK_1, 100)]).unwrap();
    let longer = content_id::media_id(&[track(DISC_TRACK_1, 101)]).unwrap();
    assert_ne!(original, longer);
}

#[test]
fn track_order_is_part_of_the_identity() {
    let forwards =
        content_id::media_id(&[track(DISC_TRACK_1, 100), track(DISC_TRACK_2, 200)]).unwrap();
    let backwards =
        content_id::media_id(&[track(DISC_TRACK_2, 200), track(DISC_TRACK_1, 100)]).unwrap();
    assert_ne!(forwards, backwards);
}

/// Without a separator between parts, `("ab", "c")` and `("a", "bc")` would
/// hash the same bytes and collide.
#[test]
fn parts_cannot_be_confused_with_each_other() {
    let split_early = content_id::media_id(&[track(DISC_TRACK_1, 1), track(DISC_TRACK_2, 23)]);
    let split_late = content_id::media_id(&[track(DISC_TRACK_1, 12), track(DISC_TRACK_2, 3)]);
    assert_ne!(split_early.unwrap(), split_late.unwrap());
}

#[test]
fn one_file_and_one_track_of_the_same_bytes_are_the_same_medium() {
    let as_file = content_id::media_id_from_file(100, DISC_TRACK_1).unwrap();
    let as_track = content_id::media_id(&[track(DISC_TRACK_1, 100)]).unwrap();
    assert_eq!(as_file, as_track);
}

#[test]
fn an_empty_file_can_never_become_an_id() {
    let zero_length = content_id::media_id_from_file(0, DISC_TRACK_1);
    assert!(matches!(
        zero_length,
        Err(ContentIdError::EmptyContent { index: 0 })
    ));
    let empty_digest =
        content_id::media_id_from_file(4, "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    assert!(matches!(
        empty_digest,
        Err(ContentIdError::EmptyContent { index: 0 })
    ));
}

#[test]
fn a_track_with_no_digest_refuses_rather_than_guessing() {
    let missing = content_id::media_id(&[track(DISC_TRACK_1, 100), track("", 200)]);
    assert!(matches!(
        missing,
        Err(ContentIdError::MissingDigest { index: 1 })
    ));
    assert!(matches!(
        content_id::media_id(&[]),
        Err(ContentIdError::NoParts)
    ));
}

/// Works and releases are minted, not folded, so two calls must differ — that
/// is what makes "find the existing row, or mint a new one" a real choice.
#[test]
fn minted_ids_are_never_reused() {
    assert_ne!(content_id::new_work_id(), content_id::new_work_id());
    assert_ne!(content_id::new_release_id(), content_id::new_release_id());
}

/// The kinds are domain-separated: a release holding exactly one medium must
/// not land on that medium's id and fight it for a primary key.
#[test]
fn the_three_kinds_are_told_apart_by_their_prefix() {
    let media = content_id::media_id(&[track(DISC_TRACK_1, 100)]).unwrap();
    assert!(media.starts_with(MEDIA_PREFIX));
    assert!(content_id::new_release_id().starts_with(RELEASE_PREFIX));
    assert!(content_id::new_work_id().starts_with(WORK_PREFIX));
    assert!(!media.starts_with(RELEASE_PREFIX));
}

#[test]
fn rendered_ids_are_a_fixed_readable_shape() {
    for id in [
        content_id::media_id(&[track(DISC_TRACK_1, 100)]).unwrap(),
        content_id::new_release_id(),
        content_id::new_work_id(),
    ] {
        assert_eq!(id.len(), 4 + 16, "{id} should be a prefix plus 16 chars");
        assert!(content_id::is_content_id(&id), "{id} should be recognised");
        let body = &id[4..];
        assert!(
            !body.contains(['I', 'L', 'O', 'U']),
            "{id} used a letter Crockford base32 excludes"
        );
    }
}

#[test]
fn a_slug_is_not_mistaken_for_an_id() {
    assert!(!content_id::is_content_id("ps1:biohazard-3"));
    assert!(!content_id::is_content_id("rel-something"));
    assert!(!content_id::is_content_id("med_TOOSHORT"));
    assert!(!content_id::is_content_id("med_IIIIIIIIIIIIIIII"));
}
