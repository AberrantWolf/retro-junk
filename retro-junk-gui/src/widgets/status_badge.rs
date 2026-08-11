use crate::state::{AssetStatus, RowStatus};

/// Draw the shared status glyph for an already-folded aggregate.
pub fn show_severity(
    ui: &mut egui::Ui,
    severity: retro_junk_backend::completion::Severity,
) -> egui::Response {
    let color = crate::theme::severity_color(severity);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            crate::theme::severity_icon(severity),
            egui::FontId::proportional(11.0),
            color,
        );
    }
    response
}

/// Draw a status circle with optional orange warning triangles for broken references
/// and/or hash warnings, and a small colored square indicating artwork-set
/// coverage. The square is independent of the archive status glyph.
pub fn show_with_warning(
    ui: &mut egui::Ui,
    status: RowStatus,
    has_broken_refs: bool,
    has_hash_warnings: bool,
    artwork_status: AssetStatus,
) -> egui::Response {
    // Keep the indicator present and gray while filesystem discovery is in
    // flight, avoiding both layout shifts and a false claim about availability.
    let show_artwork = true;
    let show_warning = has_broken_refs || has_hash_warnings;
    let width =
        10.0 + if show_warning { 10.0 } else { 0.0 } + if show_artwork { 10.0 } else { 0.0 };
    let color = status.color();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 10.0), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        let mut x = rect.left() + 5.0;

        // Status glyph. The shape carries the state as well as the hue does:
        // green and red are the two most important states and are also the
        // most common colour blindness, and at this size hue is the least
        // reliable channel. `Incomplete` against `Unmeasured` depends on it
        // most — both read as "not done" by colour alone.
        ui.painter().text(
            egui::pos2(x, rect.center().y),
            egui::Align2::CENTER_CENTER,
            crate::theme::severity_icon(status.severity()),
            egui::FontId::proportional(11.0),
            color,
        );
        x += 10.0;

        // Warning triangle for broken references or hash warnings
        if show_warning {
            paint_warning_triangle(ui.painter(), egui::pos2(x, rect.center().y), 4.5);
            x += 10.0;
        }

        // Supplemental artwork-set square. This never changes the archive
        // status glyph painted above.
        if show_artwork {
            let center = egui::pos2(x, rect.center().y);
            let half = 3.0;
            let sq_rect = egui::Rect::from_center_size(center, egui::vec2(half * 2.0, half * 2.0));
            match artwork_status {
                AssetStatus::None => {
                    // Hollow dim square
                    let stroke_color = ui.visuals().text_color().linear_multiply(0.25);
                    ui.painter().rect_stroke(
                        sq_rect,
                        0.0,
                        egui::Stroke::new(1.0, stroke_color),
                        egui::StrokeKind::Middle,
                    );
                }
                AssetStatus::Partial { .. } => {
                    // Orange/yellow filled square
                    ui.painter()
                        .rect_filled(sq_rect, 0.0, crate::theme::STATUS_WARN_STRONG);
                }
                AssetStatus::Complete => {
                    // Green filled square
                    ui.painter()
                        .rect_filled(sq_rect, 0.0, crate::theme::STATUS_OK);
                }
                AssetStatus::Unknown => {
                    let gray = ui.visuals().text_color().linear_multiply(0.25);
                    ui.painter().rect_filled(sq_rect, 0.0, gray);
                }
            }
        }
    }
    response
}

/// Paint a small orange warning triangle (equilateral, pointing up).
fn paint_warning_triangle(painter: &egui::Painter, center: egui::Pos2, half_size: f32) {
    let color = crate::theme::STATUS_WARN_STRONG;
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
