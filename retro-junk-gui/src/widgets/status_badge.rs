use crate::state::EntryStatus;

/// Draw a small colored circle indicating the entry's status.
/// Returns the response for tooltip handling.
pub fn show(ui: &mut egui::Ui, status: EntryStatus) -> egui::Response {
    let color = status.color();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().circle_filled(rect.center(), 4.0, color);
    }
    response
}

/// Draw a status circle with an optional orange warning triangle for broken references.
pub fn show_with_warning(
    ui: &mut egui::Ui,
    status: EntryStatus,
    has_broken_refs: bool,
) -> egui::Response {
    let width = if has_broken_refs { 20.0 } else { 10.0 };
    let color = status.color();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 10.0), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        let circle_center = egui::pos2(rect.left() + 5.0, rect.center().y);
        ui.painter().circle_filled(circle_center, 4.0, color);

        if has_broken_refs {
            let tri_center = egui::pos2(rect.left() + 15.0, rect.center().y);
            paint_warning_triangle(ui.painter(), tri_center, 4.5);
        }
    }
    response
}

/// Paint a small orange warning triangle (equilateral, pointing up).
fn paint_warning_triangle(painter: &egui::Painter, center: egui::Pos2, half_size: f32) {
    let color = egui::Color32::from_rgb(230, 160, 30);
    // Equilateral triangle vertices centered on `center`
    let top = egui::pos2(center.x, center.y - half_size);
    let bottom_left = egui::pos2(center.x - half_size, center.y + half_size * 0.6);
    let bottom_right = egui::pos2(center.x + half_size, center.y + half_size * 0.6);

    painter.add(egui::Shape::convex_polygon(
        vec![top, bottom_right, bottom_left],
        color,
        egui::Stroke::NONE,
    ));
}
