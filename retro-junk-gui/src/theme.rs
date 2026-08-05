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

/// How one severity looks. The single place a status becomes a colour and a
/// glyph.
///
/// Colour alone is not enough here: the two most important states are green
/// and red — the most common colour blindness — the indicator is a small dot
/// at table density where hue reads worst, and screenshots and the CLI lose
/// colour entirely. So every severity carries a glyph that is sufficient on
/// its own, and `Incomplete` against `Unmeasured` especially depends on it:
/// both read as "not done" by colour and mean quite different things.
#[must_use]
pub fn severity_color(severity: retro_junk_backend::completion::Severity) -> egui::Color32 {
    use retro_junk_backend::completion::Severity;
    match severity {
        Severity::Verified => STATUS_OK,
        Severity::Asserted => STATUS_INFO,
        Severity::Incomplete => STATUS_WARN_STRONG,
        Severity::Broken => STATUS_ERR,
        Severity::Unmeasured => STATUS_MUTED,
    }
}

/// The glyph for a severity, readable without colour.
#[must_use]
pub fn severity_icon(severity: retro_junk_backend::completion::Severity) -> &'static str {
    use retro_junk_backend::completion::Severity;
    match severity {
        Severity::Verified => egui_phosphor::regular::CHECK_CIRCLE,
        Severity::Asserted => egui_phosphor::regular::PENCIL_SIMPLE,
        Severity::Incomplete => egui_phosphor::regular::CIRCLE_HALF,
        Severity::Broken => egui_phosphor::regular::WARNING,
        Severity::Unmeasured => egui_phosphor::regular::CIRCLE_DASHED,
    }
}

/// What a severity means, and what closes it. Frontends render this rather
/// than inventing their own wording.
#[must_use]
pub fn severity_tooltip(severity: retro_junk_backend::completion::Severity) -> &'static str {
    use retro_junk_backend::completion::Severity;
    match severity {
        Severity::Verified => "Complete and checked against the catalog",
        Severity::Asserted => {
            "Identified by hand, or content no catalog lists — correct, but not machine-verified"
        }
        Severity::Incomplete => "Usable, but something is missing or unverified",
        Severity::Broken => {
            "Not usable: nothing identifies this, or a hash contradicts what it claims to be"
        }
        Severity::Unmeasured => "Nothing can be measured yet — usually a missing catalog import",
    }
}

/// Dimmed indicator for states that carry no measurement.
pub const STATUS_MUTED: egui::Color32 = egui::Color32::from_rgb(130, 130, 130);
