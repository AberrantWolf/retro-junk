use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, OnceLock};

use crate::app::RetroJunkApp;
use crate::state::{AppMessage, BackgroundOperation, OperationKind, ProgressDisplay};

/// Playable builds have a long parallelizable preparation phase, but currently
/// also publish archive evidence, frontend metadata, and a database projection.
/// Keep whole jobs FIFO-ish and exclusive until those commit phases are split
/// from conversion.
static PLAYABLE_BUILD_QUEUE: OnceLock<Mutex<()>> = OnceLock::new();

pub fn start(
    app: &mut RetroJunkApp,
    release: retro_junk_db::ArchivedPlayableGap,
    format: retro_junk_archive::RepresentationFormat,
    playable_platform_id: String,
    ctx: &egui::Context,
) {
    let Some(profile) = app.settings.library.active_profile().cloned() else {
        app.push_error("Archive action", "No active collection profile".to_owned());
        return;
    };
    let Some(db_path) = app.db_path.clone() else {
        app.push_error(
            "Archive action",
            "Catalog database is unavailable".to_owned(),
        );
        return;
    };
    let op_id = crate::state::next_operation_id();
    let cancel = Arc::new(AtomicBool::new(false));
    let release_label = if release.region.trim().is_empty() {
        release.title.clone()
    } else {
        format!("{} ({})", release.title, release.region)
    };
    app.operations.push(BackgroundOperation::new(
        op_id,
        if release.needs_playable {
            format!("Queued playable build for {release_label}")
        } else {
            format!("Queued archive verification for {release_label}")
        },
        Arc::clone(&cancel),
        OperationKind::Other,
        "archive".to_owned(),
        ProgressDisplay::Count,
    ));
    let sender = app.message_tx.clone();
    let chdman = PathBuf::from(app.settings.general.chdman_path.trim());
    let processing_workspace_root = profile.processing_workspace_root();
    let media_directory = retro_junk_lib::util::asset_dir_for_console(
        &profile.playable_root,
        &playable_platform_id,
        &app.settings.general.assets_dir,
    );
    let metadata_dir_setting = app.settings.general.metadata_dir.clone();
    let handle = std::thread::spawn(move || {
        let result = (|| {
            let _queue_turn = PLAYABLE_BUILD_QUEUE
                .get_or_init(|| Mutex::new(()))
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return Err("Playable build was cancelled while queued".to_owned());
            }
            let _ = sender.send(AppMessage::OperationPhase {
                op_id,
                description: format!("Starting playable build for {release_label}"),
                display: ProgressDisplay::Count,
                current: 0,
                total: 0,
            });
            let _archive_lock = retro_junk_archive::ArchiveLock::acquire(&profile.archive_root)
                .map_err(|error| error.to_string())?;
            let connection =
                retro_junk_db::open_database(&db_path).map_err(|error| error.to_string())?;
            // Verify every prerequisite first. A bad or mismatched disc stops
            // the release before any new playable derivatives are published.
            for carrier in &release.carriers {
                if carrier.catalog_verified || (release.allow_unverified && release.needs_playable)
                {
                    continue;
                }
                let media_id = carrier
                    .catalog_media_id
                    .as_deref()
                    .ok_or_else(|| format!("{} has no catalog disc binding", release.title))?;
                let media = retro_junk_db::get_media_by_id(&connection, media_id)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("Catalog medium {media_id} was not found"))?;
                let mut tracks = retro_junk_db::find_media_tracks(&connection, media_id)
                    .map_err(|error| error.to_string())?
                    .into_iter()
                    .map(|track| retro_junk_archive::TrackDigest {
                        number: u32::try_from(track.track_number).unwrap_or(0),
                        size: u64::try_from(track.file_size).unwrap_or(0),
                        crc32: track.crc32,
                        md5: track.md5,
                        sha1: track.sha1,
                    })
                    .collect::<Vec<_>>();
                if tracks.is_empty() && media.file_size > 0 {
                    tracks.push(retro_junk_archive::TrackDigest {
                        number: 1,
                        size: u64::try_from(media.file_size).unwrap_or(0),
                        crc32: media.crc32.clone(),
                        md5: media.md5.clone(),
                        sha1: media.sha1.clone(),
                    });
                }
                let dump_id = carrier
                    .dump_id
                    .clone()
                    .ok_or_else(|| format!("{} has no preservation dump", release.title))?;
                let disc_label = if carrier.sequence_number > 0 {
                    format!("Disc {}", carrier.sequence_number)
                } else {
                    "Disc".to_owned()
                };
                retro_junk_lib::playable_build::verify_dump_against_catalog(
                    &retro_junk_lib::playable_build::CatalogVerificationRequest {
                        archive_root: profile.archive_root.clone(),
                        workspace_root: processing_workspace_root.clone(),
                        dump_id,
                        redumper_path: PathBuf::new(),
                        expected_tracks: tracks,
                        catalog: retro_junk_archive::CatalogEvidence {
                            source: media.dat_source,
                            system: playable_platform_id.clone(),
                            version: String::new(),
                            game: media.dat_name,
                            complete_track_set: true,
                        },
                    },
                    &|description, current, total| {
                        send_progress(
                            &sender,
                            op_id,
                            &format!("{disc_label}: {description}"),
                            current,
                            total,
                        );
                    },
                    &cancel,
                )
                .map_err(|error| error.to_string())?;
            }
            let existing_playlist_files = if release.needs_playlist && !release.needs_playable {
                Some(load_existing_disc_paths(
                    &connection,
                    &release.archive_release_id,
                    &profile.playable_root,
                    release.expected_disc_count,
                )?)
            } else {
                None
            };
            let mut canonical_names = std::collections::HashMap::new();
            let mut catalog_game_names = Vec::new();
            for carrier in &release.carriers {
                let Some(media_id) = carrier.catalog_media_id.as_deref() else {
                    continue;
                };
                let Some(media) = retro_junk_db::get_media_by_id(&connection, media_id)
                    .map_err(|error| error.to_string())?
                else {
                    continue;
                };
                catalog_game_names.push(media.dat_name.clone());
                let canonical_source = if media.rom_name.trim().is_empty() {
                    media.dat_name.as_str()
                } else {
                    media.rom_name.as_str()
                };
                let stem = std::path::Path::new(canonical_source)
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or(canonical_source)
                    .to_owned();
                canonical_names.insert(carrier.carrier_id.clone(), stem);
            }
            let game_name_refs = catalog_game_names
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>();
            let canonical_release_name =
                retro_junk_core::disc::derive_base_game_name(&game_name_refs);
            drop(connection);

            let mut outputs = Vec::new();
            for carrier in &release.carriers {
                if !carrier.needs_playable {
                    continue;
                }
                let dump_id = carrier
                    .dump_id
                    .clone()
                    .ok_or_else(|| format!("{} has no preservation dump", release.title))?;
                let disc_label = if carrier.sequence_number > 0 {
                    format!("Disc {}", carrier.sequence_number)
                } else {
                    "Game".to_owned()
                };
                let request = retro_junk_lib::playable_build::PlayableBuildRequest {
                    archive_root: profile.archive_root.clone(),
                    playable_root: profile.playable_root.clone(),
                    workspace_root: processing_workspace_root.clone(),
                    dump_id,
                    format: format.clone(),
                    chdman_path: chdman.clone(),
                    redumper_path: PathBuf::new(),
                    dolphin_tool_path: PathBuf::new(),
                    allow_unverified: release.allow_unverified,
                    retain_intermediate: release.retain_intermediate,
                    options: std::collections::BTreeMap::new(),
                    playable_platform_id: playable_platform_id.clone(),
                    expected_disc_count: release.expected_disc_count,
                    canonical_output_stem: canonical_names
                        .get(&carrier.carrier_id)
                        .cloned()
                        .unwrap_or_default(),
                    canonical_release_name: canonical_release_name.clone(),
                };
                let outcome = retro_junk_lib::playable_build::build_playable(
                    &request,
                    &|description, current, total| {
                        send_progress(
                            &sender,
                            op_id,
                            &format!("{disc_label}: {description}"),
                            current,
                            total,
                        );
                    },
                    &cancel,
                )
                .map_err(|error| error.to_string())?;
                outputs.push(outcome.output);
            }
            if let Some(files) = existing_playlist_files {
                outputs.push(write_existing_playlist(
                    &profile.playable_root,
                    &playable_platform_id,
                    &release.title,
                    &release.region,
                    &canonical_release_name,
                    &files,
                )?);
            }
            let frontend_output = outputs.last().cloned();
            let snapshot = retro_junk_archive::scan_archive(&profile.archive_root)
                .map_err(|error| error.to_string())?;
            if let Some(media_directory) = media_directory.as_deref()
                && let Some(indexed_release) = snapshot.releases.iter().find(|item| {
                    item.manifest.archive_release_id.to_string() == release.archive_release_id
                })
            {
                retro_junk_lib::archive_assets::project_release_assets(
                    indexed_release,
                    media_directory,
                    &retro_junk_lib::archive_assets::release_media_stems(indexed_release),
                    &cancel,
                )
                .map_err(|error| error.to_string())?;
            }
            if let Some(frontend_output) = frontend_output.as_deref() {
                upsert_esde_entry(
                    &profile.playable_root,
                    &playable_platform_id,
                    &metadata_dir_setting,
                    media_directory.as_deref(),
                    frontend_output,
                    &release.title,
                )?;
            }
            let mut connection =
                retro_junk_db::open_database(&db_path).map_err(|error| error.to_string())?;
            retro_junk_db::reconcile_archive_snapshot(
                &mut connection,
                &snapshot,
                &profile.playable_root,
                &profile.workspace_root,
            )
            .map_err(|error| error.to_string())?;
            Ok(outputs.pop())
        })();
        let _ = sender.send(AppMessage::PlayableBuildComplete { op_id, result });
    });
    app.op_threads.insert(op_id, handle);
    ctx.request_repaint_after(std::time::Duration::from_millis(20));
}

fn upsert_esde_entry(
    playable_root: &std::path::Path,
    platform_id: &str,
    metadata_dir_setting: &str,
    media_directory: Option<&std::path::Path>,
    output: &std::path::Path,
    title: &str,
) -> Result<(), String> {
    let rom_dir = playable_root.join(platform_id);
    let relative = output.strip_prefix(&rom_dir).map_err(|_| {
        format!(
            "Playable output {} is outside {}",
            output.display(),
            rom_dir.display()
        )
    })?;
    let rom_filename = relative.to_string_lossy().replace('\\', "/");
    let stem = if relative
        .parent()
        .and_then(std::path::Path::file_name)
        .and_then(|value| value.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().ends_with(".m3u"))
    {
        relative
            .parent()
            .and_then(std::path::Path::file_name)
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_owned()
    } else {
        relative
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_owned()
    };
    let media_directory = media_directory.unwrap_or(&rom_dir);
    let assets = crate::state::collect_existing_assets(media_directory, &stem);
    let metadata_directory = retro_junk_lib::util::metadata_dir_for_console(
        playable_root,
        platform_id,
        metadata_dir_setting,
    );
    retro_junk_frontend::esde::upsert_game_metadata(
        &retro_junk_frontend::ScrapedGame {
            rom_stem: stem,
            rom_filename,
            name: title.to_owned(),
            description: String::new(),
            developer: String::new(),
            publisher: String::new(),
            genre: String::new(),
            players: String::new(),
            rating: None,
            release_date: String::new(),
            assets,
            cover_title: String::new(),
        },
        &rom_dir,
        &metadata_directory,
        media_directory,
    )
    .map(|_| ())
    .map_err(|error| error.to_string())
}

fn load_existing_disc_paths(
    connection: &retro_junk_db::Connection,
    archive_release_id: &str,
    playable_root: &std::path::Path,
    expected_disc_count: u32,
) -> Result<Vec<PathBuf>, String> {
    let mut statement = connection
        .prepare(
            "SELECT DISTINCT m.disc_number,e.game_entry_json,lc.folder_path
             FROM archive_releases ar
             JOIN physical_copies pc ON pc.archive_release_id=ar.id
             JOIN carriers c ON c.physical_copy_id=pc.id
             JOIN media m ON m.id=c.catalog_media_id AND m.disc_number>0
             JOIN library_entries e ON e.dat_game_name=m.dat_name
             JOIN library_consoles lc ON lc.id=e.console_id
             JOIN library_roots lr ON lr.id=lc.root_id
             WHERE ar.id=?1 AND lr.root_path=?2
             ORDER BY m.disc_number",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            (archive_release_id, playable_root.to_string_lossy().as_ref()),
            |row| {
                Ok((
                    row.get::<_, u32>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let mut paths = Vec::new();
    for (index, (disc_number, json, folder)) in rows.into_iter().enumerate() {
        if disc_number != u32::try_from(index + 1).unwrap_or(u32::MAX) {
            return Err("Existing playable discs are not a complete ordered set".to_owned());
        }
        let value: serde_json::Value =
            serde_json::from_str(&json).map_err(|error| error.to_string())?;
        let path = value
            .get("SingleFile")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "An existing disc is not a single-file playable entry".to_owned())?;
        let path = PathBuf::from(path);
        paths.push(if path.is_absolute() {
            path
        } else {
            PathBuf::from(folder).join(path)
        });
    }
    if paths.len() != expected_disc_count as usize {
        return Err(format!(
            "Found {} of {expected_disc_count} existing playable discs",
            paths.len()
        ));
    }
    Ok(paths)
}

fn write_existing_playlist(
    playable_root: &std::path::Path,
    playable_platform_id: &str,
    title: &str,
    region: &str,
    canonical_release_name: &str,
    files: &[PathBuf],
) -> Result<PathBuf, String> {
    let display_name = if region.is_empty() {
        title.to_owned()
    } else {
        format!("{title} ({region})")
    };
    let stem = if canonical_release_name.trim().is_empty() {
        display_name
    } else {
        canonical_release_name.to_owned()
    };
    let directory = playable_root
        .join(retro_junk_archive::slugify(playable_platform_id))
        .join(format!("{stem}.m3u"));
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let playlist = directory.join(format!("{stem}.m3u"));
    if playlist.is_file() {
        return Ok(playlist);
    }
    let contents = files
        .iter()
        .map(|file| {
            pathdiff::diff_paths(file, &directory)
                .unwrap_or_else(|| file.clone())
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let temporary = directory.join(format!(
        ".playlist-{}.tmp",
        retro_junk_archive::BuildId::new()
    ));
    if let Err(error) =
        std::fs::write(&temporary, contents).and_then(|()| std::fs::rename(&temporary, &playlist))
    {
        let _ = std::fs::remove_file(&temporary);
        return Err(error.to_string());
    }
    Ok(playlist)
}

fn send_progress(
    sender: &crate::state::AppMessageSender,
    op_id: u64,
    description: &str,
    current: u64,
    total: u64,
) {
    let display = if total == 0 {
        ProgressDisplay::Count
    } else {
        ProgressDisplay::Bytes
    };
    let _ = sender.send(AppMessage::OperationPhase {
        op_id,
        description: description.to_owned(),
        display,
        current,
        total,
    });
}
