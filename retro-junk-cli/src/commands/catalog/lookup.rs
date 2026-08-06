use std::collections::{HashMap, HashSet};

use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;

use crate::CliError;
use crate::cli_types::{CatalogEntityType, CatalogLookupArgs};

use super::{default_catalog_db_path, format_file_size_or, or_str, truncate_str};

/// Ask for one platform by its id, rather than searching for the text.
///
/// A platform id is a short slug — `ps1`, `snes` — that would otherwise read
/// as a search term, so it needs a marker. Works, releases and media do not:
/// their ids already begin with `wrk_`, `rel_` or `med_`, which says what kind
/// of thing they name. There used to be a display wrapper adding `wrk-`,
/// `rel-` and `med-` on top of ids that began with neither, and it could not
/// tell its own `rel-` prefix from a title slug that happened to start with
/// "rel-".
const PREFIX_PLATFORM: &str = "plt-";

/// Entry point for `catalog lookup`.
pub(crate) fn run_catalog_lookup(args: CatalogLookupArgs) -> Result<(), CliError> {
    let CatalogLookupArgs {
        query,
        r#type: entity_type,
        platform,
        manufacturer,
        crc,
        sha1,
        md5,
        serial,
        limit,
        offset,
        group,
        db: db_path,
    } = args;
    let db_path = db_path.unwrap_or_else(default_catalog_db_path);
    if !db_path.exists() {
        log::warn!("No catalog database found at {}", db_path.display());
        log::info!("Run 'retro-junk catalog import all' first.");
        return Ok(());
    }

    let conn = retro_junk_db::open_database(&db_path)
        .map_err(|e| CliError::database(format!("Failed to open catalog database: {e}")))?;

    // ── Hash / serial lookups (original behavior) ─────────────────────
    // Mutual exclusivity of --crc/--sha1/--md5/--serial is enforced by
    // clap via the "hash_lookup" ArgGroup.
    let has_hash_or_serial = crc.is_some() || sha1.is_some() || md5.is_some() || serial.is_some();

    if has_hash_or_serial {
        let platform_label = make_platform_label(&conn);
        let company_label = make_company_label(&conn);

        if let Some(ref hash) = crc {
            let hash = hash.to_lowercase();
            lookup_by_hash(
                &conn,
                "CRC32",
                &hash,
                |h| retro_junk_db::find_media_by_crc32(&conn, h),
                &platform,
                &platform_label,
                &company_label,
            );
        } else if let Some(ref hash) = sha1 {
            let hash = hash.to_lowercase();
            lookup_by_hash(
                &conn,
                "SHA1",
                &hash,
                |h| retro_junk_db::find_media_by_sha1(&conn, h),
                &platform,
                &platform_label,
                &company_label,
            );
        } else if let Some(ref hash) = md5 {
            let hash = hash.to_lowercase();
            lookup_by_hash(
                &conn,
                "MD5",
                &hash,
                |h| retro_junk_db::find_media_by_md5(&conn, h),
                &platform,
                &platform_label,
                &company_label,
            );
        } else if let Some(ref s) = serial {
            lookup_by_serial(&conn, s, &platform, &platform_label, &company_label);
        }
        return Ok(());
    }

    // ── Browse/search modes ───────────────────────────────────────────
    match query {
        Some(q) if is_id_lookup(&q) => dispatch_id_lookup(&conn, &q),
        Some(q) => dispatch_search(&conn, &q, entity_type, &platform, limit, offset),
        None => dispatch_listing(
            &conn,
            entity_type,
            &platform,
            &manufacturer,
            limit,
            offset,
            group,
        ),
    }

    Ok(())
}

// ── Routing helpers ─────────────────────────────────────────────────────────

/// Is this query naming one row outright, rather than describing what to
/// search for?
fn is_id_lookup(q: &str) -> bool {
    q.starts_with(PREFIX_PLATFORM) || retro_junk_catalog::content_id::is_content_id(q)
}

// ── Hash Lookup ─────────────────────────────────────────────────────────────

/// Look up releases by a hash, resolving media → release.
fn lookup_by_hash<F>(
    conn: &retro_junk_db::Connection,
    hash_type: &str,
    hash: &str,
    find_fn: F,
    platform_filter: &str,
    platform_label: &dyn Fn(&str) -> String,
    company_label: &dyn Fn(&str) -> String,
) where
    F: FnOnce(&str) -> Result<Vec<retro_junk_catalog::types::Media>, retro_junk_db::OperationError>,
{
    let media_list = match find_fn(hash) {
        Ok(m) => m,
        Err(e) => {
            log::error!("Hash lookup failed: {e}");
            return;
        }
    };

    if media_list.is_empty() {
        log::info!("No media found for {hash_type} {hash}.");
        return;
    }

    // Resolve parent releases
    let mut seen = HashSet::new();
    for media in &media_list {
        if !seen.insert(media.release_id.clone()) {
            continue;
        }
        let release = match retro_junk_db::get_release_by_id(conn, &media.release_id) {
            Ok(Some(r)) => r,
            Ok(None) => {
                log::warn!(
                    "Media {} references unknown release {}",
                    media.id,
                    media.release_id
                );
                continue;
            }
            Err(e) => {
                log::error!("Failed to fetch release: {e}");
                continue;
            }
        };

        if !platform_filter.is_empty() && release.platform_id != platform_filter {
            continue;
        }

        print_release_detail(conn, &release, platform_label, company_label);
    }
}

// ── Serial Lookup ───────────────────────────────────────────────────────────

fn lookup_by_serial(
    conn: &retro_junk_db::Connection,
    serial: &str,
    platform_filter: &str,
    platform_label: &dyn Fn(&str) -> String,
    company_label: &dyn Fn(&str) -> String,
) {
    let mut release_ids: HashSet<String> = HashSet::new();
    let mut releases: Vec<retro_junk_catalog::types::Release> = Vec::new();

    // Search release serials
    if let Ok(found) = retro_junk_db::find_release_by_serial(conn, serial) {
        for r in found {
            if release_ids.insert(r.id.clone()) {
                releases.push(r);
            }
        }
    }

    // Search media serials → resolve parent release
    if let Ok(media_hits) = retro_junk_db::find_media_by_serial(conn, serial) {
        for m in &media_hits {
            if !release_ids.contains(&m.release_id)
                && let Ok(Some(r)) = retro_junk_db::get_release_by_id(conn, &m.release_id)
            {
                release_ids.insert(r.id.clone());
                releases.push(r);
            }
        }
    }

    // Apply platform filter
    if !platform_filter.is_empty() {
        releases.retain(|r| r.platform_id == platform_filter);
    }

    if releases.is_empty() {
        log::info!("No releases found for serial \"{serial}\".");
        return;
    }

    if releases.len() == 1 {
        print_release_detail(conn, &releases[0], platform_label, company_label);
    } else {
        log::info!(
            "{}",
            format!(
                "Found {} releases for serial \"{}\":",
                releases.len(),
                serial
            )
            .if_supports_color(Stdout, |t| t.bold()),
        );
        crate::log_blank();
        for r in &releases {
            let plat = platform_label(&r.platform_id);
            let date_str = &r.release_date;
            let rid = r.id.clone();
            log::info!(
                "  {:<40} {:<10} {:<7} {:<12} {}",
                truncate_str(&r.title, 40),
                plat,
                &r.region,
                date_str,
                rid.if_supports_color(Stdout, |t| t.dimmed()),
            );
        }
    }
}

// ── ID Lookup ───────────────────────────────────────────────────────────────

fn dispatch_id_lookup(conn: &retro_junk_db::Connection, q: &str) {
    let platform_label = make_platform_label(conn);
    let company_label = make_company_label(conn);

    if let Some(id) = q.strip_prefix(PREFIX_PLATFORM) {
        match retro_junk_db::get_platform_by_id(conn, id) {
            Ok(Some(p)) => print_platform_detail(conn, &p),
            Ok(None) => log::info!("No platform found with ID \"{id}\"."),
            Err(e) => log::error!("Lookup failed: {e}"),
        }
    } else if q.starts_with(retro_junk_catalog::content_id::WORK_PREFIX) {
        match retro_junk_db::get_work_by_id(conn, q) {
            Ok(Some(w)) => print_work_detail(conn, &w, &platform_label),
            Ok(None) => log::info!("No work found with ID \"{q}\"."),
            Err(e) => log::error!("Lookup failed: {e}"),
        }
    } else if q.starts_with(retro_junk_catalog::content_id::RELEASE_PREFIX) {
        match retro_junk_db::get_release_by_id(conn, q) {
            Ok(Some(r)) => print_release_detail(conn, &r, &platform_label, &company_label),
            Ok(None) => log::info!("No release found with ID \"{q}\"."),
            Err(e) => log::error!("Lookup failed: {e}"),
        }
    } else if q.starts_with(retro_junk_catalog::content_id::MEDIA_PREFIX) {
        match retro_junk_db::get_media_by_id(conn, q) {
            Ok(Some(m)) => print_media_detail(conn, &m, &platform_label),
            Ok(None) => log::info!("No media found with ID \"{q}\"."),
            Err(e) => log::error!("Lookup failed: {e}"),
        }
    }
}

// ── Search ──────────────────────────────────────────────────────────────────

// Flat dispatch over entity types, each with its own result formatting.
#[allow(clippy::too_many_lines)]
fn dispatch_search(
    conn: &retro_junk_db::Connection,
    query: &str,
    entity_type: Option<CatalogEntityType>,
    platform: &str,
    limit: u32,
    offset: u32,
) {
    let platform_label = make_platform_label(conn);
    // DB search functions treat a None platform as "no filter".
    let platform = (!platform.is_empty()).then_some(platform);

    match entity_type {
        Some(CatalogEntityType::Works) => {
            let results = match retro_junk_db::search_works(conn, query, limit, offset) {
                Ok(r) => r,
                Err(e) => {
                    log::error!("Search failed: {e}");
                    return;
                }
            };
            if results.is_empty() {
                log::info!("No works found matching \"{query}\".");
                return;
            }
            print_works_table(&results, offset);
        }
        Some(CatalogEntityType::Releases) => {
            let results =
                match retro_junk_db::search_releases_paged(conn, query, platform, limit, offset) {
                    Ok(r) => r,
                    Err(e) => {
                        log::error!("Search failed: {e}");
                        return;
                    }
                };
            if results.is_empty() {
                log::info!("No releases found matching \"{query}\".");
                return;
            }
            print_releases_table(&results, &platform_label, offset, limit);
        }
        Some(CatalogEntityType::Media) => {
            let results = match retro_junk_db::search_media(conn, query, platform, limit, offset) {
                Ok(r) => r,
                Err(e) => {
                    log::error!("Search failed: {e}");
                    return;
                }
            };
            if results.is_empty() {
                log::info!("No media found matching \"{query}\".");
                return;
            }
            print_media_table(conn, &results, &platform_label, offset, limit);
        }
        // Unified search across all types
        None | Some(CatalogEntityType::Platforms) => {
            let works = retro_junk_db::search_works(conn, query, limit, 0).unwrap_or_default();
            let releases = retro_junk_db::search_releases_paged(conn, query, platform, limit, 0)
                .unwrap_or_default();
            let media =
                retro_junk_db::search_media(conn, query, platform, limit, 0).unwrap_or_default();

            if works.is_empty() && releases.is_empty() && media.is_empty() {
                log::info!("No results found matching \"{query}\".");
                return;
            }

            if !works.is_empty() {
                log::info!(
                    "{}",
                    format!("Works ({}):", works.len()).if_supports_color(Stdout, |t| t.bold()),
                );
                for w in &works {
                    let wid = w.id.clone();
                    log::info!(
                        "  {:<50} {}",
                        w.canonical_name,
                        wid.if_supports_color(Stdout, |t| t.dimmed()),
                    );
                }
                crate::log_blank();
            }

            if !releases.is_empty() {
                log::info!(
                    "{}",
                    format!("Releases ({}):", releases.len())
                        .if_supports_color(Stdout, |t| t.bold()),
                );
                for r in &releases {
                    let plat = platform_label(&r.platform_id);
                    let date_str = &r.release_date;
                    let rid = r.id.clone();
                    log::info!(
                        "  {:<35} {:<8} {:<7} {:<12} {}",
                        truncate_str(&r.title, 35),
                        plat,
                        &r.region,
                        date_str,
                        rid.if_supports_color(Stdout, |t| t.dimmed()),
                    );
                }
                crate::log_blank();
            }

            if !media.is_empty() {
                log::info!(
                    "{}",
                    format!("Media ({}):", media.len()).if_supports_color(Stdout, |t| t.bold()),
                );
                for m in &media {
                    let name = or_str(&m.dat_name, &m.id);
                    let size_str = format_file_size_or(m.file_size, "");
                    let plat = resolve_media_platform(conn, &m.release_id, &platform_label);
                    let mid = m.id.clone();
                    log::info!(
                        "  {:<35} {:<8} {:>8}  {}",
                        truncate_str(name, 35),
                        plat,
                        size_str,
                        mid.if_supports_color(Stdout, |t| t.dimmed()),
                    );
                }
                crate::log_blank();
            }

            log::info!(
                "Use --type to search a single type with pagination, or pass an id for details."
            );
        }
    }
}

// ── Listing (no query) ──────────────────────────────────────────────────────

fn dispatch_listing(
    conn: &retro_junk_db::Connection,
    entity_type: Option<CatalogEntityType>,
    platform: &str,
    manufacturer: &str,
    limit: u32,
    offset: u32,
    group: bool,
) {
    match entity_type {
        None | Some(CatalogEntityType::Platforms) => {
            list_platforms(conn, manufacturer, group);
        }
        Some(CatalogEntityType::Works) => {
            log::info!(
                "Listing works requires a search query. Try: catalog lookup <query> --type works"
            );
        }
        Some(CatalogEntityType::Releases) => {
            if platform.is_empty() {
                log::info!(
                    "Listing releases requires --platform. Try: catalog lookup --type releases --platform nes"
                );
            } else {
                list_releases_for_platform(conn, platform, limit, offset);
            }
        }
        Some(CatalogEntityType::Media) => {
            if platform.is_empty() {
                log::info!(
                    "Listing media requires --platform. Try: catalog lookup --type media --platform nes"
                );
            } else {
                list_media_for_platform(conn, platform, limit, offset);
            }
        }
    }
}

// ── Platform listing ────────────────────────────────────────────────────────

fn list_platforms(conn: &retro_junk_db::Connection, manufacturer_filter: &str, group: bool) {
    let platforms = match retro_junk_db::list_platforms(conn) {
        Ok(p) => p,
        Err(e) => {
            log::error!("Failed to list platforms: {e}");
            return;
        }
    };

    let release_counts: HashMap<String, i64> = retro_junk_db::platform_release_counts(conn)
        .unwrap_or_default()
        .into_iter()
        .collect();
    let media_counts: HashMap<String, i64> = retro_junk_db::platform_media_counts(conn)
        .unwrap_or_default()
        .into_iter()
        .collect();

    let filtered: Vec<_> = platforms
        .iter()
        .filter(|p| {
            manufacturer_filter.is_empty()
                || p.manufacturer
                    .to_lowercase()
                    .contains(&manufacturer_filter.to_lowercase())
        })
        .collect();

    if filtered.is_empty() {
        if manufacturer_filter.is_empty() {
            log::info!("No platforms in the catalog.");
        } else {
            log::info!("No platforms found for manufacturer \"{manufacturer_filter}\".");
        }
        return;
    }

    if group {
        // Group by manufacturer
        let mut by_mfr: Vec<(String, Vec<&retro_junk_db::PlatformRow>)> = Vec::new();
        let mut current_mfr = String::new();
        for p in &filtered {
            if p.manufacturer != current_mfr {
                current_mfr.clone_from(&p.manufacturer);
                by_mfr.push((current_mfr.clone(), Vec::new()));
            }
            // SAFETY: just pushed an entry above when manufacturer changes, or on first iteration
            by_mfr.last_mut().unwrap().1.push(p);
        }

        for (mfr, group_platforms) in &by_mfr {
            log::info!("{}", mfr.if_supports_color(Stdout, |t| t.bold()));
            print_platform_table_rows(group_platforms, &release_counts, &media_counts);
            crate::log_blank();
        }
    } else {
        log::info!(
            "  {:<14} {:<40} {:<12} {:>5}  {:<5} {:>9} {:>9}",
            "ID".if_supports_color(Stdout, |t| t.dimmed()),
            "Name".if_supports_color(Stdout, |t| t.dimmed()),
            "Mfr".if_supports_color(Stdout, |t| t.dimmed()),
            "Year".if_supports_color(Stdout, |t| t.dimmed()),
            "Type".if_supports_color(Stdout, |t| t.dimmed()),
            "Releases".if_supports_color(Stdout, |t| t.dimmed()),
            "Media".if_supports_color(Stdout, |t| t.dimmed()),
        );
        print_platform_table_rows(&filtered, &release_counts, &media_counts);
    }

    crate::log_blank();
    log::info!("{} platforms.", filtered.len());
}

fn print_platform_table_rows(
    platforms: &[&retro_junk_db::PlatformRow],
    release_counts: &HashMap<String, i64>,
    media_counts: &HashMap<String, i64>,
) {
    for p in platforms {
        let year_str = if p.release_year == 0 {
            String::new()
        } else {
            p.release_year.to_string()
        };
        let rel_count = release_counts.get(&p.id).copied().unwrap_or(0);
        let med_count = media_counts.get(&p.id).copied().unwrap_or(0);
        log::info!(
            "  {:<14} {:<40} {:<12} {:>5}  {:<5} {:>9} {:>9}",
            format!("{}{}", PREFIX_PLATFORM, &p.id).if_supports_color(Stdout, |t| t.dimmed()),
            truncate_str(&p.display_name, 40),
            truncate_str(&p.manufacturer, 12),
            year_str,
            &p.media_type,
            format_count(rel_count),
            format_count(med_count),
        );
    }
}

// ── Release / media listing for a platform ──────────────────────────────────

fn list_releases_for_platform(
    conn: &retro_junk_db::Connection,
    platform_id: &str,
    limit: u32,
    offset: u32,
) {
    let platform_label = make_platform_label(conn);

    // Use a search with empty-ish pattern to list all
    let results =
        match retro_junk_db::search_releases_paged(conn, "%", Some(platform_id), limit, offset) {
            Ok(r) => r,
            Err(e) => {
                log::error!("Query failed: {e}");
                return;
            }
        };

    if results.is_empty() {
        log::info!("No releases found for platform \"{platform_id}\".");
        return;
    }

    let plat_name = platform_label(platform_id);
    log::info!(
        "{}",
        format!("Releases for {plat_name} (offset {offset}):")
            .if_supports_color(Stdout, |t| t.bold()),
    );
    crate::log_blank();
    print_releases_table(&results, &platform_label, offset, limit);
}

fn list_media_for_platform(
    conn: &retro_junk_db::Connection,
    platform_id: &str,
    limit: u32,
    offset: u32,
) {
    let platform_label = make_platform_label(conn);

    let results = match retro_junk_db::search_media(conn, "%", Some(platform_id), limit, offset) {
        Ok(r) => r,
        Err(e) => {
            log::error!("Query failed: {e}");
            return;
        }
    };

    if results.is_empty() {
        log::info!("No media found for platform \"{platform_id}\".");
        return;
    }

    let plat_name = platform_label(platform_id);
    log::info!(
        "{}",
        format!("Media for {plat_name} (offset {offset}):").if_supports_color(Stdout, |t| t.bold()),
    );
    crate::log_blank();
    print_media_table(conn, &results, &platform_label, offset, limit);
}

// ── Detail printers ─────────────────────────────────────────────────────────

fn print_platform_detail(conn: &retro_junk_db::Connection, p: &retro_junk_db::PlatformRow) {
    let dash = "--";
    let year_str = if p.release_year == 0 {
        dash.to_string()
    } else {
        p.release_year.to_string()
    };
    let gen_str = if p.generation == 0 {
        dash.to_string()
    } else {
        p.generation.to_string()
    };

    let release_counts: HashMap<String, i64> = retro_junk_db::platform_release_counts(conn)
        .unwrap_or_default()
        .into_iter()
        .collect();
    let media_counts: HashMap<String, i64> = retro_junk_db::platform_media_counts(conn)
        .unwrap_or_default()
        .into_iter()
        .collect();

    let rel_count = release_counts.get(&p.id).copied().unwrap_or(0);
    let med_count = media_counts.get(&p.id).copied().unwrap_or(0);

    log::info!("{}", p.display_name.if_supports_color(Stdout, |t| t.bold()));
    log::info!("  ID:           {}{}", PREFIX_PLATFORM, &p.id);
    log::info!("  Short name:   {}", &p.short_name);
    log::info!("  Manufacturer: {}", &p.manufacturer);
    log::info!("  Generation:   {gen_str}");
    log::info!("  Media type:   {}", &p.media_type);
    log::info!("  Release year: {year_str}");
    log::info!("  Releases:     {}", format_count(rel_count));
    log::info!("  Media:        {}", format_count(med_count));
    crate::log_blank();
}

fn print_work_detail(
    conn: &retro_junk_db::Connection,
    w: &retro_junk_db::WorkRow,
    platform_label: &dyn Fn(&str) -> String,
) {
    log::info!(
        "{}",
        w.canonical_name.if_supports_color(Stdout, |t| t.bold()),
    );
    log::info!("  ID: {}", &w.id);

    let releases = retro_junk_db::releases_for_work(conn, &w.id).unwrap_or_default();
    if releases.is_empty() {
        log::info!("  Releases: 0");
    } else {
        log::info!("  Releases: {}", releases.len());
        for r in &releases {
            let plat = platform_label(&r.platform_id);
            let date_str = &r.release_date;
            let rid = r.id.clone();
            log::info!(
                "    {:<35} {:<8} {:<7} {:<12} {}",
                truncate_str(&r.title, 35),
                plat,
                &r.region,
                date_str,
                rid.if_supports_color(Stdout, |t| t.dimmed()),
            );
        }
    }
    crate::log_blank();
}

/// Print a detailed view of a single release.
// Linear report of every release field/section in display order.
#[allow(clippy::too_many_lines)]
fn print_release_detail(
    conn: &retro_junk_db::Connection,
    release: &retro_junk_catalog::types::Release,
    platform_label: &dyn Fn(&str) -> String,
    company_label: &dyn Fn(&str) -> String,
) {
    let plat = platform_label(&release.platform_id);
    let dash = "--";

    log::info!(
        "{}",
        format!("{} ({}, {})", release.title, plat, release.region)
            .if_supports_color(Stdout, |t| t.bold()),
    );

    let serial_str = or_str(&release.game_serial, dash);
    let publisher = release
        .publisher_id
        .as_deref()
        .map_or_else(|| dash.to_string(), company_label);
    let developer = release
        .developer_id
        .as_deref()
        .map_or_else(|| dash.to_string(), company_label);
    let date_str = or_str(&release.release_date, dash);
    let genre_str = or_str(&release.genre, dash);
    let players_str = or_str(&release.players, dash);
    let rating_str = release
        .rating
        .map_or_else(|| dash.to_string(), |r| format!("{r:.1}"));

    log::info!("  ID:           {}", &release.id);
    if !release.alt_title.is_empty() {
        log::info!("  Alt title:    {}", release.alt_title);
    }
    if !release.screen_title.is_empty() {
        log::info!("  Screen title: {}", release.screen_title);
    }
    if !release.cover_title.is_empty() {
        log::info!("  Cover title:  {}", release.cover_title);
    }
    log::info!("  Serial:       {serial_str}");
    log::info!("  Publisher:    {publisher}");
    log::info!("  Developer:    {developer}");
    log::info!("  Release date: {date_str}");
    log::info!("  Genre:        {genre_str}");
    log::info!("  Players:      {players_str}");
    log::info!("  Rating:       {rating_str}");

    if !release.description.is_empty() {
        let desc = &release.description;
        let short = if desc.len() > 200 {
            format!("{}...", &desc[..200])
        } else {
            desc.clone()
        };
        log::info!("  Description:  {short}");
    }

    // Media entries
    match retro_junk_db::media_for_release(conn, &release.id) {
        Ok(media) if !media.is_empty() => {
            crate::log_blank();
            log::info!("  {}", "Media:".if_supports_color(Stdout, |t| t.bold()));
            for (i, m) in media.iter().enumerate() {
                let name = or_str(&m.dat_name, &m.id);
                log::info!("    {}. {}", i + 1, name);
                let crc = or_str(&m.crc32, dash);
                let sha1_val = or_str(&m.sha1, dash);
                let sha1_short = if sha1_val.len() > 12 {
                    &sha1_val[..12]
                } else {
                    sha1_val
                };
                let size_str = format_file_size_or(m.file_size, dash);
                log::info!("       CRC32: {crc}  SHA1: {sha1_short}...  Size: {size_str}");

                let status = format!("{:?}", m.status).to_lowercase();
                let source = or_str(&m.dat_source, dash);
                log::info!("       Status: {status}  Source: {source}");

                // Check collection status
                if let Ok(Some(entry)) =
                    retro_junk_db::find_collection_entry(conn, &m.id, "default")
                {
                    let verified = if entry.verified_at.is_empty() {
                        String::new()
                    } else {
                        format!("(verified {})", entry.verified_at)
                    };
                    let status = if entry.owned { "owned" } else { "not owned" };
                    log::info!(
                        "       Collection: {} {}",
                        status,
                        verified.if_supports_color(Stdout, |t| t.dimmed()),
                    );
                }
            }
        }
        _ => {}
    }

    // Asset summary
    match retro_junk_db::assets_for_release(conn, &release.id) {
        Ok(assets) if !assets.is_empty() => {
            let types: Vec<&str> = assets.iter().map(|a| a.asset_type.as_str()).collect();
            let unique: HashSet<&&str> = types.iter().collect();
            let type_list: Vec<&&str> = {
                let mut v: Vec<_> = unique.into_iter().collect();
                v.sort();
                v
            };
            crate::log_blank();
            log::info!(
                "  Assets: {} ({})",
                assets.len(),
                type_list.iter().map(|t| **t).collect::<Vec<_>>().join(", "),
            );
        }
        _ => {}
    }

    crate::log_blank();
}

fn print_media_detail(
    conn: &retro_junk_db::Connection,
    m: &retro_junk_catalog::types::Media,
    platform_label: &dyn Fn(&str) -> String,
) {
    let dash = "--";
    let name = or_str(&m.dat_name, &m.id);

    log::info!("{}", name.if_supports_color(Stdout, |t| t.bold()));
    log::info!("  ID:        {}", &m.id);

    // Resolve parent release for platform info
    if let Ok(Some(release)) = retro_junk_db::get_release_by_id(conn, &m.release_id) {
        let plat = platform_label(&release.platform_id);
        log::info!("  Release:   {}", &m.release_id);
        log::info!("  Title:     {}", &release.title);
        log::info!("  Platform:  {plat}");
        log::info!("  Region:    {}", &release.region);
    } else {
        log::info!("  Release:   {}", &m.release_id);
    }

    let size_str = format_file_size_or(m.file_size, dash);
    let crc = or_str(&m.crc32, dash);
    let sha1_val = or_str(&m.sha1, dash);
    let md5_val = or_str(&m.md5, dash);
    let status = format!("{:?}", m.status).to_lowercase();
    let source = or_str(&m.dat_source, dash);

    log::info!("  Size:      {size_str}");
    log::info!("  CRC32:     {crc}");
    log::info!("  SHA1:      {sha1_val}");
    log::info!("  MD5:       {md5_val}");
    log::info!("  Status:    {status}");
    log::info!("  Source:    {source}");

    // Check collection status
    if let Ok(Some(entry)) = retro_junk_db::find_collection_entry(conn, &m.id, "default") {
        let verified = if entry.verified_at.is_empty() {
            String::new()
        } else {
            format!("(verified {})", entry.verified_at)
        };
        let coll_status = if entry.owned { "owned" } else { "not owned" };
        log::info!(
            "  Collection: {} {}",
            coll_status,
            verified.if_supports_color(Stdout, |t| t.dimmed()),
        );
    }

    crate::log_blank();
}

// ── Table printers ──────────────────────────────────────────────────────────

fn print_works_table(works: &[retro_junk_db::WorkRow], offset: u32) {
    for w in works {
        let wid = w.id.clone();
        log::info!(
            "  {:<50} {}",
            w.canonical_name,
            wid.if_supports_color(Stdout, |t| t.dimmed()),
        );
    }
    crate::log_blank();
    log::info!("{} works shown (offset {}).", works.len(), offset);
}

fn print_releases_table(
    releases: &[retro_junk_catalog::types::Release],
    platform_label: &dyn Fn(&str) -> String,
    offset: u32,
    limit: u32,
) {
    for r in releases {
        let plat = platform_label(&r.platform_id);
        let date_str = &r.release_date;
        let serial_str = r.game_serial.as_str();
        let rid = r.id.clone();
        log::info!(
            "  {:<35} {:<8} {:<7} {:<12} {:<14} {}",
            truncate_str(&r.title, 35),
            plat,
            &r.region,
            date_str,
            serial_str.if_supports_color(Stdout, |t| t.dimmed()),
            rid.if_supports_color(Stdout, |t| t.dimmed()),
        );
    }
    crate::log_blank();
    if releases.len() as u32 == limit {
        log::info!(
            "Showing {} results (offset {}). Use --offset {} to see more.",
            releases.len(),
            offset,
            offset + limit,
        );
    } else {
        log::info!("{} results shown (offset {}).", releases.len(), offset);
    }
}

fn print_media_table(
    conn: &retro_junk_db::Connection,
    media: &[retro_junk_catalog::types::Media],
    platform_label: &dyn Fn(&str) -> String,
    offset: u32,
    limit: u32,
) {
    for m in media {
        let name = or_str(&m.dat_name, &m.id);
        let size_str = format_file_size_or(m.file_size, "");
        let plat = resolve_media_platform(conn, &m.release_id, platform_label);
        let mid = m.id.clone();
        log::info!(
            "  {:<35} {:<8} {:>8}  {}",
            truncate_str(name, 35),
            plat,
            size_str,
            mid.if_supports_color(Stdout, |t| t.dimmed()),
        );
    }
    crate::log_blank();
    if media.len() as u32 == limit {
        log::info!(
            "Showing {} results (offset {}). Use --offset {} to see more.",
            media.len(),
            offset,
            offset + limit,
        );
    } else {
        log::info!("{} results shown (offset {}).", media.len(), offset);
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn make_platform_label(conn: &retro_junk_db::Connection) -> impl Fn(&str) -> String + '_ {
    move |pid: &str| -> String {
        retro_junk_db::get_platform_display_name(conn, pid)
            .ok()
            .flatten()
            .unwrap_or_else(|| pid.to_uppercase())
    }
}

fn make_company_label(conn: &retro_junk_db::Connection) -> impl Fn(&str) -> String + '_ {
    move |cid: &str| -> String {
        retro_junk_db::get_company_name(conn, cid)
            .ok()
            .flatten()
            .unwrap_or_else(|| cid.to_string())
    }
}

fn resolve_media_platform(
    conn: &retro_junk_db::Connection,
    release_id: &str,
    platform_label: &dyn Fn(&str) -> String,
) -> String {
    retro_junk_db::get_release_by_id(conn, release_id)
        .ok()
        .flatten()
        .map(|r| platform_label(&r.platform_id))
        .unwrap_or_default()
}

fn format_count(n: i64) -> String {
    if n == 0 {
        "--".to_string()
    } else if n >= 1_000 {
        format!("{},{:03}", n / 1_000, n % 1_000)
    } else {
        n.to_string()
    }
}
