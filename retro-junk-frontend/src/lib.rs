pub mod asset_types;
pub mod error;
pub mod esde;
pub mod miximage;
pub mod miximage_layout;

pub use asset_types::AssetType;
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
    /// Game description
    pub description: Option<String>,
    /// Developer name
    pub developer: Option<String>,
    /// Publisher name
    pub publisher: Option<String>,
    /// Genre
    pub genre: Option<String>,
    /// Number of players (e.g., "1", "1-4")
    pub players: Option<String>,
    /// Rating from 0.0 to 1.0
    pub rating: Option<f32>,
    /// Release date in YYYYMMDD format
    pub release_date: Option<String>,
    /// Map of asset type to downloaded file path
    pub assets: std::collections::HashMap<AssetType, std::path::PathBuf>,
    /// Box/cover title from catalog DB (used as ES-DE `<name>` when present)
    pub cover_title: Option<String>,
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
