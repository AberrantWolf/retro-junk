use std::path::{Path, PathBuf};

use retro_junk_frontend::MediaType;
use tokio::sync::mpsc;

use crate::client::ScreenScraperClient;
use crate::error::ScrapeError;
use crate::scrape::ScrapeEvent;
use crate::types::GameInfo;

/// Configuration for which media types to download.
#[derive(Debug, Clone)]
pub struct MediaSelection {
    pub types: Vec<MediaType>,
}

impl Default for MediaSelection {
    fn default() -> Self {
        Self {
            types: vec![
                MediaType::Cover,
                MediaType::Cover3D,
                MediaType::Screenshot,
                MediaType::Marquee,
                MediaType::PhysicalMedia,
                MediaType::Video,
            ],
        }
    }
}

impl MediaSelection {
    pub fn all() -> Self {
        Self {
            types: vec![
                MediaType::Cover,
                MediaType::Cover3D,
                MediaType::Screenshot,
                MediaType::TitleScreen,
                MediaType::Marquee,
                MediaType::Video,
                MediaType::Fanart,
                MediaType::PhysicalMedia,
            ],
        }
    }

    /// Parse from a comma-separated list (e.g., "covers,screenshots,videos").
    pub fn from_names(names: &[String]) -> Self {
        let types = names
            .iter()
            .filter_map(|n| match n.as_str() {
                "covers" | "cover" => Some(MediaType::Cover),
                "3dboxes" | "3dbox" | "cover3d" => Some(MediaType::Cover3D),
                "screenshots" | "screenshot" => Some(MediaType::Screenshot),
                "titlescreens" | "titlescreen" => Some(MediaType::TitleScreen),
                "marquees" | "marquee" => Some(MediaType::Marquee),
                "videos" | "video" => Some(MediaType::Video),
                "fanart" => Some(MediaType::Fanart),
                "physicalmedia" => Some(MediaType::PhysicalMedia),
                _ => None,
            })
            .collect();
        Self { types }
    }
}

/// Map a MediaType to the ScreenScraper media type string.
fn ss_media_type(mt: MediaType) -> &'static str {
    match mt {
        MediaType::Screenshot => "ss",
        MediaType::TitleScreen => "sstitle",
        MediaType::Cover => "box-2D",
        MediaType::Cover3D => "box-3D",
        MediaType::Marquee => "wheel-hd",
        MediaType::Video => "video-normalized",
        MediaType::Fanart => "fanart",
        MediaType::PhysicalMedia => "support-2D",
        MediaType::Miximage => unreachable!("Miximage is generated, not downloaded"),
    }
}

/// Fallback ScreenScraper media type if the primary isn't found.
fn ss_media_type_fallback(mt: MediaType) -> Option<&'static str> {
    match mt {
        MediaType::Marquee => Some("wheel"),
        MediaType::Video => Some("video"),
        _ => None,
    }
}

/// Subdirectory name for a media type (matches ES-DE layout).
pub fn media_subdir(mt: MediaType) -> &'static str {
    match mt {
        MediaType::Cover => "covers",
        MediaType::Cover3D => "3dboxes",
        MediaType::Screenshot => "screenshots",
        MediaType::TitleScreen => "titlescreens",
        MediaType::Marquee => "marquees",
        MediaType::Video => "videos",
        MediaType::Fanart => "fanart",
        MediaType::PhysicalMedia => "physicalmedia",
        MediaType::Miximage => "miximages",
    }
}

/// Collect paths for media files that already exist on disk for a given ROM.
///
/// Returns a map of MediaType -> path for every selected media type that has
/// an existing file in the expected location. Miximage is excluded from the
/// returned map (it's checked separately).
pub fn collect_existing_media(
    selection: &MediaSelection,
    media_dir: &Path,
    rom_stem: &str,
) -> std::collections::HashMap<MediaType, PathBuf> {
    let mut found = std::collections::HashMap::new();

    for &mt in &selection.types {
        if mt == MediaType::Miximage {
            continue;
        }
        let subdir = media_dir.join(media_subdir(mt));
        let ext = mt.default_extension();
        let path = subdir.join(format!("{}.{}", rom_stem, ext));
        if path.exists() {
            found.insert(mt, path);
        }
    }

    found
}

/// Download all selected media for a game.
///
/// Returns a map of MediaType -> downloaded file path.
pub async fn download_game_media(
    client: &ScreenScraperClient,
    game: &GameInfo,
    selection: &MediaSelection,
    media_dir: &Path,
    rom_stem: &str,
    preferred_region: &str,
    force_redownload: bool,
    index: usize,
    filename: &str,
    events: &mpsc::UnboundedSender<ScrapeEvent>,
) -> Result<std::collections::HashMap<MediaType, PathBuf>, ScrapeError> {
    let mut results = std::collections::HashMap::new();
    let mut downloads = Vec::new();

    for &mt in &selection.types {
        // Miximage is generated locally, never downloaded
        if mt == MediaType::Miximage {
            continue;
        }
        let ss_type = ss_media_type(mt);
        let media = game
            .media_for_region(ss_type, preferred_region)
            .or_else(|| {
                ss_media_type_fallback(mt)
                    .and_then(|fb| game.media_for_region(fb, preferred_region))
            });

        if let Some(media) = media {
            let ext = if media.format.is_empty() {
                mt.default_extension()
            } else {
                &media.format
            };
            let subdir = media_dir.join(media_subdir(mt));
            let dest = subdir.join(format!("{}.{}", rom_stem, ext));

            // Skip if file already exists (unless force redownload)
            if !force_redownload && dest.exists() {
                results.insert(mt, dest);
                continue;
            }

            downloads.push((mt, media.url.clone(), subdir, dest));
        }
    }

    // Build (MediaType, Future) pairs so we can emit events before each download
    let handles: Vec<_> = downloads
        .into_iter()
        .map(|(mt, url, subdir, dest)| {
            let client_url = url.clone();
            let client_ref = client;
            let fut = async move {
                std::fs::create_dir_all(&subdir)?;
                let bytes = client_ref.download_media(&client_url).await?;
                std::fs::write(&dest, &bytes)?;
                Ok::<PathBuf, ScrapeError>(dest)
            };
            (mt, fut)
        })
        .collect();

    // Run sequentially per game, emitting an event before each download
    for (mt, fut) in handles {
        let _ = events.send(ScrapeEvent::GameDownloadingMedia {
            index,
            file: filename.to_string(),
            media_type: mt.to_string(),
        });
        match fut.await {
            Ok(path) => {
                results.insert(mt, path);
            }
            Err(e) => {
                // Log but don't fail the whole scrape for a single media download failure
                log::debug!("Failed to download media: {}", e);
            }
        }
    }

    Ok(results)
}
