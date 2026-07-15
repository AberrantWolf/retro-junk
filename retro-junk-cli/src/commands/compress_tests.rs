//! Tests for the `compress` command's planning integration.
//!
//! `plan_batch` itself (skip classification, duplicate-output rejection,
//! already-CHD counting) is exercised thoroughly in
//! `retro-junk-lib/src/tests/chd_convert_tests.rs` against a stub analyzer.
//! The test here instead confirms the CLI-relevant wiring end to end: a real
//! platform analyzer (PSP) correctly classifies its declared-unconvertible
//! extension (`.cso`) through `plan_batch`, which is what E1's "report skips
//! instead of silence" fix relies on to explain an all-`.cso` folder.

use std::fs;

use retro_junk_lib::chd_convert::{ChdConvertError, SourceSkipClass, plan_batch};
use retro_junk_sony::PspAnalyzer;

#[test]
fn plan_batch_classifies_psp_cso_as_unreadable_container() {
    let dir = tempfile::tempdir().unwrap();
    let cso_paths: Vec<_> = (0..3)
        .map(|i| {
            let path = dir.path().join(format!("game{i}.cso"));
            fs::write(&path, [0u8; 16]).unwrap();
            path
        })
        .collect();

    let batch = plan_batch(&cso_paths, &PspAnalyzer);

    assert!(batch.jobs.is_empty(), "no .cso file is convertible");
    assert_eq!(batch.already_chd, 0);
    assert_eq!(
        batch.skips.len(),
        cso_paths.len(),
        "every .cso file should surface as a skip, not silence"
    );
    for skip in &batch.skips {
        match &skip.error {
            ChdConvertError::UnsupportedSource { class, .. } => {
                assert_eq!(*class, SourceSkipClass::UnreadableContainer);
            }
            other => panic!("expected UnsupportedSource, got {other:?}"),
        }
        // The Display text is what the CLI prints verbatim as the warn line;
        // confirm it names the platform and carries the class hint.
        let msg = skip.error.to_string();
        assert!(msg.contains("PlayStation Portable"), "got: {msg}");
        assert!(msg.contains(".cso"), "got: {msg}");
        assert!(msg.contains("container format"), "got: {msg}");
    }
}
