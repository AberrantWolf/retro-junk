pub mod error;
pub mod esde;
pub mod media_types;

pub use error::FrontendError;
pub use media_types::MediaType;

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
    /// Map of media type to downloaded file path
    pub media: std::collections::HashMap<MediaType, std::path::PathBuf>,
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

    /// Return the expected media subdirectory layout for this frontend.
    fn media_subdirs(&self) -> &[(&str, MediaType)];
}
