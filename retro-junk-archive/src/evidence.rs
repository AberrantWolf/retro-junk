//! Canonical evidence-currency predicates.
//!
//! Evidence describes the dump manifest it was computed from; it is current
//! only while `input_manifest_sha256` still matches the dump's manifest hash.
//! Every consumer (projection reconcile, build prerequisites, identification,
//! CLI reporting) must agree on this rule, so it lives here exactly once.

use crate::index::IndexedDump;
use crate::manifest::{CatalogEvidence, VerificationKind, VerificationOutcome};

/// Whether the dump has a current, successful verification of `kind`.
///
/// "Current" means the evidence was computed from the dump manifest as it
/// exists now. Stale evidence is history, not a claim about present bytes.
#[must_use]
pub fn dump_has_current_evidence(dump: &IndexedDump, kind: VerificationKind) -> bool {
    dump.verifications.iter().any(|verification| {
        verification.evidence.kind == kind
            && verification.evidence.outcome == VerificationOutcome::Verified
            && verification.evidence.input_manifest_sha256 == dump.manifest_sha256
    })
}

/// The current catalog evidence that makes this dump catalog-verified, if any.
///
/// A single matching track is deliberately insufficient for multi-track
/// discs; every current writer records `complete_track_set: true` only when
/// the whole ordered set (or the single file, for file-shaped masters)
/// matched one catalog medium.
///
/// This is the one place that decides what "catalog-verified" means. Callers
/// that only need the verdict use [`dump_catalog_verified`]; callers that need
/// the identity the catalog agreed on (game name, source, version) read it from
/// the returned evidence rather than re-deriving the rule.
#[must_use]
pub fn dump_catalog_evidence(dump: &IndexedDump) -> Option<&CatalogEvidence> {
    dump.verifications.iter().find_map(|verification| {
        if verification.evidence.kind != VerificationKind::Catalog
            || verification.evidence.outcome != VerificationOutcome::Verified
            || verification.evidence.input_manifest_sha256 != dump.manifest_sha256
        {
            return None;
        }
        verification
            .evidence
            .catalog
            .as_ref()
            .filter(|catalog| catalog.complete_track_set)
    })
}

/// Whether the dump is catalog-verified by current evidence covering the
/// complete track set.
#[must_use]
pub fn dump_catalog_verified(dump: &IndexedDump) -> bool {
    dump_catalog_evidence(dump).is_some()
}
