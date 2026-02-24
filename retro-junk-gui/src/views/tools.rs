/// Render the Tools view (placeholder).
pub fn show(ui: &mut egui::Ui) {
    ui.heading("Tools");
    ui.separator();
    ui.add_space(8.0);
    ui.label("Tool utilities will be available in a future update.");
    ui.add_space(8.0);
    ui.label("Planned tools:");
    ui.label("  - DAT cache management");
    ui.label("  - Miximage layout customizer");
}
