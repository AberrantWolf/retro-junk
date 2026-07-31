pub mod asset_types;
pub mod error;
pub mod esde;
pub mod miximage;
pub mod miximage_layout;

pub use asset_types::{AssetSelection, AssetType, DISPLAY_ASSET_TYPES, collect_existing_assets};
pub use error::FrontendError;

use std::path::Path;

/// A scraped game with metadata and media paths.
#[derive(Debug, Clone)]
pub struct ScrapedGame {
    /// ROM filename (stem only, no extension)
    pub rom_stem: String,
    /// ROM filename with extension
    pub rom_filename: String,
    /// Display name
    pub name: String,
    /// Game description (empty = unknown, omitted from output)
    pub description: String,
    /// Developer name (empty = unknown, omitted from output)
    pub developer: String,
    /// Publisher name (empty = unknown, omitted from output)
    pub publisher: String,
    /// Genre (empty = unknown, omitted from output)
    pub genre: String,
    /// Number of players (e.g., "1", "1-4"; empty = unknown, omitted from output)
    pub players: String,
    /// Rating from 0.0 to 1.0
    pub rating: Option<f32>,
    /// Release date in YYYYMMDD format (empty = unknown, omitted from output)
    pub release_date: String,
    /// Map of asset type to downloaded file path
    pub assets: std::collections::HashMap<AssetType, std::path::PathBuf>,
    /// Box/cover title from catalog DB (used as ES-DE `<name>` when non-empty)
    pub cover_title: String,
}

/// Trait for gaming frontend metadata generators.
pub trait Frontend {
    fn name(&self) -> &'static str;

    /// Generate metadata file(s) for a set of scraped games.
    fn write_metadata(
        &self,
        games: &[ScrapedGame],
        rom_dir: &Path,
        metadata_dir: &Path,
        media_dir: &Path,
    ) -> Result<(), FrontendError>;

    /// Return the expected asset subdirectory layout for this frontend.
    fn asset_subdirs(&self) -> &[(&str, AssetType)];
}
