//! Confirmation dialog for CHD compression.
//!
//! Doubles as the explanation surface when chdman is missing: instead of
//! hiding or silently disabling the feature, the dialog tells the user why
//! their disc rips can't be compressed and how to install chdman.

use retro_junk_lib::chd_convert::ChdmanUnavailable;
use retro_junk_lib::util::format_bytes_approx;

use crate::app::RetroJunkApp;
use crate::widgets::results_dialog::STATUS_ERR;

pub fn show(ctx: &egui::Context, app: &mut RetroJunkApp) {
    if app.chd_compress_prompt.is_none() {
        return;
    }

    let mut start = false;
    let mut close = false;
    let mut open = true;
    {
        let prompt = app.chd_compress_prompt.as_mut().unwrap();
        egui::Window::new("Compress to CHD")
            .collapsible(false)
            .resizable(true)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .default_width(580.0)
            .show(ctx, |ui| match &prompt.chdman {
                Err(unavailable) => {
                    ui.colored_label(STATUS_ERR, "chdman is not available");
                    ui.label(unavailable.to_string());
                    ui.separator();
                    ui.label(
                        "CHD compression uses chdman, the MAME project's disc-image tool. \
                         It converts cue/bin, GDI, and ISO rips into much smaller .chd files \
                         that emulators read directly — but your disc rips cannot be \
                         compressed until chdman is installed.",
                    );
                    ui.add_space(4.0);
                    ui.label(ChdmanUnavailable::install_hint());
                    ui.add_space(4.0);
                    ui.label(
                        "If chdman is already installed somewhere unusual, set its full path \
                         in Settings → External Tools.",
                    );
                    ui.separator();
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                }
                Ok(chdman) => {
                    if chdman.version.is_empty() {
                        ui.label(format!("Using chdman ({})", chdman.path.display()));
                    } else {
                        ui.label(format!(
                            "Using chdman {} ({})",
                            chdman.version,
                            chdman.path.display()
                        ));
                    }
                    ui.separator();

                    if prompt.items.is_empty() {
                        ui.label("Nothing in the selection can be compressed to CHD.");
                    } else {
                        let total: u64 = prompt.items.iter().map(|i| i.job.input_bytes).sum();
                        ui.label(format!(
                            "{} disc(s), {} of source data:",
                            prompt.items.len(),
                            format_bytes_approx(total)
                        ));
                        // D8: `display_line` is precomputed at plan time (D1's
                        // background worker), so this loop does no per-frame
                        // string/byte-formatting allocation; `show_rows`
                        // virtualizes rendering so only visible rows are laid
                        // out even for a large "Compress All" batch.
                        let row_height = ui.text_style_height(&egui::TextStyle::Body);
                        egui::ScrollArea::vertical().max_height(260.0).show_rows(
                            ui,
                            row_height,
                            prompt.items.len(),
                            |ui, range| {
                                for item in &prompt.items[range] {
                                    ui.label(&item.display_line);
                                }
                            },
                        );
                    }

                    if !prompt.skipped.is_empty() {
                        ui.add_space(4.0);
                        egui::CollapsingHeader::new(format!(
                            "Cannot compress ({})",
                            prompt.skipped.len()
                        ))
                        .show(ui, |ui| {
                            for skip in &prompt.skipped {
                                ui.label(format!("{}: {}", skip.entry_name, skip.reason));
                            }
                        });
                    }

                    ui.separator();
                    ui.checkbox(
                        &mut prompt.delete_sources,
                        "Delete original files after verified compression",
                    );
                    ui.label(
                        egui::RichText::new(
                            "Each CHD is re-extracted and compared track-by-track against \
                             the originals. Originals are only ever deleted when that \
                             verification passes; a CHD that fails verification is discarded.",
                        )
                        .small()
                        .weak(),
                    );

                    ui.separator();
                    ui.horizontal(|ui| {
                        let label = format!("Compress {} disc(s)", prompt.items.len());
                        if ui
                            .add_enabled(!prompt.items.is_empty(), egui::Button::new(label))
                            .clicked()
                        {
                            start = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                }
            });
    }

    if start {
        crate::backend::chd_compress::start_compression(app, ctx);
    } else if close || !open {
        app.chd_compress_prompt = None;
    }
}
