//! CLI type definitions: command enums and argument structs.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use retro_junk_lib::Platform;

#[derive(Parser)]
#[command(name = "retro-junk")]
#[command(about = "Analyze retro game ROMs and disc images", long_about = None)]
pub(crate) struct Cli {
    /// Library path containing console folders (falls back to saved config, then cwd)
    #[arg(short = 'L', long, global = true)]
    pub library_path: Option<PathBuf>,

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

/// Common arguments for commands that filter by console.
#[derive(Args, Clone)]
pub(crate) struct ConsoleFilterArgs {
    /// Console names or aliases (e.g., snes,n64,ps1,gc,gg)
    #[arg(short, long, value_delimiter = ',')]
    pub consoles: Option<Vec<Platform>>,

    /// Maximum number of ROMs to process per console
    #[arg(short, long)]
    pub limit: Option<usize>,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    /// Analyze games in a library directory structure
    Analyze {
        /// Quick mode: read as little data as possible (useful for network shares)
        #[arg(short, long)]
        quick: bool,

        #[command(flatten)]
        roms: ConsoleFilterArgs,
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
        roms: ConsoleFilterArgs,

        /// Use DAT files from this directory instead of the cache
        #[arg(long)]
        dat_dir: Option<PathBuf>,

        /// Directory for media files (default: <root>-media)
        #[arg(long)]
        media_dir: Option<PathBuf>,

        /// Don't rename media files alongside ROMs
        #[arg(long)]
        no_media: bool,
    },

    /// Organize loose disc images into ES-DE .m3u folders using Redump DAT names
    Organize {
        /// Show planned organization without executing
        #[arg(short = 'n', long)]
        dry_run: bool,

        #[command(flatten)]
        roms: ConsoleFilterArgs,

        /// Use DAT files from this directory instead of the cache
        #[arg(long)]
        dat_dir: Option<PathBuf>,

        /// Also organize single-disc games into .m3u folders (default: multi-disc only)
        #[arg(long)]
        include_single_disc: bool,

        /// Fall back to hashing when serial lookup fails (slower but catches more files)
        #[arg(long)]
        hash_fallback: bool,
    },

    /// Compress disc images (cue/bin, GDI, ISO) to CHD using chdman
    ///
    /// Every CHD is round-trip verified (re-extracted and compared
    /// track-by-track against the originals) before being reported as
    /// compressed. Originals are kept unless --delete-sources is given,
    /// and are never deleted when verification fails.
    Compress {
        /// Show planned compressions without executing
        #[arg(short = 'n', long)]
        dry_run: bool,

        /// Delete original files after each verified compression
        #[arg(long)]
        delete_sources: bool,

        /// Skip the per-console confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,

        /// Path to the chdman executable (default: chdman from PATH)
        #[arg(long)]
        chdman: Option<PathBuf>,

        #[command(flatten)]
        roms: ConsoleFilterArgs,
    },

    /// Fix CDRWin-format CUE sheets for wider emulator compatibility
    FixCue {
        /// Show planned fixes without executing
        #[arg(short = 'n', long)]
        dry_run: bool,

        /// Don't create .cue.bak backup files
        #[arg(long)]
        no_backup: bool,

        #[command(flatten)]
        roms: ConsoleFilterArgs,
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
        roms: ConsoleFilterArgs,

        /// Use DAT files from this directory instead of the cache
        #[arg(long)]
        dat_dir: Option<PathBuf>,
    },

    /// Scrape metadata and media into ES-DE gamelists via ScreenScraper.fr
    Scrape {
        #[command(flatten)]
        roms: ConsoleFilterArgs,

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

        /// Fallback region when ROM header detection fails (e.g., us, eu, jp). ROM-detected region is always preferred.
        #[arg(long, default_value = "us")]
        region: String,

        /// Language for descriptions: "match" derives from ROM region (default), or a code like "en", "ja", "fr"
        #[arg(long, default_value = "match")]
        language: String,

        /// Fallback language when region-matched language has no data (default: en)
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

    /// Manage cached DAT and GDB files
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },

    /// Manage ScreenScraper credentials
    Credentials {
        #[command(subcommand)]
        action: CredentialsAction,
    },

    /// Manage application settings (library path, etc.)
    Settings {
        #[command(subcommand)]
        action: SettingsAction,
    },

    /// Manage the game catalog database
    Catalog {
        #[command(subcommand)]
        action: CatalogAction,
    },

    /// List all supported systems and their capabilities
    Systems {
        /// Filter by manufacturer (e.g., Nintendo, Sega, Sony)
        #[arg(long, default_value = "")]
        manufacturer: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum CacheAction {
    /// Manage cached DAT files (No-Intro, Redump)
    Dat {
        #[command(subcommand)]
        action: DatCacheAction,
    },

    /// Manage cached GDB (GameDataBase) CSV files
    Gdb {
        #[command(subcommand)]
        action: GdbCacheAction,
    },
}

#[derive(Subcommand)]
pub(crate) enum DatCacheAction {
    /// List cached DAT files
    List,

    /// Remove all cached DAT files
    Clear,

    /// Download DAT files for specified systems
    #[command(
        after_help = "Examples:\n  retro-junk cache dat fetch all            Download all DATs\n  retro-junk cache dat fetch saturn --force  Re-download Saturn DAT"
    )]
    Fetch {
        /// Systems to fetch (e.g., snes,n64) or "all". Run 'retro-junk systems' for a full list.
        #[arg(value_delimiter = ',')]
        systems: Vec<String>,
        /// Re-download even if cached
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum GdbCacheAction {
    /// List cached GDB CSV files
    List,

    /// Remove all cached GDB CSV files
    Clear,

    /// Download GDB CSV files for specified systems
    #[command(
        after_help = "Examples:\n  retro-junk cache gdb fetch all            Download all GDB CSVs\n  retro-junk cache gdb fetch nes --force    Re-download NES GDB CSV"
    )]
    Fetch {
        /// Systems to fetch (e.g., nes,snes) or "all". Run 'retro-junk systems' for a full list.
        #[arg(value_delimiter = ',')]
        systems: Vec<String>,
        /// Re-download even if cached
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum CredentialsAction {
    /// Show current credentials and their sources
    Show,

    /// Interactively set up credentials
    Setup,

    /// Test credentials against the ScreenScraper API
    Test,

    /// Print the credentials file path
    Path,
}

#[derive(Subcommand)]
pub(crate) enum SettingsAction {
    /// Show all saved settings
    Show,

    /// Show or set the library path
    LibraryPath {
        /// New library path to save (omit to display current)
        path: Option<PathBuf>,

        /// Clear the saved library path
        #[arg(long)]
        clear: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum CatalogAction {
    /// Import DAT files into the catalog database
    #[command(
        after_help = "Examples:\n  retro-junk catalog import            Import all systems\n  retro-junk catalog import nes,snes   Import specific systems"
    )]
    Import {
        /// Systems to import (e.g., nes,snes) or "all". Defaults to all if omitted.
        #[arg(value_delimiter = ',')]
        systems: Vec<String>,

        /// Path to catalog YAML data directory
        #[arg(long, default_value = "catalog")]
        catalog_dir: PathBuf,

        /// Path to the catalog database file (default: ~/.cache/retro-junk/catalog.db)
        #[arg(long)]
        db: Option<PathBuf>,

        /// Use DAT files from this directory instead of the cache
        #[arg(long)]
        dat_dir: Option<PathBuf>,
    },

    /// Enrich catalog releases with GameDataBase metadata (Japanese titles, developer/publisher, genre)
    EnrichGdb {
        /// Systems to enrich (e.g., nes,snes) or "all". Defaults to all if omitted.
        #[arg(value_delimiter = ',')]
        systems: Vec<String>,

        /// Path to the catalog database file
        #[arg(long)]
        db: Option<PathBuf>,

        /// Maximum releases to process per system
        #[arg(long)]
        limit: Option<u32>,

        /// Use GDB CSV files from this directory instead of the cache
        #[arg(long)]
        gdb_dir: Option<PathBuf>,
    },

    /// Enrich catalog releases with ScreenScraper metadata
    Enrich {
        /// Systems to enrich (e.g., nes,snes) or "all". Defaults to all if omitted.
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
        /// System to scan (e.g., saturn). Run 'retro-junk systems' for a full list.
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
        /// System to verify (e.g., saturn). Run 'retro-junk systems' for a full list.
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
        #[arg(long, default_value = "")]
        system: String,

        /// Filter by field name (e.g., release_date, title)
        #[arg(long, default_value = "")]
        field: String,

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
        /// System to analyze (e.g., saturn). Run 'retro-junk systems' for a full list.
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
    #[command(group = clap::ArgGroup::new("hash_lookup").multiple(false))]
    Lookup {
        /// Search query, prefixed ID (plt-X, wrk-X, rel-X, med-X), or omit to list
        query: Option<String>,

        /// Filter by entity type: platforms, works, releases, media
        #[arg(long, short = 't', default_value = "")]
        r#type: String,

        /// Filter by platform short name (e.g., nes, snes, psx)
        #[arg(long, default_value = "")]
        platform: String,

        /// Filter by manufacturer (e.g., Nintendo, Sega)
        #[arg(long, default_value = "")]
        manufacturer: String,

        /// Look up by CRC32 hash
        #[arg(long, group = "hash_lookup")]
        crc: Option<String>,

        /// Look up by SHA1 hash
        #[arg(long, group = "hash_lookup")]
        sha1: Option<String>,

        /// Look up by MD5 hash
        #[arg(long, group = "hash_lookup")]
        md5: Option<String>,

        /// Look up by serial number
        #[arg(long, group = "hash_lookup")]
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
        /// Systems to reconcile (e.g., nes,snes) or "all". Defaults to all if omitted.
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
        /// System to unenrich (e.g., saturn). Run 'retro-junk systems' for a full list.
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
