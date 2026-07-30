//! retro-junk GUI
//!
//! Desktop application for scanning, viewing, and managing a retro game ROM library.
//! Uses egui/eframe for the UI and background threads for all I/O operations.

// Immediate-mode egui draw functions and the app message dispatcher are single
// linear sequences of UI/match arms; splitting them into single-caller helpers
// with long parameter lists would hurt readability, not help it.
#![allow(clippy::too_many_lines)]

mod app;
mod backend;
mod cache;
mod fingerprint;
pub mod fonts;
pub mod log_capture;
mod menu;
mod settings;
mod state;
#[cfg(test)]
mod test_support;
mod theme;
mod util;
mod views;
mod widgets;

/// Run the retro-junk GUI application.
pub fn run() -> eframe::Result {
    log_capture::init();

    // Per-user folder for eframe's window-state file, alongside the TOML
    // settings so everything the app remembers lives in one place.
    let persistence_path = dirs::config_dir().map(|dir| dir.join("retro-junk"));

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([800.0, 500.0]),
        persist_window: true,
        persistence_path,
        ..Default::default()
    };

    eframe::run_native(
        "retro-junk",
        options,
        Box::new(|cc| Ok(Box::new(app::RetroJunkApp::new(cc)))),
    )
}
