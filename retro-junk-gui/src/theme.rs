//! Shared status colors for the GUI.
//!
//! Every widget that paints a "good / needs attention / broken" state pulls
//! from this palette so status colors stay consistent across the app. Before
//! this module existed, the same RGB literals were scattered across eight
//! files.

/// Green: successful/verified/complete.
pub const STATUS_OK: egui::Color32 = egui::Color32::from_rgb(50, 180, 50);

/// Blue: identified/informational, but not integrity-verified.
pub const STATUS_INFO: egui::Color32 = egui::Color32::from_rgb(70, 140, 220);

/// Red: errors and unrecoverable problems.
pub const STATUS_ERR: egui::Color32 = egui::Color32::from_rgb(220, 50, 50);

/// Yellow: warnings and "needs attention" states.
pub const STATUS_WARN: egui::Color32 = egui::Color32::from_rgb(220, 180, 30);

/// Orange: stronger warnings (log Warn level, compatibility issues,
/// partial assets).
pub const STATUS_WARN_STRONG: egui::Color32 = egui::Color32::from_rgb(230, 160, 30);

/// Color for a log level.
///
/// `Color32::PLACEHOLDER` for Info is a sentinel meaning "use the default
/// text color" — callers check for it before applying.
pub fn log_level_color(level: log::Level) -> egui::Color32 {
    match level {
        log::Level::Error => STATUS_ERR,
        log::Level::Warn => STATUS_WARN_STRONG,
        log::Level::Info => egui::Color32::PLACEHOLDER,
        log::Level::Debug => egui::Color32::from_rgb(140, 140, 140),
        log::Level::Trace => egui::Color32::from_rgb(100, 100, 100),
    }
}
