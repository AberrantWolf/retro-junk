use std::cell::RefCell;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::{STATUS_OK, show_results_dialog};

#[test]
fn dialog_renders_summary_and_dismisses_on_ok() {
    let results: RefCell<Option<Vec<&'static str>>> = RefCell::new(Some(vec!["a", "b"]));

    let mut harness = Harness::new_ui(|ui| {
        let mut opt = results.borrow_mut();
        show_results_dialog(
            ui.ctx(),
            "Test Results",
            &mut opt,
            |items| format!("{} items", items.len()),
            |ui, item| {
                ui.colored_label(STATUS_OK, "Row status");
                ui.label(*item);
            },
        );
    });
    harness.run();

    // Summary line and both rows render.
    harness.get_by_label("2 items");
    harness.get_by_label("a");
    harness.get_by_label("b");

    // Clicking the dialog's OK button clears the Option (dismisses it).
    harness.get_by_label("OK").click();
    harness.run();

    assert!(
        results.borrow().is_none(),
        "clicking OK should clear the results option"
    );
}

#[test]
fn no_dialog_when_results_is_none() {
    let results: RefCell<Option<Vec<&'static str>>> = RefCell::new(None);

    let mut harness = Harness::new_ui(|ui| {
        let mut opt = results.borrow_mut();
        show_results_dialog(
            ui.ctx(),
            "Test Results",
            &mut opt,
            |items| format!("{} items", items.len()),
            |ui, item| {
                ui.label(*item);
            },
        );
    });
    harness.run();

    assert!(results.borrow().is_none());
}
