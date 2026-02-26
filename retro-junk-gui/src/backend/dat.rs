use std::sync::Arc;
use std::sync::mpsc;

use retro_junk_dat::{DatIndex, cache};
use retro_junk_lib::{AnalysisContext, Platform};

use crate::state::AppMessage;

/// Load DAT files for a console on a background thread.
///
/// Called automatically after quick-scan completes. Loads cached DATs if
/// available, sends `DatLoaded` or `DatLoadFailed` back to the UI.
pub fn load_dat_for_console(
    tx: mpsc::Sender<AppMessage>,
    context: Arc<AnalysisContext>,
    platform: Platform,
    folder_name: String,
    ctx: egui::Context,
) {
    std::thread::spawn(move || {
        let registered = match context.get_by_platform(platform) {
            Some(r) => r,
            None => return,
        };

        let analyzer = registered.analyzer.as_ref();
        if !analyzer.has_dat_support() {
            let _ = tx.send(AppMessage::DatLoadFailed {
                folder_name,
                error: "No DAT support for this console".to_string(),
            });
            ctx.request_repaint();
            return;
        }

        let short_name = analyzer.short_name();
        let dat_names = analyzer.dat_names();
        let download_ids = analyzer.dat_download_ids();
        let dat_source = analyzer.dat_source();

        match cache::load_dats(short_name, dat_names, download_ids, None, dat_source) {
            Ok(dats) => {
                let index = DatIndex::from_dats(dats);
                let _ = tx.send(AppMessage::DatLoaded {
                    folder_name,
                    platform,
                    index,
                });
            }
            Err(e) => {
                let _ = tx.send(AppMessage::DatLoadFailed {
                    folder_name,
                    error: e.to_string(),
                });
            }
        }
        ctx.request_repaint();
    });
}
