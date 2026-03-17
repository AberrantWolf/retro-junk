use crate::log_capture;

/// Color for a log level.
fn level_color(level: log::Level) -> egui::Color32 {
    match level {
        log::Level::Error => egui::Color32::from_rgb(220, 50, 50),
        log::Level::Warn => egui::Color32::from_rgb(230, 160, 30),
        log::Level::Info => egui::Color32::PLACEHOLDER, // sentinel: use default text color
        log::Level::Debug => egui::Color32::from_rgb(140, 140, 140),
        log::Level::Trace => egui::Color32::from_rgb(100, 100, 100),
    }
}

/// Render the status bar. Returns `true` if the user clicked it (toggle log viewer).
pub fn show(ctx: &egui::Context) -> bool {
    let mut clicked = false;

    egui::TopBottomPanel::bottom("status_bar")
        .exact_height(22.0)
        .show(ctx, |ui| {
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
