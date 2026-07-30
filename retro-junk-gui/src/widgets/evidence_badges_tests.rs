use super::EvidenceLevel;

/// A release with nothing expected of a class must read "not expected", not
/// "complete": a cartridge release has no desired playable conversion, and
/// claiming its playable evidence is complete would be a lie the convergence
/// matrix explicitly forbids.
#[test]
fn nothing_expected_is_not_applicable_rather_than_complete() {
    assert_eq!(EvidenceLevel::of(0, 0), EvidenceLevel::NotApplicable);
    assert_eq!(EvidenceLevel::of(3, 0), EvidenceLevel::NotApplicable);
}

#[test]
fn partial_evidence_never_reads_as_complete() {
    assert_eq!(EvidenceLevel::of(0, 2), EvidenceLevel::Absent);
    assert_eq!(EvidenceLevel::of(1, 2), EvidenceLevel::Partial);
    assert_eq!(EvidenceLevel::of(2, 2), EvidenceLevel::Complete);
    // More evidence than expected (an extra physical copy verified) is
    // still complete, not an error state.
    assert_eq!(EvidenceLevel::of(3, 2), EvidenceLevel::Complete);
}
