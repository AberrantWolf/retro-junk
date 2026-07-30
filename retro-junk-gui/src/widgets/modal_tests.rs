use std::cell::Cell;

use egui_kittest::Harness;

use super::show;

/// Escape closes a dialog. The `egui::Window`-based dialogs this scaffold
/// replaced each decided for themselves whether Escape did anything (the
/// error and results dialogs ignored it entirely), so this is the behaviour
/// the shared scaffold exists to make uniform.
#[test]
fn escape_dismisses_the_dialog() {
    let dismissed: Cell<bool> = Cell::new(false);

    let mut harness = Harness::new_ui(|ui| {
        let outcome = show(ui.ctx(), "test_modal", "Test", 300.0, |ui| {
            ui.label("body");
        });
        if outcome.dismissed {
            dismissed.set(true);
        }
    });
    harness.run();
    assert!(
        !dismissed.get(),
        "the dialog must stay open until dismissed"
    );

    harness.key_press(egui::Key::Escape);
    harness.run();

    assert!(dismissed.get(), "Escape should dismiss the dialog");
}
