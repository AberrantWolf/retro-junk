pub mod client;
pub mod credentials;
pub mod error;
pub mod log;
pub mod lookup;
pub mod media;
pub mod scrape;
pub mod systems;
pub mod types;

pub use client::{ScreenScraperClient, create_client};
pub use credentials::{
    CredentialSource, CredentialSources, Credentials, config_path, credential_sources,
    has_embedded_dev_credentials, save_to_file,
};
pub use error::ScrapeError;
pub use log::{LogEntry, ScrapeLog};
pub use lookup::{LookupMethod, LookupResult};
pub use media::{MediaSelection, media_subdir};
pub use scrape::{ScrapeEvent, ScrapeOptions, ScrapeResult, scrape_folder};
pub use systems::{expects_serial, region_to_language, region_to_ss_code, screenscraper_system_id};
