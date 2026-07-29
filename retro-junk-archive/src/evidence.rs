//! Canonical evidence-currency predicates.
//!
//! Evidence describes the dump manifest it was computed from; it is current
//! only while `input_manifest_sha256` still matches the dump's manifest hash.
//! Every consumer (projection reconcile, build prerequisites, identification,
//! CLI reporting) must agree on this rule, so it lives here exactly once.

use crate::index::IndexedDump;
use crate::manifest::{VerificationKind, VerificationOutcome};

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

/// Whether the dump is catalog-verified by current evidence covering the
/// complete track set.
///
/// A single matching track is deliberately insufficient for multi-track
/// discs; every current writer records `complete_track_set: true` only when
/// the whole ordered set (or the single file, for file-shaped masters)
/// matched one catalog medium.
#[must_use]
pub fn dump_catalog_verified(dump: &IndexedDump) -> bool {
    dump.verifications.iter().any(|verification| {
        verification.evidence.kind == VerificationKind::Catalog
            && verification.evidence.outcome == VerificationOutcome::Verified
            && verification.evidence.input_manifest_sha256 == dump.manifest_sha256
            && verification
                .evidence
                .catalog
                .as_ref()
                .is_some_and(|catalog| catalog.complete_track_set)
    })
}
