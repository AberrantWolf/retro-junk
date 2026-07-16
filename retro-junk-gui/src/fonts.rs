//! Font configuration for egui: UI symbol coverage + optional CJK.
//!
//! # Symbols (always installed)
//!
//! egui's bundled fonts (Ubuntu-Light, Noto Emoji, emoji-icon-font, Hack)
//! cannot draw several symbols this GUI uses: ✘ (U+2718) and 🗜 (U+1F5DC)
//! exist in none of them, and → ● □ ↔ exist only in Hack, which serves the
//! monospace family — so they all render as tofu (□) in regular labels.
//!
//! Two fonts from the Noto Sans Symbols family close every gap:
//! - **Noto Sans Symbols 2**: dingbats and geometric shapes (✔ ✘ ⚠ ▶ • ✂ 🗜 💿 ● □)
//! - **Noto Sans Symbols**: arrows (→ ↔)
//!
//! They are inserted into the fallback chain *before* egui's emoji fonts, so
//! paired glyphs like ✔/✘ render from one family with matching stroke
//! weight; the emoji fonts keep serving the pictographs only they cover
//! (🔧 📁 📝 ℹ). `fonts_tests.rs` scans the crate's sources for every symbol
//! used in a string literal and asserts glyph coverage, so a new symbol that
//! no font can draw fails CI instead of shipping as tofu.
//!
//! # CJK (Cargo features)
//!
//! - `cjk-full` (used by the `retro-junk-gui-cjk` wrapper): Embeds
//!   NotoSansCJKjp-Regular.otf (~16MB) covering Japanese, Chinese
//!   (Simplified + Traditional), and Korean.
//! - `cjk-jp`: Embeds NotoSansJP-Regular.otf (~4.3MB) covering Japanese only.
//! - Neither (default): No CJK font embedded; CJK characters render as tofu.
//!
//! Font sources: <https://github.com/notofonts/symbols> (Symbols2 2.008,
//! Symbols 2.003) and <https://github.com/notofonts/noto-cjk> (Sans2.004).
//! License: SIL Open Font License 1.1 (see fonts/LICENSE)

#[cfg(test)]
#[path = "fonts_tests.rs"]
mod fonts_tests;

const SYMBOLS2_FONT_DATA: &[u8] = include_bytes!("../fonts/NotoSansSymbols2-Regular.ttf");
const SYMBOLS_FONT_DATA: &[u8] = include_bytes!("../fonts/NotoSansSymbols-Regular.ttf");

#[cfg(feature = "cjk-full")]
const CJK_FONT_DATA: &[u8] = include_bytes!("../fonts/NotoSansCJKjp-Regular.otf");

#[cfg(all(feature = "cjk-jp", not(feature = "cjk-full")))]
const CJK_FONT_DATA: &[u8] = include_bytes!("../fonts/NotoSansJP-Regular.otf");

/// Install the app's font stack: egui defaults + Noto symbol fonts
/// (+ CJK fallback when a `cjk-*` feature is enabled).
pub fn configure_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    fonts.font_data.insert(
        "noto_sans_symbols2".to_owned(),
        egui::FontData::from_static(SYMBOLS2_FONT_DATA).into(),
    );
    fonts.font_data.insert(
        "noto_sans_symbols".to_owned(),
        egui::FontData::from_static(SYMBOLS_FONT_DATA).into(),
    );

    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        let list = fonts.families.entry(family).or_default();
        // After the text fonts (Latin keeps its current look), before the
        // emoji fonts (dingbats come from one consistent family).
        let pos = list
            .iter()
            .position(|f| f == "NotoEmoji-Regular")
            .unwrap_or(list.len());
        list.insert(pos, "noto_sans_symbols".to_owned());
        list.insert(pos, "noto_sans_symbols2".to_owned());
    }

    add_cjk_fallback(&mut fonts);

    ctx.set_fonts(fonts);
}

/// Append the CJK font as the last fallback for both families, so CJK glyphs
/// render correctly while Latin text and symbols use the fonts above.
#[cfg(any(feature = "cjk-full", feature = "cjk-jp"))]
fn add_cjk_fallback(fonts: &mut egui::FontDefinitions) {
    fonts.font_data.insert(
        "noto_sans_cjk".to_owned(),
        egui::FontData::from_static(CJK_FONT_DATA).into(),
    );

    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("noto_sans_cjk".to_owned());
    }
}

#[cfg(not(any(feature = "cjk-full", feature = "cjk-jp")))]
fn add_cjk_fallback(_fonts: &mut egui::FontDefinitions) {
    // No CJK font embedded — nothing to add.
}
