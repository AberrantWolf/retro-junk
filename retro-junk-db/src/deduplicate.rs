//! Catalog entries that look like the same thing but are not.
//!
//! There used to be a merge pass here too, for rows that were byte-for-byte
//! identical. It existed because catalog ids were built out of the game's
//! title: correcting a title minted a whole new work/release/media triple
//! beside the old one, with the same hashes. Media ids are now folded from the
//! medium's own digests, so two rows with identical content land on the same
//! `PRIMARY KEY` and the second insert simply updates the first. The exact
//! duplicate cannot be created, so there is nothing to merge.
//!
//! What remains is the harder question, which no key can answer: rows that
//! claim to be the same edition of the same release — same disc number, same
//! revision, same serials — while their bytes disagree. That is a catalog
//! problem for a person to look at, not something to collapse automatically.

use std::collections::BTreeMap;

use rusqlite::Connection;

/// Media that describe the same edition of the same release while disagreeing
/// about what the bytes are.
#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct SuspectedDuplicateGroup {
    pub release_id: String,
    pub platform_id: String,
    /// The media in the group, lowest id first.
    pub media_ids: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct CatalogDeduplicationReport {
    pub platform: Option<String>,
    pub suspected_groups: Vec<SuspectedDuplicateGroup>,
}

pub fn analyze_catalog_duplicates(
    conn: &Connection,
    platform: Option<&str>,
) -> Result<CatalogDeduplicationReport, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT m.id,m.release_id,r.platform_id,
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
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    // Group by what the entries *claim* to be, then keep only the groups whose
    // members disagree about the bytes. Members that agree are impossible now
    // — identical digests fold to one id — so any group left over is a real
    // question about the catalog rather than a key artefact.
    let mut claimed = BTreeMap::<String, Vec<(String, String, String, String)>>::new();
    for (id, release_id, platform_id, revision, disc, serial, size, crc, sha1, md5) in rows {
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
        let identity = format!("{release_id}\u{1d}{disc}\u{1d}{revision}\u{1d}{serials}");
        let content = format!("{size}\u{1d}{crc}\u{1d}{sha1}\u{1d}{md5}\u{1d}{tracks}");
        claimed
            .entry(identity)
            .or_default()
            .push((id, release_id, platform_id, content));
    }

    let mut suspected_groups = Vec::new();
    for members in claimed.into_values() {
        if members.len() < 2 {
            continue;
        }
        let first_content = &members[0].3;
        if members.iter().all(|member| &member.3 == first_content) {
            continue;
        }
        suspected_groups.push(SuspectedDuplicateGroup {
            release_id: members[0].1.clone(),
            platform_id: members[0].2.clone(),
            media_ids: members.into_iter().map(|member| member.0).collect(),
        });
    }
    suspected_groups.sort_by(|a, b| a.media_ids.cmp(&b.media_ids));
    Ok(CatalogDeduplicationReport {
        platform: platform.map(str::to_owned),
        suspected_groups,
    })
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
