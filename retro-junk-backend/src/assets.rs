//! Media-asset availability: which scraped files exist on disk for an entry.
//!
//! One definition of "how complete is this entry's artwork" shared by asset
//! discovery, scraping, and the frontends' status badges. Moved here from the
//! GUI so no frontend owns the answer.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use retro_junk_frontend::{AssetType, DISPLAY_ASSET_TYPES};

/// The 5 scrapeable asset types that a media re-scrape downloads
/// (matches `AssetSelection::default()` minus Video, which
/// [`collect_existing_assets`] skips).
pub const SCRAPEABLE_ASSET_TYPES: &[AssetType] = &[
    AssetType::Cover,
    AssetType::Cover3D,
    AssetType::Screenshot,
    AssetType::Marquee,
    AssetType::PhysicalMedia,
];

/// How much of an entry's scrapeable artwork is present on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetStatus {
    /// Filesystem availability has not been discovered yet.
    Unknown,
    /// Discovered, no scrapeable assets found
    None,
    /// Some but not all scrapeable asset types present
    Partial { found: u8, total: u8 },
    /// All scrapeable asset types present
    Complete,
}

/// Renderable artwork state from the authoritative completion projection.
/// Archived release facts stay separate from playable-library asset scans;
/// callers choose which projection they are presenting instead of taking an
/// optimistic maximum across the two stores.
#[must_use]
pub fn asset_status_from_completion(fraction: crate::completion::Fraction) -> AssetStatus {
    match fraction {
        crate::completion::Fraction::Unknown(_) => AssetStatus::Unknown,
        crate::completion::Fraction::Known { have: 0, .. } => AssetStatus::None,
        crate::completion::Fraction::Known { have, want } if want == 0 || have >= want => {
            AssetStatus::Complete
        }
        crate::completion::Fraction::Known { have, want } => AssetStatus::Partial {
            found: u8::try_from(have).unwrap_or(u8::MAX),
            total: u8::try_from(want).unwrap_or(u8::MAX),
        },
    }
}

fn asset_status_from_paths(media: &HashMap<AssetType, PathBuf>) -> AssetStatus {
    let total = SCRAPEABLE_ASSET_TYPES.len() as u8;
    let found = SCRAPEABLE_ASSET_TYPES
        .iter()
        .filter(|mt| media.contains_key(mt))
        .count() as u8;
    match found {
        0 => AssetStatus::None,
        n if n == total => AssetStatus::Complete,
        n => AssetStatus::Partial { found: n, total },
    }
}

/// Summarize discovered media as (completeness, has a miximage).
pub fn asset_availability(media: &HashMap<AssetType, PathBuf>) -> (AssetStatus, bool) {
    (
        asset_status_from_paths(media),
        media.contains_key(&AssetType::Miximage),
    )
}

/// Discover media files on disk for a given ROM entry, in display order.
pub fn collect_existing_assets(media_dir: &Path, rom_stem: &str) -> HashMap<AssetType, PathBuf> {
    retro_junk_frontend::collect_existing_assets(DISPLAY_ASSET_TYPES, media_dir, rom_stem)
}

/// Absolute paths of a release's archived artwork, keyed by asset type.
/// Archive rows record asset names as strings; this drops any name the
/// frontend does not recognize.
pub fn archived_asset_paths(
    release: &retro_junk_db::ArchivedLibraryListItem,
) -> HashMap<AssetType, PathBuf> {
    release
        .archived_assets
        .iter()
        .filter_map(|asset| {
            AssetType::from_archive_name(&asset.asset_type)
                .map(|asset_type| (asset_type, PathBuf::from(&asset.absolute_path)))
        })
        .collect()
}
