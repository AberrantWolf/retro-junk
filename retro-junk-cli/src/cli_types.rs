//! CLI type definitions: command enums and argument structs.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use retro_junk_lib::Platform;

/// Every help screen, with the build's version above the description.
///
/// Everything after the first line is what clap's own template produces; only
/// the `{name} {version}` header is added.
const HELP_TEMPLATE: &str = "\
{name} {version}
{about-with-newline}
{usage-heading} {usage}

{all-args}{after-help}";

/// The action kinds `sync --only` accepts, taken from the list itself so the
/// two can never drift apart.
///
/// Naming them here means a typo is rejected while clap is still reading the
/// command line, with the valid spellings printed — rather than after the
/// archive has been scanned and the projection rebuilt.
fn action_kind_values() -> clap::builder::PossibleValuesParser {
    clap::builder::PossibleValuesParser::new(
        retro_junk_db::convergence::ActionKind::all()
            .iter()
            .map(|kind| {
                let (canonical, aliases) = kind
                    .spellings()
                    .split_first()
                    .expect("every action kind names itself at least once");
                // Aliases are accepted but stay out of the help line, which
                // would otherwise run to three times the width for no gain.
                clap::builder::PossibleValue::new(*canonical).aliases(aliases.iter().copied())
            })
            .collect::<Vec<_>>(),
    )
}

/// Apply [`HELP_TEMPLATE`] to a command and everything under it.
///
/// clap hands a subcommand its parent's version but not its parent's help
/// template, so without this walk only the top-level `--help` would name the
/// build — and the top level is the screen a person is least likely to be
/// reading when something has gone wrong.
pub(crate) fn version_in_help(command: clap::Command) -> clap::Command {
    let names = command
        .get_subcommands()
        .map(|sub| sub.get_name().to_owned())
        .collect::<Vec<_>>();
    let mut command = command.help_template(HELP_TEMPLATE);
    for name in names {
        command = command.mut_subcommand(name, version_in_help);
    }
    command
}

#[derive(Parser)]
#[command(name = "retro-junk")]
#[command(about = "Analyze retro game ROMs and disc images", long_about = None)]
// A bug report that quotes help output should say which build it came from,
// so the version leads every help screen as well as answering `--version`.
// `propagate_version` gives every subcommand the version; [`HELP_TEMPLATE`]
// and [`version_in_help`] put it on the page.
#[command(version, propagate_version = true)]
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

/// Shared `--dat-dir` argument for commands that read DAT files.
#[derive(Args, Clone)]
pub(crate) struct DatDirArg {
    /// Use DAT files from this directory instead of the cache
    #[arg(long)]
    pub dat_dir: Option<PathBuf>,
}

/// Arguments for the `analyze` command.
#[derive(Args)]
pub(crate) struct AnalyzeArgs {
    /// Quick mode: read as little data as possible (useful for network shares)
    #[arg(short, long)]
    pub quick: bool,

    #[command(flatten)]
    pub roms: ConsoleFilterArgs,
}

/// Arguments for the `rename` command.
#[derive(Args)]
pub(crate) struct RenameArgs {
    /// Show planned renames without executing
    #[arg(short = 'n', long)]
    pub dry_run: bool,

    /// Force CRC32 hash-based matching (reads full files)
    #[arg(long)]
    pub hash: bool,

    #[command(flatten)]
    pub roms: ConsoleFilterArgs,

    #[command(flatten)]
    pub dat: DatDirArg,

    /// Directory for media files (default: <root>-media)
    #[arg(long)]
    pub media_dir: Option<PathBuf>,

    /// Don't rename media files alongside ROMs
    #[arg(long)]
    pub no_media: bool,
}

/// Arguments for the `organize` command.
#[derive(Args)]
pub(crate) struct OrganizeArgs {
    /// Show planned organization without executing
    #[arg(short = 'n', long)]
    pub dry_run: bool,

    #[command(flatten)]
    pub roms: ConsoleFilterArgs,

    #[command(flatten)]
    pub dat: DatDirArg,

    /// Also organize single-disc games into .m3u folders (default: multi-disc only)
    #[arg(long)]
    pub include_single_disc: bool,

    /// Fall back to hashing when serial lookup fails (slower but catches more files)
    #[arg(long)]
    pub hash_fallback: bool,
}

/// Arguments for the `compress` command.
#[derive(Args)]
pub(crate) struct CompressArgs {
    /// Show planned compressions without executing
    #[arg(short = 'n', long)]
    pub dry_run: bool,

    /// Delete original files after each verified compression
    #[arg(long)]
    pub delete_sources: bool,

    /// Skip the per-console confirmation prompt
    #[arg(short = 'y', long)]
    pub yes: bool,

    /// Path to the chdman executable (default: chdman from PATH)
    #[arg(long)]
    pub chdman: Option<PathBuf>,

    #[command(flatten)]
    pub roms: ConsoleFilterArgs,
}

/// Arguments for the `fix-cue` command.
#[derive(Args)]
pub(crate) struct FixCueArgs {
    /// Show planned fixes without executing
    #[arg(short = 'n', long)]
    pub dry_run: bool,

    /// Don't create .cue.bak backup files
    #[arg(long)]
    pub no_backup: bool,

    #[command(flatten)]
    pub roms: ConsoleFilterArgs,
}

/// Arguments for the `repair` command.
#[derive(Args)]
pub(crate) struct RepairArgs {
    /// Show planned repairs without executing
    #[arg(short = 'n', long)]
    pub dry_run: bool,

    /// Don't create .bak backup files
    #[arg(long)]
    pub no_backup: bool,

    #[command(flatten)]
    pub roms: ConsoleFilterArgs,

    #[command(flatten)]
    pub dat: DatDirArg,
}

/// Arguments for the `scrape` command.
// Each bool mirrors an independent clap flag; grouping them into enums would
// change the CLI surface.
#[allow(clippy::struct_excessive_bools)]
#[derive(Args)]
pub(crate) struct ScrapeArgs {
    #[command(flatten)]
    pub roms: ConsoleFilterArgs,

    /// Media types to download (e.g., covers,screenshots,videos,marquees)
    #[arg(long, value_delimiter = ',')]
    pub media_types: Option<Vec<String>>,

    /// Directory for metadata files (default: <root>-metadata).
    /// Set to the same path as --root to place gamelist.xml inside ROM directories,
    /// which is needed for ES-DE with `LegacyGamelistFileLocation` enabled
    #[arg(long)]
    pub metadata_dir: Option<PathBuf>,

    /// Directory for media files (default: <root>-media)
    #[arg(long)]
    pub media_dir: Option<PathBuf>,

    /// Frontend to generate metadata for
    #[arg(long, default_value = "esde")]
    pub frontend: String,

    /// Fallback region when ROM header detection fails (e.g., us, eu, jp). ROM-detected region is always preferred.
    #[arg(long, default_value = "us")]
    pub region: String,

    /// Language for descriptions: "match" derives from ROM region (default), or a code like "en", "ja", "fr"
    #[arg(long, default_value = "match")]
    pub language: String,

    /// Fallback language when region-matched language has no data (default: en)
    #[arg(long, default_value = "en")]
    pub language_fallback: String,

    /// Hash all files even when serial/filename should suffice
    #[arg(long)]
    pub force_full_hash: bool,

    /// Show what would be scraped without downloading
    #[arg(short = 'n', long)]
    pub dry_run: bool,

    /// Skip games that already have metadata
    #[arg(long)]
    pub skip_existing: bool,

    /// Disable scrape log file
    #[arg(long)]
    pub no_log: bool,

    /// Disable miximage generation
    #[arg(long)]
    pub no_miximage: bool,

    /// Force redownload of all media, ignoring existing files
    #[arg(long)]
    pub force_redownload: bool,

    /// Maximum concurrent API threads (default: server-granted max)
    #[arg(long)]
    pub threads: Option<usize>,

    /// Archive downloaded originals before treating frontend media as projections
    #[arg(long)]
    pub archive_root: Option<PathBuf>,
}

/// Run every pending convergence action: verify, build, project, gamelist.
#[derive(clap::Args)]
pub(crate) struct SyncArgs {
    /// Collection profile id or display name (default: the active profile)
    #[arg(long)]
    pub profile: Option<String>,
    /// Archive root (with --playable-root, overrides profile resolution)
    #[arg(long)]
    pub archive_root: Option<PathBuf>,
    /// Playable library root
    #[arg(long)]
    pub playable_root: Option<PathBuf>,
    /// Scratch workspace root
    #[arg(long)]
    pub workspace_root: Option<PathBuf>,
    /// Restrict to one archive platform id
    #[arg(long)]
    pub platform: Option<String>,
    /// Restrict to one archive release id
    #[arg(long)]
    pub release: Option<String>,
    /// Restrict to specific action kinds
    #[arg(long, value_delimiter = ',', value_parser = action_kind_values())]
    pub only: Vec<String>,
    /// Print the derived plan without executing; non-zero exit when
    /// anything is blocked
    #[arg(long)]
    pub dry_run: bool,
    /// Execute at most this many actions
    #[arg(long)]
    pub limit: Option<usize>,
    /// Path to chdman for CHD builds
    #[arg(long)]
    pub chdman: Option<PathBuf>,
    /// Path to redumper for raw-master reproduction
    #[arg(long)]
    pub redumper: Option<PathBuf>,
    /// Path to `DolphinTool` for RVZ builds
    #[arg(long)]
    pub dolphin_tool: Option<PathBuf>,
    /// Frontend media root (default: the playable root's sibling media dir)
    #[arg(long)]
    pub media_root: Option<PathBuf>,
    /// Frontend metadata root (default: the playable root's sibling metadata dir)
    #[arg(long)]
    pub metadata_root: Option<PathBuf>,
    /// Catalog database path
    #[arg(long)]
    pub db: Option<PathBuf>,
}

/// Force a rebuild for one release regardless of whether convergence
/// currently reads it as satisfied.
///
/// The normal `sync`/build path skips a release once its preferred playable
/// looks present — by a live representation row or a bound library entry —
/// so a release whose evidence points at bytes that moved, were
/// regenerated, or were adopted against a file that turned out not to be
/// there can get stuck with no path back to a build. This is that path: it
/// bypasses the "already satisfied" check specifically, not a genuine
/// blocker like a missing preferred format or an incomplete archive.
#[derive(clap::Args)]
pub(crate) struct RebuildPlayableArgs {
    /// Collection profile id or display name (default: the active profile)
    #[arg(long)]
    pub profile: Option<String>,
    /// Archive root (with --playable-root, overrides profile resolution)
    #[arg(long)]
    pub archive_root: Option<PathBuf>,
    /// Playable library root
    #[arg(long)]
    pub playable_root: Option<PathBuf>,
    /// Scratch workspace root
    #[arg(long)]
    pub workspace_root: Option<PathBuf>,
    /// Archive release UUID to rebuild
    #[arg(long)]
    pub release_id: String,
    /// Print what would be rebuilt without building it
    #[arg(long)]
    pub dry_run: bool,
    /// Path to chdman for CHD builds
    #[arg(long)]
    pub chdman: Option<PathBuf>,
    /// Path to redumper for raw-master reproduction
    #[arg(long)]
    pub redumper: Option<PathBuf>,
    /// Path to `DolphinTool` for RVZ builds
    #[arg(long)]
    pub dolphin_tool: Option<PathBuf>,
    /// Catalog database path
    #[arg(long)]
    pub db: Option<PathBuf>,
}

/// Rename built playables whose name is no longer what the catalog calls
/// them — the usual cause is a playable built before the naming rule was
/// corrected, or a DAT that has since renamed the game.
#[derive(clap::Args)]
pub(crate) struct RenamePlayablesArgs {
    /// Collection profile id or display name (default: the active profile)
    #[arg(long)]
    pub profile: Option<String>,
    /// Archive root (with --playable-root, overrides profile resolution)
    #[arg(long)]
    pub archive_root: Option<PathBuf>,
    /// Playable library root
    #[arg(long)]
    pub playable_root: Option<PathBuf>,
    /// Limit the repair to one archive release (default: every release)
    #[arg(long)]
    pub release_id: Option<String>,
    /// List what would be renamed without renaming anything
    #[arg(long)]
    pub dry_run: bool,
    /// Frontend media root, so artwork named after a playable follows it
    #[arg(long)]
    pub media_root: Option<PathBuf>,
    /// Catalog database path
    #[arg(long)]
    pub db: Option<PathBuf>,
}

/// Summarize convergence state, daemon liveness, and open suggestions.
#[derive(clap::Args)]
pub(crate) struct StatusArgs {
    /// Collection profile id or display name (default: the active profile)
    #[arg(long)]
    pub profile: Option<String>,
    /// Archive root (with --playable-root, overrides profile resolution)
    #[arg(long)]
    pub archive_root: Option<PathBuf>,
    /// Playable library root
    #[arg(long)]
    pub playable_root: Option<PathBuf>,
    /// Catalog database path
    #[arg(long)]
    pub db: Option<PathBuf>,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    /// Manage preservation masters and their playable derivatives
    Archive {
        #[command(subcommand)]
        action: ArchiveAction,
    },

    /// Run pending convergence actions (verify, build, scrape, project, gamelist)
    Sync(SyncArgs),

    /// Force a fresh playable build for one release, even if convergence
    /// currently reads it as already satisfied
    RebuildPlayable(RebuildPlayableArgs),

    /// Rename built playables to the names the catalog gives them
    RenamePlayables(RenamePlayablesArgs),

    /// Show convergence counts, daemon liveness, and open suggestions
    Status(StatusArgs),

    /// Run the convergence daemon (foreground; watches incoming + playable roots)
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },

    /// Review and apply proposed-but-unapplied actions
    Suggestions {
        #[command(subcommand)]
        action: SuggestionsAction,
    },

    /// Analyze games in a library directory structure
    Analyze(AnalyzeArgs),

    /// Rename ROM files to `NoIntro` canonical names
    Rename(RenameArgs),

    /// Organize loose disc images into ES-DE .m3u folders using Redump DAT names
    Organize(OrganizeArgs),

    /// Compress disc images (cue/bin, GDI, ISO) to CHD using chdman
    ///
    /// Every CHD is round-trip verified (re-extracted and compared
    /// track-by-track against the originals) before being reported as
    /// compressed. Originals are kept unless --delete-sources is given,
    /// and are never deleted when verification fails.
    Compress(CompressArgs),

    /// Fix CDRWin-format CUE sheets for wider emulator compatibility
    FixCue(FixCueArgs),

    /// [Experimental] Repair trimmed/truncated ROMs by padding to match DAT checksums
    Repair(RepairArgs),

    /// Scrape metadata and media into ES-DE gamelists via ScreenScraper.fr
    Scrape(ScrapeArgs),

    /// Manage cached DAT and GDB files
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },

    /// Manage `ScreenScraper` credentials
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
pub(crate) enum ArchiveAction {
    /// Initialize a portable preservation archive
    Init {
        /// Archive root to create
        archive_root: PathBuf,

        /// Human-readable collection name
        #[arg(long, default_value = "Retro Collection")]
        name: String,
    },

    /// Discover, identify, and import one dump folder or a directory of dump folders
    Import {
        source: PathBuf,

        #[arg(long)]
        archive_root: PathBuf,

        #[arg(long)]
        db: Option<PathBuf>,

        #[arg(long)]
        platform: Option<String>,

        #[arg(long, default_value = "default")]
        owner: String,

        #[arg(long)]
        new_physical_copy: bool,

        /// Path to Redumper for complete-track identification of raw packages
        #[arg(long)]
        redumper: Option<PathBuf>,

        /// Create a round-trip verified playable CHD after import
        #[arg(long, requires = "playable_root")]
        make_playable: bool,

        /// Root directory for automatically created playable CHDs
        #[arg(long, requires = "make_playable")]
        playable_root: Option<PathBuf>,

        /// Path to chdman
        #[arg(long)]
        chdman: Option<PathBuf>,

        /// Exclude the supplied CUE and referenced BIN tracks after CHD verification
        #[arg(long, requires = "make_playable")]
        discard_redundant_bin_cue: bool,

        /// Disposable workspace used for Redumper identification
        #[arg(long)]
        workspace_root: Option<PathBuf>,

        /// Remove sources only after a verified new import or exact duplicate match
        #[arg(long)]
        consume: bool,

        #[arg(long)]
        dry_run: bool,

        /// Execute without an interactive confirmation
        #[arg(long)]
        yes: bool,
    },

    /// Promote an existing playable library into preservation masters without moving it
    ImportPlayable {
        /// Existing ROM-library root, normally containing platform directories
        playable_root: PathBuf,

        #[arg(long)]
        archive_root: PathBuf,

        #[arg(long)]
        db: Option<PathBuf>,

        #[arg(long)]
        platform: Option<String>,

        #[arg(long, default_value = "default")]
        owner: String,

        #[arg(long)]
        new_physical_copy: bool,

        #[arg(long)]
        redumper: Option<PathBuf>,

        #[arg(long)]
        workspace_root: Option<PathBuf>,

        /// Existing frontend media root; defaults to the playable root's sibling media directory
        #[arg(long)]
        media_root: Option<PathBuf>,

        #[arg(long)]
        dry_run: bool,

        /// Execute without an interactive confirmation
        #[arg(long)]
        yes: bool,
    },

    /// Copy a dump into the archive and verify the staged bytes before publishing
    Ingest {
        /// File or directory containing one dump event
        source: PathBuf,

        /// Initialized archive root
        #[arg(long)]
        archive_root: PathBuf,

        /// Platform identifier, such as psx, saturn, or nes
        #[arg(long)]
        platform: String,

        /// Release title
        #[arg(long)]
        title: String,

        /// Preservation representation: redumper-raw, rom, cue-bin, iso, chd, or rvz
        #[arg(long)]
        format: String,

        #[arg(long, default_value = "")]
        region: String,

        #[arg(long, default_value = "")]
        revision: String,

        #[arg(long, default_value = "")]
        variant: String,

        #[arg(long, default_value = "")]
        serial: String,

        #[arg(long, default_value_t = 0)]
        sequence_number: u32,

        #[arg(long, default_value = "default")]
        owner: String,

        #[arg(long, default_value = "")]
        physical_copy_label: String,

        #[arg(long, default_value = "")]
        carrier_label: String,
    },

    /// Copy artwork, video, a document, or metadata into an archived release
    AddReleaseFile {
        /// Original downloaded image, video, manual, or metadata file
        source_file: PathBuf,

        #[arg(long)]
        archive_root: PathBuf,

        #[arg(long)]
        release_id: retro_junk_archive::ArchiveReleaseId,

        #[arg(long, default_value = "artwork")]
        category: String,

        /// Semantic type such as box-front, screenshot, video, or manual
        #[arg(long)]
        asset_type: String,

        #[arg(long, default_value = "")]
        source: String,

        #[arg(long, default_value = "")]
        source_url: String,

        #[arg(long, default_value = "")]
        caption: String,
    },

    /// Archive a photo, provenance record, or document for one physical copy
    AddPhysicalCopyFile {
        source_file: PathBuf,

        #[arg(long)]
        archive_root: PathBuf,

        #[arg(long)]
        physical_copy_id: retro_junk_archive::PhysicalCopyId,

        /// `photo`, `provenance`, or `document`
        #[arg(long)]
        category: String,

        #[arg(long)]
        asset_type: String,

        #[arg(long, default_value = "")]
        source: String,

        #[arg(long, default_value = "")]
        caption: String,
    },

    /// Show a manifest-derived archive summary
    Status {
        /// Initialized archive root
        archive_root: PathBuf,
    },

    /// Re-hash preservation-master bytes and append verification evidence
    Verify {
        /// Initialized archive root
        archive_root: PathBuf,
    },

    /// Verify single-file masters against imported DAT catalog hashes
    VerifyCatalog {
        /// Initialized archive root
        archive_root: PathBuf,

        /// Catalog database path
        #[arg(long)]
        db: Option<PathBuf>,

        /// Verify only this dump UUID
        #[arg(long)]
        dump_id: Option<String>,
    },

    /// Regenerate Redump-compatible tracks from raw Redumper masters in scratch space
    AuditRedumper {
        /// Initialized archive root
        archive_root: PathBuf,

        /// Audit only this dump UUID
        #[arg(long)]
        dump_id: Option<String>,

        /// Scratch root; defaults to ARCHIVE/.retro-junk/work
        #[arg(long)]
        workspace_root: Option<PathBuf>,

        /// Path to redumper; defaults to PATH lookup
        #[arg(long)]
        redumper: Option<PathBuf>,

        /// Catalog database used for complete-track Redump matching
        #[arg(long)]
        db: Option<PathBuf>,
    },

    /// Build and round-trip verify a playable CHD from an archived dump
    BuildChd {
        /// Initialized archive root
        archive_root: PathBuf,

        /// Playable-library root
        #[arg(long)]
        playable_root: PathBuf,

        /// Dump UUID to convert
        #[arg(long)]
        dump_id: String,

        /// Scratch root; defaults to ARCHIVE/.retro-junk/work
        #[arg(long)]
        workspace_root: Option<PathBuf>,

        /// Path to chdman; defaults to PATH lookup
        #[arg(long)]
        chdman: Option<PathBuf>,

        /// Path to redumper when the master is redumper-raw
        #[arg(long)]
        redumper: Option<PathBuf>,

        /// Permit a playable derivative before a complete catalog match exists
        #[arg(long)]
        allow_unverified: bool,
    },

    /// Build and round-trip verify a playable RVZ from an archived ISO
    BuildRvz {
        archive_root: PathBuf,

        #[arg(long)]
        playable_root: PathBuf,

        #[arg(long)]
        dump_id: String,

        #[arg(long)]
        workspace_root: Option<PathBuf>,

        /// Path to `DolphinTool`; defaults to PATH lookup
        #[arg(long)]
        dolphin_tool: Option<PathBuf>,

        #[arg(long)]
        allow_unverified: bool,
    },

    /// Mirror a single-file preservation master into the playable library
    Mirror {
        /// Initialized archive root
        archive_root: PathBuf,

        /// Playable-library root
        #[arg(long)]
        playable_root: PathBuf,

        /// Dump UUID to mirror
        #[arg(long)]
        dump_id: String,
    },

    /// Set or clear a carrier's desired playable representation
    Policy {
        /// Initialized archive root
        archive_root: PathBuf,

        /// Physical carrier UUID
        #[arg(long)]
        carrier_id: retro_junk_archive::CarrierId,

        /// Desired format, such as chd, rvz, or rom
        #[arg(long, required_unless_present = "clear")]
        format: Option<String>,

        /// Remove the carrier override and inherit the platform default
        #[arg(long)]
        clear: bool,

        /// Keep generated canonical intermediates such as BIN/CUE
        #[arg(long)]
        retain_intermediate: bool,

        /// Permit builds without current catalog verification
        #[arg(long)]
        allow_unverified: bool,
    },

    /// Set or clear the default playable policy for a platform
    PolicyDefault {
        /// Initialized archive root
        archive_root: PathBuf,

        /// Platform identifier such as psx, saturn, or nes
        #[arg(long)]
        platform: String,

        /// Desired format inherited by media without an override
        #[arg(long, required_unless_present = "clear")]
        format: Option<String>,

        #[arg(long)]
        clear: bool,

        #[arg(long)]
        retain_intermediate: bool,

        #[arg(long)]
        allow_unverified: bool,
    },

    /// Execute all pending playable policies; reruns skip satisfied outputs
    Build {
        /// Initialized archive root
        archive_root: PathBuf,

        /// Playable-library root
        #[arg(long)]
        playable_root: PathBuf,

        /// Scratch root; defaults to ARCHIVE/.retro-junk/work
        #[arg(long)]
        workspace_root: Option<PathBuf>,

        #[arg(long)]
        chdman: Option<PathBuf>,

        #[arg(long)]
        redumper: Option<PathBuf>,

        #[arg(long)]
        dolphin_tool: Option<PathBuf>,

        /// Catalog database used for release-wide disc counts and verification
        #[arg(long)]
        db: Option<PathBuf>,

        /// Frontend media root; defaults to the playable root's sibling media directory
        #[arg(long)]
        media_root: Option<PathBuf>,

        /// ES-DE metadata root; defaults to the playable root's sibling metadata directory
        #[arg(long)]
        metadata_root: Option<PathBuf>,

        /// Do not restore archived artwork after successful builds
        #[arg(long)]
        no_project_assets: bool,

        /// Do not add newly-created playable entries to ES-DE gamelist.xml files
        #[arg(long)]
        no_update_gamelists: bool,

        /// Show pending work without building it
        #[arg(short = 'n', long)]
        dry_run: bool,

        /// Process at most this many pending media
        #[arg(long)]
        limit: Option<usize>,
    },

    /// Rebuild frontend media files from archived scraped originals
    #[command(name = "project-frontend-files")]
    ProjectFrontendFiles {
        archive_root: PathBuf,

        /// Frontend media root (platform and asset subdirectories are created below it)
        #[arg(long)]
        media_root: PathBuf,
    },

    /// Generate miximages from archived components and store them in the archive
    GenerateMiximages {
        archive_root: PathBuf,

        /// Playable-library root
        #[arg(long)]
        playable_root: PathBuf,

        /// Frontend media root; defaults to the playable root's sibling media directory
        #[arg(long)]
        media_root: Option<PathBuf>,

        /// Scratch root; defaults to ARCHIVE/.retro-junk/work
        #[arg(long)]
        workspace_root: Option<PathBuf>,

        /// Limit generation to one archive release ID
        #[arg(long)]
        release_id: Option<String>,
    },

    /// Make the archive account for the playable files that are actually
    /// there: re-adopt outputs that moved out from under their build evidence,
    /// then adopt unarchived files that exactly match archived masters
    AdoptPlayable {
        archive_root: PathBuf,

        #[arg(long)]
        playable_root: PathBuf,

        #[arg(long)]
        db: Option<PathBuf>,

        /// Re-adopt moved outputs for only this archive release UUID
        #[arg(long)]
        release_id: Option<String>,

        /// Report what would be adopted without appending any evidence
        #[arg(long)]
        dry_run: bool,
    },

    /// Move abandoned staging/work directories into a recoverable quarantine
    Recover { archive_root: PathBuf },

    /// Rebuild the disposable `SQLite` archive projection from manifests
    Reindex {
        /// Initialized archive root
        archive_root: PathBuf,

        /// Playable-library root paired with this archive
        #[arg(long)]
        playable_root: Option<PathBuf>,

        /// Scratch root for splitting and verification
        #[arg(long)]
        workspace_root: Option<PathBuf>,

        /// Catalog database path
        #[arg(long)]
        db: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub(crate) enum CacheAction {
    /// Manage cached DAT files (No-Intro, Redump)
    Dat {
        #[command(subcommand)]
        action: DatCacheAction,
    },

    /// Manage cached GDB (`GameDataBase`) CSV files
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

    /// Test credentials against the `ScreenScraper` API
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
    /// Report catalog media that claim to be the same edition but hash differently
    Deduplicate {
        /// Restrict the report to one catalog platform
        #[arg(long)]
        platform: Option<String>,

        /// Print a machine-readable JSON summary
        #[arg(long)]
        json: bool,

        /// Path to the catalog database file
        #[arg(long)]
        db: Option<PathBuf>,
    },

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

    /// Enrich catalog releases with `GameDataBase` metadata (Japanese titles, developer/publisher, genre)
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

    /// Enrich catalog releases with `ScreenScraper` metadata
    Enrich(CatalogEnrichArgs),

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

        /// Filter by field name (e.g., `release_date`, title)
        #[arg(long, default_value = "")]
        field: String,

        /// Maximum number of disagreements to show
        #[arg(long, default_value = "50")]
        limit: u32,
    },

    /// Resolve a disagreement by choosing a value
    Resolve {
        /// Disagreement ID to resolve
        id: retro_junk_catalog::DisagreementId,

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
    Lookup(CatalogLookupArgs),

    /// Merge duplicate works that share a `ScreenScraper` ID
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

    /// Clear enrichment status for releases (`screenscraper_id` and `scraper_not_found`)
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

/// Arguments for `catalog enrich`.
#[derive(Args)]
pub(crate) struct CatalogEnrichArgs {
    /// Systems to enrich (e.g., nes,snes) or "all". Defaults to all if omitted.
    #[arg(value_delimiter = ',')]
    pub systems: Vec<String>,

    /// Path to the catalog database file
    #[arg(long)]
    pub db: Option<PathBuf>,

    /// Maximum releases to process per system
    #[arg(long)]
    pub limit: Option<u32>,

    /// Re-enrich releases that already have `ScreenScraper` data
    #[arg(long)]
    pub force: bool,

    /// Download media assets
    #[arg(long)]
    pub download_assets: bool,

    /// Directory for downloaded media assets
    #[arg(long)]
    pub asset_dir: Option<PathBuf>,

    /// Preferred region for names and media (default: us)
    #[arg(long, default_value = "us")]
    pub region: String,

    /// Preferred language for descriptions (default: en)
    #[arg(long, default_value = "en")]
    pub language: String,

    /// Maximum concurrent API threads (default: server-granted max)
    #[arg(long)]
    pub threads: Option<usize>,

    /// Skip automatic work reconciliation after enrichment
    #[arg(long)]
    pub no_reconcile: bool,
}

/// Which kind of catalog row `catalog lookup` should search or list.
///
/// A closed set, so clap rejects anything else at parse time and lists the
/// valid values in `--help` — rather than the command opening the database,
/// doing its work, and only then complaining.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum CatalogEntityType {
    Platforms,
    Works,
    Releases,
    Media,
}

/// Arguments for `catalog lookup`.
#[derive(Args)]
pub(crate) struct CatalogLookupArgs {
    /// Search query, an id (`wrk_…`, `rel_…`, `med_…`, or `plt-<platform>`),
    /// or omit to list
    pub query: Option<String>,

    /// Filter by entity type
    #[arg(long, short = 't', value_enum)]
    pub r#type: Option<CatalogEntityType>,

    /// Filter by platform short name (e.g., nes, snes, psx)
    #[arg(long, default_value = "")]
    pub platform: String,

    /// Filter by manufacturer (e.g., Nintendo, Sega)
    #[arg(long, default_value = "")]
    pub manufacturer: String,

    /// Look up by CRC32 hash
    #[arg(long, group = "hash_lookup")]
    pub crc: Option<String>,

    /// Look up by SHA1 hash
    #[arg(long, group = "hash_lookup")]
    pub sha1: Option<String>,

    /// Look up by MD5 hash
    #[arg(long, group = "hash_lookup")]
    pub md5: Option<String>,

    /// Look up by serial number
    #[arg(long, group = "hash_lookup")]
    pub serial: Option<String>,

    /// Maximum number of results (default 25)
    #[arg(long, default_value = "25")]
    pub limit: u32,

    /// Skip this many results (for pagination)
    #[arg(long, default_value = "0")]
    pub offset: u32,

    /// Group results (e.g., platforms by manufacturer)
    #[arg(long)]
    pub group: bool,

    /// Path to the catalog database file
    #[arg(long)]
    pub db: Option<PathBuf>,
}

#[derive(Subcommand)]
pub(crate) enum DaemonAction {
    /// Start the daemon in the foreground
    Start {
        /// Collection profile id or display name (default: the active profile)
        #[arg(long)]
        profile: Option<String>,
        /// Catalog database path
        #[arg(long)]
        db: Option<PathBuf>,
        /// Required: the daemon does not self-daemonize
        #[arg(long)]
        foreground: bool,
        /// Event-wait tick in seconds (default 30)
        #[arg(long)]
        tick: Option<u64>,
        /// Path to chdman for CHD builds
        #[arg(long)]
        chdman: Option<PathBuf>,
        /// Path to redumper for raw-master reproduction
        #[arg(long)]
        redumper: Option<PathBuf>,
        /// Path to `DolphinTool` for RVZ builds
        #[arg(long)]
        dolphin_tool: Option<PathBuf>,
    },
    /// Signal the running daemon to stop and wait for a clean exit
    Stop,
    /// Report daemon liveness plus the convergence summary
    Status {
        /// Catalog database path
        #[arg(long)]
        db: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub(crate) enum SuggestionsAction {
    /// List open suggestions
    List {
        /// Only this kind (import, scrape, `adopt_playable`)
        #[arg(long)]
        kind: Option<String>,
        /// Only targets matching this path pattern: `*.txt`, `*/rvz/*`, or a
        /// bare word to match anywhere in the path
        #[arg(long = "match")]
        pattern: Option<String>,
        /// Catalog database path
        #[arg(long)]
        db: Option<PathBuf>,
    },
    /// Show one suggestion's payload
    Show {
        id: i64,
        /// Catalog database path
        #[arg(long)]
        db: Option<PathBuf>,
    },
    /// Apply a suggestion (imports re-validate and execute)
    Apply {
        id: i64,
        /// For a review with several candidates, the one to accept (its id, as
        /// shown by `suggestions show`)
        #[arg(long)]
        choice: Option<String>,
        /// Collection profile id or display name (default: the active profile)
        #[arg(long)]
        profile: Option<String>,
        /// Catalog database path
        #[arg(long)]
        db: Option<PathBuf>,
    },
    /// Dismiss suggestions without applying them
    ///
    /// Dismissing closes review rows and never touches a file. It is not
    /// durable: re-running the sweep that proposed them files them again. To
    /// stop that, record an ignore rule instead.
    Dismiss {
        /// Suggestion ids to dismiss
        ids: Vec<i64>,
        /// Dismiss everything of this kind instead of naming ids
        #[arg(long)]
        kind: Option<String>,
        /// Dismiss everything whose target matches this path pattern
        #[arg(long = "match")]
        pattern: Option<String>,
        /// Print what would be dismissed and stop
        #[arg(long)]
        dry_run: bool,
        /// Catalog database path
        #[arg(long)]
        db: Option<PathBuf>,
    },
    /// Undo a dismissal, putting suggestions back in front of you
    Reopen {
        /// Suggestion ids to reopen
        ids: Vec<i64>,
        /// Catalog database path
        #[arg(long)]
        db: Option<PathBuf>,
    },
    /// Never file playable files matching this pattern again, and close the
    /// reviews it covers
    Ignore {
        /// Path pattern relative to the playable root: `*.txt`, `*/rvz/*`
        pattern: String,
        /// Why, for your own reference later
        #[arg(long, default_value = "")]
        note: String,
        /// Collection profile id or display name (default: the active profile)
        #[arg(long)]
        profile: Option<String>,
        /// Catalog database path
        #[arg(long)]
        db: Option<PathBuf>,
    },
    /// List the ignore rules in force for this collection
    Ignores {
        /// Collection profile id or display name (default: the active profile)
        #[arg(long)]
        profile: Option<String>,
        /// Catalog database path
        #[arg(long)]
        db: Option<PathBuf>,
    },
    /// Revoke an ignore rule; the next sweep files those files again
    Unignore {
        /// The pattern of the rule to revoke
        pattern: String,
        /// Collection profile id or display name (default: the active profile)
        #[arg(long)]
        profile: Option<String>,
        /// Catalog database path
        #[arg(long)]
        db: Option<PathBuf>,
    },
}
