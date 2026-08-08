//! The convergence executor's scrape handler.
//!
//! Coordination only, as with every other action kind: it reads the release's
//! identity, calls the one scrape orchestration in `retro-junk-scraper`, and
//! translates the verdict. It owns no scraping logic — the GUI's artwork
//! actions and `retro-junk scrape` run exactly the same code.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use retro_junk_db::convergence::{ProposedAction, WorkTarget, scrape_gaps};
use retro_junk_db::library::ScrapeIdentityTier;
use retro_junk_io::{PhaseProgressFn, ProgressUnit};

use crate::executor::{ExecContext, WorkError};

/// How often the handler reports progress while a scrape is in flight.
///
/// The claim heartbeat piggy-backs on the progress callback and the claim
/// times out after two minutes, so a run that is only waiting on the API's
/// rate limiter still has to speak up or it loses its claim to the next tick.
const HEARTBEAT_HINT: Duration = Duration::from_secs(20);

/// What one release's scrape did.
#[derive(Debug, Default)]
pub struct ScrapeReport {
    /// Supporting files newly added to the archive.
    pub published: usize,
    /// Set when the match was too weak to publish unattended.
    pub needs_review: Option<WeakMatch>,
}

/// A release whose only match came from the weakest lookup tier.
#[derive(Debug, Clone)]
pub struct WeakMatch {
    pub archive_release_id: String,
    pub label: String,
    pub missing: Vec<String>,
}

/// Fetch one release's missing artwork.
pub fn scrape_release_artwork(
    ctx: &ExecContext,
    action: &ProposedAction,
    conn: &retro_junk_db::Connection,
    progress: &PhaseProgressFn<'_>,
    cancelled: &AtomicBool,
) -> Result<ScrapeReport, WorkError> {
    let WorkTarget::Release(release_id) = &action.target else {
        return Err(WorkError::Message(format!(
            "expected a release target, got {}",
            action.target.kind()
        )));
    };

    let gap = scrape_gaps(conn, &action.profile_id, &ctx.scrape.expected_assets)?
        .into_iter()
        .find(|gap| gap.archive_release_id == *release_id)
        .ok_or_else(|| {
            WorkError::Message(format!(
                "archive release {release_id} is no longer in the projection"
            ))
        })?;
    if gap.missing.is_empty() {
        // Another run filled the gap between derivation and execution.
        return Ok(ScrapeReport::default());
    }
    warn_if_scraping_without_a_catalog(conn, &action.platform_id, progress);
    if ctx.scrape.only_when_unambiguous && gap.identity <= ScrapeIdentityTier::Filename {
        return Ok(ScrapeReport {
            published: 0,
            needs_review: Some(WeakMatch {
                archive_release_id: gap.archive_release_id.clone(),
                label: action.label.clone(),
                missing: retro_junk_frontend::AssetSelection {
                    types: gap.missing.clone(),
                }
                .names(),
            }),
        });
    }

    let target = build_target(ctx, action, conn, &gap, release_id)?;
    let platform = target.rom.platform;
    let system_id = retro_junk_scraper::screenscraper_system_id(platform).ok_or_else(|| {
        WorkError::Message(format!(
            "{} is not a platform ScreenScraper covers",
            action.platform_id
        ))
    })?;

    // Only ask for what is missing: the run is unattended and the daily
    // budget is shared with every other release.
    let selection = retro_junk_frontend::AssetSelection {
        types: gap.missing.clone(),
    };
    let options = retro_junk_scraper::ScrapeSessionOptions {
        request_reserve: ctx.scrape.daily_request_reserve,
        ..retro_junk_scraper::ScrapeSessionOptions::default()
    };

    let run = run_one(RunOne {
        ctx,
        label: &action.label,
        release_id,
        system_id,
        target: &target,
        selection: &selection,
        options: &options,
        progress,
        cancelled,
    })?;

    if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(WorkError::Cancelled);
    }
    if let Some(message) = run.fatal {
        return Err(WorkError::Message(message));
    }
    match run.outcomes.first().map(|outcome| &outcome.state) {
        Some(retro_junk_scraper::TargetState::Failed { message }) => {
            Err(WorkError::Message(message.clone()))
        }
        Some(retro_junk_scraper::TargetState::NotFound { .. }) => Err(WorkError::Message(
            "ScreenScraper has no entry for this release".to_owned(),
        )),
        // A target the core declined to ask about — a mod with no parent
        // recorded — is not a failure and must not be backed off like one, but
        // an explicit run deserves to hear why nothing happened.
        Some(retro_junk_scraper::TargetState::Skipped { reason, .. }) => {
            progress(reason, ProgressUnit::Items, 1, 1);
            log::info!("{}: {reason}", action.label);
            Ok(ScrapeReport::default())
        }
        _ => Ok(ScrapeReport {
            published: run.published,
            needs_review: None,
        }),
    }
}

/// The catalog's record of a derivation, in the terms the scrape core speaks.
///
/// One translation, shared by the executor and the GUI's artwork actions —
/// the only two layers that can see both crates. Two copies would be two
/// chances to disagree about what a mod is looked up as, and disagreeing means
/// one surface publishing artwork the other would have refused.
#[must_use]
pub fn scrape_derivation(
    derivation: &retro_junk_db::CatalogDerivation,
) -> retro_junk_scraper::Derivation {
    use retro_junk_db::CatalogDerivation;
    match derivation {
        CatalogDerivation::Own => retro_junk_scraper::Derivation::Own,
        CatalogDerivation::Homebrew => retro_junk_scraper::Derivation::Standalone,
        CatalogDerivation::Modded { parent: None } => retro_junk_scraper::Derivation::UnknownParent,
        CatalogDerivation::Modded {
            parent: Some(parent),
        } => retro_junk_scraper::Derivation::Parent(retro_junk_scraper::ParentIdentity {
            filename: parent.filename.clone(),
            file_size: parent.file_size,
            serial: parent.serial.clone(),
            hashes: retro_junk_scraper::RomHashes::complete(
                &parent.crc32,
                &parent.md5,
                &parent.sha1,
            ),
        }),
    }
}

/// Inputs to one release's scrape run.
#[derive(Clone, Copy)]
struct RunOne<'a> {
    ctx: &'a ExecContext,
    label: &'a str,
    release_id: &'a str,
    system_id: u32,
    target: &'a retro_junk_scraper::ScrapeTarget,
    selection: &'a retro_junk_frontend::AssetSelection,
    options: &'a retro_junk_scraper::ScrapeSessionOptions,
    progress: &'a PhaseProgressFn<'a>,
    cancelled: &'a AtomicBool,
}

/// Drive the shared scrape core, reporting progress as it goes.
///
/// The executor's claim heartbeat rides on `progress`, so this reports on
/// every event *and* at least once per [`HEARTBEAT_HINT`] — a scrape that is
/// only waiting on the API's rate limiter still has to speak up or it loses
/// its claim to the next tick.
fn run_one(run: RunOne<'_>) -> Result<retro_junk_scraper::ScrapeRunResult, WorkError> {
    (run.progress)("Connecting to ScreenScraper", ProgressUnit::Items, 0, 1);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| WorkError::Message(format!("could not start async runtime: {error}")))?;
    let (client, max_workers) = runtime
        .block_on(retro_junk_scraper::create_client(None))
        .map_err(|error| WorkError::Message(error.to_string()))?;

    let (events, mut received) = tokio::sync::mpsc::unbounded_channel();
    let publication = retro_junk_scraper::ArchivePublication {
        archive_root: &run.ctx.profile.archive_root,
        scratch_root: run
            .ctx
            .profile
            .processing_workspace_root()
            .join("scrape")
            .join(run.release_id),
        // The executor already holds the whole-archive lock for this action;
        // taking it again here would deadlock.
        acquire_lock: false,
    };
    let request = retro_junk_scraper::ScrapeRequest {
        client: &client,
        system_id: run.system_id,
        targets: std::slice::from_ref(run.target),
        selection: run.selection,
        options: run.options,
        archive: Some(&publication),
        max_workers,
        events: &events,
        cancel: run.cancelled,
    };
    let last = std::cell::Cell::new(Instant::now());
    Ok(runtime.block_on(async {
        let scrape = retro_junk_scraper::run_scrape(&request);
        tokio::pin!(scrape);
        loop {
            tokio::select! {
                result = &mut scrape => break result,
                event = received.recv() => {
                    if let Some(event) = event {
                        report_event(&event, run.progress, &last);
                    }
                }
                () = tokio::time::sleep(HEARTBEAT_HINT) => {
                    (run.progress)(&format!("Scraping {}", run.label), ProgressUnit::Items, 0, 1);
                    last.set(Instant::now());
                }
            }
        }
    }))
}

/// The one warning about scraping a platform with no catalog behind it.
///
/// A scrape is only as good as the identity it searches with. With a DAT
/// imported we query using the catalog entry's own title and digests, which
/// is what the entry *is*; without one there is nothing to speak for the game
/// but whatever the local file happens to hash to, and matches get noticeably
/// worse. That is worth saying out loud rather than silently returning worse
/// artwork, so it goes through the shared progress channel and reaches the CLI
/// log and the GUI toast alike — one message, not one per frontend.
fn warn_if_scraping_without_a_catalog(
    conn: &retro_junk_db::Connection,
    platform_id: &str,
    progress: &PhaseProgressFn<'_>,
) {
    if retro_junk_db::identify::catalog_has_platform(conn, platform_id).unwrap_or(true) {
        return;
    }
    let message = format!(
        "No DAT imported for {platform_id} — scraping falls back to local file hashes and will          match less accurately. Import the catalog for this platform to improve it."
    );
    log::warn!("{message}");
    progress(&message, retro_junk_io::ProgressUnit::Items, 0, 0);
}

/// Assemble the scrape target for one archived release.
///
/// The identity is the archive's own catalog binding, and the media stem is
/// the playable output it recorded — downloaded artwork has to land on the
/// same names the projection and gamelist use or the frontend never finds it.
fn build_target(
    ctx: &ExecContext,
    action: &ProposedAction,
    conn: &retro_junk_db::Connection,
    gap: &retro_junk_db::convergence::ScrapeGap,
    release_id: &str,
) -> Result<retro_junk_scraper::ScrapeTarget, WorkError> {
    let identity =
        retro_junk_db::library::query_archived_scrape_identities(conn, &action.profile_id)
            .map_err(WorkError::Library)?
            .remove(release_id)
            .ok_or_else(|| {
                WorkError::Message("release has no catalog identity to look up".to_owned())
            })?;
    let release_uuid = release_id
        .parse::<retro_junk_archive::ArchiveReleaseId>()
        .map_err(|error| WorkError::Message(format!("bad archive release id: {error}")))?;
    let platform: retro_junk_core::Platform = action
        .platform_id
        .parse()
        .map_err(|_| WorkError::Message(format!("unknown platform '{}'", action.platform_id)))?;

    let rom_stem = release_media_stems(conn, release_id)?
        .into_iter()
        .next()
        .unwrap_or_else(|| retro_junk_archive::slugify(&gap.title));
    let scraper_serial = ctx
        .analyzers
        .get_by_platform(platform)
        .and_then(|console| console.analyzer.extract_scraper_serial(&identity.serial))
        .unwrap_or_default();

    Ok(retro_junk_scraper::ScrapeTarget {
        key: 0,
        label: action.label.clone(),
        rom_stem,
        rom: retro_junk_scraper::RomInfo {
            serial: identity.serial.clone(),
            scraper_serial,
            filename: identity.filename.clone(),
            file_size: identity.file_size,
            hashes: retro_junk_scraper::RomHashes::complete(
                &identity.crc32,
                &identity.md5,
                &identity.sha1,
            ),
            platform,
            expects_serial: retro_junk_scraper::expects_serial(platform),
        },
        derivation: scrape_derivation(&identity.derivation),
        region: retro_junk_scraper::systems::region_slug_to_ss_code(&gap.region).to_owned(),
        language: "en".to_owned(),
        destination: retro_junk_scraper::ScrapeDestination::Archive {
            release_id: release_uuid,
            media_dir: ctx.roots.media_root.join(&action.playable_platform_id),
        },
        archived_assets: HashMap::new(),
    })
}

/// Report one scrape event as executor progress, keeping the claim alive.
fn report_event(
    event: &retro_junk_scraper::ScrapeEvent,
    progress: &PhaseProgressFn<'_>,
    last: &std::cell::Cell<Instant>,
) {
    use retro_junk_scraper::ScrapeEvent;

    let description = match event {
        ScrapeEvent::GameLookingUp { file, .. } => format!("Looking up {file}"),
        ScrapeEvent::GameDownloadingMedia {
            media_type, file, ..
        } => format!("Downloading {media_type} for {file}"),
        ScrapeEvent::Publishing { files } => format!("Publishing {files} file(s) to the archive"),
        ScrapeEvent::QuotaExhausted { used, max } => {
            format!("ScreenScraper daily quota reached ({used}/{max})")
        }
        _ => return,
    };
    progress(&description, ProgressUnit::Items, 0, 1);
    last.set(Instant::now());
}

/// Frontend media stems for a release, from the playable outputs the archive
/// recorded. Downloaded artwork must land on the same names the projection
/// and gamelist use, or the frontend will not find it.
fn release_media_stems(
    conn: &retro_junk_db::Connection,
    release_id: &str,
) -> Result<Vec<String>, WorkError> {
    let mut statement = conn
        .prepare(
            "SELECT DISTINCT rep.relative_path
             FROM representations rep
             JOIN carriers c ON c.id=rep.carrier_id
             JOIN physical_copies pc ON pc.id=c.physical_copy_id
             WHERE pc.archive_release_id=?1 AND rep.role='playable'
             ORDER BY rep.relative_path",
        )
        .map_err(|error| WorkError::Message(error.to_string()))?;
    let paths = statement
        .query_map([release_id], |row| row.get::<_, String>(0))
        .map_err(|error| WorkError::Message(error.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| WorkError::Message(error.to_string()))?;
    Ok(paths
        .iter()
        .filter_map(|path| frontend_stem(path))
        .collect())
}

/// The media stem for one playable output.
///
/// Multi-disc sets keep the `.m3u` directory name, because ES-DE keys media
/// by that name rather than by the individual disc files.
fn frontend_stem(relative_path: &str) -> Option<String> {
    let path = std::path::Path::new(relative_path);
    let in_playlist_directory = path
        .parent()
        .and_then(std::path::Path::file_name)
        .and_then(|name| name.to_str())
        .filter(|name| name.to_ascii_lowercase().ends_with(".m3u"));
    if let Some(directory) = in_playlist_directory {
        return Some(directory.to_owned());
    }
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_owned)
}

#[cfg(test)]
#[path = "tests/scrape_tests.rs"]
mod tests;
