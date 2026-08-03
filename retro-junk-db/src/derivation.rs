//! What the catalog knows about files that are not what they appear to be.
//!
//! A mod and a homebrew title are the two things in a collection that no DAT
//! describes. The catalog still records them — a mark applied to this database
//! mints a homebrew work, or a `modded` medium under the work a mod was made
//! from — and this module reads that record back out in the terms a scraper
//! lookup needs: whose identity to offer for these bytes.
//!
//! It resolves the *inputs*, never the decision. What to do with a resolved
//! derivation is `retro_junk_scraper::derivation`'s single answer, shared by
//! every surface that scrapes.

use rusqlite::{Connection, OptionalExtension, params};

use crate::library::{ArchivedScrapeIdentity, LibraryEntryId, LibraryError};

/// `SQLite`'s default limit is 999 bound parameters; stay well inside it so a
/// whole-console scrape does not have to think about batching.
const ID_CHUNK: usize = 400;

/// What the catalog says a medium is, and therefore whose identity a scraper
/// must be asked about.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CatalogDerivation {
    /// Catalogued from a DAT: the file is what it appears to be.
    #[default]
    Own,
    /// A mod of a catalogued work.
    ///
    /// `parent` is that work's own medium — absent when this machine's catalog
    /// does not hold it, which is normal on a device that has not imported the
    /// DAT the parent came from. Absent is not a failure: it is the difference
    /// between "ask about the parent" and "there is nothing to ask".
    Modded {
        parent: Option<Box<ArchivedScrapeIdentity>>,
    },
    /// Its own work, with no catalogued parent.
    Homebrew,
}

impl CatalogDerivation {
    /// Read one tag as this database records it. Anything unrecognized is
    /// [`CatalogDerivation::Own`]: an unknown tag must not silently suppress a
    /// lookup.
    fn from_tag(tag: &str, parent: Option<Box<ArchivedScrapeIdentity>>) -> Self {
        match tag.trim() {
            "modded" => Self::Modded { parent },
            "homebrew" => Self::Homebrew,
            _ => Self::Own,
        }
    }
}

/// Which catalogued work a derivative belongs to, and where.
///
/// A mod's medium lives under its parent's work, so the work id is the link;
/// platform and region only order the candidates.
pub(crate) struct ParentScope<'a> {
    pub work_id: &'a str,
    pub platform_id: &'a str,
    pub region: &'a str,
}

/// The derivation of a medium whose tag and work are already known.
pub(crate) fn derivation_for_media(
    conn: &Connection,
    tag: &str,
    scope: &ParentScope<'_>,
) -> Result<CatalogDerivation, LibraryError> {
    let parent = if tag.trim() == "modded" {
        parent_identity(conn, scope)?.map(Box::new)
    } else {
        None
    };
    Ok(CatalogDerivation::from_tag(tag, parent))
}

/// The catalogued medium a mod was made from.
///
/// Prefers the derivative's own region and a complete digest triple, because
/// those are what raise the lookup from a name guess to an exact match. An
/// untagged medium under the same work is by construction the DAT-imported
/// original: the tag is what the mark added.
pub(crate) fn parent_identity(
    conn: &Connection,
    scope: &ParentScope<'_>,
) -> Result<Option<ArchivedScrapeIdentity>, LibraryError> {
    if scope.work_id.trim().is_empty() {
        return Ok(None);
    }
    Ok(conn
        .query_row(
            "SELECT m.rom_name,m.dat_name,r.title,m.file_size,m.media_serial,
                    m.crc32,m.md5,m.sha1,
                    EXISTS(SELECT 1 FROM media_tracks mt WHERE mt.media_id=m.id)
             FROM media m
             JOIN releases r ON r.id=m.release_id
             WHERE r.work_id=?1
               AND COALESCE(m.tag,'')=''
               AND (?2='' OR r.platform_id=?2)
             ORDER BY (r.region=?3) DESC,
                      (m.crc32<>'' AND m.md5<>'' AND m.sha1<>'') DESC,
                      (m.media_serial<>'') DESC,
                      m.id
             LIMIT 1",
            params![scope.work_id, scope.platform_id, scope.region],
            |row| {
                let multi_track = row.get::<_, i64>(8)? != 0;
                Ok(ArchivedScrapeIdentity {
                    filename: lookup_name(
                        &row.get::<_, String>(0)?,
                        &row.get::<_, String>(1)?,
                        &row.get::<_, String>(2)?,
                        multi_track,
                    ),
                    file_size: lookup_size(
                        u64::try_from(row.get::<_, i64>(3)?).unwrap_or_default(),
                        multi_track,
                    ),
                    serial: row.get(4)?,
                    crc32: row.get(5)?,
                    md5: row.get(6)?,
                    sha1: row.get(7)?,
                    derivation: CatalogDerivation::Own,
                })
            },
        )
        .optional()?)
}

/// The name to offer a scraper's filename tier for one medium.
///
/// The ROM name is what a DAT calls the file; the DAT game name describes the
/// whole medium; the release title is all a synthesized medium — a homebrew
/// work minted from a mark — ever has. An empty name is the one answer that
/// cannot work, because it spends a request asking about nothing.
///
/// A multi-track medium is the exception, and it is the same exception
/// `retro_junk_dat::tracks::whole_medium_stem` exists for: the catalog's
/// `rom_name` there is the largest *member track*, so offering it asks about
/// `Some Game (USA) (Track 2).bin` — a file that exists in no collection and
/// names a track rather than the disc. What the archive holds, and what the
/// library's own scrape offers for the same disc, is the whole medium; the DAT
/// game name is what names that. Without this the two surfaces ask different
/// questions about one disc.
pub(crate) fn lookup_name(
    rom_name: &str,
    dat_name: &str,
    title: &str,
    multi_track: bool,
) -> String {
    let candidates: [&str; 3] = if multi_track {
        [dat_name, title, ""]
    } else {
        [rom_name, dat_name, title]
    };
    candidates
        .into_iter()
        .find(|candidate| !candidate.trim().is_empty())
        .unwrap_or_default()
        .to_owned()
}

/// The size to offer alongside that name, or `0` for none.
///
/// `romtaille` narrows a name match to files of exactly that size, so it must
/// be omitted when the name and the size describe different things. For a
/// multi-track medium they do: the name is now the disc, while the catalog's
/// `file_size` is one track's. The catalog holds no size for the whole medium
/// — that is what `media_tracks` exists to record — so the honest answer is to
/// send no size rather than a track's.
pub(crate) fn lookup_size(file_size: u64, multi_track: bool) -> u64 {
    if multi_track { 0 } else { file_size }
}

/// The name that names a catalogued work to another machine.
///
/// Ids are not portable — a media id encodes the DAT release it was minted
/// against — so a mod records what its parent is *called*. A DAT game name is
/// what the importer matched on and what a scraper's filename tier wants; a
/// work with no DAT-derived medium (itself synthesized from a mark) has only
/// its canonical name, which is still better than nothing to carry.
pub fn work_lookup_name(
    conn: &Connection,
    work_id: &str,
    platform_id: &str,
) -> Result<String, LibraryError> {
    let dat_name: Option<String> = conn
        .query_row(
            "SELECT m.dat_name FROM media m
             JOIN releases r ON r.id=m.release_id
             WHERE r.work_id=?1 AND m.dat_name<>'' AND COALESCE(m.tag,'')=''
               AND (?2='' OR r.platform_id=?2)
             ORDER BY m.id LIMIT 1",
            params![work_id, platform_id],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(dat_name) = dat_name.filter(|name| !name.trim().is_empty()) {
        return Ok(dat_name);
    }
    Ok(conn
        .query_row(
            "SELECT canonical_name FROM works WHERE id=?1",
            [work_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .unwrap_or_default())
}

/// One tag decision, in the form that has to survive this database.
#[derive(Debug, Clone, Copy)]
pub enum MarkDecision<'a> {
    /// A work with no catalogued parent.
    Homebrew { name: &'a str, region: &'a str },
    /// A file derived from the catalogued work `parent_work_id`.
    Modded {
        parent_work_id: &'a str,
        region: &'a str,
    },
    /// The user took the tag off.
    Cleared,
}

/// Record — or forget — the durable form of one library row's tag.
///
/// This database is device-local and rebuilt from DATs, so a decision kept
/// only here has to be made again on every machine the collection reaches, and
/// "unmatched by every DAT" is the *normal* state for the files these
/// decisions are about. The mark travels with the files instead; the row
/// becomes a cache of it.
///
/// Returns the mark file written, or `None` when there was nothing durable to
/// key on: content is the only identity a mod or a homebrew title has, so an
/// unhashed row cannot be recorded portably yet. The tag itself is still set —
/// hashing the row later is what makes it travel.
pub fn record_entry_mark(
    conn: &Connection,
    entry_id: LibraryEntryId,
    collection_root: &std::path::Path,
    decision: MarkDecision<'_>,
) -> Result<Option<std::path::PathBuf>, LibraryError> {
    let Some(subject) = mark_subject(conn, entry_id)? else {
        return Ok(None);
    };
    let (kind, region, parent_work_id) = match decision {
        MarkDecision::Homebrew { region, .. } => {
            (retro_junk_archive::MarkKind::Homebrew, region, "")
        }
        MarkDecision::Modded {
            parent_work_id,
            region,
        } => (retro_junk_archive::MarkKind::Modded, region, parent_work_id),
        // Removal only needs the file's identity: a mark's path is its
        // platform and digest, never its kind.
        MarkDecision::Cleared => (retro_junk_archive::MarkKind::Homebrew, "", ""),
    };
    let name = match decision {
        MarkDecision::Homebrew { name, .. } if !name.trim().is_empty() => name.to_owned(),
        _ => subject.display_name.clone(),
    };
    let parent_dat_name = if parent_work_id.is_empty() {
        String::new()
    } else {
        work_lookup_name(conn, parent_work_id, &subject.platform_id)?
    };
    let mark = retro_junk_archive::CollectionMark {
        schema_version: retro_junk_archive::marks::MARK_SCHEMA_VERSION,
        kind,
        platform_id: subject.platform_id,
        region: region.to_owned(),
        name,
        parent_work_id: parent_work_id.to_owned(),
        parent_dat_name,
        content: subject.content,
        note: String::new(),
    };
    let outcome = if matches!(decision, MarkDecision::Cleared) {
        retro_junk_archive::remove_mark(collection_root, &mark).map(|_| None)
    } else {
        retro_junk_archive::write_mark(collection_root, &mark).map(Some)
    };
    outcome.map_err(|error| LibraryError::CatalogMutation(error.to_string()))
}

/// What a durable mark needs to know about one library row.
struct MarkSubject {
    /// The platform's short name, which is both how a mark files itself and
    /// how the catalog keys a work's releases.
    platform_id: String,
    display_name: String,
    content: retro_junk_archive::MarkedContent,
}

fn mark_subject(
    conn: &Connection,
    entry_id: LibraryEntryId,
) -> Result<Option<MarkSubject>, LibraryError> {
    let row = conn
        .query_row(
            "SELECT lc.platform,e.display_name,e.crc32,e.sha1,e.md5,e.data_size
             FROM library_entries e
             JOIN library_consoles lc ON lc.id=e.console_id
             WHERE e.id=?1",
            [entry_id.0],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    retro_junk_archive::MarkedContent {
                        crc32: row.get(2)?,
                        sha1: row.get(3)?,
                        md5: row.get(4)?,
                        size: u64::try_from(row.get::<_, i64>(5)?).unwrap_or_default(),
                    },
                ))
            },
        )
        .optional()?;
    let Some((platform, display_name, content)) = row else {
        return Err(LibraryError::NotFound);
    };
    if content.crc32.is_empty() && content.sha1.is_empty() {
        log::debug!("Not recording a portable mark for {display_name}: it has not been hashed yet");
        return Ok(None);
    }
    let platform_id = platform.parse::<retro_junk_core::Platform>().map_or_else(
        |_| platform.to_ascii_lowercase(),
        |parsed| parsed.short_name().to_owned(),
    );
    Ok(Some(MarkSubject {
        platform_id,
        display_name,
        content,
    }))
}

/// The derivation of each named library entry that has one.
///
/// Keyed on the row's own tag rather than on the bound medium: a file tagged
/// through the plain menu has no synthesized medium at all, and it still must
/// not be looked up by bytes no catalog has ever seen. Entries absent from the
/// result are [`CatalogDerivation::Own`].
pub fn query_entry_derivations(
    conn: &Connection,
    entry_ids: &[LibraryEntryId],
) -> Result<std::collections::HashMap<LibraryEntryId, CatalogDerivation>, LibraryError> {
    let mut output = std::collections::HashMap::new();
    for chunk in entry_ids.chunks(ID_CHUNK) {
        let placeholders = std::iter::repeat_n("?", chunk.len())
            .collect::<Vec<_>>()
            .join(",");
        let mut statement = conn.prepare(&format!(
            "SELECT e.id,COALESCE(e.tag,''),COALESCE(r.work_id,''),
                    COALESCE(r.platform_id,''),COALESCE(r.region,'')
             FROM library_entries e
             LEFT JOIN library_entry_media_bindings b ON b.library_entry_id=e.id
             LEFT JOIN media m ON m.id=b.catalog_media_id
             LEFT JOIN releases r ON r.id=m.release_id
             WHERE e.id IN ({placeholders}) AND COALESCE(e.tag,'')<>''
             ORDER BY e.id,m.id"
        ))?;
        let parameters = rusqlite::params_from_iter(chunk.iter().map(|id| id.0));
        let rows = statement
            .query_map(parameters, |row| {
                Ok((
                    LibraryEntryId(row.get(0)?),
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for (entry_id, tag, work_id, platform_id, region) in rows {
            if output.contains_key(&entry_id) {
                continue;
            }
            let derivation = derivation_for_media(
                conn,
                &tag,
                &ParentScope {
                    work_id: &work_id,
                    platform_id: &platform_id,
                    region: &region,
                },
            )?;
            output.insert(entry_id, derivation);
        }
    }
    Ok(output)
}
