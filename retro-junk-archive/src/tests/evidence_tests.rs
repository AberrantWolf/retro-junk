//! Evidence-currency predicate behavior: stale hashes, wrong kinds, failed
//! outcomes, and incomplete track sets must all read as "not verified".

use std::path::PathBuf;

use crate::evidence::{dump_catalog_attempted, dump_catalog_verified, dump_has_current_evidence};
use crate::index::{IndexedDump, IndexedVerification};
use crate::manifest::{
    CarrierId, CatalogEvidence, DumpManifest, RepresentationFormat, VerificationEvidence,
    VerificationId, VerificationKind, VerificationOutcome,
};

const CURRENT_SHA: &str = "aaaa";

fn dump_with(verifications: Vec<VerificationEvidence>) -> IndexedDump {
    let manifest = DumpManifest::new(CarrierId::new(), RepresentationFormat::Rom);
    IndexedDump {
        directory: PathBuf::from("/archive/dump"),
        manifest,
        manifest_sha256: CURRENT_SHA.to_owned(),
        verifications: verifications
            .into_iter()
            .map(|evidence| IndexedVerification {
                path: PathBuf::from("/archive/dump/evidence/v.json"),
                evidence,
            })
            .collect(),
        builds: Vec::new(),
    }
}

fn evidence(
    kind: VerificationKind,
    outcome: VerificationOutcome,
    input_sha: &str,
    catalog: Option<CatalogEvidence>,
) -> VerificationEvidence {
    VerificationEvidence {
        schema_version: crate::MANIFEST_SCHEMA_VERSION,
        verification_id: VerificationId::new(),
        representation_id: crate::manifest::RepresentationId::new(),
        performed_at: "2026-07-29T00:00:00Z".to_owned(),
        input_manifest_sha256: input_sha.to_owned(),
        kind,
        outcome,
        tool: None,
        catalog,
        tracks: Vec::new(),
        detail: String::new(),
    }
}

fn complete_catalog() -> CatalogEvidence {
    CatalogEvidence {
        source: "redump".to_owned(),
        system: "psx".to_owned(),
        version: "2026".to_owned(),
        game: "Example".to_owned(),
        complete_track_set: true,
    }
}

#[test]
fn current_verified_evidence_counts() {
    let dump = dump_with(vec![evidence(
        VerificationKind::Integrity,
        VerificationOutcome::Verified,
        CURRENT_SHA,
        None,
    )]);
    assert!(dump_has_current_evidence(
        &dump,
        VerificationKind::Integrity
    ));
    assert!(!dump_has_current_evidence(&dump, VerificationKind::Catalog));
}

#[test]
fn stale_input_manifest_is_history_not_current() {
    let dump = dump_with(vec![
        evidence(
            VerificationKind::Integrity,
            VerificationOutcome::Verified,
            "old-sha",
            None,
        ),
        evidence(
            VerificationKind::Catalog,
            VerificationOutcome::Verified,
            "old-sha",
            Some(complete_catalog()),
        ),
    ]);
    assert!(!dump_has_current_evidence(
        &dump,
        VerificationKind::Integrity
    ));
    assert!(!dump_catalog_verified(&dump));
}

#[test]
fn failed_outcome_never_verifies() {
    let dump = dump_with(vec![evidence(
        VerificationKind::Catalog,
        VerificationOutcome::Failed,
        CURRENT_SHA,
        Some(complete_catalog()),
    )]);
    assert!(!dump_has_current_evidence(&dump, VerificationKind::Catalog));
    assert!(!dump_catalog_verified(&dump));
}

/// Failing to match is not a reason to try again. Reproducing a disc costs a
/// full copy-and-split of its raw dump, and "no catalog match" is exactly the
/// state that makes a dump look unidentified — so without this, every
/// convergence run would redo that work for the same unmatchable disc forever.
#[test]
fn an_unmatched_conclusion_is_not_retried() {
    for outcome in [
        VerificationOutcome::Unmatched,
        VerificationOutcome::Ambiguous,
        VerificationOutcome::Verified,
    ] {
        let dump = dump_with(vec![evidence(
            VerificationKind::Catalog,
            outcome,
            CURRENT_SHA,
            None,
        )]);
        assert!(
            dump_catalog_attempted(&dump),
            "{outcome:?} is an answer about these bytes, not a reason to redo the work"
        );
    }
}

/// A broken reproduction is not an answer about the dump — redumper was
/// missing, the scratch disk filled, the run was cancelled — so the next run
/// gets to try again.
#[test]
fn a_broken_reproduction_stays_eligible() {
    let dump = dump_with(vec![evidence(
        VerificationKind::Catalog,
        VerificationOutcome::Failed,
        CURRENT_SHA,
        None,
    )]);
    assert!(!dump_catalog_attempted(&dump));
}

/// Changing the dump's bytes makes it a different question, so an earlier
/// conclusion stops applying.
#[test]
fn a_conclusion_about_other_bytes_does_not_count() {
    let dump = dump_with(vec![evidence(
        VerificationKind::Catalog,
        VerificationOutcome::Unmatched,
        "old-sha",
        None,
    )]);
    assert!(!dump_catalog_attempted(&dump));
}

/// Integrity verification says the stored bytes are intact; it says nothing
/// about which game they are.
#[test]
fn integrity_evidence_is_not_an_identification_attempt() {
    let dump = dump_with(vec![evidence(
        VerificationKind::Integrity,
        VerificationOutcome::Verified,
        CURRENT_SHA,
        None,
    )]);
    assert!(!dump_catalog_attempted(&dump));
}

#[test]
fn catalog_verification_requires_complete_track_set() {
    let partial = CatalogEvidence {
        complete_track_set: false,
        ..complete_catalog()
    };
    let dump = dump_with(vec![evidence(
        VerificationKind::Catalog,
        VerificationOutcome::Verified,
        CURRENT_SHA,
        Some(partial),
    )]);
    // Current + Verified + Catalog, but only a partial track match: the
    // generic predicate sees current evidence, the catalog predicate refuses.
    assert!(dump_has_current_evidence(&dump, VerificationKind::Catalog));
    assert!(!dump_catalog_verified(&dump));

    let dump = dump_with(vec![evidence(
        VerificationKind::Catalog,
        VerificationOutcome::Verified,
        CURRENT_SHA,
        Some(complete_catalog()),
    )]);
    assert!(dump_catalog_verified(&dump));
}

#[test]
fn missing_catalog_details_do_not_verify() {
    let dump = dump_with(vec![evidence(
        VerificationKind::Catalog,
        VerificationOutcome::Verified,
        CURRENT_SHA,
        None,
    )]);
    assert!(!dump_catalog_verified(&dump));
}

/// Build a dump of `format` holding `files` archived files.
fn shaped_dump(format: RepresentationFormat, files: usize) -> IndexedDump {
    let mut dump = dump_with(vec![evidence(
        VerificationKind::Catalog,
        VerificationOutcome::Verified,
        CURRENT_SHA,
        Some(CatalogEvidence {
            // The legacy shape: verified, but the flag was never written.
            complete_track_set: false,
            ..complete_catalog()
        }),
    )]);
    dump.manifest.format = format;
    dump.manifest.files = (0..files)
        .map(|index| crate::manifest::ArchivedFile {
            path: format!("file-{index}.bin"),
            size: 1,
            crc32: String::new(),
            md5: String::new(),
            sha1: String::new(),
            sha256: "s".to_owned(),
        })
        .collect();
    dump
}

/// A cartridge is one file and one "track", so a verified match against it was
/// always the complete set — the older verification path simply predated the
/// flag. 244 records on the reference archive were stuck unverified this way,
/// which blocked carrier re-resolution and hash adoption behind them. Their
/// shape proves what the flag failed to record, so nothing is rewritten.
#[test]
fn a_legacy_cartridge_verification_is_complete_by_its_shape() {
    assert!(dump_catalog_verified(&shaped_dump(
        RepresentationFormat::Rom,
        1
    )));
}

/// The narrowness is the point. A single-file ISO can be a data-track-only
/// image of a multi-track disc, matched on the medium's primary digests alone
/// — the exact case `complete_track_set` exists to catch, and the one non-`rom`
/// record in that population. A multi-file dump is not a cartridge either.
#[test]
fn only_a_single_file_cartridge_gets_that_treatment() {
    assert!(
        !dump_catalog_verified(&shaped_dump(RepresentationFormat::Iso, 1)),
        "a lone ISO may be one track of several"
    );
    assert!(
        !dump_catalog_verified(&shaped_dump(RepresentationFormat::Rom, 2)),
        "more than one file is not a cartridge dump"
    );
    assert!(
        !dump_catalog_verified(&shaped_dump(RepresentationFormat::CueBin, 1)),
        "a cue/bin set is a track set by definition"
    );
}
