use std::fs;
use std::io::Write;
use std::path::Path;

use crate::{AssetType, Frontend, FrontendError, ScrapedGame};

/// Return the ES-DE system directory for an archive/catalog platform identity.
///
/// Archive identities are intentionally frontend-neutral (`ps1`, for example);
/// they must not leak into a playable projection when the selected frontend
/// requires a different system name (`psx` in ES-DE).
#[must_use]
pub fn system_directory(platform_id: &str, region: Option<&str>) -> String {
    let platform = platform_id.trim().to_ascii_lowercase();
    let region = region.unwrap_or_default().trim().to_ascii_lowercase();
    let japan = matches!(region.as_str(), "japan" | "jp" | "jpn");
    let north_america = matches!(
        region.as_str(),
        "usa" | "us" | "canada" | "north america" | "north-america"
    );
    match platform.as_str() {
        "ps1" | "psx" => "psx".to_owned(),
        "gamecube" | "gcn" | "ngc" => "gc".to_owned(),
        "3ds" => "n3ds".to_owned(),
        "super-famicom" | "super famicom" => "sfc".to_owned(),
        "snes" if north_america => "snesna".to_owned(),
        "pce" if north_america => "tg16".to_owned(),
        "pce" => "pcengine".to_owned(),
        "pcecd" if north_america => "tg-cd".to_owned(),
        "pcecd" => "pcenginecd".to_owned(),
        "genesis" if japan => "megadrivejp".to_owned(),
        "genesis" if !north_america && !region.is_empty() => "megadrive".to_owned(),
        "segacd" if japan => "megacdjp".to_owned(),
        "segacd" if !north_america && !region.is_empty() => "megacd".to_owned(),
        "32x" | "sega32x" if north_america => "sega32xna".to_owned(),
        "32x" => "sega32x".to_owned(),
        _ => platform,
    }
}

#[cfg(test)]
mod system_directory_tests {
    use super::system_directory;

    #[test]
    fn maps_internal_playstation_name_to_es_de_system() {
        assert_eq!(system_directory("ps1", None), "psx");
        assert_eq!(system_directory("psx", None), "psx");
    }

    #[test]
    fn maps_pc_engine_cd_by_es_de_region_system() {
        assert_eq!(system_directory("pcecd", Some("Japan")), "pcenginecd");
        assert_eq!(system_directory("pcecd", Some("USA")), "tg-cd");
    }

    #[test]
    fn maps_regional_es_de_systems_without_collapsing_them() {
        assert_eq!(system_directory("snes", Some("Europe")), "snes");
        assert_eq!(system_directory("snes", Some("USA")), "snesna");
        assert_eq!(system_directory("snesna", Some("USA")), "snesna");
        assert_eq!(system_directory("super-famicom", Some("Japan")), "sfc");
        assert_eq!(system_directory("pce", Some("Japan")), "pcengine");
        assert_eq!(system_directory("pce", Some("USA")), "tg16");
        // Europe is PC Engine, not TurboGrafx: NEC's European machine ran
        // the US card library in localized boxes, so no European-specific
        // cards were ever pressed and a European shelf is imported Japanese
        // software. The archive agrees (it leaves such a release under
        // `pce`); this pair used to disagree.
        assert_eq!(system_directory("pce", Some("Europe")), "pcengine");
        assert_eq!(system_directory("pcecd", Some("Europe")), "pcenginecd");
        assert_eq!(system_directory("genesis", Some("USA")), "genesis");
        assert_eq!(system_directory("genesis", Some("Europe")), "megadrive");
        assert_eq!(system_directory("genesis", Some("Japan")), "megadrivejp");
        assert_eq!(system_directory("saturn", Some("Japan")), "saturn");
    }

    #[test]
    fn preserves_distinct_game_boy_systems() {
        assert_eq!(system_directory("gb", None), "gb");
        assert_eq!(system_directory("gbc", None), "gbc");
        assert_eq!(system_directory("sgb", None), "sgb");
    }
}

/// ES-DE (`EmulationStation` Desktop Edition) frontend.
#[derive(Default)]
pub struct EsDeFrontend;

impl Frontend for EsDeFrontend {
    fn name(&self) -> &'static str {
        "ES-DE"
    }

    // Linear XML emission, one short block per optional tag; splitting adds no clarity.
    #[allow(clippy::too_many_lines)]
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
            xml.push_str(&render_game(game, rom_dir, media_dir));
        }

        xml.push_str("</gameList>\n");

        let gamelist_path = metadata_dir.join("gamelist.xml");
        write_atomic(&gamelist_path, xml.as_bytes())?;

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

/// Add a newly-created playable entry to an ES-DE gamelist without replacing
/// existing entries or user-managed fields such as favorites and play counts.
///
/// ES-DE discovers media by filename in `MediaDirectory`, so this metadata is
/// primarily for the display name and compatibility with other frontends.
pub fn upsert_game_metadata(
    game: &ScrapedGame,
    rom_dir: &Path,
    metadata_dir: &Path,
    media_dir: &Path,
) -> Result<bool, FrontendError> {
    fs::create_dir_all(metadata_dir)?;
    let gamelist_path = metadata_dir.join("gamelist.xml");
    let wanted_path = format!("./{}", game.rom_filename.replace('\\', "/"));
    if !gamelist_path.is_file() {
        let xml = format!(
            "<?xml version=\"1.0\"?>\n<gameList>\n{}</gameList>\n",
            render_game(game, rom_dir, media_dir)
        );
        write_atomic(&gamelist_path, xml.as_bytes())?;
        return Ok(true);
    }

    let existing = fs::read_to_string(&gamelist_path)?;
    if existing
        .lines()
        .any(|line| tag_value(line, "path").is_some_and(|value| unescape_xml(value) == wanted_path))
    {
        return Ok(false);
    }
    let Some(insert_at) = existing.rfind("</gameList>") else {
        return Err(FrontendError::InvalidMetadata(format!(
            "{} has no closing gameList element",
            gamelist_path.display()
        )));
    };
    let mut updated = existing;
    updated.insert_str(insert_at, &render_game(game, rom_dir, media_dir));
    write_atomic(&gamelist_path, updated.as_bytes())?;
    Ok(true)
}

fn render_game(game: &ScrapedGame, rom_dir: &Path, media_dir: &Path) -> String {
    let mut xml = String::new();
    xml.push_str("  <game>\n");
    write_tag(
        &mut xml,
        "path",
        &format!("./{}", game.rom_filename.replace('\\', "/")),
    );
    let display_name = if game.cover_title.is_empty() {
        &game.name
    } else {
        &game.cover_title
    };
    write_tag(&mut xml, "name", display_name);

    for (tag, value) in [
        ("desc", game.description.as_str()),
        ("developer", game.developer.as_str()),
        ("publisher", game.publisher.as_str()),
        ("genre", game.genre.as_str()),
        ("players", game.players.as_str()),
    ] {
        if !value.is_empty() {
            write_tag(&mut xml, tag, value);
        }
    }
    if let Some(rating) = game.rating {
        write_tag(&mut xml, "rating", &format!("{rating:.1}"));
    }
    if !game.release_date.is_empty() {
        write_tag(
            &mut xml,
            "releasedate",
            &format_esde_date(&game.release_date),
        );
    }

    let image_type = if game.assets.contains_key(&AssetType::Miximage) {
        AssetType::Miximage
    } else {
        AssetType::Screenshot
    };
    write_asset_tag(&mut xml, "image", game, image_type, rom_dir, media_dir);
    for (tag, asset_type) in [
        ("cover", AssetType::Cover),
        ("marquee", AssetType::Marquee),
        ("screenshot", AssetType::Screenshot),
        ("titlescreen", AssetType::TitleScreen),
        ("video", AssetType::Video),
        ("fanart", AssetType::Fanart),
    ] {
        write_asset_tag(&mut xml, tag, game, asset_type, rom_dir, media_dir);
    }
    xml.push_str("  </game>\n");
    xml
}

fn tag_value<'a>(line: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = line.find(&open)? + open.len();
    let end = line.rfind(&close)?;
    (end >= start).then_some(&line[start..end])
}

fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), FrontendError> {
    static NEXT_TEMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "metadata path has no parent",
        )
    })?;
    let name = path
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("metadata");
    let temp = parent.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let result = (|| -> Result<(), std::io::Error> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)?;
        if let Ok(directory) = fs::File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.map_err(FrontendError::from)
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

/// Remove the `<game>` entries whose ROM path is one of `paths`.
///
/// A gamelist entry outlives the file it points at, because upserting is keyed
/// on the path: rebuild a playable under a corrected name and the old entry
/// stays, so the frontend shows the game twice — once playable, once dead.
/// Removing the entry is the other half of renaming it.
///
/// `paths` are ROM paths relative to the system directory, in the `./Name.chd`
/// form the file stores; a bare `Name.chd` is accepted too, since that is how
/// callers naturally hold them. Returns how many entries were dropped, and
/// writes nothing when that is zero.
///
/// Operates line-by-line on the simple one-tag-per-line XML this crate
/// generates, the same assumption [`rewrite_gamelist_stems`] makes.
pub fn remove_game_entries(
    gamelist_path: &Path,
    paths: &std::collections::BTreeSet<String>,
) -> Result<usize, FrontendError> {
    if paths.is_empty() || !gamelist_path.is_file() {
        return Ok(0);
    }
    let wanted = paths
        .iter()
        .map(|path| {
            path.trim_start_matches("./")
                .replace('\\', "/")
                .to_ascii_lowercase()
        })
        .collect::<std::collections::BTreeSet<_>>();

    let content = fs::read_to_string(gamelist_path)?;
    let mut kept = String::with_capacity(content.len());
    let mut block: Vec<&str> = Vec::new();
    let mut in_game = false;
    let mut drop_block = false;
    let mut removed = 0;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("<game>") {
            in_game = true;
            drop_block = false;
            block.clear();
            block.push(line);
            continue;
        }
        if !in_game {
            kept.push_str(line);
            kept.push('\n');
            continue;
        }
        block.push(line);
        if let Some(value) = tag_value(trimmed, "path") {
            let path = unescape_xml(value)
                .trim_start_matches("./")
                .replace('\\', "/")
                .to_ascii_lowercase();
            drop_block = wanted.contains(&path);
        }
        if trimmed.starts_with("</game>") {
            in_game = false;
            if drop_block {
                removed += 1;
            } else {
                for kept_line in &block {
                    kept.push_str(kept_line);
                    kept.push('\n');
                }
            }
            block.clear();
        }
    }
    // An unterminated final block is malformed input, not something to drop
    // silently — keep it exactly as it was.
    for kept_line in &block {
        kept.push_str(kept_line);
        kept.push('\n');
    }

    if removed > 0 {
        write_atomic(gamelist_path, kept.as_bytes())?;
    }
    Ok(removed)
}

/// Read a gamelist.xml and compute its content after applying a stem rename
/// map. Returns `(path, new_content)` for a transactional write, or `None`
/// when the file doesn't exist or nothing changes.
#[must_use]
pub fn plan_gamelist_rewrite<S: std::hash::BuildHasher>(
    gamelist_path: &Path,
    stem_map: &std::collections::HashMap<String, String, S>,
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
#[must_use]
pub fn rewrite_gamelist_stems<S: std::hash::BuildHasher>(
    content: &str,
    stem_map: &std::collections::HashMap<String, String, S>,
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
fn rewrite_gamelist_line<S: std::hash::BuildHasher>(
    line: &str,
    stem_map: &std::collections::HashMap<String, String, S>,
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
        format!("{cleaned}T000000")
    }
}

#[cfg(test)]
#[path = "tests/esde_tests.rs"]
mod tests;
