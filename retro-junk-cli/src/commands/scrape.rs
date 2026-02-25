use std::path::PathBuf;
use std::sync::Arc;

use indicatif::{ProgressBar, ProgressStyle};
use log::LevelFilter;
use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;

use retro_junk_lib::{AnalysisContext, Platform};

use crate::scan_folders;
use crate::spinner;

pub(crate) async fn connect_screenscraper(
    threads: Option<usize>,
    quiet: bool,
) -> Option<(Arc<retro_junk_scraper::ScreenScraperClient>, usize)> {
    let pb = if quiet {
        ProgressBar::hidden()
    } else {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::with_template("  {spinner:.cyan} {msg}")
                .unwrap()
                .tick_chars("/-\\|"),
        );
        pb.set_message("Connecting to ScreenScraper...");
        pb.enable_steady_tick(std::time::Duration::from_millis(100));
        pb
    };

    let (client, max_workers) = match retro_junk_scraper::client::create_client(threads).await {
        Ok(result) => result,
        Err(e) => {
            pb.finish_and_clear();
            log::error!(
                "{} Failed to connect to ScreenScraper: {}",
                "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                e,
            );
            if e.to_string().contains("credentials") {
                log::error!("");
                log::error!("Set credentials via environment variables:");
                log::error!("  SCREENSCRAPER_DEVID, SCREENSCRAPER_DEVPASSWORD");
                log::error!("  SCREENSCRAPER_SSID, SCREENSCRAPER_SSPASSWORD (optional)");
                log::error!("");
                log::error!("Or run 'retro-junk config setup' to configure credentials.");
            }
            return None;
        }
    };
    pb.finish_and_clear();

    if let Some(quota) = client.current_quota().await {
        log::info!(
            "{} Connected to ScreenScraper (requests today: {}/{}, using: {} workers)",
            "\u{2714}".if_supports_color(Stdout, |t| t.green()),
            quota.requests_today(),
            quota.max_requests_per_day(),
            max_workers,
        );
    } else {
        log::info!(
            "{} Connected to ScreenScraper (using: {} workers)",
            "\u{2714}".if_supports_color(Stdout, |t| t.green()),
            max_workers,
        );
    }
    log::info!("");

    Some((client, max_workers))
}

/// Run the scrape command.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_scrape(
    ctx: &AnalysisContext,
    consoles: Option<Vec<Platform>>,
    limit: Option<usize>,
    media_types: Option<Vec<String>>,
    metadata_dir: Option<PathBuf>,
    media_dir: Option<PathBuf>,
    _frontend: String,
    region: String,
    language: String,
    language_fallback: String,
    force_full_hash: bool,
    dry_run: bool,
    skip_existing: bool,
    no_log: bool,
    no_miximage: bool,
    force_redownload: bool,
    threads: Option<usize>,
    root: Option<PathBuf>,
    quiet: bool,
) {
    let root_path =
        root.unwrap_or_else(|| std::env::current_dir().expect("Failed to get current directory"));

    // Build scrape options
    let mut options = retro_junk_scraper::ScrapeOptions::new(root_path.clone());
    options.region = region;
    options.language = language;
    options.language_fallback = language_fallback;
    options.force_hash = force_full_hash;
    options.dry_run = dry_run;
    options.skip_existing = skip_existing;
    options.no_log = no_log;
    options.no_miximage = no_miximage;
    options.force_redownload = force_redownload;
    options.limit = limit;

    // Load miximage layout unless disabled
    if !no_miximage {
        match retro_junk_frontend::miximage_layout::MiximageLayout::load_or_create() {
            Ok(layout) => {
                options.miximage_layout = Some(layout);
            }
            Err(e) => {
                log::warn!(
                    "{} Failed to load miximage layout, disabling miximages: {}",
                    "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                    e,
                );
            }
        }
    }

    if let Some(mdir) = metadata_dir {
        options.metadata_dir = mdir;
    }
    if let Some(mdir) = media_dir {
        options.media_dir = mdir;
    }
    if let Some(ref types) = media_types {
        options.media_selection = retro_junk_scraper::MediaSelection::from_names(types);
    }

    log::info!(
        "Scraping ROMs in: {}",
        root_path.display().if_supports_color(Stdout, |t| t.cyan()),
    );
    if dry_run {
        log::info!(
            "{}",
            "Dry run: no files will be downloaded".if_supports_color(Stdout, |t| t.dimmed()),
        );
    }
    if let Some(n) = limit {
        log::info!(
            "{}",
            format!("Limit: {} ROMs per console", n).if_supports_color(Stdout, |t| t.dimmed()),
        );
    }
    log::info!(
        "Metadata: {}",
        options.metadata_dir.display().if_supports_color(Stdout, |t| t.dimmed()),
    );
    log::info!(
        "Media:    {}",
        options.media_dir.display().if_supports_color(Stdout, |t| t.dimmed()),
    );
    log::info!("");

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");

    rt.block_on(async {
        let (client, max_workers) = match connect_screenscraper(threads, quiet).await {
            Some(r) => r,
            None => return,
        };

        let scan = match scan_folders(ctx, &root_path, &consoles) {
            Some(s) => s,
            None => return,
        };

        let esde = retro_junk_frontend::esde::EsDeFrontend::new();
        let mut total_games = 0usize;
        let mut total_media = 0usize;
        let mut total_errors = 0usize;
        let mut total_unidentified = 0usize;

        for cf in &scan.matches {
            let console = ctx.get_by_platform(cf.platform).unwrap();
            let path = &cf.path;
            let folder_name = &cf.folder_name;

            // Check if this system has a ScreenScraper ID
            if retro_junk_scraper::screenscraper_system_id(cf.platform).is_none() {
                log::warn!(
                    "  {} Skipping \"{}\" — no ScreenScraper system ID",
                    "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                    folder_name,
                );
                continue;
            }

            log::info!(
                "{} {}",
                console
                    .metadata
                    .platform_name
                    .if_supports_color(Stdout, |t| t.bold()),
                format!("({})", folder_name).if_supports_color(Stdout, |t| t.dimmed()),
            );

            // Set up MultiProgress with N spinner slots
            let mut pool = spinner::SpinnerPool::new(max_workers, quiet, true);
            let mut scan_total = 0usize;

            let (event_tx, event_rx) =
                tokio::sync::mpsc::unbounded_channel::<retro_junk_scraper::ScrapeEvent>();

            let scrape_future = retro_junk_scraper::scrape_folder(
                &client,
                path,
                console.analyzer.as_ref(),
                &options,
                folder_name,
                max_workers,
                event_tx,
            );

            let scrape_result = retro_junk_lib::async_util::run_with_events(
                scrape_future,
                event_rx,
                |e| match e {
                    retro_junk_scraper::ScrapeEvent::Scanning => {
                        pool.claim(usize::MAX, "Scanning for ROM files...".into());
                    }
                    retro_junk_scraper::ScrapeEvent::ScanComplete { total } => {
                        scan_total = total;
                        pool.release(usize::MAX);
                    }
                    retro_junk_scraper::ScrapeEvent::GameStarted { index, ref file } => {
                        pool.claim(index, format!("[{}/{}] {}", index + 1, scan_total, file));
                    }
                    retro_junk_scraper::ScrapeEvent::GameLookingUp { index, ref file } => {
                        pool.update(index, format!("[{}/{}] Looking up {}", index + 1, scan_total, file));
                    }
                    retro_junk_scraper::ScrapeEvent::GameDownloading { index, ref file } => {
                        pool.update(index, format!("[{}/{}] Downloading media for {}", index + 1, scan_total, file));
                    }
                    retro_junk_scraper::ScrapeEvent::GameDownloadingMedia { index, ref file, ref media_type } => {
                        pool.update(index, format!("[{}/{}] Downloading {} for {}", index + 1, scan_total, media_type, file));
                    }
                    retro_junk_scraper::ScrapeEvent::GameSkipped { index, ref file, ref reason } => {
                        pool.update(index, format!("[{}/{}] Skipped {}: {}", index + 1, scan_total, file, reason));
                        pool.release(index);
                    }
                    retro_junk_scraper::ScrapeEvent::GameCompleted { index, ref file, ref game_name } => {
                        pool.update(index, format!("[{}/{}] {} -> \"{}\"", index + 1, scan_total, file, game_name));
                        pool.release(index);
                    }
                    retro_junk_scraper::ScrapeEvent::GameFailed { index, ref file, ref reason } => {
                        pool.update(index, format!("[{}/{}] {} failed: {}", index + 1, scan_total, file, reason));
                        pool.release(index);
                    }
                    retro_junk_scraper::ScrapeEvent::GameGrouped { .. } => {
                        // Grouped discs happen after the concurrent phase; no spinner
                    }
                    retro_junk_scraper::ScrapeEvent::FatalError { ref message } => {
                        pool.clear_all();
                        log::warn!(
                            "  {} Fatal: {}",
                            "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                            message,
                        );
                    }
                    retro_junk_scraper::ScrapeEvent::Done => {}
                },
            ).await;

            pool.clear_all();

            match scrape_result {
                Ok(result) => {
                    let summary = result.log.summary();
                    total_games += summary.total_success + summary.total_partial + summary.total_grouped;
                    total_media += summary.media_downloaded;
                    total_errors += summary.total_errors;
                    total_unidentified += summary.total_unidentified;

                    let has_issues = summary.total_unidentified > 0 || summary.total_errors > 0;

                    // In quiet mode, re-emit console header as warn for context
                    if has_issues && log::max_level() < LevelFilter::Info {
                        log::warn!(
                            "{} ({}):",
                            console.metadata.platform_name,
                            folder_name,
                        );
                    }

                    // Print per-system summary
                    if summary.total_success > 0 {
                        log::info!(
                            "  {} {} games scraped (serial: {}, filename: {}, hash: {})",
                            "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                            summary.total_success,
                            summary.by_serial,
                            summary.by_filename,
                            summary.by_hash,
                        );
                    }
                    if summary.total_grouped > 0 {
                        log::info!(
                            "  {} {} discs grouped with primary",
                            "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                            summary.total_grouped,
                        );
                    }
                    if summary.media_downloaded > 0 {
                        log::info!(
                            "  {} {} media files downloaded",
                            "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                            summary.media_downloaded,
                        );
                    }
                    if summary.total_unidentified > 0 {
                        log::warn!(
                            "  {} {} unidentified",
                            "?".if_supports_color(Stdout, |t| t.yellow()),
                            summary.total_unidentified,
                        );
                    }
                    if summary.total_errors > 0 {
                        log::warn!(
                            "  {} {} errors",
                            "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                            summary.total_errors,
                        );
                    }

                    // Print per-game details for problem entries
                    for entry in result.log.entries() {
                        match entry {
                            retro_junk_scraper::LogEntry::Unidentified {
                                file,
                                serial_tried,
                                scraper_serial_tried,
                                filename_tried,
                                hashes_tried,
                                crc32,
                                md5,
                                sha1,
                                errors,
                            } => {
                                log::warn!("  ? {}: unidentified", file);
                                if let Some(s) = serial_tried {
                                    log::warn!("      ROM serial: {}", s);
                                }
                                if let Some(s) = scraper_serial_tried {
                                    log::warn!("      Scraper serial tried: {}", s);
                                }
                                if *filename_tried {
                                    log::warn!("      Filename lookup: tried");
                                }
                                if *hashes_tried {
                                    log::warn!("      Hash lookup: tried");
                                    if let Some(c) = crc32 {
                                        log::warn!("        CRC32: {}", c);
                                    }
                                    if let Some(m) = md5 {
                                        log::warn!("        MD5:   {}", m);
                                    }
                                    if let Some(s) = sha1 {
                                        log::warn!("        SHA1:  {}", s);
                                    }
                                }
                                for e in errors {
                                    log::warn!("      Error: {}", e);
                                }
                            }
                            retro_junk_scraper::LogEntry::Partial {
                                file,
                                game_name,
                                warnings,
                            } => {
                                log::warn!(
                                    "  {} {}: \"{}\"",
                                    "~".if_supports_color(Stdout, |t| t.green()),
                                    file,
                                    game_name.if_supports_color(Stdout, |t| t.green()),
                                );
                                for w in warnings {
                                    log::warn!("      {}", w);
                                }
                            }
                            retro_junk_scraper::LogEntry::Error { file, message } => {
                                log::warn!("  {} {}: {}", "\u{2718}", file, message);
                            }
                            _ => {}
                        }
                    }

                    // Write metadata
                    if !result.games.is_empty() && !dry_run {
                        let system_metadata_dir = options.metadata_dir.join(folder_name);
                        let system_media_dir = options.media_dir.join(folder_name);

                        use retro_junk_frontend::Frontend;
                        if let Err(e) = esde.write_metadata(
                            &result.games,
                            path,
                            &system_metadata_dir,
                            &system_media_dir,
                        ) {
                            log::warn!(
                                "  {} Error writing metadata: {}",
                                "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                                e,
                            );
                        } else {
                            log::info!(
                                "  {} gamelist.xml written to {}",
                                "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                                system_metadata_dir.display(),
                            );
                        }
                    }

                    // Write scrape log
                    if !no_log && !dry_run {
                        let log_path = options.metadata_dir.join(format!(
                            "scrape-log-{}-{}.txt",
                            folder_name,
                            chrono::Local::now().format("%Y%m%d-%H%M%S"),
                        ));
                        if let Err(e) = std::fs::create_dir_all(&options.metadata_dir) {
                            log::warn!("Warning: could not create metadata dir: {}", e);
                        } else if let Err(e) = result.log.write_to_file(&log_path) {
                            log::warn!("Warning: could not write scrape log: {}", e);
                        }
                    }
                }
                Err(e) => {
                    log::warn!(
                        "  {} Error: {}",
                        "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                        e,
                    );
                    total_errors += 1;
                }
            }
            log::info!("");
        }

        // Print overall summary
        if total_games > 0 || total_errors > 0 || total_unidentified > 0 {
            log::info!("{}", "Summary:".if_supports_color(Stdout, |t| t.bold()));
            if total_games > 0 {
                log::info!(
                    "  {} {} games scraped, {} media files",
                    "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                    total_games,
                    total_media,
                );
            }
            if total_unidentified > 0 {
                log::warn!(
                    "  {} {} unidentified",
                    "?".if_supports_color(Stdout, |t| t.yellow()),
                    total_unidentified,
                );
            }
            if total_errors > 0 {
                log::warn!(
                    "  {} {} errors",
                    "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                    total_errors,
                );
            }

            // Show remaining quota
            if let Some(quota) = client.current_quota().await {
                log::info!(
                    "  Quota: {}/{} requests used today",
                    quota.requests_today(),
                    quota.max_requests_per_day(),
                );
            }
        }
    });
}
