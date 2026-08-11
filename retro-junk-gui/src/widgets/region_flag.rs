//! Small vector region flags for the Library table.
//!
//! Unicode flag emoji are pairs of regional-indicator glyphs. egui's font
//! renderer does not shape those pairs into flags consistently, so they can
//! appear as two boxed letters depending on the platform. Drawing the tiny
//! marks here keeps them crisp, dependency-free, and identical everywhere.

use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};
use retro_junk_backend::store::LibraryRegionPresentation;
use retro_junk_core::Region;

const FLAG_SIZE: Vec2 = Vec2::new(18.0, 12.0);

pub fn show(ui: &mut egui::Ui, presentation: &LibraryRegionPresentation) {
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        for region in &presentation.regions {
            let (rect, response) = ui.allocate_exact_size(FLAG_SIZE, Sense::hover());
            paint(ui.painter(), rect, *region);
            response.on_hover_text(region.name());
        }
        ui.label(presentation.label());
    });
}

fn paint(painter: &egui::Painter, rect: Rect, region: Region) {
    let white = Color32::from_rgb(245, 245, 242);
    let red = Color32::from_rgb(196, 35, 45);
    let blue = Color32::from_rgb(31, 66, 135);
    painter.rect_filled(rect, 1.0, white);
    match region {
        Region::Japan => {
            painter.circle_filled(rect.center(), rect.height() * 0.29, red);
        }
        Region::Usa => {
            for stripe in 0..7 {
                let top = rect.top() + stripe as f32 * rect.height() / 7.0;
                painter.rect_filled(
                    Rect::from_min_size(
                        Pos2::new(rect.left(), top),
                        Vec2::new(rect.width(), rect.height() / 7.0),
                    ),
                    0.0,
                    if stripe % 2 == 0 { red } else { white },
                );
            }
            painter.rect_filled(
                Rect::from_min_size(
                    rect.min,
                    Vec2::new(rect.width() * 0.43, rect.height() * 0.54),
                ),
                0.0,
                blue,
            );
        }
        Region::Europe => {
            painter.rect_filled(rect, 1.0, Color32::from_rgb(20, 70, 155));
            let center = rect.center();
            for index in 0..8 {
                let angle = index as f32 * std::f32::consts::TAU / 8.0;
                painter.circle_filled(
                    center + Vec2::angled(angle) * 3.5,
                    0.65,
                    Color32::from_rgb(255, 205, 25),
                );
            }
        }
        Region::Australia => {
            painter.rect_filled(rect, 1.0, Color32::from_rgb(20, 45, 110));
            let canton = Rect::from_min_size(rect.min, rect.size() * 0.5);
            painter.line_segment(
                [canton.left_center(), canton.right_center()],
                Stroke::new(1.5, white),
            );
            painter.line_segment(
                [canton.center_top(), canton.center_bottom()],
                Stroke::new(1.5, white),
            );
            painter.circle_filled(rect.center() + Vec2::new(4.0, 1.0), 1.1, white);
        }
        Region::Korea => {
            painter.circle_filled(rect.center() + Vec2::new(0.0, -1.1), 2.7, red);
            painter.circle_filled(
                rect.center() + Vec2::new(0.0, 1.1),
                2.7,
                Color32::from_rgb(20, 75, 160),
            );
        }
        Region::China => {
            painter.rect_filled(rect, 1.0, Color32::from_rgb(220, 35, 40));
            painter.circle_filled(
                rect.left_top() + Vec2::new(4.2, 3.5),
                1.4,
                Color32::from_rgb(255, 220, 30),
            );
        }
        Region::Taiwan => {
            painter.rect_filled(rect, 1.0, Color32::from_rgb(220, 35, 45));
            let canton = Rect::from_min_size(rect.min, rect.size() * 0.5);
            painter.rect_filled(canton, 0.0, Color32::from_rgb(20, 55, 130));
            painter.circle_filled(canton.center(), 1.5, white);
        }
        Region::Brazil => {
            painter.rect_filled(rect, 1.0, Color32::from_rgb(35, 145, 70));
            let center = rect.center();
            painter.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(center.x, rect.top() + 1.5),
                    Pos2::new(rect.right() - 1.5, center.y),
                    Pos2::new(center.x, rect.bottom() - 1.5),
                    Pos2::new(rect.left() + 1.5, center.y),
                ],
                Color32::from_rgb(250, 205, 35),
                Stroke::NONE,
            ));
            painter.circle_filled(center, 2.2, Color32::from_rgb(25, 65, 145));
        }
        Region::Asia | Region::LatinAmerica | Region::World | Region::Unknown => {
            paint_globe(painter, rect, region);
        }
    }
}

fn paint_globe(painter: &egui::Painter, rect: Rect, region: Region) {
    let color = match region {
        Region::Asia => Color32::from_rgb(30, 135, 120),
        Region::LatinAmerica => Color32::from_rgb(45, 145, 80),
        Region::World => Color32::from_rgb(45, 105, 175),
        _ => Color32::from_gray(125),
    };
    let center = rect.center();
    let radius = rect.height() * 0.38;
    painter.circle_stroke(center, radius, Stroke::new(1.2, color));
    painter.line_segment(
        [
            Pos2::new(center.x - radius, center.y),
            Pos2::new(center.x + radius, center.y),
        ],
        Stroke::new(0.8, color),
    );
    painter.line_segment(
        [
            Pos2::new(center.x, center.y - radius),
            Pos2::new(center.x, center.y + radius),
        ],
        Stroke::new(0.8, color),
    );
}
