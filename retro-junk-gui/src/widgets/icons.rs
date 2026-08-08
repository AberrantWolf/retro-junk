//! Named icon glyphs used across the GUI.
//!
//! All glyphs come from Phosphor (Regular variant) via [`egui_phosphor`],
//! installed as a font fallback by [`crate::fonts::configure_fonts`] so they
//! render inline with ordinary text. Naming them here rather than pasting
//! `egui_phosphor::regular::…` at each call site keeps the icon vocabulary
//! discoverable in one place, and makes changing an icon — or the whole
//! variant — a one-line edit.
//!
//! These constants replace the ad-hoc `\u{26a0}` / `\u{2192}` escapes the UI
//! used to carry: those depended on the Noto Symbols fallback and drew at a
//! different weight than the surrounding text.

use egui_phosphor::regular as phosphor;

// ── Status ────────────────────────────────────────────────────────────────

/// Something needs attention but is not fatal: broken references, hash
/// warnings, non-standard CUE sheets, unmatched organize candidates.
pub const WARNING: &str = phosphor::WARNING;
/// A completed action, used by toasts.
pub const SUCCESS: &str = phosphor::CHECK_CIRCLE;

// ── Direction ─────────────────────────────────────────────────────────────

/// "this becomes that" — organize previews, compression plans.
pub const ARROW_RIGHT: &str = phosphor::ARROW_RIGHT;

// ── Actions ───────────────────────────────────────────────────────────────

pub const OPEN_FOLDER: &str = phosphor::FOLDER_OPEN;
pub const FILTER: &str = phosphor::MAGNIFYING_GLASS;
pub const HASH: &str = phosphor::FINGERPRINT;
pub const DETAIL_PANEL: &str = phosphor::SIDEBAR_SIMPLE;
pub const RESCAN: &str = phosphor::ARROW_CLOCKWISE;
pub const SCRAPE: &str = phosphor::DOWNLOAD_SIMPLE;
pub const MIXIMAGE: &str = phosphor::IMAGES_SQUARE;
pub const RENAME: &str = phosphor::PENCIL_SIMPLE;
pub const COMPRESS: &str = phosphor::ARCHIVE;
pub const REGION: &str = phosphor::GLOBE;
pub const TAG: &str = phosphor::TAG;
pub const REVEAL: &str = phosphor::FOLDER_OPEN;
pub const COPY: &str = phosphor::COPY;
pub const BUILD_PLAYABLE: &str = phosphor::PLAY_CIRCLE;
pub const VERIFY: &str = phosphor::SEAL_CHECK;

// ── Sidebar / views ───────────────────────────────────────────────────────

pub const COLLECTION: &str = phosphor::ARCHIVE_BOX;
pub const LIBRARY: &str = phosphor::GAME_CONTROLLER;
pub const SETTINGS: &str = phosphor::GEAR;
pub const TOOLS: &str = phosphor::TOOLBOX;
pub const INBOX: &str = phosphor::TRAY;

/// `"{icon}  {label}"` — the spacing every icon-prefixed button uses.
#[must_use]
pub fn labeled(icon: &str, label: &str) -> String {
    format!("{icon}  {label}")
}
