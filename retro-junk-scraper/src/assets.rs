use std::path::{Path, PathBuf};

use retro_junk_frontend::AssetType;
use tokio::sync::mpsc;

use crate::client::ScreenScraperClient;
use crate::error::ScrapeError;
use crate::scrape::ScrapeEvent;
use crate::types::GameInfo;

/// Configuration for which asset types to download.
#[derive(Debug, Clone)]
pub struct AssetSelection {
    pub types: Vec<AssetType>,
}

impl Default for AssetSelection {
    fn default() -> Self {
        Self {
            types: vec![
                AssetType::Cover,
                AssetType::Cover3D,
                AssetType::Screenshot,
                AssetType::Marquee,
                AssetType::PhysicalMedia,
                AssetType::Video,
            ],
        }
    }
}

impl AssetSelection {
    pub fn all() -> Self {
        Self {
            types: vec![
                AssetType::Cover,
                AssetType::Cover3D,
                AssetType::Screenshot,
                AssetType::TitleScreen,
                AssetType::Marquee,
                AssetType::Video,
                AssetType::Fanart,
                AssetType::PhysicalMedia,
            ],
        }
    }

    /// Parse from a comma-separated list (e.g., "covers,screenshots,videos").
    pub fn from_names(names: &[String]) -> Self {
        let types = names
            .iter()
            .filter_map(|n| match n.as_str() {
                "covers" | "cover" => Some(AssetType::Cover),
                "3dboxes" | "3dbox" | "cover3d" => Some(AssetType::Cover3D),
                "screenshots" | "screenshot" => Some(AssetType::Screenshot),
                "titlescreens" | "titlescreen" => Some(AssetType::TitleScreen),
                "marquees" | "marquee" => Some(AssetType::Marquee),
                "videos" | "video" => Some(AssetType::Video),
                "fanart" => Some(AssetType::Fanart),
                "physicalmedia" => Some(AssetType::PhysicalMedia),
                _ => None,
            })
            .collect();
        Self { types }
    }
}

/// Map an AssetType to the ScreenScraper media type string.
fn ss_asset_type(at: AssetType) -> &'static str {
    match at {
        AssetType::Screenshot => "ss",
        AssetType::TitleScreen => "sstitle",
        AssetType::Cover => "box-2D",
        AssetType::Cover3D => "box-3D",
        AssetType::Marquee => "wheel-hd",
        AssetType::Video => "video-normalized",
        AssetType::Fanart => "fanart",
        AssetType::PhysicalMedia => "support-2D",
        AssetType::Miximage => unreachable!("Miximage is generated, not downloaded"),
    }
}

/// Fallback ScreenScraper media type if the primary isn't found.
fn ss_asset_type_fallback(at: AssetType) -> Option<&'static str> {
    match at {
        AssetType::Marquee => Some("wheel"),
        AssetType::Video => Some("video"),
        _ => None,
    }
}

/// Subdirectory name for an asset type (matches ES-DE layout).
pub fn asset_subdir(at: AssetType) -> &'static str {
    match at {
        AssetType::Cover => "covers",
        AssetType::Cover3D => "3dboxes",
        AssetType::Screenshot => "screenshots",
        AssetType::TitleScreen => "titlescreens",
        AssetType::Marquee => "marquees",
        AssetType::Video => "videos",
        AssetType::Fanart => "fanart",
        AssetType::PhysicalMedia => "physicalmedia",
        AssetType::Miximage => "miximages",
    }
}

/// Collect paths for asset files that already exist on disk for a given ROM.
///
/// Returns a map of AssetType -> path for every selected asset type that has
/// an existing file in the expected location. Miximage is excluded from the
/// returned map (it's checked separately).
pub fn collect_existing_assets(
    selection: &AssetSelection,
    media_dir: &Path,
    rom_stem: &str,
) -> std::collections::HashMap<AssetType, PathBuf> {
    let mut found = std::collections::HashMap::new();

    for &at in &selection.types {
        if at == AssetType::Miximage {
            continue;
        }
        let subdir = media_dir.join(asset_subdir(at));
        let ext = at.default_extension();
        let path = subdir.join(format!("{}.{}", rom_stem, ext));
        if path.exists() {
            found.insert(at, path);
        }
    }

    found
}

/// Download all selected assets for a game.
///
/// Returns a map of AssetType -> downloaded file path.
#[allow(clippy::too_many_arguments)]
pub async fn download_game_assets(
    client: &ScreenScraperClient,
    game: &GameInfo,
    selection: &AssetSelection,
    media_dir: &Path,
    rom_stem: &str,
    preferred_region: &str,
    force_redownload: bool,
    index: usize,
    filename: &str,
    events: &mpsc::UnboundedSender<ScrapeEvent>,
) -> Result<std::collections::HashMap<AssetType, PathBuf>, ScrapeError> {
    let mut results = std::collections::HashMap::new();
    let mut downloads = Vec::new();

    for &at in &selection.types {
        // Miximage is generated locally, never downloaded
        if at == AssetType::Miximage {
            continue;
        }
        let ss_type = ss_asset_type(at);
        let media = game
            .media_for_region(ss_type, preferred_region)
            .or_else(|| {
                ss_asset_type_fallback(at)
                    .and_then(|fb| game.media_for_region(fb, preferred_region))
            });

        if let Some(media) = media {
            let ext = if media.format.is_empty() {
                at.default_extension()
            } else {
                &media.format
            };
            let subdir = media_dir.join(asset_subdir(at));
            let dest = subdir.join(format!("{}.{}", rom_stem, ext));

            // Skip if file already exists (unless force redownload)
            if !force_redownload && dest.exists() {
                results.insert(at, dest);
                continue;
            }

            downloads.push((at, media.url.clone(), subdir, dest));
        }
    }

    // Build (AssetType, Future) pairs so we can emit events before each download
    let handles: Vec<_> = downloads
        .into_iter()
        .map(|(at, url, subdir, dest)| {
            let client_url = url.clone();
            let client_ref = client;
            let fut = async move {
                std::fs::create_dir_all(&subdir)?;
                let bytes = client_ref.download_media(&client_url).await?;
                std::fs::write(&dest, &bytes)?;
                Ok::<PathBuf, ScrapeError>(dest)
            };
            (at, fut)
        })
        .collect();

    // Run sequentially per game, emitting an event before each download
    for (at, fut) in handles {
        let _ = events.send(ScrapeEvent::GameDownloadingMedia {
            index,
            file: filename.to_string(),
            media_type: at.to_string(),
        });
        match fut.await {
            Ok(path) => {
                results.insert(at, path);
            }
            Err(e) => {
                // Log but don't fail the whole scrape for a single asset download failure
                log::debug!("Failed to download asset: {}", e);
            }
        }
    }

    Ok(results)
}
