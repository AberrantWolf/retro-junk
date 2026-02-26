use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;

use super::{default_catalog_db_path, format_file_size, truncate_str};

/// Entity type prefixes for ID-based lookups.
const PREFIX_PLATFORM: &str = "plt-";
const PREFIX_WORK: &str = "wrk-";
const PREFIX_RELEASE: &str = "rel-";
const PREFIX_MEDIA: &str = "med-";

/// Entry point for `catalog lookup`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_catalog_lookup(
    query: Option<String>,
    platform: Option<String>,
    entity_type: Option<String>,
    manufacturer: Option<String>,
    crc: Option<String>,
    sha1: Option<String>,
    md5: Option<String>,
    serial: Option<String>,
    limit: u32,
    offset: u32,
    group: bool,
    db_path: Option<PathBuf>,
) {
    let db_path = db_path.unwrap_or_else(default_catalog_db_path);
    if !db_path.exists() {
        log::warn!("No catalog database found at {}", db_path.display());
        log::info!("Run 'retro-junk catalog import all' first.");
        return;
    }

    let conn = match retro_junk_db::open_database(&db_path) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to open catalog database: {}", e);
            std::process::exit(1);
        }
    };

    // ── Hash / serial lookups (original behavior) ─────────────────────
    let has_hash_or_serial = crc.is_some() || sha1.is_some() || md5.is_some() || serial.is_some();

    if has_hash_or_serial {
        let mode_count = [
            crc.is_some(),
            sha1.is_some(),
            md5.is_some(),
            serial.is_some(),
        ]
        .iter()
        .filter(|&&b| b)
        .count();
        if mode_count > 1 {
            log::error!("Only one of --crc, --sha1, --md5, or --serial at a time.");
            std::process::exit(1);
        }

        let platform_label = make_platform_label(&conn);
        let company_label = make_company_label(&conn);

        if let Some(ref hash) = crc {
            let hash = hash.to_lowercase();
            lookup_by_hash(
                &conn,
                "CRC32",
                &hash,
                |h| retro_junk_db::find_media_by_crc32(&conn, h),
                platform.as_deref(),
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
                platform.as_deref(),
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
                platform.as_deref(),
                &platform_label,
                &company_label,
            );
        } else if let Some(ref s) = serial {
            lookup_by_serial(
                &conn,
                s,
                platform.as_deref(),
                &platform_label,
                &company_label,
            );
        }
        return;
    }

    // ── Browse/search modes ───────────────────────────────────────────
    match query {
        Some(q) if is_prefixed_id(&q) => dispatch_id_lookup(&conn, &q),
        Some(q) => dispatch_search(
            &conn,
            &q,
            entity_type.as_deref(),
            platform.as_deref(),
            limit,
            offset,
        ),
        None => dispatch_listing(
            &conn,
            entity_type.as_deref(),
            platform.as_deref(),
            manufacturer.as_deref(),
            limit,
            offset,
            group,
        ),
    }
}

// ── Routing helpers ─────────────────────────────────────────────────────────

fn is_prefixed_id(q: &str) -> bool {
    q.starts_with(PREFIX_PLATFORM)
        || q.starts_with(PREFIX_WORK)
        || q.starts_with(PREFIX_RELEASE)
        || q.starts_with(PREFIX_MEDIA)
}

// ── Hash Lookup ─────────────────────────────────────────────────────────────

/// Look up releases by a hash, resolving media → release.
fn lookup_by_hash<F>(
    conn: &retro_junk_db::Connection,
    hash_type: &str,
    hash: &str,
    find_fn: F,
    platform_filter: Option<&str>,
    platform_label: &dyn Fn(&str) -> String,
    company_label: &dyn Fn(&str) -> String,
) where
    F: FnOnce(&str) -> Result<Vec<retro_junk_catalog::types::Media>, retro_junk_db::OperationError>,
{
    let media_list = match find_fn(hash) {
        Ok(m) => m,
        Err(e) => {
            log::error!("Hash lookup failed: {}", e);
            std::process::exit(1);
        }
    };

    if media_list.is_empty() {
        log::info!("No media found for {} {}.", hash_type, hash);
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
                log::error!("Failed to fetch release: {}", e);
                continue;
            }
        };

        if let Some(cf) = platform_filter
            && release.platform_id != cf
        {
            continue;
        }

        print_release_detail(conn, &release, platform_label, company_label);
    }
}

// ── Serial Lookup ───────────────────────────────────────────────────────────

fn lookup_by_serial(
    conn: &retro_junk_db::Connection,
    serial: &str,
    platform_filter: Option<&str>,
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
    if let Some(pf) = platform_filter {
        releases.retain(|r| r.platform_id == *pf);
    }

    if releases.is_empty() {
        log::info!("No releases found for serial \"{}\".", serial);
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
        log::info!("");
        for r in &releases {
            let plat = platform_label(&r.platform_id);
            let date_str = r.release_date.as_deref().unwrap_or("");
            let rid = format!("{}{}", PREFIX_RELEASE, &r.id);
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
            Ok(None) => log::info!("No platform found with ID \"{}\".", id),
            Err(e) => log::error!("Lookup failed: {}", e),
        }
    } else if let Some(id) = q.strip_prefix(PREFIX_WORK) {
        match retro_junk_db::get_work_by_id(conn, id) {
            Ok(Some(w)) => print_work_detail(conn, &w, &platform_label),
            Ok(None) => log::info!("No work found with ID \"{}\".", id),
            Err(e) => log::error!("Lookup failed: {}", e),
        }
    } else if let Some(id) = q.strip_prefix(PREFIX_RELEASE) {
        match retro_junk_db::get_release_by_id(conn, id) {
            Ok(Some(r)) => print_release_detail(conn, &r, &platform_label, &company_label),
            Ok(None) => log::info!("No release found with ID \"{}\".", id),
            Err(e) => log::error!("Lookup failed: {}", e),
        }
    } else if let Some(id) = q.strip_prefix(PREFIX_MEDIA) {
        match retro_junk_db::get_media_by_id(conn, id) {
            Ok(Some(m)) => print_media_detail(conn, &m, &platform_label),
            Ok(None) => log::info!("No media found with ID \"{}\".", id),
            Err(e) => log::error!("Lookup failed: {}", e),
        }
    }
}

// ── Search ──────────────────────────────────────────────────────────────────

fn dispatch_search(
    conn: &retro_junk_db::Connection,
    query: &str,
    entity_type: Option<&str>,
    platform: Option<&str>,
    limit: u32,
    offset: u32,
) {
    let platform_label = make_platform_label(conn);

    match entity_type {
        Some("works" | "work") => {
            let results = match retro_junk_db::search_works(conn, query, limit, offset) {
                Ok(r) => r,
                Err(e) => {
                    log::error!("Search failed: {}", e);
                    return;
                }
            };
            if results.is_empty() {
                log::info!("No works found matching \"{}\".", query);
                return;
            }
            print_works_table(&results, offset);
        }
        Some("releases" | "release") => {
            let results =
                match retro_junk_db::search_releases_paged(conn, query, platform, limit, offset) {
                    Ok(r) => r,
                    Err(e) => {
                        log::error!("Search failed: {}", e);
                        return;
                    }
                };
            if results.is_empty() {
                log::info!("No releases found matching \"{}\".", query);
                return;
            }
            print_releases_table(&results, &platform_label, offset, limit);
        }
        Some("media") => {
            let results = match retro_junk_db::search_media(conn, query, platform, limit, offset) {
                Ok(r) => r,
                Err(e) => {
                    log::error!("Search failed: {}", e);
                    return;
                }
            };
            if results.is_empty() {
                log::info!("No media found matching \"{}\".", query);
                return;
            }
            print_media_table(conn, &results, &platform_label, offset, limit);
        }
        Some(other) => {
            log::error!(
                "Unknown type \"{}\". Use: platforms, works, releases, media",
                other
            );
            std::process::exit(1);
        }
        // Unified search across all types
        None => {
            let works = retro_junk_db::search_works(conn, query, limit, 0).unwrap_or_default();
            let releases = retro_junk_db::search_releases_paged(conn, query, platform, limit, 0)
                .unwrap_or_default();
            let media =
                retro_junk_db::search_media(conn, query, platform, limit, 0).unwrap_or_default();

            if works.is_empty() && releases.is_empty() && media.is_empty() {
                log::info!("No results found matching \"{}\".", query);
                return;
            }

            if !works.is_empty() {
                log::info!(
                    "{}",
                    format!("Works ({}):", works.len()).if_supports_color(Stdout, |t| t.bold()),
                );
                for w in &works {
                    let wid = format!("{}{}", PREFIX_WORK, &w.id);
                    log::info!(
                        "  {:<50} {}",
                        w.canonical_name,
                        wid.if_supports_color(Stdout, |t| t.dimmed()),
                    );
                }
                log::info!("");
            }

            if !releases.is_empty() {
                log::info!(
                    "{}",
                    format!("Releases ({}):", releases.len())
                        .if_supports_color(Stdout, |t| t.bold()),
                );
                for r in &releases {
                    let plat = platform_label(&r.platform_id);
                    let date_str = r.release_date.as_deref().unwrap_or("");
                    let rid = format!("{}{}", PREFIX_RELEASE, &r.id);
                    log::info!(
                        "  {:<35} {:<8} {:<7} {:<12} {}",
                        truncate_str(&r.title, 35),
                        plat,
                        &r.region,
                        date_str,
                        rid.if_supports_color(Stdout, |t| t.dimmed()),
                    );
                }
                log::info!("");
            }

            if !media.is_empty() {
                log::info!(
                    "{}",
                    format!("Media ({}):", media.len()).if_supports_color(Stdout, |t| t.bold()),
                );
                for m in &media {
                    let name = m.dat_name.as_deref().unwrap_or(&m.id);
                    let size_str = m.file_size.map(format_file_size).unwrap_or_default();
                    let plat = resolve_media_platform(conn, &m.release_id, &platform_label);
                    let mid = format!("{}{}", PREFIX_MEDIA, &m.id);
                    log::info!(
                        "  {:<35} {:<8} {:>8}  {}",
                        truncate_str(name, 35),
                        plat,
                        size_str,
                        mid.if_supports_color(Stdout, |t| t.dimmed()),
                    );
                }
                log::info!("");
            }

            log::info!(
                "Use --type to search a single type with pagination, or pass a prefixed ID for details."
            );
        }
    }
}

// ── Listing (no query) ──────────────────────────────────────────────────────

fn dispatch_listing(
    conn: &retro_junk_db::Connection,
    entity_type: Option<&str>,
    platform: Option<&str>,
    manufacturer: Option<&str>,
    limit: u32,
    offset: u32,
    group: bool,
) {
    match entity_type {
        None | Some("platforms" | "platform") => {
            list_platforms(conn, manufacturer, group);
        }
        Some("works" | "work") => {
            log::info!(
                "Listing works requires a search query. Try: catalog lookup <query> --type works"
            );
        }
        Some("releases" | "release") => {
            if let Some(pid) = platform {
                list_releases_for_platform(conn, pid, limit, offset);
            } else {
                log::info!(
                    "Listing releases requires --platform. Try: catalog lookup --type releases --platform nes"
                );
            }
        }
        Some("media") => {
            if let Some(pid) = platform {
                list_media_for_platform(conn, pid, limit, offset);
            } else {
                log::info!(
                    "Listing media requires --platform. Try: catalog lookup --type media --platform nes"
                );
            }
        }
        Some(other) => {
            log::error!(
                "Unknown type \"{}\". Use: platforms, works, releases, media",
                other
            );
            std::process::exit(1);
        }
    }
}

// ── Platform listing ────────────────────────────────────────────────────────

fn list_platforms(
    conn: &retro_junk_db::Connection,
    manufacturer_filter: Option<&str>,
    group: bool,
) {
    let platforms = match retro_junk_db::list_platforms(conn) {
        Ok(p) => p,
        Err(e) => {
            log::error!("Failed to list platforms: {}", e);
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
            manufacturer_filter
                .map(|mf| p.manufacturer.to_lowercase().contains(&mf.to_lowercase()))
                .unwrap_or(true)
        })
        .collect();

    if filtered.is_empty() {
        if let Some(mf) = manufacturer_filter {
            log::info!("No platforms found for manufacturer \"{}\".", mf);
        } else {
            log::info!("No platforms in the catalog.");
        }
        return;
    }

    if group {
        // Group by manufacturer
        let mut by_mfr: Vec<(String, Vec<&retro_junk_db::PlatformRow>)> = Vec::new();
        let mut current_mfr = String::new();
        for p in &filtered {
            if p.manufacturer != current_mfr {
                current_mfr = p.manufacturer.clone();
                by_mfr.push((current_mfr.clone(), Vec::new()));
            }
            by_mfr.last_mut().unwrap().1.push(p);
        }

        for (mfr, group_platforms) in &by_mfr {
            log::info!("{}", mfr.if_supports_color(Stdout, |t| t.bold()),);
            print_platform_table_rows(group_platforms, &release_counts, &media_counts);
            log::info!("");
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

    log::info!("");
    log::info!("{} platforms.", filtered.len());
}

fn print_platform_table_rows(
    platforms: &[&retro_junk_db::PlatformRow],
    release_counts: &HashMap<String, i64>,
    media_counts: &HashMap<String, i64>,
) {
    for p in platforms {
        let year_str = p.release_year.map(|y| y.to_string()).unwrap_or_default();
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
                log::error!("Query failed: {}", e);
                return;
            }
        };

    if results.is_empty() {
        log::info!("No releases found for platform \"{}\".", platform_id);
        return;
    }

    let plat_name = platform_label(platform_id);
    log::info!(
        "{}",
        format!("Releases for {} (offset {}):", plat_name, offset)
            .if_supports_color(Stdout, |t| t.bold()),
    );
    log::info!("");
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
            log::error!("Query failed: {}", e);
            return;
        }
    };

    if results.is_empty() {
        log::info!("No media found for platform \"{}\".", platform_id);
        return;
    }

    let plat_name = platform_label(platform_id);
    log::info!(
        "{}",
        format!("Media for {} (offset {}):", plat_name, offset)
            .if_supports_color(Stdout, |t| t.bold()),
    );
    log::info!("");
    print_media_table(conn, &results, &platform_label, offset, limit);
}

// ── Detail printers ─────────────────────────────────────────────────────────

fn print_platform_detail(conn: &retro_junk_db::Connection, p: &retro_junk_db::PlatformRow) {
    let dash = "--";
    let year_str = p
        .release_year
        .map(|y| y.to_string())
        .unwrap_or_else(|| dash.to_string());
    let gen_str = p
        .generation
        .map(|g| g.to_string())
        .unwrap_or_else(|| dash.to_string());

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

    log::info!("{}", p.display_name.if_supports_color(Stdout, |t| t.bold()),);
    log::info!("  ID:           {}{}", PREFIX_PLATFORM, &p.id);
    log::info!("  Short name:   {}", &p.short_name);
    log::info!("  Manufacturer: {}", &p.manufacturer);
    log::info!("  Generation:   {}", gen_str);
    log::info!("  Media type:   {}", &p.media_type);
    log::info!("  Release year: {}", year_str);
    log::info!("  Releases:     {}", format_count(rel_count));
    log::info!("  Media:        {}", format_count(med_count));
    log::info!("");
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
    log::info!("  ID: {}{}", PREFIX_WORK, &w.id);

    let releases = retro_junk_db::releases_for_work(conn, &w.id).unwrap_or_default();
    if releases.is_empty() {
        log::info!("  Releases: 0");
    } else {
        log::info!("  Releases: {}", releases.len());
        for r in &releases {
            let plat = platform_label(&r.platform_id);
            let date_str = r.release_date.as_deref().unwrap_or("");
            let rid = format!("{}{}", PREFIX_RELEASE, &r.id);
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
    log::info!("");
}

/// Print a detailed view of a single release.
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

    let serial_str = release.game_serial.as_deref().unwrap_or(dash);
    let publisher = release
        .publisher_id
        .as_deref()
        .map(company_label)
        .unwrap_or_else(|| dash.to_string());
    let developer = release
        .developer_id
        .as_deref()
        .map(company_label)
        .unwrap_or_else(|| dash.to_string());
    let date_str = release.release_date.as_deref().unwrap_or(dash);
    let genre_str = release.genre.as_deref().unwrap_or(dash);
    let players_str = release.players.as_deref().unwrap_or(dash);
    let rating_str = release
        .rating
        .map(|r| format!("{:.1}", r))
        .unwrap_or_else(|| dash.to_string());

    log::info!("  ID:           {}{}", PREFIX_RELEASE, &release.id);
    if let Some(ref alt) = release.alt_title {
        log::info!("  Alt title:    {}", alt);
    }
    if let Some(ref st) = release.screen_title {
        log::info!("  Screen title: {}", st);
    }
    if let Some(ref ct) = release.cover_title {
        log::info!("  Cover title:  {}", ct);
    }
    log::info!("  Serial:       {}", serial_str);
    log::info!("  Publisher:    {}", publisher);
    log::info!("  Developer:    {}", developer);
    log::info!("  Release date: {}", date_str);
    log::info!("  Genre:        {}", genre_str);
    log::info!("  Players:      {}", players_str);
    log::info!("  Rating:       {}", rating_str);

    if let Some(ref desc) = release.description {
        let short = if desc.len() > 200 {
            format!("{}...", &desc[..200])
        } else {
            desc.clone()
        };
        log::info!("  Description:  {}", short);
    }

    // Media entries
    match retro_junk_db::media_for_release(conn, &release.id) {
        Ok(media) if !media.is_empty() => {
            log::info!("");
            log::info!("  {}", "Media:".if_supports_color(Stdout, |t| t.bold()),);
            for (i, m) in media.iter().enumerate() {
                let name = m.dat_name.as_deref().unwrap_or(&m.id);
                log::info!("    {}. {}", i + 1, name,);
                let crc = m.crc32.as_deref().unwrap_or(dash);
                let sha1_val = m.sha1.as_deref().unwrap_or(dash);
                let sha1_short = if sha1_val.len() > 12 {
                    &sha1_val[..12]
                } else {
                    sha1_val
                };
                let size_str = m
                    .file_size
                    .map(format_file_size)
                    .unwrap_or_else(|| dash.to_string());
                log::info!(
                    "       CRC32: {}  SHA1: {}...  Size: {}",
                    crc,
                    sha1_short,
                    size_str,
                );

                let status = format!("{:?}", m.status).to_lowercase();
                let source = m.dat_source.as_deref().unwrap_or(dash);
                log::info!("       Status: {}  Source: {}", status, source,);

                // Check collection status
                if let Ok(Some(entry)) =
                    retro_junk_db::find_collection_entry(conn, &m.id, "default")
                {
                    let verified = entry
                        .verified_at
                        .as_deref()
                        .map(|v| format!("(verified {})", v))
                        .unwrap_or_default();
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
            log::info!("");
            log::info!(
                "  Assets: {} ({})",
                assets.len(),
                type_list.iter().map(|t| **t).collect::<Vec<_>>().join(", "),
            );
        }
        _ => {}
    }

    log::info!("");
}

fn print_media_detail(
    conn: &retro_junk_db::Connection,
    m: &retro_junk_catalog::types::Media,
    platform_label: &dyn Fn(&str) -> String,
) {
    let dash = "--";
    let name = m.dat_name.as_deref().unwrap_or(&m.id);

    log::info!("{}", name.if_supports_color(Stdout, |t| t.bold()),);
    log::info!("  ID:        {}{}", PREFIX_MEDIA, &m.id);

    // Resolve parent release for platform info
    if let Ok(Some(release)) = retro_junk_db::get_release_by_id(conn, &m.release_id) {
        let plat = platform_label(&release.platform_id);
        log::info!("  Release:   {}{}", PREFIX_RELEASE, &m.release_id);
        log::info!("  Title:     {}", &release.title);
        log::info!("  Platform:  {}", plat);
        log::info!("  Region:    {}", &release.region);
    } else {
        log::info!("  Release:   {}{}", PREFIX_RELEASE, &m.release_id);
    }

    let size_str = m
        .file_size
        .map(format_file_size)
        .unwrap_or_else(|| dash.to_string());
    let crc = m.crc32.as_deref().unwrap_or(dash);
    let sha1_val = m.sha1.as_deref().unwrap_or(dash);
    let md5_val = m.md5.as_deref().unwrap_or(dash);
    let status = format!("{:?}", m.status).to_lowercase();
    let source = m.dat_source.as_deref().unwrap_or(dash);

    log::info!("  Size:      {}", size_str);
    log::info!("  CRC32:     {}", crc);
    log::info!("  SHA1:      {}", sha1_val);
    log::info!("  MD5:       {}", md5_val);
    log::info!("  Status:    {}", status);
    log::info!("  Source:    {}", source);

    // Check collection status
    if let Ok(Some(entry)) = retro_junk_db::find_collection_entry(conn, &m.id, "default") {
        let verified = entry
            .verified_at
            .as_deref()
            .map(|v| format!("(verified {})", v))
            .unwrap_or_default();
        let coll_status = if entry.owned { "owned" } else { "not owned" };
        log::info!(
            "  Collection: {} {}",
            coll_status,
            verified.if_supports_color(Stdout, |t| t.dimmed()),
        );
    }

    log::info!("");
}

// ── Table printers ──────────────────────────────────────────────────────────

fn print_works_table(works: &[retro_junk_db::WorkRow], offset: u32) {
    for w in works {
        let wid = format!("{}{}", PREFIX_WORK, &w.id);
        log::info!(
            "  {:<50} {}",
            w.canonical_name,
            wid.if_supports_color(Stdout, |t| t.dimmed()),
        );
    }
    log::info!("");
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
        let date_str = r.release_date.as_deref().unwrap_or("");
        let serial_str = r.game_serial.as_deref().unwrap_or("");
        let rid = format!("{}{}", PREFIX_RELEASE, &r.id);
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
    log::info!("");
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
        let name = m.dat_name.as_deref().unwrap_or(&m.id);
        let size_str = m.file_size.map(format_file_size).unwrap_or_default();
        let plat = resolve_media_platform(conn, &m.release_id, platform_label);
        let mid = format!("{}{}", PREFIX_MEDIA, &m.id);
        log::info!(
            "  {:<35} {:<8} {:>8}  {}",
            truncate_str(name, 35),
            plat,
            size_str,
            mid.if_supports_color(Stdout, |t| t.dimmed()),
        );
    }
    log::info!("");
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
