//! Glyph-coverage regression test: every symbol character the GUI renders
//! must have a glyph in the configured fonts, or it silently draws as tofu.
//!
//! The inventory is scanned from this crate's own sources rather than kept as
//! a hand-maintained list, so adding a new symbol to any UI string
//! automatically extends the test. The scan is heuristic — it takes the text
//! between double quotes on each line (both literal non-ASCII characters and
//! `\u{...}` escapes) — which can over-collect from quoted text in comments.
//! That errs conservative: over-collected characters demand coverage we don't
//! strictly need, never the reverse.

use std::collections::BTreeSet;
use std::path::Path;

use egui_kittest::Harness;

use crate::app::RetroJunkApp;

/// Collect candidate UI characters from every `.rs` file under `src/`.
fn ui_chars_from_source() -> BTreeSet<char> {
    let mut chars = BTreeSet::new();
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    visit(&src, &mut chars);
    chars
}

fn visit(dir: &Path, chars: &mut BTreeSet<char>) {
    for entry in std::fs::read_dir(dir).expect("read src dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            visit(&path, chars);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&path).expect("read source file");
            scan_string_literals(&text, chars);
        }
    }
}

/// Extract non-ASCII chars and `\u{...}` escapes from quoted segments.
fn scan_string_literals(text: &str, chars: &mut BTreeSet<char>) {
    for line in text.lines() {
        // Odd-indexed segments of a split on `"` are (heuristically) inside
        // string literals.
        for (i, segment) in line.split('"').enumerate() {
            if i % 2 == 0 {
                continue;
            }
            chars.extend(segment.chars().filter(|c| !c.is_ascii()));
            let mut rest = segment;
            while let Some(start) = rest.find("\\u{") {
                rest = &rest[start + 3..];
                if let Some(end) = rest.find('}')
                    && let Ok(cp) = u32::from_str_radix(&rest[..end], 16)
                    && let Some(c) = char::from_u32(cp)
                {
                    chars.insert(c);
                }
            }
        }
    }
}

#[test]
fn all_ui_symbols_have_glyphs_in_configured_fonts() {
    let chars = ui_chars_from_source();
    // Sanity: the scan must find the symbols we know are in use (✔, ⚠, →).
    for known in ['\u{2714}', '\u{26a0}', '\u{2192}'] {
        assert!(
            chars.contains(&known),
            "source scan lost a known symbol ({known:?}) — heuristic broke"
        );
    }

    // `with_parts` applies `fonts::configure_fonts`, same as the real app.
    let mut harness = Harness::new_eframe(|cc| {
        RetroJunkApp::with_parts(
            &cc.egui_ctx,
            crate::settings::AppSettings::default(),
            None,
            None,
        )
    });
    harness.run();

    // Check the family's actual character map, NOT `FontsView::has_glyph`:
    // that API falsely reports "missing" for any char served by the same font
    // that owns the replacement character ◻ (upstream TODO in epaint's
    // `Font::has_glyph`) — which is exactly Noto Sans Symbols 2 here.
    let mut missing = Vec::new();
    harness.ctx.fonts_mut(|fonts| {
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let mut font = fonts.fonts.font(&family);
            let supported = font.characters();
            for &c in &chars {
                if !supported.contains_key(&c) {
                    missing.push(format!("U+{:04X} {c:?} ({family:?})", c as u32));
                }
            }
        }
    });

    assert!(
        missing.is_empty(),
        "these characters render as tofu — add a font with coverage or use a \
         different character:\n{}",
        missing.join("\n")
    );
}
