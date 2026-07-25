use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use egui_extras::{Column, TableBuilder};

use crate::app::RetroJunkApp;
use crate::state::{
    AppMessage, BackgroundOperation, DumpImportDialogState, OperationKind, PhysicalCopyEditor,
    ProgressDisplay, next_operation_id,
};

enum ImportModalAction {
    None,
    Close,
    Import(retro_junk_archive_import::DumpImportPlan, bool),
}

pub fn show(ui: &mut egui::Ui, app: &mut RetroJunkApp) {
    ui.heading("Collection");
    ui.add_space(4.0);
    let Some(profile) = app.settings.library.active_profile().cloned() else {
        ui.label("Choose a playable library root to create a collection profile.");
        ui.weak("The profile pairs archive, playable, and scratch roots without storing device-specific paths in the portable archive.");
        return;
    };
    let archive_initialized = profile
        .archive_root
        .join("retro-junk-archive.toml")
        .is_file();
    ui.horizontal(|ui| {
        ui.label(&profile.display_name);
        ui.weak(format!("archive: {}", profile.archive_root.display()));
        let busy = app
            .operations
            .iter()
            .any(|operation| operation.scope == "archive");
        if ui
            .add_enabled(!busy, egui::Button::new("Refresh index"))
            .clicked()
        {
            crate::backend::archive::start_archive_operation(app, &profile, false);
        }
        if ui
            .add_enabled(!busy, egui::Button::new("Verify stored bytes"))
            .clicked()
        {
            crate::backend::archive::start_archive_operation(app, &profile, true);
        }
        if ui
            .add_enabled(
                !busy && app.db_path.is_some(),
                egui::Button::new("Identify archived carriers"),
            )
            .on_hover_text(
                "Reproduce unbound or stale Redumper masters and match complete track sets against the current catalog",
            )
            .clicked()
        {
            crate::backend::archive::start_catalog_identification_operation(app, &profile);
        }
        if ui
            .add_enabled(
                !busy && archive_initialized,
                egui::Button::new("Import dumps…"),
            )
            .clicked()
            && let Some(source) = rfd::FileDialog::new().pick_folder()
        {
            start_dump_import_planning(app, &profile, source, false);
        }
        if ui
            .add_enabled(
                !busy && archive_initialized,
                egui::Button::new("Import existing playable library…"),
            )
            .on_hover_text("Copy archival-equivalent ROMs into the archive while retaining and adopting the existing playable files")
            .clicked()
            && let Some(source) = rfd::FileDialog::new()
                .set_directory(&profile.playable_root)
                .pick_folder()
        {
            start_dump_import_planning(app, &profile, source, true);
        }
    });
    if !archive_initialized {
        ui.add_space(8.0);
        ui.colored_label(
            egui::Color32::YELLOW,
            "Archive root is not initialized. Initialize it or change the profile paths in Settings.",
        );
        return;
    }
    let profile_id = profile.profile_id.to_string();
    if app.ui_state.collection_profile_id.as_deref() != Some(profile_id.as_str())
        && !app.ui_state.collection_summaries_loading
    {
        start_collection_summary_load(app, profile_id.clone(), ui.ctx());
    }
    if app.ui_state.collection_summaries_loading
        && app.ui_state.collection_profile_id.as_deref() != Some(profile_id.as_str())
    {
        ui.spinner();
        ui.label("Loading collection index…");
        return;
    }
    let summaries = std::sync::Arc::clone(&app.ui_state.collection_summaries);
    if summaries.is_empty() {
        ui.add_space(8.0);
        ui.label("No archived releases are indexed yet.");
        return;
    }
    ui.add_space(8.0);
    if app.ui_state.collection_selected_release.is_some() {
        let available_height = ui.available_height();
        let detail_min = (available_height * 0.25).clamp(60.0, 140.0);
        let detail_max = (available_height - 80.0).max(detail_min);
        let default_height = (available_height * 0.45).clamp(detail_min, detail_max);
        egui::Panel::bottom("collection_detail_panel")
            .resizable(true)
            .show_separator_line(true)
            .default_size(default_height)
            .size_range(detail_min..=detail_max)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("collection_detail_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if app.ui_state.collection_editor_loading.is_some() {
                            ui.spinner();
                            ui.label("Loading release details…");
                        } else {
                            show_editor(ui, app, &profile);
                        }
                    });
            });
    }
    let selected_release_id = app.ui_state.collection_selected_release.as_deref();
    let body_height = (ui.available_height() - 24.0).max(0.0);
    let row_height = egui::TextStyle::Body
        .resolve(ui.style())
        .size
        .max(ui.spacing().interact_size.y);
    let mut clicked_release = None;
    egui::ScrollArea::horizontal()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            TableBuilder::new(ui)
                .striped(true)
                .resizable(true)
                .sense(egui::Sense::click())
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::initial(90.0).at_least(55.0))
                .column(Column::initial(300.0).at_least(120.0))
                .column(Column::initial(145.0).at_least(90.0))
                .column(Column::initial(115.0).at_least(80.0))
                .column(Column::initial(115.0).at_least(80.0))
                .column(Column::initial(85.0).at_least(65.0))
                .column(Column::initial(230.0).at_least(120.0))
                .min_scrolled_height(0.0)
                .max_scroll_height(body_height)
                .header(24.0, |mut header| {
                    for label in [
                        "Platform", "Release", "Physical", "Archive", "Playable", "Desired",
                        "Evidence",
                    ] {
                        header.col(|ui| {
                            ui.strong(label);
                        });
                    }
                })
                .body(|body| {
                    body.rows(row_height, summaries.len(), |mut row| {
                        let release = &summaries[row.index()];
                        let display_title = release_display_title(release);
                        row.set_selected(
                            selected_release_id == Some(release.archive_release_id.as_str()),
                        );
                        let mut response =
                            row.col(|ui| paint_cell_text(ui, &release.platform_id)).1;
                        response |= row.col(|ui| paint_cell_text(ui, &display_title)).1;
                        response |= row
                            .col(|ui| {
                                paint_cell_text(
                                    ui,
                                    &format!(
                                        "{} copy / {} carrier",
                                        release.physical_copy_count, release.carrier_count
                                    ),
                                );
                            })
                            .1;
                        response |= row
                            .col(|ui| {
                                let text = if release.archive_complete {
                                    format!(
                                        "Complete ({}/{})",
                                        release.verified_disc_count, release.expected_disc_count
                                    )
                                } else if release.expected_disc_count > 0 {
                                    format!(
                                        "Incomplete ({}/{})",
                                        release.verified_disc_count, release.expected_disc_count
                                    )
                                } else {
                                    "Unknown (not catalog-bound)".to_owned()
                                };
                                paint_cell_text(ui, &text);
                            })
                            .1;
                        response |= row
                            .col(|ui| {
                                paint_cell_text(
                                    ui,
                                    &format!(
                                        "{}/{} present",
                                        release.playable_present_count, release.playable_count
                                    ),
                                );
                            })
                            .1;
                        let desired =
                            if release.desired_playable_count > release.satisfied_playable_count {
                                "pending"
                            } else if release.desired_playable_count > 0 {
                                "satisfied"
                            } else {
                                "—"
                            };
                        response |= row.col(|ui| paint_cell_text(ui, desired)).1;
                        response |= row
                            .col(|ui| {
                                paint_cell_text(
                                    ui,
                                    &format!(
                                        "I {} · Repro {} · Catalog {} · RT {}",
                                        release.integrity_verified_count,
                                        release.reproduction_verified_count,
                                        release.catalog_verified_count,
                                        release.round_trip_verified_count
                                    ),
                                );
                            })
                            .1;
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::SelectableLabel,
                                true,
                                &display_title,
                            )
                        });
                        if response.clicked() {
                            clicked_release = Some(release.archive_release_id.clone());
                        }
                        response.on_hover_text("Show game, physical-copy, and playable details");
                    });
                });
        });
    if let Some(release_id) = clicked_release {
        start_collection_editor_load(app, profile.archive_root.clone(), release_id, ui.ctx());
    }
}

fn start_collection_summary_load(app: &mut RetroJunkApp, profile_id: String, ctx: &egui::Context) {
    let Some(db_path) = app.db_path.clone() else {
        let result = app.catalog_db.as_ref().map_or_else(
            || Err("Catalog database is unavailable".to_owned()),
            |connection| {
                retro_junk_db::list_archive_release_summaries(connection, &profile_id)
                    .map_err(|error| error.to_string())
            },
        );
        match result {
            Ok(summaries) => {
                app.ui_state.collection_profile_id = Some(profile_id);
                app.ui_state.collection_summaries = std::sync::Arc::new(summaries);
            }
            Err(error) => app.push_error("Collection", error),
        }
        return;
    };
    app.ui_state.collection_summaries_loading = true;
    let sender = app.message_tx.clone();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let result = retro_junk_db::open_database(&db_path)
            .map_err(|error| error.to_string())
            .and_then(|connection| {
                retro_junk_db::list_archive_release_summaries(&connection, &profile_id)
                    .map_err(|error| error.to_string())
            });
        let _ = sender.send(AppMessage::CollectionSummariesReady { profile_id, result });
        ctx.request_repaint();
    });
}

fn start_collection_editor_load(
    app: &mut RetroJunkApp,
    archive_root: std::path::PathBuf,
    release_id: String,
    ctx: &egui::Context,
) {
    let Some(db_path) = app.db_path.clone() else {
        let result = app.catalog_db.as_ref().map_or_else(
            || Err("Catalog database is unavailable".to_owned()),
            |connection| load_editor(connection, &archive_root, &release_id),
        );
        match result {
            Ok(editor) => {
                app.ui_state.collection_selected_release = Some(release_id);
                app.ui_state.collection_editor = Some(editor);
            }
            Err(error) => app.push_error("Collection details", error),
        }
        return;
    };
    app.ui_state.collection_editor_loading = Some(release_id.clone());
    app.ui_state.collection_selected_release = Some(release_id.clone());
    let sender = app.message_tx.clone();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let result = retro_junk_db::open_database(&db_path)
            .map_err(|error| error.to_string())
            .and_then(|connection| load_editor(&connection, &archive_root, &release_id));
        let _ = sender.send(AppMessage::CollectionEditorReady { release_id, result });
        ctx.request_repaint();
    });
}

fn load_editor(
    connection: &retro_junk_db::Connection,
    root: &std::path::Path,
    release_id: &str,
) -> Result<PhysicalCopyEditor, String> {
    let details = retro_junk_db::load_archive_collection_details(connection, release_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "release is no longer present in the archive index".to_owned())?;
    Ok(PhysicalCopyEditor {
        archive_release_id: details.archive_release_id,
        platform_id: details.platform_id,
        title: details.title,
        region: details.region,
        revision: details.revision,
        variant: details.variant,
        catalog_release_id: details.catalog_release_id.unwrap_or_default(),
        catalog_source: details.catalog_source,
        release_binding_state: details.release_binding_state,
        carrier_kind: details.carrier_kind,
        carrier_serial: details.carrier_serial,
        carrier_binding_state: details.carrier_binding_state,
        physical_copy_id: details.physical_copy_id,
        physical_copy_manifest_path: root.join(details.physical_copy_manifest_path),
        carrier_manifest_path: root.join(details.carrier_manifest_path),
        label: details.label,
        condition: details.condition,
        notes: details.notes,
        date_acquired: details.date_acquired,
        provenance: details.provenance,
        desired_format: details.desired_format.unwrap_or_default().replace('_', "-"),
        retain_intermediate: details.retain_intermediate,
        allow_unverified: details.allow_unverified,
        ingest_format: "rom".to_owned(),
        release_asset_type: "cover".to_owned(),
    })
}

fn show_editor(
    ui: &mut egui::Ui,
    app: &mut RetroJunkApp,
    profile: &retro_junk_archive::CollectionProfile,
) {
    let Some(editor) = app.ui_state.collection_editor.as_mut() else {
        return;
    };
    let platform = editor.platform_id.parse::<retro_junk_core::Platform>().ok();
    let supported_formats = platform
        .map(super::playable_formats::supported)
        .unwrap_or_default();
    ui.separator();
    ui.heading(&editor.title);
    egui::Grid::new("archive_game_identity")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Platform");
            ui.label(&editor.platform_id);
            ui.end_row();
            ui.label("Release");
            ui.label(release_identity(editor));
            ui.end_row();
            ui.label("Catalog");
            if editor.catalog_release_id.is_empty() {
                if editor.release_binding_state == "carrier_resolved" {
                    ui.label(format!(
                        "{} · exact carrier matches (compatible masterings)",
                        editor.catalog_source
                    ));
                } else {
                    ui.colored_label(egui::Color32::YELLOW, &editor.release_binding_state);
                }
            } else {
                ui.label(format!(
                    "{} · {}",
                    editor.catalog_source, editor.release_binding_state
                ))
                .on_hover_text(&editor.catalog_release_id);
            }
            ui.end_row();
            ui.label("Carrier");
            let carrier = if editor.carrier_serial.is_empty() {
                editor.carrier_kind.clone()
            } else {
                format!("{} · {}", editor.carrier_kind, editor.carrier_serial)
            };
            ui.label(format!("{} · {}", carrier, editor.carrier_binding_state));
            ui.end_row();
        });
    ui.add_space(6.0);
    ui.heading("Physical copy and playable policy");
    ui.weak("Physical condition, acquisition, and provenance are intentionally user-supplied; catalog identity is shown separately above.");
    egui::Grid::new("physical_copy_editor").show(ui, |ui| {
        ui.label("Label");
        ui.text_edit_singleline(&mut editor.label);
        ui.end_row();
        ui.label("Condition");
        ui.text_edit_singleline(&mut editor.condition);
        ui.end_row();
        ui.label("Acquired");
        ui.text_edit_singleline(&mut editor.date_acquired);
        ui.end_row();
        ui.label("Provenance");
        ui.text_edit_multiline(&mut editor.provenance);
        ui.end_row();
        ui.label("Notes");
        ui.text_edit_multiline(&mut editor.notes);
        ui.end_row();
        ui.label("Desired playable format");
        let selected_format = parse_format(&editor.desired_format).ok();
        let selected_label = if editor.desired_format.is_empty() {
            "inherit / none".to_owned()
        } else if let Some(format) = selected_format.as_ref() {
            let label = super::playable_formats::label(Some(format));
            if supported_formats.contains(format) {
                label.to_owned()
            } else {
                format!("{label} (unsupported)")
            }
        } else {
            format!("{} (unsupported)", editor.desired_format)
        };
        egui::ComboBox::from_id_salt("desired_playable_format")
            .selected_text(selected_label)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut editor.desired_format, String::new(), "inherit / none");
                if supported_formats.is_empty() {
                    ui.weak("No modeled emulator-ready format");
                }
                for format in &supported_formats {
                    ui.selectable_value(
                        &mut editor.desired_format,
                        format_slug(format).to_owned(),
                        super::playable_formats::label(Some(format)),
                    );
                }
            })
            .response
            .on_hover_text("Formats accepted directly by mainstream emulators for this platform");
        ui.end_row();
        ui.label("");
        ui.checkbox(
            &mut editor.retain_intermediate,
            "Retain canonical intermediate",
        );
        ui.end_row();
        ui.label("New dump format");
        egui::ComboBox::from_id_salt("archive_ingest_format")
            .selected_text(&editor.ingest_format)
            .show_ui(ui, |ui| {
                for format in ["rom", "redumper-raw", "cue-bin", "iso"] {
                    ui.selectable_value(&mut editor.ingest_format, format.to_owned(), format);
                }
            });
        ui.end_row();
        ui.label("");
        ui.checkbox(
            &mut editor.allow_unverified,
            "Allow build without catalog evidence",
        );
        ui.end_row();
        ui.label("New release artwork type");
        egui::ComboBox::from_id_salt("archive_release_asset_type")
            .selected_text(&editor.release_asset_type)
            .show_ui(ui, |ui| {
                for asset_type in [
                    "cover",
                    "3D box",
                    "screenshot",
                    "title screen",
                    "marquee",
                    "fanart",
                    "physical media",
                    "miximage",
                ] {
                    ui.selectable_value(
                        &mut editor.release_asset_type,
                        asset_type.to_owned(),
                        asset_type,
                    );
                }
            });
        ui.end_row();
    });
    let save = ui.button("Save physical copy and policy").clicked();
    let mut add_file = None;
    let mut add_release_artwork = false;
    let mut ingest_source = None;
    ui.horizontal(|ui| {
        if ui.button("Add release artwork…").clicked() {
            add_release_artwork = true;
        }
        if ui.button("Add physical-copy photo…").clicked() {
            add_file = Some((
                retro_junk_archive::PhysicalCopyFileCategory::Photo,
                "physical-copy-photo",
            ));
        }
        if ui.button("Add provenance document…").clicked() {
            add_file = Some((
                retro_junk_archive::PhysicalCopyFileCategory::Provenance,
                "provenance-document",
            ));
        }
    });
    ui.horizontal(|ui| {
        if ui.button("Ingest dump file…").clicked() {
            ingest_source = rfd::FileDialog::new().pick_file();
        }
        if ui.button("Ingest dump directory…").clicked() {
            ingest_source = rfd::FileDialog::new().pick_folder();
        }
    });
    let editor_snapshot = editor.clone();
    if save {
        match save_editor(&editor_snapshot, &profile.archive_root) {
            Ok(()) => {
                crate::backend::archive::start_archive_operation(app, profile, false);
            }
            Err(error) => app.push_error("Collection details", error),
        }
    }
    if let Some((category, asset_type)) = add_file
        && let Some(source_file) = rfd::FileDialog::new().pick_file()
    {
        let result = (|| -> Result<(), String> {
            let _archive_lock = retro_junk_archive::ArchiveLock::acquire(&profile.archive_root)
                .map_err(|error| error.to_string())?;
            let physical_copy_id = editor_snapshot
                .physical_copy_id
                .parse::<retro_junk_archive::PhysicalCopyId>()
                .map_err(|error| error.to_string())?;
            retro_junk_archive::add_physical_copy_file(
                &profile.archive_root,
                retro_junk_archive::NewPhysicalCopyFile {
                    physical_copy_id,
                    source_file: &source_file,
                    category,
                    asset_type,
                    source: "user",
                    caption: "",
                },
                &AtomicBool::new(false),
            )
            .map_err(|error| error.to_string())?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                crate::backend::archive::start_archive_operation(app, profile, false);
            }
            Err(error) => app.push_error("Physical-copy file", error),
        }
    }
    if add_release_artwork
        && let Some(source_file) = rfd::FileDialog::new()
            .add_filter("Artwork", &["png", "jpg", "jpeg", "webp"])
            .pick_file()
    {
        let result = (|| -> Result<(), String> {
            let _archive_lock = retro_junk_archive::ArchiveLock::acquire(&profile.archive_root)
                .map_err(|error| error.to_string())?;
            let release_id = editor_snapshot
                .archive_release_id
                .parse::<retro_junk_archive::ArchiveReleaseId>()
                .map_err(|error| error.to_string())?;
            retro_junk_archive::add_release_file(
                &profile.archive_root,
                retro_junk_archive::NewReleaseFile {
                    release_id,
                    source_file: &source_file,
                    category: retro_junk_archive::ReleaseFileCategory::Artwork,
                    asset_type: &editor_snapshot.release_asset_type,
                    source: "user",
                    source_url: "",
                    caption: "",
                },
                &AtomicBool::new(false),
            )
            .map_err(|error| error.to_string())?;
            Ok(())
        })();
        match result {
            Ok(()) => crate::backend::archive::start_archive_operation(app, profile, false),
            Err(error) => app.push_error("Release artwork", error),
        }
    }
    if let Some(source) = ingest_source {
        match parse_format(&editor_snapshot.ingest_format) {
            Ok(format) => start_archive_ingest(app, profile, &editor_snapshot, source, format),
            Err(error) => app.push_error("Archive ingest", error),
        }
    }
}

fn save_editor(editor: &PhysicalCopyEditor, archive_root: &std::path::Path) -> Result<(), String> {
    let _archive_lock = retro_junk_archive::ArchiveLock::acquire(archive_root)
        .map_err(|error| error.to_string())?;
    let mut item: retro_junk_archive::PhysicalCopyManifest =
        retro_junk_archive::read_toml(&editor.physical_copy_manifest_path)
            .map_err(|error| error.to_string())?;
    item.label.clone_from(&editor.label);
    item.condition.clone_from(&editor.condition);
    item.notes.clone_from(&editor.notes);
    item.date_acquired.clone_from(&editor.date_acquired);
    item.provenance.clone_from(&editor.provenance);
    retro_junk_archive::write_toml_atomic(&editor.physical_copy_manifest_path, &item)
        .map_err(|error| error.to_string())?;
    let mut medium: retro_junk_archive::CarrierManifest =
        retro_junk_archive::read_toml(&editor.carrier_manifest_path)
            .map_err(|error| error.to_string())?;
    medium.playable_policy = if editor.desired_format.is_empty() {
        None
    } else {
        Some(retro_junk_archive::DesiredPlayablePolicy {
            format: parse_format(&editor.desired_format)?,
            retain_canonical_intermediate: editor.retain_intermediate,
            allow_unverified: editor.allow_unverified,
            options: std::collections::BTreeMap::new(),
        })
    };
    retro_junk_archive::write_toml_atomic(&editor.carrier_manifest_path, &medium)
        .map_err(|error| error.to_string())
}

fn parse_format(value: &str) -> Result<retro_junk_archive::RepresentationFormat, String> {
    match value {
        "rom" => Ok(retro_junk_archive::RepresentationFormat::Rom),
        "chd" => Ok(retro_junk_archive::RepresentationFormat::Chd),
        "rvz" => Ok(retro_junk_archive::RepresentationFormat::Rvz),
        "iso" => Ok(retro_junk_archive::RepresentationFormat::Iso),
        "cue-bin" => Ok(retro_junk_archive::RepresentationFormat::CueBin),
        _ => Err(format!("unsupported playable format: {value}")),
    }
}

fn format_slug(format: &retro_junk_archive::RepresentationFormat) -> &str {
    match format {
        retro_junk_archive::RepresentationFormat::Rom => "rom",
        retro_junk_archive::RepresentationFormat::Chd => "chd",
        retro_junk_archive::RepresentationFormat::Rvz => "rvz",
        retro_junk_archive::RepresentationFormat::Iso => "iso",
        retro_junk_archive::RepresentationFormat::CueBin => "cue-bin",
        retro_junk_archive::RepresentationFormat::RedumperRaw => "redumper-raw",
        retro_junk_archive::RepresentationFormat::Other(value) => value,
    }
}

fn start_dump_import_planning(
    app: &mut RetroJunkApp,
    profile: &retro_junk_archive::CollectionProfile,
    source: std::path::PathBuf,
    promote_playable: bool,
) {
    let Some(db_path) = app.db_path.clone() else {
        app.push_error("Import dumps", "Catalog database is unavailable".to_owned());
        return;
    };
    let op_id = next_operation_id();
    let cancel = Arc::new(AtomicBool::new(false));
    app.operations.push(BackgroundOperation::new(
        op_id,
        format!("Enumerating package files in {}", source.display()),
        Arc::clone(&cancel),
        OperationKind::ArchiveImport,
        "archive".to_owned(),
        ProgressDisplay::Bytes,
    ));
    app.ui_state.dump_import_dialog = Some(DumpImportDialogState::Planning {
        op_id,
        source: source.clone(),
    });
    let sender = app.message_tx.clone();
    let progress_sender = sender.clone();
    let context = Arc::clone(&app.context);
    let playable_root = promote_playable.then(|| source.clone());
    let request = retro_junk_archive_import::DumpImportRequest {
        source,
        archive_root: profile.archive_root.clone(),
        platform_hint: None,
        owner_id: "default".to_owned(),
        new_physical_copy: false,
        redumper_path: None,
        workspace_root: Some(profile.workspace_root.clone()),
        stage_packages_locally: profile.network_mode,
        playable_root,
    };
    let handle = std::thread::spawn(move || {
        let result = (|| {
            let catalog =
                retro_junk_db::open_database(&db_path).map_err(|error| error.to_string())?;
            retro_junk_archive_import::plan_import(
                request,
                context.as_ref(),
                &catalog,
                &cancel,
                |current, total| {
                    let _ = progress_sender.send(AppMessage::OperationProgress {
                        op_id,
                        current,
                        total,
                    });
                },
                |phase| {
                    let display = match phase.kind {
                        retro_junk_archive_import::PlanningProgressKind::Bytes => {
                            ProgressDisplay::Bytes
                        }
                        retro_junk_archive_import::PlanningProgressKind::Items
                        | retro_junk_archive_import::PlanningProgressKind::Indeterminate => {
                            ProgressDisplay::Count
                        }
                    };
                    let _ = progress_sender.send(AppMessage::OperationPhase {
                        op_id,
                        description: phase.description,
                        display,
                        current: phase.current,
                        total: phase.total,
                    });
                },
            )
            .map_err(|error| error.to_string())
        })();
        let _ = sender.send(AppMessage::ArchiveImportPlanReady { op_id, result });
    });
    app.op_threads.insert(op_id, handle);
}

fn start_dump_import(
    app: &mut RetroJunkApp,
    profile: &retro_junk_archive::CollectionProfile,
    plan: retro_junk_archive_import::DumpImportPlan,
    consume: bool,
) {
    let op_id = next_operation_id();
    let cancel = Arc::new(AtomicBool::new(false));
    let mut operation = BackgroundOperation::new(
        op_id,
        "Importing preservation dumps".to_owned(),
        Arc::clone(&cancel),
        OperationKind::ArchiveImport,
        "archive".to_owned(),
        ProgressDisplay::Bytes,
    );
    operation.progress_total = plan.total_source_bytes;
    app.operations.push(operation);
    app.ui_state.dump_import_dialog = Some(DumpImportDialogState::Importing { op_id });
    let sender = app.message_tx.clone();
    let progress_sender = sender.clone();
    let db_path = app.db_path.clone();
    let profile = profile.clone();
    let media_dir_setting = app.settings.general.assets_dir.clone();
    let handle = std::thread::spawn(move || {
        let result = retro_junk_archive_import::execute_import(
            plan,
            consume,
            &cancel,
            |progress| {
                let _ = progress_sender.send(AppMessage::OperationProgress {
                    op_id,
                    current: progress.copied_bytes,
                    total: progress.total_bytes,
                });
            },
            |phase| {
                let display = match phase.kind {
                    retro_junk_archive_import::PlanningProgressKind::Bytes => {
                        ProgressDisplay::Bytes
                    }
                    retro_junk_archive_import::PlanningProgressKind::Items
                    | retro_junk_archive_import::PlanningProgressKind::Indeterminate => {
                        ProgressDisplay::Count
                    }
                };
                let _ = progress_sender.send(AppMessage::OperationPhase {
                    op_id,
                    description: phase.description,
                    display,
                    current: phase.current,
                    total: phase.total,
                });
            },
        )
        .map_err(|error| error.to_string())
        .and_then(|result| {
            if let Some(db_path) = db_path {
                let _ = progress_sender.send(AppMessage::OperationPhase {
                    op_id,
                    description: "Refreshing the collection index".to_owned(),
                    display: ProgressDisplay::Count,
                    current: 0,
                    total: 0,
                });
                let _archive_lock = retro_junk_archive::ArchiveLock::acquire(&profile.archive_root)
                    .map_err(|error| error.to_string())?;
                let mut snapshot = retro_junk_archive::scan_archive(&profile.archive_root)
                    .map_err(|error| error.to_string())?;
                let mut connection =
                    retro_junk_db::open_database(&db_path).map_err(|error| error.to_string())?;
                let adopted = crate::backend::assets::adopt_playable_artwork(
                    &connection,
                    &snapshot,
                    &profile,
                    &media_dir_setting,
                    &cancel,
                )?;
                if adopted > 0 {
                    log::info!(
                        "Adopted {adopted} existing playable artwork file(s) into the archive"
                    );
                    snapshot = retro_junk_archive::scan_archive(&profile.archive_root)
                        .map_err(|error| error.to_string())?;
                }
                retro_junk_db::reconcile_archive_snapshot(
                    &mut connection,
                    &snapshot,
                    &profile.playable_root,
                    &profile.workspace_root,
                )
                .map_err(|error| error.to_string())?;
            }
            Ok(result)
        });
        let _ = sender.send(AppMessage::ArchiveImportComplete { op_id, result });
    });
    app.op_threads.insert(op_id, handle);
}

pub fn show_import_modal(ctx: &egui::Context, app: &mut RetroJunkApp) {
    let Some(mut dialog) = app.ui_state.dump_import_dialog.take() else {
        return;
    };
    let mut action = ImportModalAction::None;
    egui::Modal::new(egui::Id::new("archive_dump_import_modal")).show(ctx, |ui| {
        ui.set_min_width(640.0);
        match &mut dialog {
            DumpImportDialogState::Planning { op_id, source } => {
                ui.heading("Inspecting dump packages");
                egui::Grid::new("archive_import_planning_paths")
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label("Source");
                        ui.label(source.display().to_string());
                        ui.end_row();
                        ui.label("Local workspace");
                        ui.label(
                            app.settings
                                .library
                                .active_profile()
                                .map_or_else(|| "Unavailable".to_owned(), |profile| {
                                    profile.workspace_root.display().to_string()
                                }),
                        );
                        ui.end_row();
                    });
                if app
                    .settings
                    .library
                    .active_profile()
                    .is_some_and(|profile| profile.network_mode)
                {
                    ui.weak(
                        "Network mode is on: package files are staged locally once, and preservation hashes are calculated during that copy.",
                    );
                } else {
                    ui.weak(
                        "Network mode is off: preservation hashes and package analysis run against the source files in place.",
                    );
                }
                show_import_progress(ui, app, *op_id);
                if ui.button("Cancel").clicked() {
                    cancel_operation(app, *op_id);
                }
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            }
            DumpImportDialogState::Review {
                plan,
                consume,
                new_physical_copy,
            } => {
                let promoting_playable = plan.request.playable_root.is_some();
                ui.heading(if promoting_playable {
                    "Review playable-library promotion"
                } else {
                    "Review dump import"
                });
                if promoting_playable {
                    ui.weak("Matching files will be copied into the archive; existing playable files remain in place and are adopted as byte-identical representations.");
                }
                ui.label(format!(
                    "{} package(s), {}",
                    plan.candidates.len(),
                    format_bytes(plan.total_source_bytes)
                ));
                ui.checkbox(
                    new_physical_copy,
                    "Create a new physical copy instead of reusing an archived copy",
                );
                ui.add_space(6.0);
                let import_request = plan.request.clone();
                egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                    for (index, candidate) in plan.candidates.iter_mut().enumerate() {
                        ui.group(|ui| {
                            ui.strong(candidate.source.display().to_string());
                            ui.horizontal_wrapped(|ui| {
                                ui.label(format!("{:?}", candidate.format));
                                ui.separator();
                                ui.label(import_identification_label(
                                    &candidate.identification,
                                ));
                                ui.separator();
                                if let Some(selected) = candidate.selected_match.as_ref() {
                                    ui.label(format!(
                                        "archive: {} · catalog: {}",
                                        candidate.archive_platform_id, selected.platform_id
                                    ));
                                    ui.separator();
                                }
                                if *new_physical_copy
                                    && matches!(candidate.disposition, retro_junk_archive_import::ImportDisposition::NeedsPhysicalCopyChoice { .. })
                                {
                                    ui.label("ready for a new physical copy");
                                } else {
                                    ui.label(import_disposition_label(&candidate.disposition));
                                }
                            });
                            match candidate.disposition.clone() {
                                retro_junk_archive_import::ImportDisposition::NeedsCatalogChoice { candidates } => {
                                    let mut selection = None;
                                    egui::ComboBox::from_id_salt(("catalog-import-choice", index))
                                        .selected_text("Choose catalog release…")
                                        .show_ui(ui, |ui| {
                                            for catalog_candidate in candidates {
                                                let label =
                                                    catalog_candidate_label(&catalog_candidate);
                                                if ui.selectable_label(false, label).clicked() {
                                                    selection = Some(catalog_candidate);
                                                }
                                            }
                                    });
                                    if let Some(selected) = selection {
                                        candidate.archive_platform_id =
                                            retro_junk_archive_import::physical_archive_platform(
                                                &import_request,
                                                &candidate.source,
                                                &selected,
                                            );
                                        candidate.selected_match = Some(selected);
                                        candidate.identification = retro_junk_archive_import::IdentificationResolution::Identified {
                                            method: retro_junk_archive_import::IdentificationMethod::UserSelection,
                                        };
                                        candidate.disposition = retro_junk_archive_import::ImportDisposition::Ready;
                                    }
                                }
                                retro_junk_archive_import::ImportDisposition::NeedsPhysicalCopyChoice { copies }
                                    if !*new_physical_copy => {
                                    let mut selection = None;
                                    egui::ComboBox::from_id_salt(("physical-copy-import-choice", index))
                                        .selected_text("Choose physical copy…")
                                        .show_ui(ui, |ui| {
                                            for copy_id in copies {
                                                let label = if copy_id.label.is_empty() {
                                                    format!("copy-{:02}", copy_id.copy_number)
                                                } else {
                                                    format!("copy-{:02} · {}", copy_id.copy_number, copy_id.label)
                                                };
                                                if ui.selectable_label(false, label).clicked() {
                                                    selection = Some(copy_id.physical_copy_id);
                                                }
                                            }
                                    });
                                    if let Some(copy_id) = selection {
                                        candidate.physical_copy_id = Some(copy_id);
                                        candidate.disposition = retro_junk_archive_import::ImportDisposition::Ready;
                                    }
                                }
                                retro_junk_archive_import::ImportDisposition::Unresolved { .. }
                                    if ui.button("Archive as an unbound release…").clicked() => {
                                    let title = candidate
                                        .source
                                        .file_stem()
                                        .and_then(|value| value.to_str())
                                        .unwrap_or("Unknown release")
                                        .to_owned();
                                    let platform_id = import_request
                                        .platform_hint
                                        .clone()
                                        .unwrap_or_else(|| candidate.archive_platform_id.clone());
                                    candidate.disposition =
                                        retro_junk_archive_import::ImportDisposition::ReadyUnbound {
                                            title,
                                            platform_id,
                                        };
                                    candidate.identification =
                                        retro_junk_archive_import::IdentificationResolution::Unresolved;
                                }
                                retro_junk_archive_import::ImportDisposition::ReadyUnbound {
                                    mut title,
                                    mut platform_id,
                                } => {
                                    ui.weak("No catalog match will be claimed. Give the release an honest local identity.");
                                    egui::Grid::new(("unbound-import-identity", index))
                                        .num_columns(2)
                                        .show(ui, |ui| {
                                            ui.label("Title");
                                            ui.text_edit_singleline(&mut title);
                                            ui.end_row();
                                            ui.label("Platform");
                                            ui.text_edit_singleline(&mut platform_id);
                                            ui.end_row();
                                        });
                                    candidate.archive_platform_id.clone_from(&platform_id);
                                    candidate.disposition =
                                        retro_junk_archive_import::ImportDisposition::ReadyUnbound {
                                            title,
                                            platform_id,
                                        };
                                }
                                _ => {}
                            }
                        });
                    }
                });
                ui.add_space(6.0);
                if !promoting_playable {
                    ui.checkbox(consume, "Remove source packages after byte-for-byte verification");
                    if *consume {
                        ui.colored_label(egui::Color32::YELLOW, "Source removal occurs only after the archived copy is rehashed successfully.");
                    }
                }
                ui.horizontal(|ui| {
                    if ui.button("Import ready packages").clicked() {
                        let mut selected_plan = plan.clone();
                        if *new_physical_copy {
                            selected_plan.request.new_physical_copy = true;
                            for candidate in &mut selected_plan.candidates {
                                candidate.physical_copy_id = None;
                                if matches!(
                                    candidate.disposition,
                                    retro_junk_archive_import::ImportDisposition::NeedsPhysicalCopyChoice { .. }
                                ) {
                                    candidate.disposition = retro_junk_archive_import::ImportDisposition::Ready;
                                }
                            }
                        }
                        action = ImportModalAction::Import(
                            selected_plan,
                            if promoting_playable { false } else { *consume },
                        );
                    }
                    if ui.button("Cancel").clicked() {
                        action = ImportModalAction::Close;
                    }
                });
            }
            DumpImportDialogState::Importing { op_id } => {
                ui.heading("Importing preservation dumps");
                show_import_progress(ui, app, *op_id);
                if ui.button("Cancel").clicked() {
                    cancel_operation(app, *op_id);
                }
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            }
            DumpImportDialogState::Complete { result } => {
                ui.heading("Dump import complete");
                let results_height = (ctx.content_rect().height() - 160.0).clamp(120.0, 420.0);
                egui::ScrollArea::vertical()
                    .max_height(results_height)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for candidate in &result.results {
                            ui.horizontal_wrapped(|ui| {
                                ui.strong(format!("{:?}", candidate.outcome));
                                ui.label(candidate.source.display().to_string());
                                ui.weak(&candidate.detail);
                                if candidate.source_removed {
                                    ui.weak("source removed");
                                }
                            });
                        }
                    });
                ui.separator();
                if ui.button("Close").clicked() {
                    action = ImportModalAction::Close;
                }
            }
        }
    });
    match action {
        ImportModalAction::None => app.ui_state.dump_import_dialog = Some(dialog),
        ImportModalAction::Close => {}
        ImportModalAction::Import(plan, consume) => {
            let Some(profile) = app.settings.library.active_profile().cloned() else {
                app.push_error(
                    "Import dumps",
                    "Collection profile is unavailable".to_owned(),
                );
                return;
            };
            start_dump_import(app, &profile, plan, consume);
        }
    }
}

fn show_import_progress(ui: &mut egui::Ui, app: &RetroJunkApp, op_id: u64) {
    ui.add(egui::Spinner::new());
    if let Some(operation) = app
        .operations
        .iter()
        .find(|operation| operation.id == op_id)
    {
        ui.strong(&operation.description);
        if operation.progress_total > 0 {
            ui.add(egui::ProgressBar::new(operation.progress_fraction()).show_percentage());
            ui.weak(match operation.display {
                ProgressDisplay::Bytes => format!(
                    "{} / {}",
                    format_bytes(operation.progress_current),
                    format_bytes(operation.progress_total)
                ),
                ProgressDisplay::Count => format!(
                    "{} / {} items",
                    operation.progress_current, operation.progress_total
                ),
                ProgressDisplay::Percent => {
                    format!("{:.0}%", operation.progress_fraction() * 100.0)
                }
            });
        } else {
            ui.spinner();
        }
    }
}

fn cancel_operation(app: &RetroJunkApp, op_id: u64) {
    if let Some(operation) = app
        .operations
        .iter()
        .find(|operation| operation.id == op_id)
    {
        operation
            .cancel_token
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

fn import_disposition_label(disposition: &retro_junk_archive_import::ImportDisposition) -> String {
    use retro_junk_archive_import::ImportDisposition;
    match disposition {
        ImportDisposition::Ready => "ready to import".to_owned(),
        ImportDisposition::ReadyUnbound { .. } => {
            "ready to import without catalog binding".to_owned()
        }
        ImportDisposition::AlreadyArchived { .. } => "already archived".to_owned(),
        ImportDisposition::NeedsCatalogChoice { .. } => "catalog choice required".to_owned(),
        ImportDisposition::NeedsPhysicalCopyChoice { .. } => {
            "physical-copy choice required".to_owned()
        }
        ImportDisposition::Unresolved { reason } | ImportDisposition::Invalid { reason } => {
            reason.clone()
        }
    }
}

fn import_identification_label(
    identification: &retro_junk_archive_import::IdentificationResolution,
) -> &'static str {
    use retro_junk_archive_import::{IdentificationMethod, IdentificationResolution};
    match identification {
        IdentificationResolution::CatalogVerified {
            method: IdentificationMethod::CompleteTrackSet,
        } => "Catalog hashes verified · complete track set",
        IdentificationResolution::CatalogVerified {
            method: IdentificationMethod::ExactFileHash,
        } => "Catalog hashes verified · exact file",
        IdentificationResolution::CatalogVerified {
            method: IdentificationMethod::FormatAwareFileHash,
        } => "Catalog hashes verified · normalized payload",
        IdentificationResolution::CatalogVerified { .. } => "Catalog hashes verified",
        IdentificationResolution::Identified {
            method: IdentificationMethod::HeaderSerial,
        } => "Catalog identity inferred from header serial · not hash verified",
        IdentificationResolution::Identified {
            method: IdentificationMethod::FolderSerial,
        } => "Catalog identity inferred from folder serial · not hash verified",
        IdentificationResolution::Identified {
            method: IdentificationMethod::UserSelection,
        } => "Catalog identity selected by user · not hash verified",
        IdentificationResolution::Identified { .. } => {
            "Catalog identity inferred · not hash verified"
        }
        IdentificationResolution::Ambiguous => "Catalog identity ambiguous",
        IdentificationResolution::Unresolved => "Catalog identity unresolved",
    }
}

fn catalog_candidate_label(candidate: &retro_junk_archive_import::CatalogCandidate) -> String {
    let mut qualifiers = [
        candidate.platform_id.as_str(),
        candidate.region.as_str(),
        candidate.revision.as_str(),
        candidate.variant.as_str(),
        candidate.serial.as_str(),
    ]
    .into_iter()
    .filter(|value| !value.is_empty())
    .map(str::to_owned)
    .collect::<Vec<_>>();
    if candidate.sequence_number > 0 {
        qualifiers.push(format!("disc {}", candidate.sequence_number));
    }
    qualifiers.push(candidate.source.clone());
    qualifiers.push(format!("release {}", candidate.release_id));
    format!("{} · {}", candidate.title, qualifiers.join(" · "))
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GiB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1} MiB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

fn release_display_title(release: &retro_junk_db::ArchiveReleaseSummary) -> String {
    let suffix = match (release.region.is_empty(), release.revision.is_empty()) {
        (true, true) => String::new(),
        (false, true) => format!(" ({})", release.region),
        (true, false) => format!(" ({})", release.revision),
        (false, false) => format!(" ({}, {})", release.region, release.revision),
    };
    format!("{}{}", release.title, suffix)
}

fn release_identity(editor: &PhysicalCopyEditor) -> String {
    [
        (!editor.region.is_empty()).then_some(editor.region.as_str()),
        (!editor.revision.is_empty()).then_some(editor.revision.as_str()),
        (!editor.variant.is_empty()).then_some(editor.variant.as_str()),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ")
}

fn paint_cell_text(ui: &mut egui::Ui, text: &str) {
    if text.is_empty() {
        return;
    }
    let rect = ui.max_rect();
    let font_id = egui::TextStyle::Body.resolve(ui.style());
    let color = ui.visuals().text_color();
    ui.painter().with_clip_rect(rect).text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        text,
        font_id,
        color,
    );
}

fn start_archive_ingest(
    app: &mut RetroJunkApp,
    profile: &retro_junk_archive::CollectionProfile,
    editor: &PhysicalCopyEditor,
    source: std::path::PathBuf,
    format: retro_junk_archive::RepresentationFormat,
) {
    let op_id = next_operation_id();
    let cancel = Arc::new(AtomicBool::new(false));
    app.operations.push(BackgroundOperation::new(
        op_id,
        format!("Ingesting {}", source.display()),
        Arc::clone(&cancel),
        OperationKind::Other,
        "archive".to_owned(),
        ProgressDisplay::Bytes,
    ));
    let sender = app.message_tx.clone();
    let profile = profile.clone();
    let carrier_manifest_path = editor.carrier_manifest_path.clone();
    let db_path = app.db_path.clone();
    let media_dir_setting = app.settings.general.assets_dir.clone();
    let handle = std::thread::spawn(move || {
        let result = (|| -> Result<String, String> {
            let _archive_lock = retro_junk_archive::ArchiveLock::acquire(&profile.archive_root)
                .map_err(|error| error.to_string())?;
            let medium: retro_junk_archive::CarrierManifest =
                retro_junk_archive::read_toml(&carrier_manifest_path)
                    .map_err(|error| error.to_string())?;
            let manifest = retro_junk_archive::DumpManifest::new(medium.carrier_id, format);
            let media_directory = carrier_manifest_path
                .parent()
                .ok_or_else(|| "media manifest has no parent directory".to_owned())?;
            let destination = retro_junk_archive::ArchiveLayout::dump_dir(
                media_directory,
                &manifest.captured_at,
                manifest.dump_id,
            );
            let plan = retro_junk_archive::plan_ingest(&source, &destination)
                .map_err(|error| error.to_string())?;
            let total = plan.total_bytes;
            let _ = sender.send(AppMessage::OperationProgress {
                op_id,
                current: 0,
                total,
            });
            let dump = retro_junk_archive::execute_ingest(
                retro_junk_archive::IngestRequest { plan, manifest },
                &cancel,
                |progress| {
                    let _ = sender.send(AppMessage::OperationProgress {
                        op_id,
                        current: progress.copied_bytes,
                        total: progress.total_bytes,
                    });
                },
            )
            .map_err(|error| error.to_string())?;
            let mut snapshot = retro_junk_archive::scan_archive(&profile.archive_root)
                .map_err(|error| error.to_string())?;
            if let Some(db_path) = db_path {
                let mut connection =
                    retro_junk_db::open_database(&db_path).map_err(|error| error.to_string())?;
                let adopted = crate::backend::assets::adopt_playable_artwork(
                    &connection,
                    &snapshot,
                    &profile,
                    &media_dir_setting,
                    &cancel,
                )?;
                if adopted > 0 {
                    log::info!(
                        "Adopted {adopted} existing playable artwork file(s) into the archive"
                    );
                    snapshot = retro_junk_archive::scan_archive(&profile.archive_root)
                        .map_err(|error| error.to_string())?;
                }
                retro_junk_db::reconcile_archive_snapshot(
                    &mut connection,
                    &snapshot,
                    &profile.playable_root,
                    &profile.workspace_root,
                )
                .map_err(|error| error.to_string())?;
            }
            Ok(format!(
                "Archived dump {} from {}; the source was retained",
                dump.dump_id,
                source.display()
            ))
        })();
        let _ = sender.send(AppMessage::ArchiveOperationComplete { op_id, result });
    });
    app.op_threads.insert(op_id, handle);
}
