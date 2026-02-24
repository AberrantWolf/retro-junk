/// Render the Settings view (placeholder).
pub fn show(ui: &mut egui::Ui) {
    ui.heading("Settings");
    ui.separator();
    ui.add_space(8.0);
    ui.label("Settings configuration will be available in a future update.");
    ui.add_space(8.0);
    ui.label("Planned settings:");
    ui.label("  - Root path for ROM library");
    ui.label("  - ScreenScraper credentials");
    ui.label("  - Preferred region and language");
    ui.label("  - Theme (dark/light)");
}
