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
    assert!(none_wanted.is_complete());
    assert_eq!(none_wanted.level(), FractionLevel::Complete);
}

#[test]
fn unresolved_binding_is_attention_not_unbound() {
    // The old projection erased unresolvable bindings to NULL, making
    // "your catalog is out of date" indistinguishable from "never
    // identified". The fold must keep them apart.
    let mut completion = all_complete();
    completion.identity = Identity::BindingUnresolved {
        claimed: "psx:some-retitled-game:psx:usa".into(),
    };
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
    use retro_junk_db::facts::{CarrierFacts, ExpectedDiscs, ReleaseFacts};

    fn carrier(id: &str, copy: &str) -> CarrierFacts {
        CarrierFacts {
            carrier_id: id.into(),
            physical_copy_id: copy.into(),
            catalog_media_id: Some(format!("media-{id}")),
            claimed_media_id: format!("media-{id}"),
            disc_number: None,
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
            catalog_release_id: Some("psx:crash-team-racing:psx:usa".into()),
            catalog_work_id: Some("psx:crash-team-racing".into()),
            claimed_release_id: "psx:crash-team-racing:psx:usa".into(),
            claimed_work_id: String::new(),
            expected_discs: Some(ExpectedDiscs {
                count: 1,
                numbered: false,
            }),
            carriers: vec![carrier("c1", "copy-1")],
            desired_playables: 1,
            satisfied_playables: 1,
            missing_playables: 0,
            archived_asset_types: Vec::new(),
        }
    }

    fn no_assets() -> retro_junk_frontend::AssetSelection {
        retro_junk_frontend::AssetSelection { types: Vec::new() }
    }

    #[test]
    fn the_original_symptom_a_filename_titled_import_reads_unidentified_not_zero_of_zero() {
        // A ReadyUnbound import: title is the source filename, no claims.
        let mut unbound = facts();
        unbound.catalog_release_id = None;
        unbound.catalog_work_id = None;
        unbound.claimed_release_id = String::new();
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

    #[test]
    fn a_claim_the_catalog_lacks_reads_as_unresolved_with_the_claim_preserved() {
        let mut orphaned = facts();
        orphaned.catalog_release_id = None;
        orphaned.catalog_work_id = None;
        // claimed_release_id stays — the manifest still names the old id.
        let completion = Completion::for_release(&orphaned, &no_assets());
        assert_eq!(
            completion.identity,
            Identity::BindingUnresolved {
                claimed: "psx:crash-team-racing:psx:usa".into()
            }
        );
        assert_eq!(completion.overall(), Overall::NeedsAttention);
        assert!(
            completion
                .attention
                .contains(&Attention::BindingUnresolved {
                    claimed: "psx:crash-team-racing:psx:usa".into()
                })
        );
    }

    #[test]
    fn fully_verified_single_disc_release_is_complete() {
        let completion = Completion::for_release(&facts(), &no_assets());
        assert_eq!(completion.catalog, Fraction::known(1, 1));
        assert_eq!(completion.overall(), Overall::Complete);
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
