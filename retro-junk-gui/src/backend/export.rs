use retro_junk_frontend::esde::EsDeFrontend;
use retro_junk_frontend::{Frontend, ScrapedGame};

use crate::app::RetroJunkApp;
use crate::backend::worker::spawn_background_op;
use crate::state::{self, AppMessage};

/// Generate a gamelist.xml (ES-DE format) for a console on a background thread.
pub fn generate_gamelist(app: &mut RetroJunkApp, console_idx: usize, ctx: &egui::Context) {
    let console = &app.library.consoles[console_idx];
    let folder_name = console.folder_name.clone();

    let root_path = match app.root_path.clone() {
        Some(p) => p,
        None => return,
    };

    // Collect the data we need from each entry before moving to the background thread.
    let entry_data: Vec<EntrySnapshot> = console
        .entries
        .iter()
        .map(|entry| EntrySnapshot {
            rom_stem: entry.game_entry.rom_stem().to_string(),
            rom_filename: entry.game_entry.display_name().to_string(),
            name: entry
                .dat_match
                .as_ref()
                .map(|m| m.game_name.clone())
                .unwrap_or_else(|| entry.game_entry.display_name().to_string()),
            cover_title: entry.cover_title.clone(),
        })
        .collect();

    if entry_data.is_empty() {
        return;
    }

    let metadata_dir_setting = app.settings.general.metadata_dir.clone();
    let media_dir_setting = app.settings.general.media_dir.clone();
    let ctx = ctx.clone();
    let description = format!("Exporting gamelist.xml for {}", folder_name);

    spawn_background_op(app, description, move |op_id, _cancel, tx| {
        let result = do_generate(
            &root_path,
            &folder_name,
            &entry_data,
            &metadata_dir_setting,
            &media_dir_setting,
        );

        let _ = tx.send(AppMessage::ExportComplete {
            folder_name,
            result,
        });
        let _ = tx.send(AppMessage::OperationComplete { op_id });
        ctx.request_repaint();
    });
}

/// Snapshot of the entry data needed for export (avoids sending non-Send types).
struct EntrySnapshot {
    rom_stem: String,
    rom_filename: String,
    name: String,
    cover_title: Option<String>,
}

fn do_generate(
    root_path: &std::path::Path,
    folder_name: &str,
    entries: &[EntrySnapshot],
    metadata_dir_setting: &str,
    media_dir_setting: &str,
) -> Result<String, String> {
    let rom_dir = root_path.join(folder_name);
    let media_dir = state::media_dir_for_console(root_path, folder_name, media_dir_setting)
        .ok_or_else(|| "Could not determine media directory".to_string())?;
    let metadata_dir =
        state::metadata_dir_for_console(root_path, folder_name, metadata_dir_setting)
            .ok_or_else(|| "Could not determine metadata directory".to_string())?;

    let games: Vec<ScrapedGame> = entries
        .iter()
        .map(|e| {
            let media = state::collect_existing_media(&media_dir, &e.rom_stem);
            // Convert HashMap<MediaType, PathBuf> — already the right type
            ScrapedGame {
                rom_stem: e.rom_stem.clone(),
                rom_filename: e.rom_filename.clone(),
                name: e.name.clone(),
                description: None,
                developer: None,
                publisher: None,
                genre: None,
                players: None,
                rating: None,
                release_date: None,
                media,
                cover_title: e.cover_title.clone(),
            }
        })
        .collect();

    let frontend = EsDeFrontend::new();
    frontend
        .write_metadata(&games, &rom_dir, &metadata_dir, &media_dir)
        .map_err(|e| e.to_string())?;

    let output_path = metadata_dir.join("gamelist.xml");
    Ok(output_path.display().to_string())
}
