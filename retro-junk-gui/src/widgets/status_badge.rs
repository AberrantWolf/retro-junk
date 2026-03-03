use crate::state::{AssetStatus, EntryStatus};

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

/// Draw a status circle with an optional orange warning triangle for broken references
/// and a small colored square indicating media status.
pub fn show_with_warning(
    ui: &mut egui::Ui,
    status: EntryStatus,
    has_broken_refs: bool,
    media_status: AssetStatus,
) -> egui::Response {
    let show_media = !matches!(media_status, AssetStatus::Unknown);
    let width =
        10.0 + if has_broken_refs { 10.0 } else { 0.0 } + if show_media { 10.0 } else { 0.0 };
    let color = status.color();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 10.0), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        let mut x = rect.left() + 5.0;

        // Status circle
        ui.painter()
            .circle_filled(egui::pos2(x, rect.center().y), 4.0, color);
        x += 10.0;

        // Warning triangle for broken references
        if has_broken_refs {
            paint_warning_triangle(ui.painter(), egui::pos2(x, rect.center().y), 4.5);
            x += 10.0;
        }

        // Media status square
        if show_media {
            let center = egui::pos2(x, rect.center().y);
            let half = 3.0;
            let sq_rect = egui::Rect::from_center_size(center, egui::vec2(half * 2.0, half * 2.0));
            match media_status {
                AssetStatus::None => {
                    // Hollow dim square
                    let stroke_color = ui.visuals().text_color().linear_multiply(0.25);
                    ui.painter()
                        .rect_stroke(sq_rect, 0.0, egui::Stroke::new(1.0, stroke_color));
                }
                AssetStatus::Partial { .. } => {
                    // Orange/yellow filled square
                    ui.painter()
                        .rect_filled(sq_rect, 0.0, egui::Color32::from_rgb(230, 160, 30));
                }
                AssetStatus::Complete => {
                    // Green filled square
                    ui.painter()
                        .rect_filled(sq_rect, 0.0, egui::Color32::from_rgb(50, 180, 50));
                }
                AssetStatus::Unknown => unreachable!(),
            }
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
