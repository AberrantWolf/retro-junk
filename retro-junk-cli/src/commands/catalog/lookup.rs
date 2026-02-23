use std::collections::HashSet;
use std::path::PathBuf;

use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;

use super::{default_catalog_db_path, format_file_size, truncate_str};

/// Show catalog database statistics.
pub(crate) fn run_catalog_lookup(
    query: Option<String>,
    console: Option<String>,
    crc: Option<String>,
    sha1: Option<String>,
    md5: Option<String>,
    serial: Option<String>,
    limit: u32,
    db_path: Option<PathBuf>,
) {
    // Validate: exactly one lookup mode
    let mode_count = [
        query.is_some(),
        crc.is_some(),
        sha1.is_some(),
        md5.is_some(),
        serial.is_some(),
    ]
    .iter()
    .filter(|&&b| b)
    .count();

    if mode_count == 0 {
        log::error!("Provide a name to search, or use --crc, --sha1, --md5, or --serial.");
        log::info!("Examples:");
        log::info!("  retro-junk catalog lookup \"mario\"");
        log::info!("  retro-junk catalog lookup --crc d445f698");
        log::info!("  retro-junk catalog lookup --serial SCUS-94163");
        std::process::exit(1);
    }
    if mode_count > 1 {
        log::error!("Only one lookup mode at a time (name, --crc, --sha1, --md5, or --serial).");
        std::process::exit(1);
    }

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

    // Helper to resolve platform_id → short display name
    let platform_label = |pid: &str| -> String {
        retro_junk_db::get_platform_display_name(&conn, pid)
            .ok()
            .flatten()
            .unwrap_or_else(|| pid.to_uppercase())
    };

    // Helper to resolve company_id → name
    let company_label = |cid: &str| -> String {
        retro_junk_db::get_company_name(&conn, cid)
            .ok()
            .flatten()
            .unwrap_or_else(|| cid.to_string())
    };

    // ── Name search ────────────────────────────────────────────────────
    if let Some(ref q) = query {
        let releases = match retro_junk_db::search_releases_filtered(
            &conn,
            q,
            console.as_deref(),
            limit,
        ) {
            Ok(r) => r,
            Err(e) => {
                log::error!("Search failed: {}", e);
                std::process::exit(1);
            }
        };

        if releases.is_empty() {
            log::info!("No releases found matching \"{}\".", q);
            return;
        }

        if releases.len() == 1 {
            print_release_detail(&conn, &releases[0], &platform_label, &company_label);
            return;
        }

        // Summary view
        log::info!(
            "{}",
            format!("Found {} releases matching \"{}\":", releases.len(), q)
                .if_supports_color(Stdout, |t| t.bold()),
        );
        log::info!("");
        for r in &releases {
            let serial_str = r.game_serial.as_deref().unwrap_or("");
            let date_str = r.release_date.as_deref().unwrap_or("");
            let plat = platform_label(&r.platform_id);
            log::info!(
                "  {:<40} {:<10} {:<7} {:<10} {}",
                truncate_str(&r.title, 40),
                plat,
                &r.region,
                date_str,
                serial_str.if_supports_color(Stdout, |t| t.dimmed()),
            );
        }
        log::info!("");
        if releases.len() as u32 == limit {
            log::info!(
                "Showing first {} results. Use --limit to see more.",
                limit,
            );
        } else {
            log::info!("Showing {} of {} results.", releases.len(), releases.len());
        }
        return;
    }

    // ── Hash lookups ───────────────────────────────────────────────────
    if let Some(ref hash) = crc {
        let hash = hash.to_lowercase();
        lookup_by_hash(&conn, "CRC32", &hash, |h| retro_junk_db::find_media_by_crc32(&conn, h), console.as_deref(), &platform_label, &company_label);
        return;
    }
    if let Some(ref hash) = sha1 {
        let hash = hash.to_lowercase();
        lookup_by_hash(&conn, "SHA1", &hash, |h| retro_junk_db::find_media_by_sha1(&conn, h), console.as_deref(), &platform_label, &company_label);
        return;
    }
    if let Some(ref hash) = md5 {
        let hash = hash.to_lowercase();
        lookup_by_hash(&conn, "MD5", &hash, |h| retro_junk_db::find_media_by_md5(&conn, h), console.as_deref(), &platform_label, &company_label);
        return;
    }

    // ── Serial lookup ──────────────────────────────────────────────────
    if let Some(ref s) = serial {
        let mut release_ids: HashSet<String> = HashSet::new();
        let mut releases: Vec<retro_junk_catalog::types::Release> = Vec::new();

        // Search release serials
        if let Ok(found) = retro_junk_db::find_release_by_serial(&conn, s) {
            for r in found {
                if release_ids.insert(r.id.clone()) {
                    releases.push(r);
                }
            }
        }

        // Search media serials → resolve parent release
        if let Ok(media_hits) = retro_junk_db::find_media_by_serial(&conn, s) {
            for m in &media_hits {
                if !release_ids.contains(&m.release_id) {
                    if let Ok(Some(r)) = retro_junk_db::get_release_by_id(&conn, &m.release_id) {
                        release_ids.insert(r.id.clone());
                        releases.push(r);
                    }
                }
            }
        }

        // Apply console filter
        if let Some(ref c) = console {
            releases.retain(|r| r.platform_id == *c);
        }

        if releases.is_empty() {
            log::info!("No releases found for serial \"{}\".", s);
            return;
        }

        if releases.len() == 1 {
            print_release_detail(&conn, &releases[0], &platform_label, &company_label);
        } else {
            log::info!(
                "{}",
                format!("Found {} releases for serial \"{}\":", releases.len(), s)
                    .if_supports_color(Stdout, |t| t.bold()),
            );
            log::info!("");
            for r in &releases {
                let plat = platform_label(&r.platform_id);
                let date_str = r.release_date.as_deref().unwrap_or("");
                log::info!(
                    "  {:<40} {:<10} {:<7} {}",
                    truncate_str(&r.title, 40),
                    plat,
                    &r.region,
                    date_str,
                );
            }
        }
    }
}

/// Look up releases by a hash, resolving media → release.
fn lookup_by_hash<F>(
    conn: &retro_junk_db::Connection,
    hash_type: &str,
    hash: &str,
    find_fn: F,
    console_filter: Option<&str>,
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
                log::warn!("Media {} references unknown release {}", media.id, media.release_id);
                continue;
            }
            Err(e) => {
                log::error!("Failed to fetch release: {}", e);
                continue;
            }
        };

        if let Some(cf) = console_filter {
            if release.platform_id != cf {
                continue;
            }
        }

        print_release_detail(conn, &release, platform_label, company_label);
    }
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
        .map(|id| company_label(id))
        .unwrap_or_else(|| dash.to_string());
    let developer = release
        .developer_id
        .as_deref()
        .map(|id| company_label(id))
        .unwrap_or_else(|| dash.to_string());
    let date_str = release.release_date.as_deref().unwrap_or(dash);
    let genre_str = release.genre.as_deref().unwrap_or(dash);
    let players_str = release.players.as_deref().unwrap_or(dash);
    let rating_str = release
        .rating
        .map(|r| format!("{:.1}", r))
        .unwrap_or_else(|| dash.to_string());

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
            log::info!(
                "  {}",
                "Media:".if_supports_color(Stdout, |t| t.bold()),
            );
            for (i, m) in media.iter().enumerate() {
                let name = m.dat_name.as_deref().unwrap_or(&m.id);
                log::info!(
                    "    {}. {}",
                    i + 1,
                    name,
                );
                let crc = m.crc32.as_deref().unwrap_or(dash);
                let sha1_val = m.sha1.as_deref().unwrap_or(dash);
                let sha1_short = if sha1_val.len() > 12 {
                    &sha1_val[..12]
                } else {
                    sha1_val
                };
                let size_str = m
                    .file_size
                    .map(|s| format_file_size(s))
                    .unwrap_or_else(|| dash.to_string());
                log::info!(
                    "       CRC32: {}  SHA1: {}...  Size: {}",
                    crc, sha1_short, size_str,
                );

                let status = format!("{:?}", m.status).to_lowercase();
                let source = m.dat_source.as_deref().unwrap_or(dash);
                log::info!(
                    "       Status: {}  Source: {}",
                    status, source,
                );

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
                type_list
                    .iter()
                    .map(|t| **t)
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }
        _ => {}
    }

    log::info!("");
}
