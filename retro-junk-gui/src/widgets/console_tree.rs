use crate::app::RetroJunkApp;
use crate::backend;
use crate::state::ScanStatus;

/// Render the manufacturer-grouped console tree.
pub fn show(ui: &mut egui::Ui, app: &mut RetroJunkApp, ctx: &egui::Context) {
    if app.library.consoles.is_empty() {
        ui.label("No consoles found.");
        return;
    }

    // Collect unique manufacturers in order
    let manufacturers: Vec<&str> = {
        let mut seen = Vec::new();
        for c in &app.library.consoles {
            if !seen.contains(&c.manufacturer) {
                seen.push(c.manufacturer);
            }
        }
        seen
    };

    for mfr in manufacturers {
        egui::CollapsingHeader::new(egui::RichText::new(mfr).strong())
            .id_salt(format!("mfr_{}", mfr))
            .default_open(true)
            .show(ui, |ui| {
                for i in 0..app.library.consoles.len() {
                    if app.library.consoles[i].manufacturer != mfr {
                        continue;
                    }

                    let console = &app.library.consoles[i];
                    let is_selected = app.selected_console == Some(i);

                    let label = match console.scan_status {
                        ScanStatus::NotScanned => console.folder_name.clone(),
                        ScanStatus::Scanning => format!("{} (...)", console.folder_name),
                        ScanStatus::Scanned => {
                            format!("{} ({})", console.folder_name, console.entries.len())
                        }
                    };

                    if ui.selectable_label(is_selected, &label).clicked() && !is_selected {
                        app.selected_console = Some(i);
                        app.focused_entry = None;
                        app.selected_entries.clear();
                        app.filter_text.clear();

                        // Trigger quick-scan if not already scanned
                        if app.library.consoles[i].scan_status == ScanStatus::NotScanned {
                            backend::scan::quick_scan_console(app, i, ctx);
                        }
                    }
                }
            });
    }
}
