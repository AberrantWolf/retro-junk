//! Catalog questions: how big is the catalog, where do the sources disagree,
//! and what rows match a browse or search request.
//!
//! Every function here takes a caller-held connection and returns plain data.
//! Paging, filter defaults, and the small quirks of each table (for example,
//! that a work search cannot report a release count) are settled here so no
//! frontend has to know them.

use retro_junk_catalog::types::{Disagreement, ImportLog, Media, Release};
use retro_junk_db::{
    CatalogStats, CollectionRow, CompanyRow, Connection, PlatformRow, WorkRow, WorkWithCount,
};

/// One page of rows plus how many rows exist in total, so a frontend can draw
/// both the table and its pagination footer from one answer.
#[derive(Debug, Clone, Default)]
pub struct Page<T> {
    pub total: i64,
    pub rows: Vec<T>,
}

/// Everything the catalog dashboard shows: headline counts, the platform list
/// its filter dropdown needs, and the currently unresolved disagreements.
#[derive(Debug, Default)]
pub struct CatalogDashboard {
    pub stats: Option<CatalogStats>,
    pub platforms: Vec<PlatformRow>,
    pub disagreements: Vec<Disagreement>,
}

/// What a disagreement is *about*, in words a person can read: the title of
/// the thing in dispute and the platform it belongs to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DisagreementContext {
    pub entity_title: String,
    pub platform_name: String,
}

/// The number of unresolved disagreements the dashboard lists at once. Beyond
/// this the list stops being something a person reviews by hand.
const DISAGREEMENT_LIST_LIMIT: u32 = 500;

/// Answers: "what does the catalog dashboard show right now?"
///
/// A missing or unreadable piece degrades to empty rather than failing the
/// whole panel — a catalog that is still being built should still render.
pub fn dashboard(
    conn: &Connection,
    platform_filter: Option<&str>,
    field_filter: Option<&str>,
) -> CatalogDashboard {
    let filter = retro_junk_db::DisagreementFilter {
        platform_id: platform_filter,
        field: field_filter,
        limit: Some(DISAGREEMENT_LIST_LIMIT),
        ..Default::default()
    };
    CatalogDashboard {
        stats: retro_junk_db::catalog_stats(conn).ok(),
        platforms: retro_junk_db::list_platforms(conn).unwrap_or_default(),
        disagreements: retro_junk_db::list_unresolved_disagreements(conn, &filter)
            .unwrap_or_default(),
    }
}

/// Answers: "which platforms does the catalog know about?"
pub fn platforms(conn: &Connection) -> Vec<PlatformRow> {
    retro_junk_db::list_platforms(conn).unwrap_or_default()
}

/// Answers: "what entity is this disagreement about?"
///
/// Falls back to the raw entity id when the row it points at is gone, so the
/// reviewer always sees something identifiable instead of a blank.
pub fn disagreement_context(conn: &Connection, disagreement: &Disagreement) -> DisagreementContext {
    let entity_id = &disagreement.entity_id;
    let (entity_title, platform_id) = match disagreement.entity_type.as_str() {
        "release" => match retro_junk_db::get_release_by_id(conn, entity_id) {
            Ok(Some(release)) => (release.title, release.platform_id),
            _ => (entity_id.clone(), String::new()),
        },
        "media" => match retro_junk_db::get_media_by_id(conn, entity_id) {
            Ok(Some(media)) => match retro_junk_db::get_release_by_id(conn, &media.release_id) {
                Ok(Some(release)) => (release.title, release.platform_id),
                _ if media.dat_name.is_empty() => (entity_id.clone(), String::new()),
                _ => (media.dat_name, String::new()),
            },
            _ => (entity_id.clone(), String::new()),
        },
        _ => (entity_id.clone(), String::new()),
    };

    let platform_name = if platform_id.is_empty() {
        String::new()
    } else {
        retro_junk_db::get_platform_display_name(conn, &platform_id)
            .ok()
            .flatten()
            .unwrap_or(platform_id)
    };

    DisagreementContext {
        entity_title,
        platform_name,
    }
}

/// Answers: "which releases match this search, on this page?"
pub fn releases_page(
    conn: &Connection,
    query: &str,
    platform_id: Option<&str>,
    limit: u32,
    offset: u32,
) -> Page<Release> {
    Page {
        total: retro_junk_db::count_releases_search(conn, query, platform_id).unwrap_or(0),
        rows: retro_junk_db::search_releases_paged(conn, query, platform_id, limit, offset)
            .unwrap_or_default(),
    }
}

/// Answers: "which media rows match this search, on this page?"
pub fn media_page(
    conn: &Connection,
    query: &str,
    platform_id: Option<&str>,
    limit: u32,
    offset: u32,
) -> Page<Media> {
    Page {
        total: retro_junk_db::count_media_search(conn, query, platform_id).unwrap_or(0),
        rows: retro_junk_db::search_media(conn, query, platform_id, limit, offset)
            .unwrap_or_default(),
    }
}

/// Answers: "which works match this search, on this page?"
///
/// The search query cannot count releases per work, so the count comes back as
/// zero rather than as a second query per row.
pub fn works_page(conn: &Connection, query: &str, limit: u32, offset: u32) -> Page<WorkWithCount> {
    Page {
        total: retro_junk_db::count_works_search(conn, query).unwrap_or(0),
        rows: retro_junk_db::search_works(conn, query, limit, offset)
            .unwrap_or_default()
            .into_iter()
            .map(|work| WorkWithCount {
                id: work.id,
                canonical_name: work.canonical_name,
                release_count: 0,
            })
            .collect(),
    }
}

/// Answers: "which companies match this search, on this page?"
pub fn companies_page(conn: &Connection, query: &str, limit: u32, offset: u32) -> Page<CompanyRow> {
    Page {
        total: retro_junk_db::count_companies_search(conn, query).unwrap_or(0),
        rows: retro_junk_db::search_companies(conn, query, limit, offset).unwrap_or_default(),
    }
}

/// Answers: "what is in the owned collection, on this page?"
pub fn collection_page(
    conn: &Connection,
    platform_id: Option<&str>,
    limit: u32,
    offset: u32,
) -> Page<CollectionRow> {
    Page {
        total: retro_junk_db::count_collection(conn, platform_id).unwrap_or(0),
        rows: retro_junk_db::list_collection_paged(conn, platform_id, limit, offset)
            .unwrap_or_default(),
    }
}

/// Answers: "what has been imported recently?"
///
/// The import log is short and read whole — the total is simply how many rows
/// came back, so its viewer needs no separate count query.
pub fn import_log_page(conn: &Connection, limit: u32) -> Page<ImportLog> {
    let rows = retro_junk_db::list_import_logs(conn, Some(limit)).unwrap_or_default();
    Page {
        total: rows.len() as i64,
        rows,
    }
}

/// Answers: "which catalog works on this platform match what the user typed?"
///
/// Used when tagging a file by hand, so the caller gets an error instead of an
/// empty list when the lookup itself fails.
pub fn works_for_platform(
    conn: &Connection,
    query: &str,
    platform_id: &str,
    limit: u32,
) -> Result<Vec<WorkRow>, String> {
    retro_junk_db::search_works_for_platform(conn, query, platform_id, limit)
        .map_err(|error| error.to_string())
}
