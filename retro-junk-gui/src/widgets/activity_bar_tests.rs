use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::state::{BackgroundOperation, OperationKind, ProgressDisplay};

use super::format_progress;

fn op(display: ProgressDisplay, current: u64, total: u64) -> BackgroundOperation {
    let mut op = BackgroundOperation::new(
        1,
        "test".to_string(),
        Arc::new(AtomicBool::new(false)),
        OperationKind::Other,
        None,
        display,
    );
    op.progress_current = current;
    op.progress_total = total;
    op
}

#[test]
fn count_formats_as_fraction() {
    assert_eq!(format_progress(&op(ProgressDisplay::Count, 3, 10)), "3/10");
}

#[test]
fn bytes_formats_with_units() {
    let text = format_progress(&op(ProgressDisplay::Bytes, 1024 * 1024, 10 * 1024 * 1024));
    assert!(text.contains("MB"), "expected byte units in {text:?}");
    assert!(text.contains('/'));
}

#[test]
fn percent_formats_as_rounded_percentage() {
    assert_eq!(
        format_progress(&op(ProgressDisplay::Percent, 42, 100)),
        "42%"
    );
}

#[test]
fn percent_with_zero_total_is_zero_percent() {
    assert_eq!(format_progress(&op(ProgressDisplay::Percent, 0, 0)), "0%");
}
