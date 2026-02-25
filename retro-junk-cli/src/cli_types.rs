//! CLI type definitions: command enums and argument structs.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use retro_junk_lib::Platform;

#[derive(Parser)]
#[command(name = "retro-junk")]
#[command(about = "Analyze retro game ROMs and disc images", long_about = None)]
pub(crate) struct Cli {
    /// Root path containing console folders (defaults to current directory)
    #[arg(short, long, global = true)]
    pub root: Option<PathBuf>,

    /// Only show warnings and errors (suppress normal output)
    #[arg(long, global = true)]
    pub quiet: bool,

    /// Enable verbose/debug logging (timestamps + debug-level messages)
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Write log output to a file (ANSI codes stripped)
    #[arg(long, global = true)]
    pub logfile: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

/// Common arguments for commands that process ROM files.
#[derive(Args, Clone)]
pub(crate) struct RomFilterArgs {
    /// Console names or aliases (e.g., snes,n64,ps1,gc,gg)
    #[arg(short, long, value_delimiter = ',')]
    pub consoles: Option<Vec<Platform>>,

    /// Maximum number of ROMs to process per console
    #[arg(short, long)]
    pub limit: Option<usize>,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    /// Analyze ROMs in a directory structure
    Analyze {
        /// Quick mode: read as little data as possible (useful for network shares)
        #[arg(short, long)]
        quick: bool,

        #[command(flatten)]
        roms: RomFilterArgs,
    },

    /// Rename ROM files to NoIntro canonical names
    Rename {
        /// Show planned renames without executing
        #[arg(short = 'n', long)]
        dry_run: bool,

        /// Force CRC32 hash-based matching (reads full files)
        #[arg(long)]
        hash: bool,

        #[command(flatten)]
        roms: RomFilterArgs,

        /// Use DAT files from this directory instead of the cache
        #[arg(long)]
        dat_dir: Option<PathBuf>,
    },

    /// [Experimental] Repair trimmed/truncated ROMs by padding to match DAT checksums
    Repair {
        /// Show planned repairs without executing
        #[arg(short = 'n', long)]
        dry_run: bool,

        /// Don't create .bak backup files
        #[arg(long)]
        no_backup: bool,

        #[command(flatten)]
        roms: RomFilterArgs,

        /// Use DAT files from this directory instead of the cache
        #[arg(long)]
        dat_dir: Option<PathBuf>,
    },

    /// Scrape game metadata and media from ScreenScraper.fr
    Scrape {
        #[command(flatten)]
        roms: RomFilterArgs,

        /// Media types to download (e.g., covers,screenshots,videos,marquees)
        #[arg(long, value_delimiter = ',')]
        media_types: Option<Vec<String>>,

        /// Directory for metadata files (default: <root>-metadata).
        /// Set to the same path as --root to place gamelist.xml inside ROM directories,
        /// which is needed for ES-DE with LegacyGamelistFileLocation enabled
        #[arg(long)]
        metadata_dir: Option<PathBuf>,

        /// Directory for media files (default: <root>-media)
        #[arg(long)]
        media_dir: Option<PathBuf>,

        /// Frontend to generate metadata for
        #[arg(long, default_value = "esde")]
        frontend: String,

        /// Preferred region for names/media (e.g., us, eu, jp)
        #[arg(long, default_value = "us")]
        region: String,

        /// Preferred language for descriptions (e.g., en, fr, match)
        #[arg(long, default_value = "en")]
        language: String,

        /// Fallback language when --language match has no data for the matched language
        #[arg(long, default_value = "en")]
        language_fallback: String,

        /// Hash all files even when serial/filename should suffice
        #[arg(long)]
        force_full_hash: bool,

        /// Show what would be scraped without downloading
        #[arg(short = 'n', long)]
        dry_run: bool,

        /// Skip games that already have metadata
        #[arg(long)]
        skip_existing: bool,

        /// Disable scrape log file
        #[arg(long)]
        no_log: bool,

        /// Disable miximage generation
        #[arg(long)]
        no_miximage: bool,

        /// Force redownload of all media, ignoring existing files
        #[arg(long)]
        force_redownload: bool,

        /// Maximum concurrent API threads (default: server-granted max)
        #[arg(long)]
        threads: Option<usize>,
    },

    /// Manage cached DAT files
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },

    /// Manage ScreenScraper credentials configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Manage the game catalog database
    Catalog {
        #[command(subcommand)]
        action: CatalogAction,
    },
}

#[derive(Subcommand)]
pub(crate) enum CacheAction {
    /// List cached DAT files
    List,

    /// Remove all cached DAT files
    Clear,

    /// Download DAT files for specified systems
    Fetch {
        /// Systems to fetch (e.g., snes,n64) or "all"
        #[arg(value_delimiter = ',')]
        systems: Vec<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum ConfigAction {
    /// Show current credentials and their sources
    Show,

    /// Interactively set up credentials
    Setup,

    /// Test credentials against the ScreenScraper API
    Test,

    /// Print the config file path
    Path,
}

#[derive(Subcommand)]
pub(crate) enum CatalogAction {
    /// Import DAT files into the catalog database
    Import {
        /// Systems to import (e.g., snes,n64) or "all"
        #[arg(value_delimiter = ',')]
        systems: Vec<String>,

        /// Path to catalog YAML data directory (default: ./catalog)
        #[arg(long)]
        catalog_dir: Option<PathBuf>,

        /// Path to the catalog database file (default: ~/.cache/retro-junk/catalog.db)
        #[arg(long)]
        db: Option<PathBuf>,

        /// Use DAT files from this directory instead of the cache
        #[arg(long)]
        dat_dir: Option<PathBuf>,
    },

    /// Enrich catalog releases with ScreenScraper metadata
    Enrich {
        /// Systems to enrich (e.g., nes,snes) or "all"
        #[arg(value_delimiter = ',')]
        systems: Vec<String>,

        /// Path to the catalog database file
        #[arg(long)]
        db: Option<PathBuf>,

        /// Maximum releases to process per system
        #[arg(long)]
        limit: Option<u32>,

        /// Re-enrich releases that already have ScreenScraper data
        #[arg(long)]
        force: bool,

        /// Download media assets
        #[arg(long)]
        download_assets: bool,

        /// Directory for downloaded media assets
        #[arg(long)]
        asset_dir: Option<PathBuf>,

        /// Preferred region for names and media (default: us)
        #[arg(long, default_value = "us")]
        region: String,

        /// Preferred language for descriptions (default: en)
        #[arg(long, default_value = "en")]
        language: String,

        /// Maximum concurrent API threads (default: server-granted max)
        #[arg(long)]
        threads: Option<usize>,

        /// Skip automatic work reconciliation after enrichment
        #[arg(long)]
        no_reconcile: bool,
    },

    /// Scan a ROM folder and add matched files to collection
    Scan {
        /// System to scan (e.g., nes, snes, n64)
        system: String,

        /// Path to ROM folder
        folder: PathBuf,

        /// Path to the catalog database file
        #[arg(long)]
        db: Option<PathBuf>,

        /// User ID for collection entries
        #[arg(long, default_value = "default")]
        user_id: String,
    },

    /// Re-verify collection entries against files on disk
    Verify {
        /// System to verify (e.g., nes, snes, n64)
        system: String,

        /// Path to the catalog database file
        #[arg(long)]
        db: Option<PathBuf>,

        /// User ID for collection entries
        #[arg(long, default_value = "default")]
        user_id: String,
    },

    /// List unresolved disagreements between data sources
    Disagreements {
        /// Path to the catalog database file
        #[arg(long)]
        db: Option<PathBuf>,

        /// Filter by system (e.g., nes, snes)
        #[arg(long)]
        system: Option<String>,

        /// Filter by field name (e.g., release_date, title)
        #[arg(long)]
        field: Option<String>,

        /// Maximum number of disagreements to show
        #[arg(long, default_value = "50")]
        limit: u32,
    },

    /// Resolve a disagreement by choosing a value
    Resolve {
        /// Disagreement ID to resolve
        id: i64,

        /// Path to the catalog database file
        #[arg(long)]
        db: Option<PathBuf>,

        /// Choose source A's value
        #[arg(long, group = "choice")]
        source_a: bool,

        /// Choose source B's value
        #[arg(long, group = "choice")]
        source_b: bool,

        /// Provide a custom resolution value
        #[arg(long, group = "choice")]
        custom: Option<String>,
    },

    /// Analyze media asset coverage gaps
    Gaps {
        /// System to analyze (e.g., nes, snes)
        system: String,

        /// Path to the catalog database file
        #[arg(long)]
        db: Option<PathBuf>,

        /// Only analyze releases in your collection
        #[arg(long)]
        collection_only: bool,

        /// Show releases missing this specific asset type (e.g., box-front, screenshot)
        #[arg(long)]
        missing: Option<String>,

        /// Maximum releases to list
        #[arg(long, default_value = "50")]
        limit: u32,
    },

    /// Browse, search, and look up games in the catalog database
    Lookup {
        /// Search query, prefixed ID (plt-X, wrk-X, rel-X, med-X), or omit to list
        query: Option<String>,

        /// Filter by entity type: platforms, works, releases, media
        #[arg(long, short = 't')]
        r#type: Option<String>,

        /// Filter by platform short name (e.g., nes, snes, psx)
        #[arg(long)]
        platform: Option<String>,

        /// Filter by manufacturer (e.g., Nintendo, Sega)
        #[arg(long)]
        manufacturer: Option<String>,

        /// Look up by CRC32 hash
        #[arg(long)]
        crc: Option<String>,

        /// Look up by SHA1 hash
        #[arg(long)]
        sha1: Option<String>,

        /// Look up by MD5 hash
        #[arg(long)]
        md5: Option<String>,

        /// Look up by serial number
        #[arg(long)]
        serial: Option<String>,

        /// Maximum number of results (default 25)
        #[arg(long, default_value = "25")]
        limit: u32,

        /// Skip this many results (for pagination)
        #[arg(long, default_value = "0")]
        offset: u32,

        /// Group results (e.g., platforms by manufacturer)
        #[arg(long)]
        group: bool,

        /// Path to the catalog database file
        #[arg(long)]
        db: Option<PathBuf>,
    },

    /// Merge duplicate works that share a ScreenScraper ID
    Reconcile {
        /// Systems to reconcile (e.g., nes,snes) or omit for all
        #[arg(value_delimiter = ',')]
        systems: Vec<String>,

        /// Path to the catalog database file
        #[arg(long)]
        db: Option<PathBuf>,

        /// Show what would be merged without making changes
        #[arg(short = 'n', long)]
        dry_run: bool,
    },

    /// Show catalog database statistics
    Stats {
        /// Path to the catalog database file
        #[arg(long)]
        db: Option<PathBuf>,
    },

    /// Clear enrichment status for releases (screenscraper_id and scraper_not_found)
    Unenrich {
        /// System to unenrich (e.g., nes, snes, n64)
        system: String,

        /// Only affect releases with titles at or after this value (case-insensitive)
        #[arg(long)]
        after: Option<String>,

        /// Path to the catalog database file
        #[arg(long)]
        db: Option<PathBuf>,

        /// Confirm the operation (required; without this, shows preview only)
        #[arg(long)]
        confirm: bool,
    },

    /// Delete and recreate the catalog database
    Reset {
        /// Path to the catalog database file
        #[arg(long)]
        db: Option<PathBuf>,

        /// Confirm database deletion (required)
        #[arg(long)]
        confirm: bool,
    },
}
