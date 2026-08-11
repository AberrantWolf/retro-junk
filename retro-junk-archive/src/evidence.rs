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
    let directly_verified = dump.verifications.iter().any(|verification| {
        verification.evidence.kind == kind
            && verification.evidence.outcome == VerificationOutcome::Verified
            && verification.evidence.input_manifest_sha256 == dump.manifest_sha256
    });
    if directly_verified {
        return true;
    }

    // Complete catalog verification is also an integrity verification. It
    // reads the stored dump, hashes every logical track/file, and proves that
    // those hashes equal the catalog's expected hashes. Requiring a second
    // integrity-only event after that stronger operation made otherwise
    // verified imports permanently amber depending only on which verifier
    // happened to run first.
    kind == VerificationKind::Integrity && dump_catalog_verified(dump)
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
    dump.verifications
        .iter()
        .filter_map(|verification| {
            if verification.evidence.kind != VerificationKind::Catalog
                || verification.evidence.outcome != VerificationOutcome::Verified
                || verification.evidence.input_manifest_sha256 != dump.manifest_sha256
            {
                return None;
            }
            verification.evidence.catalog.as_ref().and_then(|catalog| {
                (catalog.complete_track_set || cartridge_set_is_trivially_complete(dump))
                    .then_some((verification, catalog))
            })
        })
        // Evidence is append-only history. Presentation and portable fallback
        // identity use the newest successful statement about these exact
        // bytes, not whichever filename sorts first on this filesystem.
        .max_by(|(left, _), (right, _)| {
            left.evidence
                .performed_at
                .cmp(&right.evidence.performed_at)
                .then_with(|| {
                    left.evidence
                        .verification_id
                        .to_string()
                        .cmp(&right.evidence.verification_id.to_string())
                })
        })
        .map(|(_, catalog)| catalog)
}

/// Whether this dump has no track set that a match could be incomplete about.
///
/// A cartridge is one file and one "track". The verification path that matched
/// it predates the `complete_track_set` flag and left it `false` — 244 records
/// on the reference archive, every one of them a `rom`-format single-file
/// master. Because the flag gates catalog-verification entirely, those dumps
/// read as unverified, which in turn blocked carrier re-resolution and hash
/// adoption behind them.
///
/// Deriving it from the dump's shape upgrades them without rewriting a single
/// evidence file: the archive is append-only history, and rewriting 244 records
/// to state something their shape already proves is churn, not evidence.
///
/// Deliberately narrow. `Iso` is excluded even when single-file, because a
/// data-track-only image of a multi-track disc is precisely the case where a
/// match *is* incomplete — the one non-`rom` record in that population is
/// exactly such a dump.
fn cartridge_set_is_trivially_complete(dump: &IndexedDump) -> bool {
    dump.manifest.format == crate::manifest::RepresentationFormat::Rom
        && dump.manifest.files.len() == 1
}

/// Whether the dump is catalog-verified by current evidence covering the
/// complete track set.
#[must_use]
pub fn dump_catalog_verified(dump: &IndexedDump) -> bool {
    dump_catalog_evidence(dump).is_some()
}

/// Whether identification already reached a conclusion about these exact bytes.
///
/// A match, "nothing in the catalog matches", and "several catalog media match"
/// are all conclusions. Only the first binds a carrier, but all three are
/// answers, and re-deriving them costs a full copy-and-split of the raw dump.
/// Without this, every convergence run would redo that work for any disc the
/// catalog cannot resolve — forever, because failing to match is exactly what
/// leaves a dump looking unidentified.
///
/// A `Failed` outcome is deliberately *not* a conclusion: it means the
/// reproduction itself broke (redumper missing, scratch disk full, cancelled
/// mid-run), so the next run should try again.
///
/// Conclusions are tied to `input_manifest_sha256`, so re-ingesting or
/// repairing the dump makes it eligible again. When the *catalog* is what
/// changed — a newly imported DAT that might now match — ask for a full pass
/// instead of a stale-only one.
#[must_use]
pub fn dump_catalog_attempted(dump: &IndexedDump) -> bool {
    dump.verifications.iter().any(|verification| {
        verification.evidence.kind == VerificationKind::Catalog
            && verification.evidence.input_manifest_sha256 == dump.manifest_sha256
            && matches!(
                verification.evidence.outcome,
                VerificationOutcome::Verified
                    | VerificationOutcome::Unmatched
                    | VerificationOutcome::Ambiguous
            )
    })
}
