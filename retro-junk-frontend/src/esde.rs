use std::fs;
use std::io::Write;
use std::path::Path;

use crate::{Frontend, FrontendError, MediaType, ScrapedGame};

/// ES-DE (EmulationStation Desktop Edition) frontend.
pub struct EsDeFrontend;

impl EsDeFrontend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for EsDeFrontend {
    fn default() -> Self {
        Self::new()
    }
}

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
            write_tag(&mut xml, "name", &game.name);

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
            write_media_tag(
                &mut xml,
                "image",
                game,
                MediaType::Screenshot,
                rom_dir,
                media_dir,
            );
            write_media_tag(
                &mut xml,
                "cover",
                game,
                MediaType::Cover,
                rom_dir,
                media_dir,
            );
            write_media_tag(
                &mut xml,
                "marquee",
                game,
                MediaType::Marquee,
                rom_dir,
                media_dir,
            );
            write_media_tag(
                &mut xml,
                "screenshot",
                game,
                MediaType::Screenshot,
                rom_dir,
                media_dir,
            );
            write_media_tag(
                &mut xml,
                "titlescreen",
                game,
                MediaType::TitleScreen,
                rom_dir,
                media_dir,
            );
            write_media_tag(
                &mut xml,
                "video",
                game,
                MediaType::Video,
                rom_dir,
                media_dir,
            );
            write_media_tag(
                &mut xml,
                "fanart",
                game,
                MediaType::Fanart,
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

    fn media_subdirs(&self) -> &[(&str, MediaType)] {
        &[
            ("covers", MediaType::Cover),
            ("screenshots", MediaType::Screenshot),
            ("titlescreens", MediaType::TitleScreen),
            ("marquees", MediaType::Marquee),
            ("3dboxes", MediaType::Cover3D),
            ("fanart", MediaType::Fanart),
            ("physicalmedia", MediaType::PhysicalMedia),
            ("videos", MediaType::Video),
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

fn write_media_tag(
    xml: &mut String,
    tag: &str,
    game: &ScrapedGame,
    media_type: MediaType,
    rom_dir: &Path,
    _media_dir: &Path,
) {
    if let Some(media_path) = game.media.get(&media_type) {
        // Try to make a relative path from the ROM directory
        let display_path = if let Ok(rel) = media_path.strip_prefix(rom_dir.parent().unwrap_or(rom_dir)) {
            format!("./{}", rel.display())
        } else {
            media_path.display().to_string()
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
mod tests {
    use super::*;

    #[test]
    fn test_format_esde_date() {
        assert_eq!(format_esde_date("1996-06-23"), "19960623T000000");
        assert_eq!(format_esde_date("19960623"), "19960623T000000");
    }

    #[test]
    fn test_escape_xml() {
        assert_eq!(escape_xml("Tom & Jerry"), "Tom &amp; Jerry");
        assert_eq!(escape_xml("a < b"), "a &lt; b");
    }
}
