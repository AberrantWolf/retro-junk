//! Bulk artwork scraping for library entries and archived releases.
//!
//! Fills in file sizes, resolves each entry's user-decided derivation from
//! the catalog, connects to `ScreenScraper`, drives the shared scrape core
//! over every work item, and settles the outcomes into a typed report: which
//! entries failed, which entries have fresh media on disk (re-read after the
//! scrape wrote and composed the files), which image paths a frontend should
//! evict from its caches, and whether the archive gained artwork. The
//! frontend only schedules the call and delivers what comes back.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use retro_junk_core::Platform;
use retro_junk_frontend::AssetType;
use retro_junk_io::ProgressUnit;
use retro_junk_lib::async_util::{cancellable, run_with_events};

use super::OpCtx;
use crate::scrape::scrape_derivation;

#[cfg(test)]
#[path = "scrape_media_tests.rs"]
mod tests;

/// One file (or archived release) to scrape, collected by the frontend from
/// its selection.
pub struct ScrapeWorkItem {
    /// The playable library row this item belongs to. `None` for a release
    /// that exists only in the archive.
    pub entry_id: Option<retro_junk_db::LibraryEntryId>,
    pub entry_name: String,
    pub rom_stem: String,
    pub filename: String,
    /// The file whose size seeds the lookup. Empty for archive-only rows,
    /// whose identity records the size directly.
    pub analysis_path: PathBuf,
    pub file_size: u64,
    /// Serial from ROM header analysis. Empty = none.
    pub serial: String,
    /// Serial adapted for `ScreenScraper` lookups. Empty = none.
    pub scraper_serial: String,
    /// Hash triple for the `ScreenScraper` hash tier (all-or-nothing).
    pub hashes: Option<retro_junk_scraper::lookup::RomHashes>,
    /// What the user decided this file is, which settles whose identity the
    /// lookup offers. Entries with an id are re-resolved from the catalog
    /// before the scrape runs; archive-only rows keep the value given here.
    pub derivation: retro_junk_scraper::Derivation,
    pub preferred_region: String,
    pub platform: Platform,
    pub archive_release_id: Option<retro_junk_archive::ArchiveReleaseId>,
    pub archived_assets: HashMap<AssetType, PathBuf>,
}

impl ScrapeWorkItem {
    /// Convert a work item into a core scrape target.
    ///
    /// An archived release downloads through the archive; anything else goes
    /// straight to the frontend tree.
    fn to_target(&self, key: u64, media_dir: &Path) -> retro_junk_scraper::ScrapeTarget {
        retro_junk_scraper::ScrapeTarget {
            key,
            label: self.entry_name.clone(),
            rom_stem: self.rom_stem.clone(),
            rom: retro_junk_scraper::lookup::RomInfo {
                serial: self.serial.clone(),
                scraper_serial: self.scraper_serial.clone(),
                filename: self.filename.clone(),
                file_size: self.file_size,
                hashes: self.hashes.clone(),
                platform: self.platform,
                expects_serial: retro_junk_scraper::expects_serial(self.platform),
            },
            derivation: self.derivation.clone(),
            region: self.preferred_region.clone(),
            language: "en".to_owned(),
            destination: self.archive_release_id.map_or_else(
                || retro_junk_scraper::ScrapeDestination::Playable {
                    media_dir: media_dir.to_path_buf(),
                },
                |release_id| retro_junk_scraper::ScrapeDestination::Archive {
                    release_id,
                    media_dir: media_dir.to_path_buf(),
                },
            ),
            archived_assets: self.archived_assets.clone(),
        }
    }
}

/// The types a scrape asks for: everything by default, images only when the
/// caller excludes video.
fn default_asset_selection(artwork_only: bool) -> retro_junk_scraper::AssetSelection {
    let mut selection = retro_junk_scraper::AssetSelection::default();
    if artwork_only {
        selection
            .types
            .retain(|asset_type| *asset_type != AssetType::Video);
    }
    selection
}

/// Everything a scrape run needs beyond the work items themselves.
pub struct ScrapeMediaRequest {
    pub platform: Platform,
    /// Playable root the console folder lives under.
    pub root_path: PathBuf,
    pub folder_name: String,
    /// The user's media-directory setting, resolved per console.
    pub media_dir_setting: String,
    pub work: Vec<ScrapeWorkItem>,
    /// Re-download every selected type instead of only the missing ones.
    pub force_redownload: bool,
    /// Exclude videos from the selection.
    pub artwork_only: bool,
    /// Active collection profile; enables publication of downloads into the
    /// archive and provides the scratch workspace.
    pub archive_profile: Option<retro_junk_archive::CollectionProfile>,
    /// Unique tag for this run's scratch directory (e.g. the operation id).
    pub scratch_tag: String,
}

/// One entry the scrape could not settle, with the user-facing reason.
pub struct ScrapeEntryFailure {
    pub entry_id: retro_junk_db::LibraryEntryId,
    pub entry_name: String,
    pub error: String,
}

/// What a scrape run hands back to the frontend.
#[derive(Default)]
pub struct ScrapeMediaReport {
    /// Errors that ended (or prevented) the whole run — connection failures,
    /// quota exhaustion, unsupported platform — plus failures of archive-only
    /// rows, which have no entry to attach to.
    pub fatal_errors: Vec<String>,
    pub entry_failures: Vec<ScrapeEntryFailure>,
    /// Per settled entry: the media now on disk, re-read after the scrape
    /// wrote (and composed) the files.
    pub entry_assets: Vec<(retro_junk_db::LibraryEntryId, HashMap<AssetType, PathBuf>)>,
    /// Image paths whose bytes may have changed; a frontend should evict any
    /// cached decodes of these.
    pub invalidated_paths: Vec<PathBuf>,
    /// The archive gained supporting files; projections should refresh.
    pub archive_assets_changed: bool,
    /// True when the run stopped early on the cancel flag.
    pub cancelled: bool,
}

/// Scrape media from `ScreenScraper` for the requested work items.
///
/// When `force_redownload` is true, re-downloads all selected media types;
/// when false, only downloads missing types. The catalog database at
/// `db_path` supplies each entry's derivation so a mod is looked up as its
/// parent rather than wasting a request on bytes no scraper knows.
pub fn scrape_media(
    db_path: Option<&Path>,
    request: ScrapeMediaRequest,
    ctx: &OpCtx,
) -> ScrapeMediaReport {
    let ScrapeMediaRequest {
        platform,
        root_path,
        folder_name,
        media_dir_setting,
        mut work,
        force_redownload,
        artwork_only,
        archive_profile,
        scratch_tag,
    } = request;

    let mut report = ScrapeMediaReport::default();
    let total = work.len() as u64;
    (ctx.progress)("Scraping media", ProgressUnit::Items, 0, total);

    for item in &mut work {
        if ctx.cancelled() {
            report.cancelled = true;
            return report;
        }
        if !item.analysis_path.as_os_str().is_empty() {
            item.file_size =
                std::fs::metadata(&item.analysis_path).map_or(0, |metadata| metadata.len());
        }
    }
    resolve_derivations(db_path, &mut work);

    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(error) => {
            log::error!("Failed to create tokio runtime: {error}");
            report
                .fatal_errors
                .push(format!("Failed to create async runtime: {error}"));
            return report;
        }
    };

    rt.block_on(async {
        log::info!(
            "Artwork scrape {scratch_tag}: connecting to ScreenScraper for {} item(s)",
            work.len()
        );
        // Cancel-aware: the initial handshake can take ~90s on a slow link.
        let (client, max_workers) =
            match cancellable(retro_junk_scraper::create_client(None), ctx.cancel).await {
                None => {
                    report.cancelled = true;
                    return;
                }
                Some(Ok(connected)) => connected,
                Some(Err(error)) => {
                    log::error!("Failed to connect to ScreenScraper: {error}");
                    report
                        .fatal_errors
                        .push(format!("ScreenScraper connection failed: {error}"));
                    return;
                }
            };

        let Some(system_id) = retro_junk_scraper::screenscraper_system_id(platform) else {
            log::error!("No ScreenScraper system ID for {platform:?}");
            report.fatal_errors.push(format!(
                "Platform {platform:?} is not supported by ScreenScraper"
            ));
            return;
        };

        let Some(media_dir) = retro_junk_lib::util::asset_dir_for_console(
            &root_path,
            &folder_name,
            &media_dir_setting,
        ) else {
            log::error!("Cannot determine media directory for {folder_name}");
            report
                .fatal_errors
                .push("Cannot determine media directory".to_owned());
            return;
        };

        let targets = work
            .iter()
            .enumerate()
            .map(|(index, item)| item.to_target(index as u64, &media_dir))
            .collect::<Vec<_>>();
        let selection = default_asset_selection(artwork_only);
        let session_options = retro_junk_scraper::ScrapeSessionOptions {
            force_redownload,
            miximage: retro_junk_frontend::miximage_layout::MiximageLayout::load_or_create()
                .map_or(retro_junk_scraper::MiximageMode::Disabled, |layout| {
                    retro_junk_scraper::MiximageMode::Enabled(layout)
                }),
            ..retro_junk_scraper::ScrapeSessionOptions::default()
        };
        let publication =
            archive_profile
                .as_ref()
                .map(|profile| retro_junk_scraper::ArchivePublication {
                    archive_root: &profile.archive_root,
                    scratch_root: profile
                        .workspace_root
                        .join("archive-scrape")
                        .join(&scratch_tag),
                    acquire_lock: true,
                });

        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let completed = Cell::new(0_u64);
        // The phase most recently announced; count-only updates re-report it
        // because every progress callback carries its description.
        let current_phase = RefCell::new("Scraping media".to_owned());
        let fatal_events = RefCell::new(Vec::new());
        let scrape_request = retro_junk_scraper::ScrapeRequest {
            client: &client,
            system_id,
            targets: &targets,
            selection: &selection,
            options: &session_options,
            archive: publication.as_ref(),
            max_workers,
            events: &event_tx,
            cancel: ctx.cancel,
        };
        let run = retro_junk_scraper::run_scrape(&scrape_request);
        let run = run_with_events(run, event_rx, |event| {
            forward_scrape_event(
                &event,
                total,
                &completed,
                &current_phase,
                &fatal_events,
                &work,
                ctx,
            );
        })
        .await;
        drop(event_tx);
        report.fatal_errors.extend(fatal_events.into_inner());

        report.archive_assets_changed = run.published > 0;
        for outcome in &run.outcomes {
            let Some(item) = work.get(outcome.key as usize) else {
                continue;
            };
            match &outcome.state {
                retro_junk_scraper::TargetState::Failed { message } => {
                    record_scrape_failure(&mut report, item, message.clone());
                }
                retro_junk_scraper::TargetState::NotFound { .. } => {
                    record_scrape_failure(
                        &mut report,
                        item,
                        "not found on ScreenScraper".to_owned(),
                    );
                }
                retro_junk_scraper::TargetState::Scraped { assets, .. }
                | retro_junk_scraper::TargetState::Skipped { assets, .. } => {
                    // Refresh the entry's media now that the core settled it.
                    // The scrape already wrote (and composed) the files; the
                    // frontend drops its cached images for anything invalidated
                    // and shows what is now on disk.
                    let Some(entry_id) = item.entry_id else {
                        continue;
                    };
                    report.invalidated_paths.extend(assets.values().cloned());
                    let final_media =
                        crate::assets::collect_existing_assets(&media_dir, &item.rom_stem);
                    report
                        .invalidated_paths
                        .extend(final_media.values().cloned());
                    report.entry_assets.push((entry_id, final_media));
                }
                retro_junk_scraper::TargetState::NotReached => {}
            }
        }
    });

    report
}

/// Fill in what the user decided each selected row is.
///
/// A mod's own bytes are in no scraper's database, so asking about them spends
/// a request to learn nothing and leaves the row unidentified forever. The
/// catalog holds the answer — which work a mod was made from — and this reads
/// it once for the whole selection, off the UI thread because it is a query.
///
/// A failure here is not fatal: the scrape falls back to asking about each
/// file as itself, which is what it did before there were marks at all.
fn resolve_derivations(db_path: Option<&Path>, work: &mut [ScrapeWorkItem]) {
    let entry_ids = work
        .iter()
        .filter_map(|item| item.entry_id)
        .collect::<Vec<_>>();
    if entry_ids.is_empty() {
        return;
    }
    let Some(db_path) = db_path else {
        return;
    };
    let derivations = match retro_junk_db::open_database(db_path)
        .map_err(|error| error.to_string())
        .and_then(|connection| {
            retro_junk_db::query_entry_derivations(&connection, &entry_ids)
                .map_err(|error| error.to_string())
        }) {
        Ok(derivations) => derivations,
        Err(error) => {
            log::warn!("Could not read collection marks for this scrape: {error}");
            return;
        }
    };
    for item in work {
        if let Some(derivation) = item.entry_id.and_then(|id| derivations.get(&id)) {
            item.derivation = scrape_derivation(derivation);
        }
    }
}

/// Translate one core event into progress (and remember fatal ones).
///
/// Progress counts *finished* targets, so it advances on every terminal event
/// rather than only on success.
fn forward_scrape_event(
    event: &retro_junk_scraper::ScrapeEvent,
    total: u64,
    completed: &Cell<u64>,
    current_phase: &RefCell<String>,
    fatal_events: &RefCell<Vec<String>>,
    work: &[ScrapeWorkItem],
    ctx: &OpCtx,
) {
    use retro_junk_scraper::ScrapeEvent;

    let label = |index: usize| {
        work.get(index)
            .map_or("", |item| item.entry_name.as_str())
            .to_owned()
    };
    match event {
        ScrapeEvent::GameCompleted { index, .. }
        | ScrapeEvent::GameSkipped { index, .. }
        | ScrapeEvent::GameFailed { index, .. } => {
            completed.set(completed.get() + 1);
            log::debug!("Artwork scrape: finished {}", label(*index));
        }
        ScrapeEvent::GameDownloadingMedia {
            index, media_type, ..
        } => {
            *current_phase.borrow_mut() = format!("Downloading {media_type} for {}", label(*index));
        }
        ScrapeEvent::Publishing { files } => {
            *current_phase.borrow_mut() = format!("Publishing scraped artwork ({files} file(s))");
        }
        ScrapeEvent::QuotaExhausted { used, max } => {
            fatal_events.borrow_mut().push(format!(
                "Daily ScreenScraper quota reached ({used}/{max}); the rest can run tomorrow"
            ));
            return;
        }
        ScrapeEvent::FatalError { message } => {
            fatal_events.borrow_mut().push(message.clone());
            return;
        }
        ScrapeEvent::Scanning
        | ScrapeEvent::ScanComplete { .. }
        | ScrapeEvent::GameStarted { .. }
        | ScrapeEvent::GameLookingUp { .. }
        | ScrapeEvent::GameDownloading { .. }
        | ScrapeEvent::GameGrouped { .. }
        | ScrapeEvent::Done => return,
    }
    (ctx.progress)(
        &current_phase.borrow(),
        ProgressUnit::Items,
        completed.get(),
        total,
    );
}

/// File a failed target under its entry, or as a run-level error when the
/// target has no library row.
fn record_scrape_failure(report: &mut ScrapeMediaReport, item: &ScrapeWorkItem, error: String) {
    if let Some(entry_id) = item.entry_id {
        report.entry_failures.push(ScrapeEntryFailure {
            entry_id,
            entry_name: item.entry_name.clone(),
            error,
        });
    } else {
        report
            .fatal_errors
            .push(format!("{}: {error}", item.entry_name));
    }
}
