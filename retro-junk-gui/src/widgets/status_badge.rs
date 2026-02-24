use crate::state::EntryStatus;

/// Draw a small colored circle indicating the entry's status.
pub fn show(ui: &mut egui::Ui, status: EntryStatus) {
    let color = status.color();
    let (rect, _response) = ui.allocate_exact_size(
        egui::vec2(10.0, 10.0),
        egui::Sense::hover(),
    );
    if ui.is_rect_visible(rect) {
        ui.painter().circle_filled(rect.center(), 4.0, color);
    }
}
