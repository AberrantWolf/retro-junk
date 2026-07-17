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
            let display_name = if game.cover_title.is_empty() {
                &game.name
            } else {
                &game.cover_title
            };
            write_tag(&mut xml, "name", display_name);

            if !game.description.is_empty() {
                write_tag(&mut xml, "desc", &game.description);
            }
            if !game.developer.is_empty() {
                write_tag(&mut xml, "developer", &game.developer);
            }
            if !game.publisher.is_empty() {
                write_tag(&mut xml, "publisher", &game.publisher);
            }
            if !game.genre.is_empty() {
                write_tag(&mut xml, "genre", &game.genre);
            }
            if !game.players.is_empty() {
                write_tag(&mut xml, "players", &game.players);
            }
            if let Some(rating) = game.rating {
                write_tag(&mut xml, "rating", &format!("{:.1}", rating));
            }
            if !game.release_date.is_empty() {
                // Convert YYYY-MM-DD or YYYYMMDD to YYYYMMDDTHHMMSS
                let formatted = format_esde_date(&game.release_date);
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

fn unescape_xml(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Tags whose values are file paths that must track ROM/media renames.
const PATH_TAGS: &[&str] = &[
    "path",
    "image",
    "cover",
    "marquee",
    "screenshot",
    "titlescreen",
    "video",
    "fanart",
];

/// Read a gamelist.xml and compute its content after applying a stem rename
/// map. Returns `(path, new_content)` for a transactional write, or `None`
/// when the file doesn't exist or nothing changes.
pub fn plan_gamelist_rewrite(
    gamelist_path: &Path,
    stem_map: &std::collections::HashMap<String, String>,
) -> Option<(std::path::PathBuf, String)> {
    let content = fs::read_to_string(gamelist_path).ok()?;
    rewrite_gamelist_stems(&content, stem_map).map(|c| (gamelist_path.to_path_buf(), c))
}

/// Rewrite path-valued tags in ES-DE gamelist.xml content when ROM files are
/// renamed: any path whose final component's stem matches an entry in
/// `stem_map` gets the new stem (same rule the media renamer uses). Returns
/// `None` when nothing changes.
///
/// Operates line-by-line on the simple one-tag-per-line XML this crate
/// generates; `<name>` and other metadata tags are left untouched.
pub fn rewrite_gamelist_stems(
    content: &str,
    stem_map: &std::collections::HashMap<String, String>,
) -> Option<String> {
    if stem_map.is_empty() {
        return None;
    }

    let mut changed = false;
    let lines: Vec<String> = content
        .lines()
        .map(|line| match rewrite_gamelist_line(line, stem_map) {
            Some(rewritten) => {
                changed = true;
                rewritten
            }
            None => line.to_string(),
        })
        .collect();

    if !changed {
        return None;
    }
    let mut out = lines.join("\n");
    if content.ends_with('\n') {
        out.push('\n');
    }
    Some(out)
}

/// Rewrite a single gamelist line if it is a path tag whose value's final
/// component matches the stem map. Returns `None` when the line is unchanged.
fn rewrite_gamelist_line(
    line: &str,
    stem_map: &std::collections::HashMap<String, String>,
) -> Option<String> {
    let trimmed = line.trim_start();
    let tag = PATH_TAGS
        .iter()
        .find(|t| trimmed.starts_with(&format!("<{t}>")))?;
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = line.find(&open)? + open.len();
    let end = line.rfind(&close)?;
    if end < start {
        return None;
    }

    let value = unescape_xml(&line[start..end]);
    let (dir, name) = match value.rfind('/') {
        Some(i) => (&value[..=i], &value[i + 1..]),
        None => ("", &value[..]),
    };
    for (old_stem, new_stem) in stem_map {
        if name.starts_with(old_stem.as_str()) && name.as_bytes().get(old_stem.len()) == Some(&b'.')
        {
            let new_value = format!("{dir}{new_stem}{}", &name[old_stem.len()..]);
            return Some(format!(
                "{}{}{}",
                &line[..start],
                escape_xml(&new_value),
                &line[end..]
            ));
        }
    }
    None
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
