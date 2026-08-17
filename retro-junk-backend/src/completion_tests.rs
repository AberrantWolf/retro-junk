//! Guards for the contradictions the old status scheme actually shipped:
//! a bare "0 / 0" rendered as if it were a measurement, and a gray overall
//! icon beside all-green evidence.

use super::*;

fn bound() -> Identity {
    Identity::Bound {
        release_id: "psx:crash-team-racing:psx:usa".into(),
    }
}

fn all_complete() -> Completion {
    Completion {
        identity: bound(),
        presence: Fraction::known(2, 2),
        integrity: Fraction::known(2, 2),
        catalog: Fraction::known(2, 2),
        playable: Fraction::known(1, 1),
        artwork: Fraction::known(3, 3),
        missing_artwork: Vec::new(),
        attention: Vec::new(),
    }
}

#[test]
fn an_unmeasurable_fraction_never_renders_as_a_ratio() {
    let unknown = Fraction::Unknown(UnknownReason::NotCatalogBound);
    assert!(!unknown.describe().contains('0'), "{}", unknown.describe());
    assert!(!unknown.is_complete());
}

#[test]
fn zero_expected_is_nothing_expected_not_zero_of_zero() {
    let none_wanted = Fraction::known(0, 0);
    assert_eq!(none_wanted.describe(), "nothing expected");
    // It does not hold back the overall state...
    assert!(none_wanted.is_complete());
    // ...but it must not be painted as gathered evidence either. A cartridge
    // release asks for no disc conversion; a green dot would claim a
    // verification nobody ever ran.
    assert_eq!(none_wanted.level(), FractionLevel::NotApplicable);
    assert_eq!(Fraction::known(3, 0).level(), FractionLevel::NotApplicable);
}

#[test]
fn extra_evidence_beyond_what_is_expected_still_reads_complete() {
    // A second physical copy verified is not an error state.
    assert_eq!(Fraction::known(3, 2).level(), FractionLevel::Complete);
    assert!(Fraction::known(3, 2).is_complete());
}

#[test]
fn missing_artwork_is_reported_without_demoting_the_archive() {
    let mut completion = all_complete();
    completion.artwork = Fraction::known(1, 3);
    completion.missing_artwork = vec![
        retro_junk_frontend::AssetType::Video,
        retro_junk_frontend::AssetType::PhysicalMedia,
    ];
    assert_eq!(completion.artwork.level(), FractionLevel::Partial);
    assert_eq!(
        completion.missing_artwork,
        vec![
            retro_junk_frontend::AssetType::Video,
            retro_junk_frontend::AssetType::PhysicalMedia,
        ]
    );
    assert!(completion.incomplete_reasons().is_empty());
    assert_eq!(completion.overall(), Overall::Complete);
    assert_eq!(completion.severity(), Severity::Verified);
}

#[test]
fn content_that_matches_nothing_is_attention_not_unidentified() {
    // "A catalog checked these bytes and this machine lists none of them" is
    // not the same as "nobody ever looked", and the fold must keep them apart:
    // the first has a fix (get a catalog that covers it), the second has a
    // different one (identify it).
    let mut completion = all_complete();
    completion.identity = Identity::ContentUnmatched;
    assert_eq!(completion.overall(), Overall::NeedsAttention);

    completion.identity = Identity::Unknown;
    assert_eq!(completion.overall(), Overall::Unidentified);
}

#[test]
fn green_evidence_cannot_coexist_with_a_gray_icon() {
    // The old gray icon meant "expected_disc_count happened to be 0" even
    // when presence and integrity were fully green. Now: if every fraction
    // is complete and the identity is bound, the fold has no path to
    // anything but Complete.
    assert_eq!(all_complete().overall(), Overall::Complete);
}

#[test]
fn a_name_alone_is_not_an_identity() {
    // Serial/filename evidence produces a display name; it must not
    // produce a verified-looking row.
    let mut completion = all_complete();
    completion.identity = Identity::Named {
        name: "Crash Team Racing (USA)".into(),
    };
    assert_eq!(completion.overall(), Overall::Unidentified);
    assert_eq!(
        completion.identity.display_name(),
        Some("Crash Team Racing (USA)")
    );
}

#[test]
fn attention_outranks_complete_fractions() {
    let mut completion = all_complete();
    completion.attention.push(Attention::StaleName {
        representation_id: "rep-1".into(),
        current: "Old Name.chd".into(),
        canonical: "Crash Team Racing (USA).chd".into(),
    });
    assert_eq!(completion.overall(), Overall::NeedsAttention);
}

mod fold {
    use super::*;
    use retro_junk_db::facts::{CarrierFacts, ExpectedDiscs, PlayableNameFacts, ReleaseFacts};

    fn carrier(id: &str, copy: &str) -> CarrierFacts {
        CarrierFacts {
            carrier_id: id.into(),
            physical_copy_id: copy.into(),
            catalog_media_id: Some(format!("media-{id}")),
            disc_number: None,
            disc_designator: String::new(),
            masters_recorded: 1,
            masters_present: 1,
            integrity_verified: true,
            catalog_verified: true,
        }
    }

    fn facts() -> ReleaseFacts {
        ReleaseFacts {
            archive_release_id: "ar-1".into(),
            platform_id: "psx".into(),
            title: "Crash Team Racing (USA)".into(),
            region: "usa".into(),
            revision: String::new(),
            variant: String::new(),
            catalog_release_id: Some("psx:crash-team-racing:psx:usa".into()),
            catalog_work_id: Some("psx:crash-team-racing".into()),
            expected_discs: Some(ExpectedDiscs {
                count: 1,
                numbered: false,
            }),
            carriers: vec![carrier("c1", "copy-1")],
            desired_playables: 1,
            satisfied_playables: 1,
            missing_playables: 0,
            archived_asset_types: Vec::new(),
            playable_names: Vec::new(),
        }
    }

    fn no_assets() -> retro_junk_frontend::AssetSelection {
        retro_junk_frontend::AssetSelection { types: Vec::new() }
    }

    #[test]
    fn the_original_symptom_a_filename_titled_import_reads_unidentified_not_zero_of_zero() {
        // A ReadyUnbound import: the title is the source filename, and nothing
        // has ever been checked against a catalog.
        let mut unbound = facts();
        unbound.catalog_release_id = None;
        unbound.catalog_work_id = None;
        unbound.carriers[0].catalog_verified = false;
        let completion = Completion::for_release(&unbound, &no_assets());
        assert_eq!(
            completion.identity,
            Identity::Named {
                name: "Crash Team Racing (USA)".into()
            }
        );
        assert_eq!(
            completion.catalog,
            Fraction::Unknown(UnknownReason::NotCatalogBound)
        );
        assert_eq!(completion.overall(), Overall::Unidentified);
        // The rendering can never show "0 / 0": describe() explains instead.
        assert!(completion.catalog.describe().contains("identify"));
    }

    /// The archive verified this disc against a catalog. This machine's
    /// catalog has no entry with those bytes — a different situation from
    /// never having looked, and with a different fix.
    #[test]
    fn bytes_a_catalog_checked_and_this_machine_cannot_name_ask_for_attention() {
        let mut unmatched = facts();
        unmatched.catalog_release_id = None;
        unmatched.catalog_work_id = None;
        unmatched.carriers[0].catalog_media_id = None;
        // catalog_verified stays true: the check happened and came back empty.
        let completion = Completion::for_release(&unmatched, &no_assets());
        assert_eq!(completion.identity, Identity::ContentUnmatched);
        assert_eq!(completion.overall(), Overall::NeedsAttention);
        assert!(
            completion
                .attention
                .contains(&Attention::ContentUnmatched { carriers: 1 })
        );
        assert_eq!(
            completion.catalog,
            Fraction::Unknown(UnknownReason::ContentUnmatched)
        );
    }

    #[test]
    fn fully_verified_single_disc_release_is_complete_without_artwork() {
        let expected_artwork = retro_junk_frontend::AssetSelection::default();
        let completion = Completion::for_release(&facts(), &expected_artwork);
        assert_eq!(completion.catalog, Fraction::known(1, 1));
        assert_eq!(
            completion.artwork,
            Fraction::known(0, expected_artwork.types.len() as u64)
        );
        assert_eq!(completion.overall(), Overall::Complete);
        assert_eq!(completion.severity(), Severity::Verified);
    }

    #[test]
    fn a_complete_set_in_the_wrong_playlist_directory_is_malformed() {
        let mut set = facts();
        set.title = "Game".to_owned();
        set.variant = "Greatest".to_owned();
        set.expected_discs = Some(ExpectedDiscs {
            count: 2,
            numbered: true,
        });
        set.playable_names = [1, 2]
            .into_iter()
            .map(|disc_number| PlayableNameFacts {
                representation_id: format!("playable-{disc_number}"),
                relative_path: format!(
                    "psx/Wrong Name.m3u/Game (USA) (Greatest) (Disc {disc_number}).chd"
                ),
                dat_name: format!("Game (USA) (Greatest) (Disc {disc_number})"),
                rom_name: String::new(),
                medium_has_tracks: true,
                disc_number,
            })
            .collect();

        let completion = Completion::for_release(&set, &no_assets());

        assert!(completion.attention.iter().any(|attention| matches!(
            attention,
            Attention::MalformedPlayableLayout {
                repairable: true,
                ..
            }
        )));
    }

    #[test]
    fn verified_discs_take_the_best_copy_not_the_sum_of_copies() {
        let mut two_copies = facts();
        two_copies.expected_discs = Some(ExpectedDiscs {
            count: 2,
            numbered: true,
        });
        // Copy 1 holds verified disc 1; copy 2 holds verified disc 2.
        // Neither copy is complete, and the discs must not pool.
        let mut c1 = carrier("c1", "copy-1");
        c1.disc_number = Some(1);
        let mut c2 = carrier("c2", "copy-2");
        c2.disc_number = Some(2);
        two_copies.carriers = vec![c1, c2];
        let completion = Completion::for_release(&two_copies, &no_assets());
        assert_eq!(completion.catalog, Fraction::known(1, 2));
        assert_eq!(completion.overall(), Overall::Incomplete);
    }

    #[test]
    fn a_missing_master_cannot_vouch_for_its_disc_and_raises_attention() {
        let mut gone = facts();
        gone.carriers[0].masters_present = 0;
        let completion = Completion::for_release(&gone, &no_assets());
        assert_eq!(completion.catalog, Fraction::known(0, 1));
        assert_eq!(completion.presence, Fraction::known(0, 1));
        assert!(completion.attention.iter().any(|attention| matches!(
            attention,
            Attention::MasterMissing { carrier_id } if carrier_id == "c1"
        )));
        assert_eq!(completion.overall(), Overall::NeedsAttention);
    }
}

#[test]
fn incomplete_requires_a_real_measured_gap() {
    let mut completion = all_complete();
    completion.playable = Fraction::known(0, 1);
    assert_eq!(completion.overall(), Overall::Incomplete);
    assert_eq!(completion.playable.level(), FractionLevel::Empty);

    completion.playable = Fraction::known(1, 2);
    assert_eq!(completion.playable.level(), FractionLevel::Partial);
}

/// The general status column and completion-bearing evidence badges must be
/// incapable of disagreeing.
///
/// They can only disagree if the summary is computed *beside* the evidence
/// instead of *from* it. Folding with `worst` makes the summary unable to be
/// greener than its greenest part, so this is a property, not a spot check:
/// no combination of fractions can produce a summary better than the worst
/// archive/playable badge shown next to it. Artwork is intentionally excluded
/// because it is supplemental coverage, not archive completeness.
#[test]
fn a_summary_can_never_read_better_than_the_worst_evidence_beside_it() {
    use crate::completion::{Fraction, Identity, UnknownReason};

    let shapes = [
        Fraction::known(2, 2),
        Fraction::known(1, 2),
        Fraction::known(0, 2),
        Fraction::known(0, 0),
        Fraction::Unknown(UnknownReason::NotCatalogBound),
        Fraction::Unknown(UnknownReason::CatalogMissingMedia),
    ];
    for presence in shapes {
        for integrity in shapes {
            for catalog in shapes {
                let completion = Completion {
                    identity: Identity::Bound {
                        release_id: "r".to_owned(),
                    },
                    presence,
                    integrity,
                    catalog,
                    playable: Fraction::known(1, 1),
                    artwork: Fraction::known(1, 1),
                    missing_artwork: Vec::new(),
                    attention: Vec::new(),
                };
                let worst_badge = completion
                    .completion_evidence()
                    .iter()
                    .map(|fraction| fraction.level().severity())
                    .max()
                    .expect("evidence is never empty");
                assert!(
                    completion.severity() >= worst_badge,
                    "summary {:?} read better than its worst badge {worst_badge:?}",
                    completion.severity()
                );
            }
        }
    }
}

/// A disc with tracks it has not accounted for is never green or blue —
/// green would claim a check that did not happen, and blue is reserved for
/// what a person asserted.
#[test]
fn incomplete_evidence_is_never_painted_as_good() {
    use crate::completion::{Fraction, Identity, Severity};

    let completion = Completion {
        identity: Identity::Bound {
            release_id: "r".to_owned(),
        },
        presence: Fraction::known(1, 3),
        integrity: Fraction::known(1, 1),
        catalog: Fraction::known(1, 1),
        playable: Fraction::known(1, 1),
        artwork: Fraction::known(1, 1),
        missing_artwork: Vec::new(),
        attention: Vec::new(),
    };
    let severity = completion.severity();
    assert_ne!(severity, Severity::Verified);
    assert_ne!(severity, Severity::Asserted);
    assert_eq!(severity, Severity::Incomplete);
}
