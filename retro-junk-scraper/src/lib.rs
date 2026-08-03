pub mod assets;
pub mod client;
pub mod credentials;
pub mod derivation;
pub mod error;
pub mod log;
pub mod lookup;
pub mod scrape;
pub mod session;
pub mod systems;
pub mod types;

pub use client::{ScreenScraperClient, create_client};
pub use credentials::{
    CREDENTIAL_FIELDS, CredentialFieldMeta, CredentialSource, CredentialSources, Credentials,
    config_path, credential_sources, ensure_config_file, has_embedded_dev_credentials,
    save_to_file,
};
pub use derivation::{Derivation, ParentIdentity};
pub use error::ScrapeError;
pub use log::{LogEntry, ScrapeLog};
pub use lookup::{LookupMethod, LookupResult, RomHashes, RomInfo};
pub use retro_junk_frontend::AssetSelection;
pub use scrape::{
    FolderArchiveBinding, FolderScrapeRequest, ScrapeOptions, ScrapeResult, scrape_folder,
};
pub use session::{
    ArchivePublication, MiximageMode, ScrapeDestination, ScrapeEvent, ScrapeRequest,
    ScrapeRunResult, ScrapeSessionOptions, ScrapeTarget, TargetOutcome, TargetState, is_fatal,
    release_ids_by_output, run_scrape, run_scrape_blocking,
};
pub use systems::{expects_serial, region_to_language, region_to_ss_code, screenscraper_system_id};
