use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;

use crate::state::{self, AppMessage};

/// Load media files for an entry on a background thread.
///
/// Discovers media files on disk and registers their bytes with egui,
/// then sends a `MediaLoaded` message to update the entry's `media_paths`.
pub fn load_media_for_entry(
    tx: mpsc::Sender<AppMessage>,
    ctx: egui::Context,
    root_path: PathBuf,
    folder_name: String,
    entry_index: usize,
    rom_stem: String,
) {
    std::thread::spawn(move || {
        let media_dir = match state::media_dir_for_console(&root_path, &folder_name) {
            Some(d) => d,
            None => {
                let _ = tx.send(AppMessage::MediaLoaded {
                    folder_name,
                    entry_index,
                    media: HashMap::new(),
                });
                ctx.request_repaint();
                return;
            }
        };

        let found = state::collect_existing_media(&media_dir, &rom_stem);

        // Register image bytes with egui before sending the message,
        // so they're available by the time the UI renders.
        for path in found.values() {
            let uri = format!("bytes://media/{}", path.display());
            if let Ok(bytes) = std::fs::read(path) {
                ctx.include_bytes(uri, bytes);
            }
        }

        let _ = tx.send(AppMessage::MediaLoaded {
            folder_name,
            entry_index,
            media: found,
        });
        ctx.request_repaint();
    });
}
