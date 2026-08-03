//! Frontend-metadata export: generate a gamelist.xml (ES-DE format) for one
//! console from its committed library entries.

use std::path::Path;

use retro_junk_frontend::esde::EsDeFrontend;
use retro_junk_frontend::{DISPLAY_ASSET_TYPES, Frontend, ScrapedGame};

use super::OpCtx;

/// One committed entry reduced to the fields the export needs.
struct EntrySnapshot {
    rom_stem: String,
    rom_filename: String,
    name: String,
    /// Box/cover title from the catalog DB. Empty = absent.
    cover_title: String,
}

/// Generate a gamelist.xml for `console_id`, returning the written path.
pub fn generate_gamelist(
    root_path: &Path,
    folder_name: &str,
    db_path: &Path,
    console_id: retro_junk_db::LibraryConsoleId,
    metadata_dir_setting: &str,
    media_dir_setting: &str,
    ctx: &OpCtx,
) -> Result<String, String> {
    let conn = retro_junk_db::open_database(db_path).map_err(|e| e.to_string())?;
    let rows = retro_junk_db::load_export_entries_for_console(&conn, console_id)
        .map_err(|e| e.to_string())?;
    let entries = rows
        .into_iter()
        .map(|row| {
            let game_entry: retro_junk_lib::scanner::GameEntry =
                serde_json::from_str(&row.game_entry_json)
                    .map_err(|e| format!("Invalid library entry: {e}"))?;
            Ok(EntrySnapshot {
                rom_stem: game_entry.rom_stem().to_owned(),
                rom_filename: game_entry.display_name().to_owned(),
                name: if row.dat_game_name.is_empty() {
                    game_entry.display_name().to_owned()
                } else {
                    row.dat_game_name
                },
                cover_title: row.cover_title,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    if entries.is_empty() {
        return Err("The console has no committed entries to export".to_owned());
    }
    if ctx.cancelled() {
        return Err("Export cancelled".to_owned());
    }

    let rom_dir = root_path.join(folder_name);
    let media_dir =
        retro_junk_lib::util::asset_dir_for_console(root_path, folder_name, media_dir_setting)
            .ok_or_else(|| "Could not determine media directory".to_string())?;
    let metadata_dir = retro_junk_lib::util::metadata_dir_for_console(
        root_path,
        folder_name,
        metadata_dir_setting,
    );

    let games: Vec<ScrapedGame> = entries
        .iter()
        .map(|e| {
            if ctx.cancelled() {
                return Err("Export cancelled".to_owned());
            }
            let assets = retro_junk_frontend::collect_existing_assets(
                DISPLAY_ASSET_TYPES,
                &media_dir,
                &e.rom_stem,
            );
            Ok(ScrapedGame {
                rom_stem: e.rom_stem.clone(),
                rom_filename: e.rom_filename.clone(),
                name: e.name.clone(),
                description: String::new(),
                developer: String::new(),
                publisher: String::new(),
                genre: String::new(),
                players: String::new(),
                rating: None,
                release_date: String::new(),
                assets,
                cover_title: e.cover_title.clone(),
            })
        })
        .collect::<Result<_, String>>()?;

    if ctx.cancelled() {
        return Err("Export cancelled".to_owned());
    }

    let frontend = EsDeFrontend;
    frontend
        .write_metadata(&games, &rom_dir, &metadata_dir, &media_dir)
        .map_err(|e| e.to_string())?;

    Ok(metadata_dir.join("gamelist.xml").display().to_string())
}
