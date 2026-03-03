use std::fs;
use std::io::Write;
use std::path::Path;

use crate::{AssetType, Frontend, FrontendError, ScrapedGame};

/// ES-DE (EmulationStation Desktop Edition) frontend.
#[derive(Default)]
pub struct EsDeFrontend;

impl Frontend for EsDeFrontend {
    fn name(&self) -> &'static str {
        "ES-DE"
    }

    fn write_metadata(
        &self,
        games: &[ScrapedGame],
        rom_dir: &Path,
        metadata_dir: &Path,
        media_dir: &Path,
    ) -> Result<(), FrontendError> {
        if games.is_empty() {
            return Ok(());
        }

        fs::create_dir_all(metadata_dir)?;

        let mut xml = String::new();
        xml.push_str("<?xml version=\"1.0\"?>\n");
        xml.push_str("<gameList>\n");

        for game in games {
            xml.push_str("  <game>\n");
            write_tag(&mut xml, "path", &format!("./{}", game.rom_filename));
            let display_name = game.cover_title.as_deref().unwrap_or(&game.name);
            write_tag(&mut xml, "name", display_name);

            if let Some(ref desc) = game.description {
                write_tag(&mut xml, "desc", desc);
            }
            if let Some(ref dev) = game.developer {
                write_tag(&mut xml, "developer", dev);
            }
            if let Some(ref pub_) = game.publisher {
                write_tag(&mut xml, "publisher", pub_);
            }
            if let Some(ref genre) = game.genre {
                write_tag(&mut xml, "genre", genre);
            }
            if let Some(ref players) = game.players {
                write_tag(&mut xml, "players", players);
            }
            if let Some(rating) = game.rating {
                write_tag(&mut xml, "rating", &format!("{:.1}", rating));
            }
            if let Some(ref date) = game.release_date {
                // Convert YYYY-MM-DD or YYYYMMDD to YYYYMMDDTHHMMSS
                let formatted = format_esde_date(date);
                write_tag(&mut xml, "releasedate", &formatted);
            }

            // Media paths — use relative paths from the ROM directory if possible
            // Prefer miximage for <image>, fall back to screenshot
            if game.assets.contains_key(&AssetType::Miximage) {
                write_asset_tag(
                    &mut xml,
                    "image",
                    game,
                    AssetType::Miximage,
                    rom_dir,
                    media_dir,
                );
            } else {
                write_asset_tag(
                    &mut xml,
                    "image",
                    game,
                    AssetType::Screenshot,
                    rom_dir,
                    media_dir,
                );
            }
            write_asset_tag(
                &mut xml,
                "cover",
                game,
                AssetType::Cover,
                rom_dir,
                media_dir,
            );
            write_asset_tag(
                &mut xml,
                "marquee",
                game,
                AssetType::Marquee,
                rom_dir,
                media_dir,
            );
            write_asset_tag(
                &mut xml,
                "screenshot",
                game,
                AssetType::Screenshot,
                rom_dir,
                media_dir,
            );
            write_asset_tag(
                &mut xml,
                "titlescreen",
                game,
                AssetType::TitleScreen,
                rom_dir,
                media_dir,
            );
            write_asset_tag(
                &mut xml,
                "video",
                game,
                AssetType::Video,
                rom_dir,
                media_dir,
            );
            write_asset_tag(
                &mut xml,
                "fanart",
                game,
                AssetType::Fanart,
                rom_dir,
                media_dir,
            );

            xml.push_str("  </game>\n");
        }

        xml.push_str("</gameList>\n");

        let gamelist_path = metadata_dir.join("gamelist.xml");
        let mut file = fs::File::create(&gamelist_path)?;
        file.write_all(xml.as_bytes())?;

        Ok(())
    }

    fn asset_subdirs(&self) -> &[(&str, AssetType)] {
        &[
            ("covers", AssetType::Cover),
            ("screenshots", AssetType::Screenshot),
            ("titlescreens", AssetType::TitleScreen),
            ("marquees", AssetType::Marquee),
            ("3dboxes", AssetType::Cover3D),
            ("fanart", AssetType::Fanart),
            ("physicalmedia", AssetType::PhysicalMedia),
            ("miximages", AssetType::Miximage),
            ("videos", AssetType::Video),
        ]
    }
}

fn write_tag(xml: &mut String, tag: &str, value: &str) {
    xml.push_str("    <");
    xml.push_str(tag);
    xml.push('>');
    xml.push_str(&escape_xml(value));
    xml.push_str("</");
    xml.push_str(tag);
    xml.push_str(">\n");
}

fn write_asset_tag(
    xml: &mut String,
    tag: &str,
    game: &ScrapedGame,
    asset_type: AssetType,
    rom_dir: &Path,
    _media_dir: &Path,
) {
    if let Some(asset_path) = game.assets.get(&asset_type) {
        // Compute a relative path from the ROM directory to the asset file.
        // This handles sibling directories (e.g., roms-media/ next to roms/)
        // by producing paths with .. components.
        let display_path = if let Some(rel) = pathdiff::diff_paths(asset_path, rom_dir) {
            format!("./{}", rel.display())
        } else {
            asset_path.display().to_string()
        };
        write_tag(xml, tag, &display_path);
    }
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Convert various date formats to ES-DE's YYYYMMDDTHHMMSS format.
fn format_esde_date(date: &str) -> String {
    // Handle YYYY-MM-DD
    let cleaned = date.replace('-', "");
    // Ensure we have at least 8 digits, pad with zeros
    if cleaned.len() >= 8 {
        format!("{}T000000", &cleaned[..8])
    } else {
        format!("{}T000000", cleaned)
    }
}

#[cfg(test)]
#[path = "tests/esde_tests.rs"]
mod tests;
