//! retro-junk GUI
//!
//! Desktop application for scanning, viewing, and managing a retro game ROM library.
//! Uses egui/eframe for the UI and background threads for all I/O operations.

mod app;
mod backend;
mod cache;
mod settings;
mod state;
mod views;
mod widgets;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    };

    eframe::run_native(
        "retro-junk",
        options,
        Box::new(|cc| Ok(Box::new(app::RetroJunkApp::new(cc)))),
    )
}
