//! Media-asset availability: which scraped files exist on disk for an entry.
//!
//! One definition of "how complete is this entry's artwork" shared by asset
//! discovery, scraping, and the frontends' status badges. Moved here from the
//! GUI so no frontend owns the answer.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use retro_junk_frontend::{AssetSelection, AssetType, DISPLAY_ASSET_TYPES};

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

fn asset_status_from_paths<S: std::hash::BuildHasher>(
    media: &HashMap<AssetType, PathBuf, S>,
    expected: &AssetSelection,
) -> AssetStatus {
    let total = u8::try_from(expected.types.len()).unwrap_or(u8::MAX);
    let found = expected
        .types
        .iter()
        .filter(|mt| media.contains_key(mt))
        .count() as u8;
    match found {
        _ if total == 0 => AssetStatus::Complete,
        0 => AssetStatus::None,
        n if n == total => AssetStatus::Complete,
        n => AssetStatus::Partial { found: n, total },
    }
}

/// Summarize discovered media as (completeness, has a miximage).
#[must_use]
pub fn asset_availability<S: std::hash::BuildHasher>(
    media: &HashMap<AssetType, PathBuf, S>,
    expected: &AssetSelection,
) -> (AssetStatus, bool) {
    (
        asset_status_from_paths(media, expected),
        media.contains_key(&AssetType::Miximage),
    )
}

/// Discover media files on disk for a given ROM entry, in display order.
#[must_use]
pub fn collect_existing_assets(media_dir: &Path, rom_stem: &str) -> HashMap<AssetType, PathBuf> {
    retro_junk_frontend::collect_existing_assets(DISPLAY_ASSET_TYPES, media_dir, rom_stem)
}

/// Absolute paths of a release's archived artwork, keyed by asset type.
/// Archive rows record asset names as strings; this drops any name the
/// frontend does not recognize.
#[must_use]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_empty_policy_is_complete() {
        let expected = AssetSelection { types: Vec::new() };
        assert_eq!(
            asset_availability(&HashMap::new(), &expected).0,
            AssetStatus::Complete
        );
    }

    #[test]
    fn configured_video_participates_in_completion() {
        let expected = AssetSelection {
            types: vec![AssetType::Cover, AssetType::Video],
        };
        let mut media = HashMap::new();
        media.insert(AssetType::Cover, PathBuf::from("cover.png"));
        assert_eq!(
            asset_availability(&media, &expected).0,
            AssetStatus::Partial { found: 1, total: 2 }
        );
        media.insert(AssetType::Video, PathBuf::from("preview.mp4"));
        assert_eq!(
            asset_availability(&media, &expected).0,
            AssetStatus::Complete
        );
    }
}
