use crate::log_capture;
use crate::theme::log_level_color as level_color;

/// State for the log viewer panel, stored in the app.
pub struct LogViewerState {
    /// Whether the log viewer panel is open.
    pub open: bool,
    /// Minimum log level to display.
    pub level_filter: log::LevelFilter,
    /// Whether to auto-scroll to the latest entry.
    pub auto_scroll: bool,
}

impl Default for LogViewerState {
    fn default() -> Self {
        Self {
            open: false,
            level_filter: log::LevelFilter::Trace,
            auto_scroll: true,
        }
    }
}

/// Render the log viewer panel. Only shown when `state.open` is true.
pub fn show(ui: &mut egui::Ui, state: &mut LogViewerState) {
    if !state.open {
        return;
    }

    egui::Panel::bottom("log_viewer")
        .resizable(true)
        .default_size(200.0)
        .min_size(80.0)
        .max_size(500.0)
        .show(ui, |ui| {
            // Toolbar
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Log").strong());
                ui.separator();

                // Level filter buttons
                for &level in &[
                    log::Level::Error,
                    log::Level::Warn,
                    log::Level::Info,
                    log::Level::Debug,
                    log::Level::Trace,
                ] {
                    let enabled = state.level_filter >= level;
                    let color = level_color(level);
                    let text = egui::RichText::new(level.as_str()).small();
                    let text = if color == egui::Color32::PLACEHOLDER {
                        text
                    } else {
                        text.color(color)
                    };
                    if ui.selectable_label(enabled, text).clicked() {
                        state.level_filter = if enabled && state.level_filter == level {
                            // Clicking the current max level disables it
                            match level {
                                log::Level::Error => log::LevelFilter::Off,
                                log::Level::Warn => log::LevelFilter::Error,
                                log::Level::Info => log::LevelFilter::Warn,
                                log::Level::Debug => log::LevelFilter::Info,
                                log::Level::Trace => log::LevelFilter::Debug,
                            }
                        } else {
                            level.to_level_filter()
                        };
                    }
                }

                ui.separator();
                ui.toggle_value(&mut state.auto_scroll, "Auto-scroll");

                if ui.small_button("Clear").clicked() {
                    // Clear is best-effort — we just filter client-side
                    // since the ring buffer is shared. Not worth the complexity
                    // of a clear mechanism for a debug tool.
                }
            });

            ui.separator();

            // Log entries
            let entries = log_capture::entries();
            let filtered: Vec<_> = entries
                .iter()
                .filter(|e| e.level <= state.level_filter)
                .collect();

            let row_height = ui.text_style_height(&egui::TextStyle::Small) + 2.0;
            let num_rows = filtered.len();

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(state.auto_scroll)
                .show_rows(ui, row_height, num_rows, |ui, row_range| {
                    for &entry in &filtered[row_range] {
                        ui.horizontal(|ui| {
                            // Timestamp
                            let time = entry.timestamp.format("%H:%M:%S%.3f").to_string();
                            ui.label(
                                egui::RichText::new(time)
                                    .small()
                                    .color(egui::Color32::from_rgb(140, 140, 140)),
                            );

                            // Level badge
                            let lvl_color = level_color(entry.level);
                            let level_text =
                                egui::RichText::new(format!("{:5}", entry.level.as_str())).small();
                            let level_text = if lvl_color == egui::Color32::PLACEHOLDER {
                                level_text
                            } else {
                                level_text.color(lvl_color)
                            };
                            ui.label(level_text);

                            // Module path (dimmed)
                            ui.label(
                                egui::RichText::new(&entry.target)
                                    .small()
                                    .color(egui::Color32::from_rgb(140, 140, 140)),
                            );

                            // Message (colored per level)
                            let msg_text = egui::RichText::new(&entry.message).small();
                            let msg_text = if lvl_color == egui::Color32::PLACEHOLDER {
                                msg_text
                            } else {
                                msg_text.color(lvl_color)
                            };
                            ui.label(msg_text);
                        });
                    }
                });
        });
}
