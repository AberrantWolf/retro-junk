/// Block interaction while the catalog schema and legacy data are being prepared.
pub fn show(ctx: &egui::Context, status: Option<&str>) {
    let Some(status) = status else {
        return;
    };

    egui::Modal::new(egui::Id::new("startup_database_modal")).show(ctx, |ui| {
        ui.set_min_width(360.0);
        ui.heading("Preparing library database");
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(20.0));
            ui.label(status);
        });
        ui.add_space(4.0);
        ui.weak("The library will be available when this finishes.");
    });
    ctx.request_repaint_after(std::time::Duration::from_millis(50));
}
