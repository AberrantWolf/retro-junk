//! Evidence-currency predicate behavior: stale hashes, wrong kinds, failed
//! outcomes, and incomplete track sets must all read as "not verified".

use std::path::PathBuf;

use crate::evidence::{dump_catalog_verified, dump_has_current_evidence};
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
