//! Lossless catalog duplicate analysis and repair.

use std::collections::BTreeMap;

use rusqlite::{Connection, params};

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct DuplicateMediaGroup {
    pub canonical_media_id: String,
    pub duplicate_media_ids: Vec<String>,
    pub release_id: String,
    pub platform_id: String,
    pub reference_count: u64,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct CatalogDeduplicationReport {
    pub platform: Option<String>,
    pub exact_groups: Vec<DuplicateMediaGroup>,
    pub suspected_groups: u64,
    pub affected_references: u64,
    pub applied: bool,
}

#[derive(Debug)]
struct MediaFingerprint {
    id: String,
    release_id: String,
    platform_id: String,
    rom_name: String,
    comparable: bool,
    references: u64,
}

pub fn analyze_catalog_duplicates(
    conn: &Connection,
    platform: Option<&str>,
) -> Result<CatalogDeduplicationReport, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT m.id,m.release_id,r.platform_id,m.rom_name,
                r.revision,m.disc_number,m.media_serial,m.file_size,m.crc32,m.sha1,m.md5
         FROM media m JOIN releases r ON r.id=m.release_id
         WHERE (?1='' OR r.platform_id=?1) ORDER BY m.id",
    )?;
    let rows = stmt
        .query_map([platform.unwrap_or_default()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut exact = BTreeMap::<String, Vec<MediaFingerprint>>::new();
    let mut suspected = BTreeMap::<String, usize>::new();
    for (id, release_id, platform_id, rom_name, revision, disc, serial, size, crc, sha1, md5) in
        rows
    {
        let serial_keys = joined_values(
            conn,
            "SELECT serial_key FROM media_serial_keys WHERE media_id=?1 ORDER BY serial_key",
            &id,
        )?;
        let serials = format!(
            "{}\u{1f}{}",
            serial.trim().to_ascii_uppercase(),
            serial_keys
        );
        let tracks = joined_tracks(conn, &id)?;
        let stored_complete = size > 0 && !crc.is_empty() && !sha1.is_empty() && !md5.is_empty();
        let tracks_complete = !tracks.is_empty()
            && tracks
                .split('\u{1e}')
                .all(|track| track.split('\u{1f}').count() == 6 && !track.contains("\u{1f}\u{1f}"));
        let identity = format!("{release_id}\u{1d}{disc}\u{1d}{revision}\u{1d}{serials}");
        let signature =
            format!("{identity}\u{1d}{size}\u{1d}{crc}\u{1d}{sha1}\u{1d}{md5}\u{1d}{tracks}");
        *suspected.entry(identity).or_default() += 1;
        exact.entry(signature).or_default().push(MediaFingerprint {
            references: reference_count(conn, &id)?,
            id,
            release_id,
            platform_id,
            rom_name,
            comparable: stored_complete || tracks_complete,
        });
    }
    let mut exact_groups = Vec::new();
    for mut group in exact.into_values().filter(|group| group.len() > 1) {
        if !group[0].comparable {
            continue;
        }
        group.sort_by(|a, b| {
            b.references
                .cmp(&a.references)
                .then_with(|| b.rom_name.is_empty().cmp(&a.rom_name.is_empty()))
                .then_with(|| a.id.cmp(&b.id))
        });
        let canonical = group.remove(0);
        let reference_count =
            canonical.references + group.iter().map(|row| row.references).sum::<u64>();
        exact_groups.push(DuplicateMediaGroup {
            canonical_media_id: canonical.id,
            duplicate_media_ids: group.into_iter().map(|row| row.id).collect(),
            release_id: canonical.release_id,
            platform_id: canonical.platform_id,
            reference_count,
        });
    }
    exact_groups.sort_by(|a, b| a.canonical_media_id.cmp(&b.canonical_media_id));
    let exact_rows = exact_groups
        .iter()
        .map(|group| group.duplicate_media_ids.len() + 1)
        .sum::<usize>();
    let suspected_groups = suspected
        .into_values()
        .filter(|count| *count > 1)
        .map(|_| 1_u64)
        .sum::<u64>()
        .saturating_sub(exact_groups.len() as u64);
    let affected_references = exact_groups.iter().map(|group| group.reference_count).sum();
    let _ = exact_rows;
    Ok(CatalogDeduplicationReport {
        platform: platform.map(str::to_owned),
        exact_groups,
        suspected_groups,
        affected_references,
        applied: false,
    })
}

pub fn deduplicate_catalog(
    conn: &Connection,
    platform: Option<&str>,
) -> Result<CatalogDeduplicationReport, rusqlite::Error> {
    let mut report = analyze_catalog_duplicates(conn, platform)?;
    conn.execute_batch("SAVEPOINT catalog_deduplicate")?;
    let result = (|| {
        for group in &report.exact_groups {
            for duplicate in &group.duplicate_media_ids {
                merge_media(conn, &group.canonical_media_id, duplicate)?;
            }
        }
        Ok::<_, rusqlite::Error>(())
    })();
    match result {
        Ok(()) => conn.execute_batch("RELEASE catalog_deduplicate")?,
        Err(error) => {
            let _ =
                conn.execute_batch("ROLLBACK TO catalog_deduplicate; RELEASE catalog_deduplicate");
            return Err(error);
        }
    }
    report.applied = true;
    Ok(report)
}

fn merge_media(conn: &Connection, canonical: &str, duplicate: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO media_serial_keys(media_id,serial_key)
         SELECT ?1,serial_key FROM media_serial_keys WHERE media_id=?2",
        params![canonical, duplicate],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO media_tracks(media_id,track_number,track_name,file_size,crc32,sha1,md5)
         SELECT ?1,track_number,track_name,file_size,crc32,sha1,md5
         FROM media_tracks WHERE media_id=?2",
        params![canonical, duplicate],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO collection(media_id,user_id,owned,condition,notes,date_acquired,rom_path,verified_at)
         SELECT ?1,user_id,owned,condition,notes,date_acquired,rom_path,verified_at
         FROM collection WHERE media_id=?2",
        params![canonical, duplicate],
    )?;
    conn.execute("DELETE FROM collection WHERE media_id=?1", [duplicate])?;
    conn.execute(
        "UPDATE media_assets SET media_id=?1 WHERE media_id=?2",
        params![canonical, duplicate],
    )?;
    conn.execute(
        "UPDATE carriers SET catalog_media_id=?1 WHERE catalog_media_id=?2",
        params![canonical, duplicate],
    )?;
    // OR REPLACE: the canonical medium may already be bound to the same
    // library row and carrier. Merging keeps one binding rather than dropping
    // the duplicate's row on the floor.
    conn.execute(
        "UPDATE OR REPLACE library_entry_media_bindings
         SET catalog_media_id=?1 WHERE catalog_media_id=?2",
        params![canonical, duplicate],
    )?;
    conn.execute("DELETE FROM media WHERE id=?1", [duplicate])?;
    Ok(())
}

fn joined_values(conn: &Connection, sql: &str, id: &str) -> rusqlite::Result<String> {
    let mut stmt = conn.prepare(sql)?;
    Ok(stmt
        .query_map([id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?
        .join("\u{1f}"))
}

fn joined_tracks(conn: &Connection, id: &str) -> rusqlite::Result<String> {
    let mut stmt = conn.prepare(
        "SELECT track_number,file_size,crc32,sha1,md5,track_name
         FROM media_tracks WHERE media_id=?1 ORDER BY track_number",
    )?;
    Ok(stmt
        .query_map([id], |row| {
            Ok(format!(
                "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?
        .join("\u{1e}"))
}

fn reference_count(conn: &Connection, id: &str) -> rusqlite::Result<u64> {
    conn.query_row(
        "SELECT
           (SELECT count(*) FROM collection WHERE media_id=?1)+
           (SELECT count(*) FROM media_assets WHERE media_id=?1)+
           (SELECT count(*) FROM carriers WHERE catalog_media_id=?1)+
           (SELECT count(*) FROM library_entry_media_bindings WHERE catalog_media_id=?1)",
        [id],
        |row| row.get(0),
    )
}
