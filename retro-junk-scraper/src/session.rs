//! The one scrape orchestration.
//!
//! Every surface that fetches media from `ScreenScraper` — CLI `scrape`, the
//! GUI's artwork actions, and the convergence executor — runs this loop. The
//! callers differ only in where their targets come from and how they render
//! progress; they must not differ in what a scrape *does*.
//!
//! The shape deliberately mirrors `retro_junk_lib::archive_ops`: a borrowed
//! request struct in, an outcome struct out, cancellation by `&AtomicBool`.
//! It does not *live* in `archive_ops` only because this crate already depends
//! on `retro-junk-lib` (for `scanner` and `util`), so orchestration there
//! would close a dependency cycle.
//!
//! Targets are supplied by the caller rather than scanned here: the GUI and the
//! executor already know exactly which releases they mean, and re-deriving that
//! from a directory walk would both be slower and disagree with the archive.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::stream::{self, StreamExt};
use retro_junk_archive::{ArchiveReleaseId, ReleaseFileCategory};
use retro_junk_frontend::miximage_layout::MiximageLayout;
use retro_junk_frontend::{AssetSelection, AssetType};
use tokio::sync::{Mutex, mpsc};
use tokio::time::Duration;

use crate::assets::{self, AssetDownloadRequest};
use crate::client::ScreenScraperClient;
use crate::derivation::Derivation;
use crate::error::ScrapeError;
use crate::lookup::{self, LookupMethod, RomInfo};
use crate::types::GameInfo;
use retro_junk_lib::archive_assets::project_asset_file as project_asset;

/// Timeout for acquiring internal mutex locks (should be near-instant).
const LOCK_TIMEOUT: Duration = Duration::from_secs(5);

/// Whether and how to generate miximages.
#[derive(Debug, Clone, Default)]
pub enum MiximageMode {
    /// Do not generate miximages.
    #[default]
    Disabled,
    /// Generate miximages using the given layout.
    Enabled(MiximageLayout),
}

impl MiximageMode {
    /// The layout to generate with, if generation is enabled.
    #[must_use]
    pub fn layout(&self) -> Option<&MiximageLayout> {
        match self {
            MiximageMode::Disabled => None,
            MiximageMode::Enabled(layout) => Some(layout),
        }
    }
}

/// Progress events emitted during scraping, consumed by the CLI or GUI.
#[derive(Debug, Clone)]
pub enum ScrapeEvent {
    /// Scanning the folder for ROM files.
    Scanning,
    /// Scan complete, total games found.
    ScanComplete { total: usize },
    /// A game has started processing (assigned to a worker).
    GameStarted { index: usize, file: String },
    /// Looking up a game on `ScreenScraper`.
    GameLookingUp { index: usize, file: String },
    /// Downloading media for a game.
    GameDownloading { index: usize, file: String },
    /// Downloading a specific media type for a game.
    GameDownloadingMedia {
        index: usize,
        file: String,
        media_type: String,
    },
    /// Game was skipped (existing media, dry run, etc.).
    GameSkipped {
        index: usize,
        file: String,
        reason: String,
    },
    /// Game was successfully scraped.
    GameCompleted {
        index: usize,
        file: String,
        game_name: String,
    },
    /// Game lookup failed (non-fatal).
    GameFailed {
        index: usize,
        file: String,
        reason: String,
    },
    /// A secondary disc was grouped with its primary.
    GameGrouped {
        index: usize,
        file: String,
        primary_file: String,
    },
    /// Downloaded media is being published into the archive.
    Publishing { files: usize },
    /// The daily request budget is spent (or within its reserve). Remaining
    /// targets are left for the next run rather than erroring one by one.
    QuotaExhausted { used: u32, max: u32 },
    /// A fatal error occurred (quota, auth, server closed). Scraping will stop.
    FatalError { message: String },
    /// All games processed.
    Done,
}

/// Where one target's media belongs.
#[derive(Debug, Clone)]
pub enum ScrapeDestination {
    /// The frontend media tree only — no archive profile in play.
    Playable { media_dir: PathBuf },
    /// Archive-first: download to a scratch directory, publish into the
    /// archive as the durable copy, then project to the frontend tree.
    Archive {
        release_id: ArchiveReleaseId,
        media_dir: PathBuf,
    },
}

impl ScrapeDestination {
    fn media_dir(&self) -> &Path {
        match self {
            Self::Playable { media_dir } | Self::Archive { media_dir, .. } => media_dir,
        }
    }

    fn release_id(&self) -> Option<ArchiveReleaseId> {
        match self {
            Self::Playable { .. } => None,
            Self::Archive { release_id, .. } => Some(*release_id),
        }
    }
}

/// One game to scrape.
#[derive(Debug, Clone)]
pub struct ScrapeTarget {
    /// Caller-owned identity, echoed back on the outcome. The caller decides
    /// what it means (entry index, row id); the core never interprets it.
    pub key: u64,
    /// Human-readable name for logs.
    pub label: String,
    /// Media file stem — what downloaded files are named after.
    pub rom_stem: String,
    /// The file's own identity, strongest tier first.
    ///
    /// What is actually offered to `ScreenScraper` is this run through
    /// [`ScrapeTarget::derivation`]: a mod is asked about as its parent, and
    /// its own bytes are never offered to a catalog that has never held them.
    pub rom: RomInfo,
    /// What the user decided this file is. [`Derivation::Own`] for the
    /// overwhelming majority — anything that matched a DAT is itself.
    pub derivation: Derivation,
    /// `ScreenScraper` region code for media and name selection ("us", "jp"…).
    pub region: String,
    /// Language for descriptions and genres.
    pub language: String,
    pub destination: ScrapeDestination,
    /// Assets this release already holds in the archive. Projected to the
    /// frontend tree, and excluded from the download.
    pub archived_assets: HashMap<AssetType, PathBuf>,
}

/// Publication settings for targets with an [`ScrapeDestination::Archive`].
pub struct ArchivePublication<'a> {
    pub archive_root: &'a Path,
    /// Directory for in-flight downloads; removed when the run finishes.
    pub scratch_root: PathBuf,
    /// `false` when the caller already holds the whole-archive lock (the
    /// convergence executor does). Taking it twice would deadlock.
    pub acquire_lock: bool,
}

/// Session-wide knobs.
#[derive(Debug, Clone)]
pub struct ScrapeSessionOptions {
    /// Refetch every selected type even when a file already exists.
    pub force_redownload: bool,
    /// Report what would be looked up and stop before any network call.
    pub dry_run: bool,
    /// Leave a target untouched entirely when it already has any selected
    /// asset — "don't re-visit games I've already scraped".
    pub skip_scraped: bool,
    /// Fallback language for localized text.
    pub language_fallback: String,
    pub miximage: MiximageMode,
    /// Stop the run when the daily request budget drops to this many
    /// remaining. `0` disables the reserve.
    pub request_reserve: u32,
}

impl Default for ScrapeSessionOptions {
    fn default() -> Self {
        Self {
            force_redownload: false,
            dry_run: false,
            skip_scraped: false,
            language_fallback: "en".to_owned(),
            miximage: MiximageMode::Disabled,
            request_reserve: 0,
        }
    }
}

/// Everything one scrape run needs.
pub struct ScrapeRequest<'a> {
    pub client: &'a ScreenScraperClient,
    /// `ScreenScraper` system id — all targets in one run share a platform.
    pub system_id: u32,
    pub targets: &'a [ScrapeTarget],
    /// Asset types to fetch. Types already present are dropped per target.
    pub selection: &'a AssetSelection,
    pub options: &'a ScrapeSessionOptions,
    pub archive: Option<&'a ArchivePublication<'a>>,
    pub max_workers: usize,
    pub events: &'a mpsc::UnboundedSender<ScrapeEvent>,
    pub cancel: &'a AtomicBool,
}

/// What happened to one target.
#[derive(Debug)]
pub enum TargetState {
    /// Identified and its media is on disk.
    Scraped {
        game: Box<GameInfo>,
        method: LookupMethod,
        warnings: Vec<String>,
        assets: HashMap<AssetType, PathBuf>,
    },
    /// Nothing to do; `assets` are the files already present.
    Skipped {
        reason: String,
        assets: HashMap<AssetType, PathBuf>,
    },
    /// `ScreenScraper` has no entry for this identity.
    NotFound { warnings: Vec<String> },
    /// A per-target failure. The run continues.
    Failed { message: String },
    /// The run stopped before reaching this target.
    NotReached,
}

#[derive(Debug)]
pub struct TargetOutcome {
    pub key: u64,
    pub state: TargetState,
}

#[derive(Debug, Default)]
pub struct ScrapeRunResult {
    pub outcomes: Vec<TargetOutcome>,
    /// Set when the run stopped early (quota, credentials, server closed).
    pub fatal: Option<String>,
    /// Supporting files newly added to the archive.
    pub published: usize,
}

impl ScrapeRunResult {
    /// Outcomes keyed by the caller's target key.
    #[must_use]
    pub fn by_key(self) -> HashMap<u64, TargetState> {
        self.outcomes
            .into_iter()
            .map(|outcome| (outcome.key, outcome.state))
            .collect()
    }
}

/// Map each archived release's built playable filenames to its release id,
/// for one platform.
///
/// This is how a filesystem-shaped scrape (the CLI walking a ROM folder)
/// binds what it finds to the archive: the archive recorded the output path
/// when it built the file, so the match is the archive's own evidence rather
/// than a guess from the title.
#[must_use]
pub fn release_ids_by_output(
    snapshot: &retro_junk_archive::ArchiveIndexSnapshot,
    platform_id: &str,
) -> HashMap<String, ArchiveReleaseId> {
    // The caller names a ROM folder (frontend spellings: "psx", "gc"), the
    // archive names platforms canonically and regionally ("ps1", "snesna"), so
    // spellings are compared as parsed platforms, not strings. Regional
    // variants of one console fold together here, which is safe because the
    // binding below is keyed on the exact built filename — the region lives in
    // the name the build wrote.
    let wanted: Option<retro_junk_core::Platform> = platform_id.parse().ok();
    let mut bindings = HashMap::new();
    for release in &snapshot.releases {
        let release_platform: Option<retro_junk_core::Platform> =
            release.manifest.platform_id.parse().ok();
        let same_platform = match (wanted, release_platform) {
            (Some(a), Some(b)) => a == b,
            _ => release
                .manifest
                .platform_id
                .eq_ignore_ascii_case(platform_id),
        };
        if !same_platform {
            continue;
        }
        let outputs = release
            .physical_copies
            .iter()
            .flat_map(|copy| &copy.carriers)
            .flat_map(|carrier| &carrier.dumps)
            .flat_map(|dump| &dump.builds)
            .filter_map(|build| {
                Path::new(&build.evidence.relative_output_path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_owned)
            });
        for output in outputs {
            bindings.insert(output, release.manifest.archive_release_id);
        }
    }
    bindings
}

/// Errors that end the whole run rather than one target.
///
/// One definition. Three surfaces used to keep their own `matches!` list and
/// had already drifted apart.
#[must_use]
pub fn is_fatal(error: &ScrapeError) -> bool {
    matches!(
        error,
        ScrapeError::QuotaExceeded { .. }
            | ScrapeError::InvalidCredentials(_)
            | ScrapeError::ServerClosed(_)
    )
}

/// A file downloaded for an archived release, awaiting publication.
struct PendingPublication {
    key: u64,
    release_id: ArchiveReleaseId,
    asset_type: AssetType,
    path: PathBuf,
    source_url: String,
    rom_stem: String,
    media_dir: PathBuf,
}

/// Per-target result before archive publication resolves it.
struct RawOutcome {
    key: u64,
    state: TargetState,
    pending: Vec<PendingPublication>,
}

/// Run one scrape to completion.
///
/// Never returns `Err` for a single target's failure: per-target verdicts are
/// in the outcomes, and only a fatal condition (quota, credentials, server
/// closed) stops the run — recorded in [`ScrapeRunResult::fatal`].
pub async fn run_scrape(request: &ScrapeRequest<'_>) -> ScrapeRunResult {
    let mut result = ScrapeRunResult::default();
    if request.targets.is_empty() {
        let _ = request.events.send(ScrapeEvent::Done);
        return result;
    }

    // A fatal error on any worker stops the ones that have not started.
    let stop = Arc::new(AtomicBool::new(false));
    let fatal_message: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    let raw: Vec<RawOutcome> = stream::iter(request.targets.iter().enumerate())
        .map(|(index, target)| {
            let stop = stop.clone();
            let fatal_message = fatal_message.clone();
            async move {
                if request.cancel.load(Ordering::Relaxed) || stop.load(Ordering::Relaxed) {
                    return RawOutcome {
                        key: target.key,
                        state: TargetState::NotReached,
                        pending: Vec::new(),
                    };
                }
                if let Some((used, max)) = quota_exhausted(request).await {
                    stop.store(true, Ordering::Relaxed);
                    let _ = request
                        .events
                        .send(ScrapeEvent::QuotaExhausted { used, max });
                    return RawOutcome {
                        key: target.key,
                        state: TargetState::NotReached,
                        pending: Vec::new(),
                    };
                }
                let outcome = scrape_one(request, index, target).await;
                if let TargetState::Failed { message } = &outcome.state
                    && outcome.fatal
                {
                    stop.store(true, Ordering::Relaxed);
                    if let Ok(mut guard) =
                        tokio::time::timeout(LOCK_TIMEOUT, fatal_message.lock()).await
                    {
                        guard.get_or_insert_with(|| message.clone());
                    }
                    let _ = request.events.send(ScrapeEvent::FatalError {
                        message: message.clone(),
                    });
                }
                RawOutcome {
                    key: outcome.key,
                    state: outcome.state,
                    pending: outcome.pending,
                }
            }
        })
        .buffer_unordered(request.max_workers.max(1))
        .collect()
        .await;

    if let Ok(guard) = tokio::time::timeout(LOCK_TIMEOUT, fatal_message.lock()).await {
        result.fatal.clone_from(&guard);
    }

    let (mut outcomes, pending) = split_outcomes(raw);
    result.published = publish_and_project(request, &pending, &mut outcomes);
    result.outcomes = outcomes;

    if let Some(archive) = request.archive
        && let Err(error) = std::fs::remove_dir_all(&archive.scratch_root)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!(
            "Could not remove completed scrape workspace {}: {error}",
            archive.scratch_root.display()
        );
    }

    let _ = request.events.send(ScrapeEvent::Done);
    result
}

/// Run [`run_scrape`] from synchronous code, owning the runtime.
///
/// The convergence executor, the CLI, and the GUI's worker threads are all
/// synchronous; this is the single place a runtime is built for scraping.
///
/// # Panics
/// Panics only if a Tokio runtime cannot be created, which means the process
/// is out of threads or file descriptors.
pub fn run_scrape_blocking(request: &ScrapeRequest<'_>) -> Result<ScrapeRunResult, ScrapeError> {
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|error| ScrapeError::Config(format!("Failed to create async runtime: {error}")))?;
    Ok(runtime.block_on(run_scrape(request)))
}

/// Whether the account's daily budget is spent down to its reserve.
///
/// Checked before each target rather than once up front, because the quota
/// figure is refreshed by every lookup response — an unattended run learns it
/// is out of budget from the work it just did.
async fn quota_exhausted(request: &ScrapeRequest<'_>) -> Option<(u32, u32)> {
    if request.options.request_reserve == 0 {
        return None;
    }
    let (remaining, max) = request.client.quota_headroom().await?;
    (remaining <= request.options.request_reserve).then_some((max.saturating_sub(remaining), max))
}

struct OneOutcome {
    key: u64,
    state: TargetState,
    pending: Vec<PendingPublication>,
    fatal: bool,
}

/// What [`prepare`] decided about a target.
enum Prepared {
    /// Ask `ScreenScraper` about `rom` for these types; `present` is what is
    /// already on disk for this target.
    Fetch {
        wanted: AssetSelection,
        present: HashMap<AssetType, PathBuf>,
        /// The identity to ask about, which is the file's own only when the
        /// file is what it appears to be.
        rom: RomInfo,
    },
    /// Finished without spending a request.
    Settled(OneOutcome),
}

/// Restore archived originals and decide whether this target needs the
/// network at all — and, if it does, what to ask about.
fn prepare(
    request: &ScrapeRequest<'_>,
    index: usize,
    target: &ScrapeTarget,
    filename: &str,
) -> Prepared {
    let options = request.options;
    let media_dir = target.destination.media_dir();
    let settled = |state: TargetState| OneOutcome {
        key: target.key,
        state,
        pending: Vec::new(),
        fatal: false,
    };

    // Archived originals are the durable copy: put them in the frontend tree
    // before anything else, so a wiped media folder is rebuilt even if the
    // lookup later fails. Not on a dry run, though — a dry run reports what
    // would happen and must leave the media tree untouched.
    if !options.dry_run {
        for (asset_type, source) in &target.archived_assets {
            if let Err(error) = project_asset(source, media_dir, &target.rom_stem, *asset_type) {
                log::warn!(
                    "Could not project archived {asset_type} for {}: {error}",
                    target.label
                );
            }
        }
    }

    let present = present_assets(request, target);
    if options.skip_scraped && !options.force_redownload && !present.is_empty() {
        return Prepared::Settled(settled(skipped(
            request,
            index,
            filename,
            "already scraped",
            with_miximage(request, target, present),
        )));
    }

    let wanted = types_to_fetch(request.selection, &present, options.force_redownload);
    if wanted.types.is_empty() {
        return Prepared::Settled(settled(skipped(
            request,
            index,
            filename,
            "media already exists",
            with_miximage(request, target, present),
        )));
    }

    // What the user decided this file is settles what to ask about — and
    // sometimes that there is nothing to ask. A mod with no parent recorded is
    // reported as skipped rather than failed: no request was spent, nothing
    // went wrong, and the missing piece is a decision only the user can make.
    let Some(rom) = target.derivation.identify(&target.rom) else {
        return Prepared::Settled(settled(skipped(
            request,
            index,
            filename,
            &target
                .derivation
                .note()
                .unwrap_or_else(|| "nothing to look up".to_owned()),
            with_miximage(request, target, present),
        )));
    };

    if options.dry_run {
        return Prepared::Settled(settled(skipped(
            request,
            index,
            filename,
            &format!("dry run (would try {})", planned_tier(&rom)),
            with_miximage(request, target, present),
        )));
    }

    Prepared::Fetch {
        wanted,
        present,
        rom,
    }
}

/// Look up and fetch one target's media.
async fn scrape_one(
    request: &ScrapeRequest<'_>,
    index: usize,
    target: &ScrapeTarget,
) -> OneOutcome {
    let options = request.options;
    let media_dir = target.destination.media_dir();
    let filename = target.rom.filename.clone();
    let done = |state: TargetState| OneOutcome {
        key: target.key,
        state,
        pending: Vec::new(),
        fatal: false,
    };

    let _ = request.events.send(ScrapeEvent::GameStarted {
        index,
        file: filename.clone(),
    });

    let (wanted, present, lookup_rom) = match prepare(request, index, target, &filename) {
        Prepared::Fetch {
            wanted,
            present,
            rom,
        } => (wanted, present, rom),
        Prepared::Settled(outcome) => return outcome,
    };

    let _ = request.events.send(ScrapeEvent::GameLookingUp {
        index,
        file: filename.clone(),
    });
    let lookup = match lookup::lookup_game(request.client, request.system_id, &lookup_rom).await {
        Ok(mut lookup) => {
            // Say whose metadata this is. Artwork that silently describes a
            // different game is worse than no artwork.
            lookup.warnings.extend(target.derivation.note());
            lookup
        }
        Err(ScrapeError::NotFound { mut warnings }) => {
            warnings.extend(target.derivation.note());
            let _ = request.events.send(ScrapeEvent::GameFailed {
                index,
                file: filename,
                reason: "Game not found".to_owned(),
            });
            return done(TargetState::NotFound { warnings });
        }
        Err(error) => return failed(request, index, target, &filename, &error),
    };

    let _ = request.events.send(ScrapeEvent::GameDownloading {
        index,
        file: filename.clone(),
    });

    // Archived targets download to scratch: the archive copy is authoritative
    // and must be published before anything reaches the frontend tree.
    let archived = target.destination.release_id().zip(request.archive);
    let (download_dir, download_stem) = match &archived {
        Some((_, archive)) => (archive.scratch_root.join(index.to_string()), "original"),
        None => (media_dir.to_path_buf(), target.rom_stem.as_str()),
    };

    let source_urls = assets::asset_source_urls(&lookup.game, &wanted, &target.region);
    let downloaded = match assets::download_game_assets(
        request.client,
        &AssetDownloadRequest {
            game: &lookup.game,
            selection: &wanted,
            media_dir: &download_dir,
            rom_stem: download_stem,
            preferred_region: &target.region,
            force_redownload: options.force_redownload,
            index,
            filename: &filename,
            events: request.events,
        },
    )
    .await
    {
        Ok(downloaded) => downloaded,
        Err(error) => return failed(request, index, target, &filename, &error),
    };

    let _ = request.events.send(ScrapeEvent::GameCompleted {
        index,
        file: filename,
        game_name: lookup
            .game
            .name_for_region(&target.region)
            .unwrap_or("Unknown")
            .to_owned(),
    });
    let publish_to = archived.map(|(release_id, _)| release_id);
    scraped_outcome(
        request,
        target,
        lookup,
        &downloaded,
        &source_urls,
        Settle {
            present,
            publish_to,
        },
    )
}

/// The parts of a finished target that decide how its downloads settle.
struct Settle {
    present: HashMap<AssetType, PathBuf>,
    /// Set when the files must go through the archive before the frontend.
    publish_to: Option<ArchiveReleaseId>,
}

/// Assemble a successful target's outcome.
fn scraped_outcome(
    request: &ScrapeRequest<'_>,
    target: &ScrapeTarget,
    lookup: lookup::LookupResult,
    downloaded: &HashMap<AssetType, PathBuf>,
    source_urls: &HashMap<AssetType, String>,
    settle: Settle,
) -> OneOutcome {
    let media_dir = target.destination.media_dir();
    let pending = settle.publish_to.map_or_else(Vec::new, |release_id| {
        downloaded
            .iter()
            .map(|(asset_type, path)| PendingPublication {
                key: target.key,
                release_id,
                asset_type: *asset_type,
                path: path.clone(),
                source_url: source_urls.get(asset_type).cloned().unwrap_or_default(),
                rom_stem: target.rom_stem.clone(),
                media_dir: media_dir.to_path_buf(),
            })
            .collect()
    });

    // Archived downloads live in scratch until publication, so the frontend
    // view of this target is what is already on disk; publication adds to it.
    let mut media = settle.present;
    if pending.is_empty() {
        media.extend(downloaded.clone());
    }

    OneOutcome {
        key: target.key,
        state: TargetState::Scraped {
            game: Box::new(lookup.game),
            method: lookup.method,
            warnings: lookup.warnings,
            assets: with_miximage(request, target, media),
        },
        pending,
        fatal: false,
    }
}

/// Turn a per-target error into its outcome.
///
/// Account-level failures propagate their fatal flag so the run stops; the
/// caller reports them once, not per target, so no `GameFailed` is emitted.
fn failed(
    request: &ScrapeRequest<'_>,
    index: usize,
    target: &ScrapeTarget,
    filename: &str,
    error: &ScrapeError,
) -> OneOutcome {
    let fatal = is_fatal(error);
    if !fatal {
        let _ = request.events.send(ScrapeEvent::GameFailed {
            index,
            file: filename.to_owned(),
            reason: error.to_string(),
        });
    }
    OneOutcome {
        key: target.key,
        state: TargetState::Failed {
            message: error.to_string(),
        },
        pending: Vec::new(),
        fatal,
    }
}

/// Narrow a selection to the types this target still needs.
///
/// Only fetching what is missing is the point: skipping any game that has
/// *some* media is how a library ends up with screenshots and no cover art
/// forever, and refetching everything burns the daily request budget.
fn types_to_fetch(
    selection: &AssetSelection,
    present: &HashMap<AssetType, PathBuf>,
    force: bool,
) -> AssetSelection {
    if force {
        return selection.clone();
    }
    selection.filtered(|asset_type| !present.contains_key(&asset_type))
}

/// Which lookup tier this identity will try first.
fn planned_tier(rom: &RomInfo) -> &'static str {
    if !rom.serial.is_empty() || !rom.scraper_serial.is_empty() {
        "serial"
    } else if rom.hashes.is_some() {
        "hash"
    } else {
        "filename"
    }
}

/// Assets already visible in the frontend tree for this target.
fn present_assets(
    request: &ScrapeRequest<'_>,
    target: &ScrapeTarget,
) -> HashMap<AssetType, PathBuf> {
    request
        .selection
        .collect_existing(target.destination.media_dir(), &target.rom_stem)
}

fn skipped(
    request: &ScrapeRequest<'_>,
    index: usize,
    filename: &str,
    reason: &str,
    assets: HashMap<AssetType, PathBuf>,
) -> TargetState {
    let _ = request.events.send(ScrapeEvent::GameSkipped {
        index,
        file: filename.to_owned(),
        reason: reason.to_owned(),
    });
    TargetState::Skipped {
        reason: reason.to_owned(),
        assets,
    }
}

/// Compose (or register) the miximage for a target whose media just settled.
fn with_miximage(
    request: &ScrapeRequest<'_>,
    target: &ScrapeTarget,
    mut media: HashMap<AssetType, PathBuf>,
) -> HashMap<AssetType, PathBuf> {
    let Some(layout) = request.options.miximage.layout() else {
        return media;
    };
    let path = miximage_path(target.destination.media_dir(), &target.rom_stem);
    if request.options.force_redownload || !path.exists() {
        match retro_junk_frontend::miximage::generate_miximage(&media, &path, layout) {
            Ok(true) => {
                media.insert(AssetType::Miximage, path);
            }
            // No screenshot to build from — not an error.
            Ok(false) => {}
            Err(error) => log::debug!("Failed to generate miximage: {error}"),
        }
    } else {
        media.insert(AssetType::Miximage, path);
    }
    media
}

/// Miximage output path for a ROM stem.
#[must_use]
pub fn miximage_path(media_dir: &Path, rom_stem: &str) -> PathBuf {
    media_dir
        .join(AssetType::Miximage.subdirectory())
        .join(format!("{rom_stem}.png"))
}

fn split_outcomes(raw: Vec<RawOutcome>) -> (Vec<TargetOutcome>, Vec<PendingPublication>) {
    let mut outcomes = Vec::with_capacity(raw.len());
    let mut pending = Vec::new();
    for item in raw {
        pending.extend(item.pending);
        outcomes.push(TargetOutcome {
            key: item.key,
            state: item.state,
        });
    }
    (outcomes, pending)
}

/// Publish downloaded files into the archive, then project them to the
/// frontend tree.
///
/// One batch for the whole run: `add_release_files` scans the archive index
/// once, and the archive lock is taken once rather than per file.
fn publish_and_project(
    request: &ScrapeRequest<'_>,
    pending: &[PendingPublication],
    outcomes: &mut [TargetOutcome],
) -> usize {
    let (Some(archive), false) = (request.archive, pending.is_empty()) else {
        return 0;
    };
    let _ = request.events.send(ScrapeEvent::Publishing {
        files: pending.len(),
    });

    let asset_names = pending
        .iter()
        .map(|item| item.asset_type.to_string())
        .collect::<Vec<_>>();
    let requests = pending
        .iter()
        .zip(&asset_names)
        .map(|(item, asset_name)| retro_junk_archive::NewReleaseFile {
            release_id: item.release_id,
            source_file: &item.path,
            category: if item.asset_type == AssetType::Video {
                ReleaseFileCategory::Video
            } else {
                ReleaseFileCategory::Artwork
            },
            asset_type: asset_name,
            source: "screenscraper",
            source_url: &item.source_url,
            caption: "",
        })
        .collect::<Vec<_>>();

    // The convergence executor already holds the whole-archive lock for the
    // action it is running; taking it again here would deadlock.
    let held = if archive.acquire_lock {
        match retro_junk_archive::ArchiveLock::acquire_wait(archive.archive_root, request.cancel) {
            Ok(Some(lock)) => Some(lock),
            Ok(None) => {
                // Cancelled while waiting for the lock. The downloads only
                // exist in scratch and are about to be discarded with it, so
                // the outcomes must stop claiming these targets were scraped —
                // otherwise a caller records converged work whose media never
                // entered the archive.
                mark_publication_failure(outcomes, pending, "cancelled before publication");
                return 0;
            }
            Err(error) => {
                log::error!("Could not lock the archive to publish scraped media: {error}");
                mark_publication_failure(outcomes, pending, &error.to_string());
                return 0;
            }
        }
    } else {
        None
    };
    let published =
        retro_junk_archive::add_release_files(archive.archive_root, &requests, request.cancel);
    drop(held);

    let published = match published {
        Ok(published) => published,
        Err(error) => {
            log::error!("Could not add scraped media to the archive: {error}");
            mark_publication_failure(outcomes, pending, &error.to_string());
            return 0;
        }
    };

    // Project from the scratch file, whose bytes `add_release_files` just
    // copy-verified into the archive — so what the frontend shows is
    // byte-identical to what the archive holds. If publication ever becomes a
    // move instead of a copy, this must switch to the archived destination.
    let mut projected: HashMap<u64, HashMap<AssetType, PathBuf>> = HashMap::new();
    for item in pending {
        match project_asset(&item.path, &item.media_dir, &item.rom_stem, item.asset_type) {
            Ok(destination) => {
                projected
                    .entry(item.key)
                    .or_default()
                    .insert(item.asset_type, destination);
            }
            Err(error) => log::warn!(
                "Could not project newly archived {} for {}: {error}",
                item.asset_type,
                item.rom_stem
            ),
        }
    }
    for outcome in outcomes.iter_mut() {
        if let Some(files) = projected.remove(&outcome.key)
            && let TargetState::Scraped { assets, .. } = &mut outcome.state
        {
            assets.extend(files);
        }
    }

    published.iter().filter(|result| result.added).count()
}

/// Turn the targets whose downloads could not be published into failures.
///
/// Reporting them as successes would leave the frontend showing media the
/// archive never accepted, which the next convergence pass would undo.
fn mark_publication_failure(
    outcomes: &mut [TargetOutcome],
    pending: &[PendingPublication],
    error: &str,
) {
    let affected: std::collections::HashSet<u64> = pending.iter().map(|item| item.key).collect();
    for outcome in outcomes.iter_mut().filter(|o| affected.contains(&o.key)) {
        outcome.state = TargetState::Failed {
            message: format!("downloaded media could not be added to the archive: {error}"),
        };
    }
}

#[cfg(test)]
#[path = "tests/session_tests.rs"]
mod tests;
