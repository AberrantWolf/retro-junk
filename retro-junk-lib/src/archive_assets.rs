//! Restore frontend media projections from authoritative archive originals.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use retro_junk_archive::{BuildId, IndexedRelease, SupportingFileId, sha256_file};
use retro_junk_frontend::AssetType;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AssetProjectionReport {
    pub copied: usize,
    pub current: usize,
    pub skipped_unknown: usize,
    /// Projected files removed because the name they carried is one this
    /// release no longer publishes under.
    pub removed_stale: usize,
    pub destinations: Vec<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum AssetProjectionError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Archive(String),
    #[error("projected asset did not match archive evidence: {0}")]
    Integrity(String),
}

/// Names which a frontend may use for one logical release.
///
/// Single-disc games use playable output stems. Multi-disc games additionally
/// use the full `.m3u` directory name because ES-DE keys media by that name.
///
/// Only *current* builds count. `evidence/` is append-only, so a release
/// rebuilt or re-adopted under a corrected name keeps both names in its
/// history — and reading all of them made every projection republish artwork
/// under names no file has carried since, one copy per name, forever.
#[must_use]
pub fn release_media_stems(release: &IndexedRelease) -> BTreeSet<String> {
    let mut stems = BTreeSet::new();
    for evidence in retro_junk_archive::current_release_builds(release) {
        let output = Path::new(&evidence.relative_output_path);
        if let Some(stem) = output.file_stem().and_then(|value| value.to_str()) {
            stems.insert(stem.to_owned());
        }
        if let Some(directory) = output
            .parent()
            .and_then(Path::file_name)
            .and_then(|value| value.to_str())
            .filter(|name| name.to_ascii_lowercase().ends_with(".m3u"))
        {
            stems.insert(directory.to_owned());
        }
    }
    if stems.is_empty() {
        stems.insert(retro_junk_archive::slugify(&release.manifest.title));
    }
    stems
}

/// Names a release used to publish under and no longer does.
///
/// What a projection has to remove rather than write. A stem that is still
/// current is never returned, so two lineages sharing one output name cannot
/// delete each other's artwork.
#[must_use]
pub fn superseded_media_stems(release: &IndexedRelease) -> BTreeSet<String> {
    let current = release_media_stems(release);
    let mut stems = BTreeSet::new();
    for evidence in retro_junk_archive::superseded_release_builds(release) {
        let output = Path::new(&evidence.relative_output_path);
        if let Some(stem) = output
            .file_stem()
            .and_then(|value| value.to_str())
            .filter(|stem| !current.contains(*stem))
        {
            stems.insert(stem.to_owned());
        }
        if let Some(directory) = output
            .parent()
            .and_then(Path::file_name)
            .and_then(|value| value.to_str())
            .filter(|name| name.to_ascii_lowercase().ends_with(".m3u"))
            .filter(|name| !current.contains(*name))
        {
            stems.insert(directory.to_owned());
        }
    }
    stems
}

/// Project every recognized archived release asset to one console's frontend
/// media directory using the supplied frontend entry stems.
pub fn project_release_assets(
    release: &IndexedRelease,
    media_directory: &Path,
    stems: &BTreeSet<String>,
    cancelled: &AtomicBool,
) -> Result<AssetProjectionReport, AssetProjectionError> {
    // Artwork belonging to a name this release has stopped publishing under.
    // Removed here rather than left behind, because the media tree is a
    // projection: a file in it that no output answers to is not a spare copy,
    // it is a second game in the frontend under the old name.
    let mut report = AssetProjectionReport {
        removed_stale: remove_stale_projections(media_directory, &superseded_media_stems(release)),
        ..AssetProjectionReport::default()
    };
    for asset in &release.supporting_files {
        let Some(asset_type) = AssetType::from_archive_name(&asset.manifest.asset_type) else {
            report.skipped_unknown += 1;
            continue;
        };
        let source = asset.directory.join(&asset.manifest.file.path);
        for stem in stems {
            let destination = project_one(
                &source,
                &asset.manifest.file.sha256,
                asset.manifest.file.size,
                media_directory,
                stem,
                asset_type,
                cancelled,
            )?;
            if destination.1 {
                report.current += 1;
            } else {
                report.copied += 1;
            }
            report.destinations.push(destination.0);
        }
    }
    Ok(report)
}

/// Project all archive assets beneath a frontend media root laid out as
/// `<media-root>/<platform>/<asset-subdirectory>/...`.
pub fn project_archive_assets(
    archive_root: &Path,
    media_root: &Path,
    release_id: Option<&str>,
    cancelled: &AtomicBool,
) -> Result<AssetProjectionReport, AssetProjectionError> {
    let snapshot = retro_junk_archive::scan_archive(archive_root)
        .map_err(|error| AssetProjectionError::Archive(error.to_string()))?;
    let mut total = AssetProjectionReport::default();
    for release in snapshot.releases.iter().filter(|release| {
        release_id.is_none_or(|id| release.manifest.archive_release_id.to_string() == id)
    }) {
        for (platform, stems) in release_media_stems_by_platform(release) {
            let report =
                project_release_assets(release, &media_root.join(platform), &stems, cancelled)?;
            total.copied += report.copied;
            total.current += report.current;
            total.skipped_unknown += report.skipped_unknown;
            total.destinations.extend(report.destinations);
        }
    }
    Ok(total)
}

/// Add the release's preferred playable entry to an ES-DE gamelist while
/// preserving any existing user-managed metadata.
///
/// Multi-disc releases are exported only when a generated M3U is present;
/// exposing the individual discs as separate games would be a worse state.
pub fn sync_esde_gamelist_for_release(
    release: &IndexedRelease,
    playable_root: &Path,
    metadata_root: &Path,
    media_root: &Path,
) -> Result<Option<PathBuf>, AssetProjectionError> {
    // Only current builds: a superseded output whose file happens to still be
    // on disk is not a game, it is the leftover of a rename — and giving it a
    // gamelist entry is how one release became two in the frontend.
    let mut outputs = retro_junk_archive::current_release_builds(release)
        .into_iter()
        .map(|evidence| PathBuf::from(&evidence.relative_output_path))
        .filter(|relative| playable_root.join(relative).is_file())
        .collect::<BTreeSet<_>>();
    let playlist = outputs.iter().find(|relative| {
        relative
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("m3u"))
    });
    let carrier_count = release
        .physical_copies
        .iter()
        .map(|copy| copy.carriers.len())
        .max()
        .unwrap_or(0);
    let selected = if let Some(playlist) = playlist {
        playlist.clone()
    } else if carrier_count <= 1 {
        let Some(output) = outputs.pop_first() else {
            return Ok(None);
        };
        output
    } else {
        return Ok(None);
    };
    let Some(platform) = selected
        .components()
        .next()
        .and_then(|component| component.as_os_str().to_str())
    else {
        return Ok(None);
    };
    let relative_rom = selected
        .strip_prefix(platform)
        .map_err(|error| AssetProjectionError::Archive(error.to_string()))?;
    let rom_dir = playable_root.join(platform);
    let media_dir = media_root.join(platform);
    let metadata_dir = metadata_root.join(platform);
    // Entries for names this release has stopped publishing under. Upserting
    // is keyed on the path, so without this a rebuild under a corrected name
    // leaves the old entry behind and the frontend lists the game twice — once
    // playable, once pointing at a file that is gone.
    let retired = retro_junk_archive::superseded_release_builds(release)
        .into_iter()
        .filter_map(|evidence| {
            Path::new(&evidence.relative_output_path)
                .strip_prefix(platform)
                .ok()
                .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        })
        .collect::<BTreeSet<_>>();
    retro_junk_frontend::esde::remove_game_entries(&metadata_dir.join("gamelist.xml"), &retired)
        .map_err(|error| AssetProjectionError::Archive(error.to_string()))?;
    let stem = if relative_rom
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().ends_with(".m3u"))
    {
        relative_rom
            .parent()
            .and_then(Path::file_name)
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_owned()
    } else {
        relative_rom
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_owned()
    };
    let assets = collect_frontend_assets(&media_dir, &stem);
    retro_junk_frontend::esde::upsert_game_metadata(
        &retro_junk_frontend::ScrapedGame {
            rom_stem: stem,
            rom_filename: relative_rom.to_string_lossy().replace('\\', "/"),
            name: release.manifest.title.clone(),
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
        &metadata_dir,
        &media_dir,
    )
    .map_err(|error| AssetProjectionError::Archive(error.to_string()))?;
    Ok(Some(metadata_dir.join("gamelist.xml")))
}

fn collect_frontend_assets(
    media_directory: &Path,
    stem: &str,
) -> std::collections::HashMap<AssetType, PathBuf> {
    let mut found = std::collections::HashMap::new();
    for asset_type in [
        AssetType::Cover,
        AssetType::Cover3D,
        AssetType::Screenshot,
        AssetType::TitleScreen,
        AssetType::Marquee,
        AssetType::Video,
        AssetType::Fanart,
        AssetType::PhysicalMedia,
        AssetType::Miximage,
    ] {
        for extension in asset_type.discovery_extensions() {
            let path = media_directory
                .join(asset_type.subdirectory())
                .join(format!("{stem}.{extension}"));
            if path.is_file() {
                found.insert(asset_type, path);
                break;
            }
        }
    }
    found
}

#[must_use]
pub fn release_media_stems_by_platform(
    release: &IndexedRelease,
) -> std::collections::BTreeMap<String, BTreeSet<String>> {
    let mut grouped = std::collections::BTreeMap::<String, BTreeSet<String>>::new();
    for evidence in retro_junk_archive::current_release_builds(release) {
        let output = Path::new(&evidence.relative_output_path);
        let platform = output
            .components()
            .next()
            .and_then(|component| component.as_os_str().to_str())
            .unwrap_or(&release.manifest.platform_id)
            .to_owned();
        let stems = grouped.entry(platform).or_default();
        if let Some(stem) = output
            .file_stem()
            .and_then(|value| value.to_str())
            .map(str::to_owned)
        {
            stems.insert(stem);
        }
        if let Some(directory) = output
            .parent()
            .and_then(Path::file_name)
            .and_then(|value| value.to_str())
            .filter(|name| name.to_ascii_lowercase().ends_with(".m3u"))
        {
            stems.insert(directory.to_owned());
        }
    }
    if grouped.is_empty() {
        grouped.insert(
            release.manifest.platform_id.clone(),
            BTreeSet::from([retro_junk_archive::slugify(&release.manifest.title)]),
        );
    }
    grouped
}

pub fn generate_frontend_miximage(
    media_directory: &Path,
    stem: &str,
    layout: &retro_junk_frontend::miximage_layout::MiximageLayout,
) -> Result<Option<PathBuf>, AssetProjectionError> {
    let assets = collect_frontend_assets(media_directory, stem);
    let output = media_directory
        .join(AssetType::Miximage.subdirectory())
        .join(format!("{stem}.png"));
    retro_junk_frontend::miximage::generate_miximage(&assets, &output, layout)
        .map(|generated| generated.then_some(output))
        .map_err(|error| AssetProjectionError::Archive(error.to_string()))
}

/// Copy one asset file into the frontend media tree.
///
/// The single place anything writes the frontend media tree, whether the
/// source is an archive original, a freshly scraped download, or a composed
/// miximage. The extension follows the source, because `ScreenScraper` returns
/// JPG for types whose default is PNG and discovery matches on extension.
pub fn project_asset_file(
    source: &Path,
    media_directory: &Path,
    stem: &str,
    asset_type: AssetType,
) -> Result<PathBuf, AssetProjectionError> {
    let (destination, temporary) = projection_paths(source, media_directory, stem, asset_type)?;
    std::fs::copy(source, &temporary)?;
    commit_projection(&temporary, &destination)?;
    Ok(destination)
}

/// Where a projection writes, and the scratch name it stages through.
/// Delete projected media carrying any of `stems`, across every asset
/// directory.
///
/// Only the media tree is touched, and only files whose stem is one this
/// release has stopped using — never a stem it still publishes under, which
/// [`superseded_media_stems`] has already excluded. Everything here is
/// rebuildable from the archive's originals, so removing it costs nothing but
/// a re-projection; leaving it costs a duplicate game in the frontend.
///
/// A file that cannot be removed is logged and skipped: failing a projection
/// because a leftover is read-only would be a worse outcome than the leftover.
fn remove_stale_projections(media_directory: &Path, stems: &BTreeSet<String>) -> usize {
    if stems.is_empty() {
        return 0;
    }
    let mut removed = 0;
    for asset_type in AssetType::EVERY {
        let directory = media_directory.join(asset_type.subdirectory());
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for path in entries.flatten().map(|entry| entry.path()) {
            let is_stale = path
                .file_stem()
                .and_then(|value| value.to_str())
                .is_some_and(|stem| stems.contains(stem));
            if !is_stale || !path.is_file() {
                continue;
            }
            match std::fs::remove_file(&path) {
                Ok(()) => {
                    log::info!(
                        "Removed {} — its release no longer publishes under that name",
                        path.display()
                    );
                    removed += 1;
                }
                Err(error) => {
                    log::warn!(
                        "Could not remove stale projection {}: {error}",
                        path.display()
                    );
                }
            }
        }
    }
    removed
}

fn projection_paths(
    source: &Path,
    media_directory: &Path,
    stem: &str,
    asset_type: AssetType,
) -> Result<(PathBuf, PathBuf), AssetProjectionError> {
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_else(|| asset_type.default_extension());
    let directory = media_directory.join(asset_type.subdirectory());
    std::fs::create_dir_all(&directory)?;
    let token = SupportingFileId::new();
    Ok((
        directory.join(format!("{stem}.{extension}")),
        directory.join(format!(".{stem}.{token}.tmp")),
    ))
}

/// Swap a fully written temporary into place.
///
/// The delicate part: an existing file is moved aside first and restored if
/// the swap fails, so a failed projection never leaves the media tree without
/// the asset it already had.
fn commit_projection(temporary: &Path, destination: &Path) -> Result<(), AssetProjectionError> {
    if !destination.exists() {
        std::fs::rename(temporary, destination)?;
        return Ok(());
    }
    let directory = destination.parent().unwrap_or(Path::new("."));
    let backup = directory.join(format!(".projection-{}.backup", BuildId::new()));
    std::fs::rename(destination, &backup)?;
    if let Err(error) = std::fs::rename(temporary, destination) {
        let _ = std::fs::rename(&backup, destination);
        let _ = std::fs::remove_file(temporary);
        return Err(error.into());
    }
    std::fs::remove_file(backup)?;
    Ok(())
}

fn project_one(
    source: &Path,
    expected_sha256: &str,
    expected_size: u64,
    media_directory: &Path,
    stem: &str,
    asset_type: AssetType,
    cancelled: &AtomicBool,
) -> Result<(PathBuf, bool), AssetProjectionError> {
    let (destination, temporary) = projection_paths(source, media_directory, stem, asset_type)?;
    if destination.is_file() {
        let (size, sha256) = sha256_file(&destination, cancelled)
            .map_err(|error| AssetProjectionError::Archive(error.to_string()))?;
        if size == expected_size && sha256 == expected_sha256 {
            return Ok((destination, true));
        }
    }
    std::fs::copy(source, &temporary)?;
    // Verify before committing: a corrupted copy must never replace a good
    // projection.
    let (size, sha256) = sha256_file(&temporary, cancelled)
        .map_err(|error| AssetProjectionError::Archive(error.to_string()))?;
    if size != expected_size || sha256 != expected_sha256 {
        let _ = std::fs::remove_file(&temporary);
        return Err(AssetProjectionError::Integrity(
            source.display().to_string(),
        ));
    }
    commit_projection(&temporary, &destination)?;
    Ok((destination, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_projection_includes_multidisc_frontend_stem() {
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("archive");
        let source = temp.path().join("cover.png");
        std::fs::write(&source, b"cover").unwrap();
        retro_junk_archive::initialize_archive(
            &archive,
            &retro_junk_archive::ArchiveRootManifest::new("Test"),
        )
        .unwrap();
        let rom = temp.path().join("disc.bin");
        std::fs::write(&rom, b"disc").unwrap();
        let ingested = retro_junk_archive::ingest_new_carrier_dump(
            &archive,
            &rom,
            retro_junk_archive::NewCarrierDump {
                platform_id: "ps1".to_owned(),
                title: "Game".to_owned(),
                region: String::new(),
                revision: String::new(),
                variant: String::new(),
                owner_id: "default".to_owned(),
                physical_copy_label: String::new(),
                serial: String::new(),
                sequence_number: 1,
                carrier_label: String::new(),
                carrier_kind: retro_junk_archive::CarrierKind::OpticalDisc,
                format: retro_junk_archive::RepresentationFormat::Rom,
                catalog_binding: retro_junk_archive::CatalogBinding::default(),
                source_package: retro_junk_archive::SourcePackageRecord::default(),
                expected_files: Vec::new(),
                physical_copy_id: None,
            },
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        retro_junk_archive::add_release_file(
            &archive,
            retro_junk_archive::NewReleaseFile {
                release_id: ingested.release.archive_release_id,
                source_file: &source,
                category: retro_junk_archive::ReleaseFileCategory::Artwork,
                asset_type: "cover",
                source: "test",
                source_url: "",
                caption: "",
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        let snapshot = retro_junk_archive::scan_archive(&archive).unwrap();
        let release = &snapshot.releases[0];
        let mut stems = BTreeSet::new();
        stems.insert("Game.m3u".to_owned());
        let media = temp.path().join("media").join("ps1");
        let report =
            project_release_assets(release, &media, &stems, &AtomicBool::new(false)).unwrap();
        assert_eq!(report.copied, 1);
        assert!(media.join("covers/Game.m3u.png").is_file());
    }

    #[test]
    fn esde_sync_adds_a_built_single_disc_entry() {
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("archive");
        let playable = temp.path().join("roms");
        let source = temp.path().join("game.nes");
        std::fs::write(&source, b"rom").unwrap();
        retro_junk_archive::initialize_archive(
            &archive,
            &retro_junk_archive::ArchiveRootManifest::new("Test"),
        )
        .unwrap();
        let ingested = retro_junk_archive::ingest_new_carrier_dump(
            &archive,
            &source,
            retro_junk_archive::NewCarrierDump {
                platform_id: "nes".to_owned(),
                title: "Game".to_owned(),
                region: String::new(),
                revision: String::new(),
                variant: String::new(),
                owner_id: "default".to_owned(),
                physical_copy_label: String::new(),
                serial: String::new(),
                sequence_number: 0,
                carrier_label: String::new(),
                carrier_kind: retro_junk_archive::CarrierKind::Cartridge,
                format: retro_junk_archive::RepresentationFormat::Rom,
                catalog_binding: retro_junk_archive::CatalogBinding::default(),
                source_package: retro_junk_archive::SourcePackageRecord::default(),
                expected_files: Vec::new(),
                physical_copy_id: None,
            },
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        crate::playable_build::build_playable(
            &crate::playable_build::PlayableBuildRequest {
                archive_root: archive.clone(),
                playable_root: playable.clone(),
                workspace_root: temp.path().join("workspace"),
                dump_id: ingested.dump.dump_id.to_string(),
                format: retro_junk_archive::RepresentationFormat::Rom,
                chdman_path: PathBuf::new(),
                redumper_path: PathBuf::new(),
                dolphin_tool_path: PathBuf::new(),
                allow_unverified: true,
                retain_intermediate: false,
                options: std::collections::BTreeMap::new(),
                playable_platform_id: "nes".to_owned(),
                expected_disc_count: 1,
                canonical_output_stem: "Game".to_owned(),
                canonical_release_name: "Game".to_owned(),
            },
            &|_, _, _, _| {},
            &AtomicBool::new(false),
        )
        .unwrap();

        let snapshot = retro_junk_archive::scan_archive(&archive).unwrap();
        let metadata = temp.path().join("metadata");
        let media = temp.path().join("media");
        let path =
            sync_esde_gamelist_for_release(&snapshot.releases[0], &playable, &metadata, &media)
                .unwrap()
                .unwrap();
        let xml = std::fs::read_to_string(path).unwrap();
        assert!(xml.contains("<path>./Game.nes</path>"));
        assert!(xml.contains("<name>Game</name>"));
    }

    /// Replacing artwork must not leave the media tree without a file: the
    /// old one is moved aside and restored if the swap fails. Every surface
    /// that writes the frontend tree goes through this one function.
    #[test]
    fn projecting_over_an_existing_asset_replaces_it() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("original.png");
        std::fs::write(&source, b"new").unwrap();
        let media = dir.path().join("media");
        let covers = media.join(AssetType::Cover.subdirectory());
        std::fs::create_dir_all(&covers).unwrap();
        std::fs::write(covers.join("Game.png"), b"old").unwrap();

        let destination = project_asset_file(&source, &media, "Game", AssetType::Cover).unwrap();

        assert_eq!(destination, covers.join("Game.png"));
        assert_eq!(std::fs::read(&destination).unwrap(), b"new");
        let leftovers = std::fs::read_dir(&covers)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with('.'))
            .count();
        assert_eq!(leftovers, 0, "temporary or backup files were left behind");
    }

    /// The extension follows the source, because `ScreenScraper` returns JPG for
    /// types whose default is PNG and discovery matches on extension.
    #[test]
    fn projection_keeps_the_source_extension() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("original.jpg");
        std::fs::write(&source, b"jpeg").unwrap();

        let destination =
            project_asset_file(&source, &dir.path().join("media"), "Game", AssetType::Cover)
                .unwrap();

        assert_eq!(
            destination.extension().and_then(|e| e.to_str()),
            Some("jpg")
        );
    }
}
