/// CJK font configuration for egui.
///
/// Controlled by Cargo features:
/// - `cjk-full` (default): Embeds NotoSansCJKjp-Regular.otf (~16MB) covering
///   Japanese, Chinese (Simplified + Traditional), and Korean.
/// - `cjk-jp`: Embeds NotoSansJP-Regular.otf (~4.3MB) covering Japanese only.
/// - Neither: No CJK font embedded; CJK characters will render as tofu (□).
///
/// Font source: <https://github.com/notofonts/noto-cjk> (Sans2.004)
/// License: SIL Open Font License 1.1 (see fonts/LICENSE)

#[cfg(feature = "cjk-full")]
const CJK_FONT_DATA: &[u8] = include_bytes!("../fonts/NotoSansCJKjp-Regular.otf");

#[cfg(all(feature = "cjk-jp", not(feature = "cjk-full")))]
const CJK_FONT_DATA: &[u8] = include_bytes!("../fonts/NotoSansJP-Regular.otf");

/// Install CJK fonts as a fallback in egui's font system.
///
/// This adds the CJK font after the default Latin fonts so CJK glyphs
/// render correctly while Latin text continues using egui's built-in font.
#[cfg(any(feature = "cjk-full", feature = "cjk-jp"))]
pub fn configure_cjk_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    fonts.font_data.insert(
        "noto_sans_cjk".to_owned(),
        egui::FontData::from_static(CJK_FONT_DATA).into(),
    );

    // Add as fallback for both proportional and monospace families
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .push("noto_sans_cjk".to_owned());

    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .push("noto_sans_cjk".to_owned());

    ctx.set_fonts(fonts);
}

#[cfg(not(any(feature = "cjk-full", feature = "cjk-jp")))]
pub fn configure_cjk_fonts(_ctx: &egui::Context) {
    // No CJK font embedded — nothing to configure.
}
