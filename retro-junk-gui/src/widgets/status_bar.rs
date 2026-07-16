use crate::log_capture;
use crate::theme::log_level_color as level_color;

/// Render the status bar. Returns `true` if the user clicked it (toggle log viewer).
pub fn show(ui: &mut egui::Ui) -> bool {
    let mut clicked = false;

    egui::Panel::bottom("status_bar")
        .exact_size(22.0)
        .show(ui, |ui| {
            // Make the entire panel area clickable, not just the labels.
            ui.set_min_width(ui.available_width());
            let rect = ui.max_rect();

            ui.horizontal_centered(|ui| {
                if let Some(entry) = log_capture::latest() {
                    let time = entry.timestamp.format("%H:%M:%S").to_string();
                    ui.label(
                        egui::RichText::new(time)
                            .small()
                            .color(egui::Color32::from_rgb(140, 140, 140)),
                    );

                    let lvl_color = level_color(entry.level);
                    let level_text = egui::RichText::new(entry.level.as_str()).small();
                    let level_text = if lvl_color == egui::Color32::PLACEHOLDER {
                        level_text
                    } else {
                        level_text.color(lvl_color)
                    };
                    ui.label(level_text);

                    let msg_text = egui::RichText::new(&entry.message).small();
                    let msg_text = if lvl_color == egui::Color32::PLACEHOLDER {
                        msg_text
                    } else {
                        msg_text.color(lvl_color)
                    };
                    ui.label(msg_text);
                } else {
                    ui.label(egui::RichText::new("Ready").small());
                }
            });

            // Overlay a click sense on the full panel rect so clicks anywhere work,
            // even on empty space or between labels.
            let response = ui.interact(rect, ui.id().with("click"), egui::Sense::click());
            if response.clicked() {
                clicked = true;
            }
        });

    clicked
}
