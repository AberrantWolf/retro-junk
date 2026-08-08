use crate::state::{OperationProgress, OperationUnit};

use super::format_progress;

fn progress(unit: OperationUnit, completed: u64, total: u64) -> OperationProgress {
    OperationProgress::Determinate {
        completed,
        total,
        unit,
    }
}

#[test]
fn count_formats_as_fraction() {
    assert_eq!(
        format_progress(&progress(OperationUnit::Items, 3, 10)),
        "3/10"
    );
}

#[test]
fn bytes_formats_with_units() {
    let text = format_progress(&progress(
        OperationUnit::Bytes,
        1024 * 1024,
        10 * 1024 * 1024,
    ));
    assert!(text.contains("MB"), "expected byte units in {text:?}");
    assert!(text.contains('/'));
}

#[test]
fn indeterminate_has_no_numeric_label() {
    assert_eq!(format_progress(&OperationProgress::Indeterminate), "");
}
