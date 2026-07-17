use std::cell::RefCell;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::{STATUS_OK, show_results_dialog};

#[test]
fn dialog_renders_summary_and_dismisses_on_ok() {
    let dismissed: RefCell<bool> = RefCell::new(false);
    let items = ["a", "b"];

    let mut harness = Harness::new_ui(|ui| {
        let result = show_results_dialog(
            ui.ctx(),
            "Test Results",
            &items,
            |items| format!("{} items", items.len()),
            |ui, item| {
                ui.colored_label(STATUS_OK, "Row status");
                ui.label(*item);
            },
        );
        if result {
            *dismissed.borrow_mut() = true;
        }
    });
    harness.run();

    // Summary line and both rows render.
    harness.get_by_label("2 items");
    harness.get_by_label("a");
    harness.get_by_label("b");

    // Clicking the dialog's OK button reports dismissal to the caller.
    harness.get_by_label("OK").click();
    harness.run();

    assert!(
        *dismissed.borrow(),
        "clicking OK should report the dialog as dismissed"
    );
}
