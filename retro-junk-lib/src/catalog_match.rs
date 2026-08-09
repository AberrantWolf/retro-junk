use std::collections::HashSet;

use retro_junk_core::{FileHashes, Region, RomAnalyzer, RomIdentification};
use retro_junk_dat::{DatError, DatFile, DatGame, DatIndex, DatRom, MatchMethod};

/// Result of resolving catalog candidates with all trustworthy header evidence.
#[derive(Debug)]
pub enum CatalogMatchResolution<'a> {
    Match {
        candidate: &'a retro_junk_db::CatalogMediaMatch,
        method: MatchMethod,
    },
    Ambiguous {
        candidates: Vec<String>,
    },
    NotFound,
}

/// Apply catalog evidence precedence without combining candidate domains.
/// Any content match excludes every weaker serial/header/name candidate;
/// fallback candidates are considered only when content found none.
pub fn prefer_content_candidates<T>(content: Vec<T>, fallback: Vec<T>) -> Vec<T> {
    if content.is_empty() {
        fallback
    } else {
        content
    }
}

/// Normalize the serial extracted by an analyzer into the key stored by the
/// catalog. This is the only serial-normalization path used by catalog queries.
#[must_use]
pub fn catalog_serial_key(
    analyzer: &dyn RomAnalyzer,
    identification: &RomIdentification,
) -> Option<String> {
    if identification.serial_number.is_empty() {
        return None;
    }
    Some(
        analyzer
            .extract_dat_game_code(&identification.serial_number)
            .unwrap_or_else(|| identification.serial_number.clone()),
    )
}

/// Determine whether a catalog region is compatible with regions decoded
/// from a ROM header. Catalog regions are stored as lowercase slugs.
#[must_use]
pub fn regions_match_catalog(detected: &[Region], catalog_region: &str) -> bool {
    retro_junk_core::matching::header_regions_match_catalog(detected, catalog_region)
}

/// Return the definitive hash method when a candidate actually matches.
fn catalog_hash_method(
    candidate: &retro_junk_db::CatalogMediaMatch,
    hashes: &FileHashes,
) -> Option<MatchMethod> {
    let crc_match = !hashes.crc32.is_empty()
        && u64::try_from(candidate.media.file_size).ok() == Some(hashes.data_size)
        && candidate.media.crc32.eq_ignore_ascii_case(&hashes.crc32);
    if crc_match {
        return Some(MatchMethod::Crc32);
    }
    let sha1 = hashes.sha1.as_deref().unwrap_or_default();
    (!sha1.is_empty() && candidate.media.sha1.eq_ignore_ascii_case(sha1))
        .then_some(MatchMethod::Sha1)
}

/// Select among definitive hash matches, preferring a header-compatible region.
fn select_catalog_hash_match<'a>(
    matches: &'a [retro_junk_db::CatalogMediaMatch],
    hashes: &FileHashes,
    detected_regions: &[Region],
) -> Option<(&'a retro_junk_db::CatalogMediaMatch, MatchMethod)> {
    matches
        .iter()
        .filter_map(|candidate| {
            catalog_hash_method(candidate, hashes).map(|method| (candidate, method))
        })
        .min_by_key(|(candidate, _)| !regions_match_catalog(detected_regions, &candidate.region))
}

/// Select a definitive hash match while using header metadata to identify the
/// correct release when multiple catalog rows contain identical bytes.
fn select_catalog_hash_match_with_identification<'a>(
    matches: &'a [retro_junk_db::CatalogMediaMatch],
    hashes: &FileHashes,
    identification: &RomIdentification,
) -> Option<(&'a retro_junk_db::CatalogMediaMatch, MatchMethod)> {
    let candidates: Vec<_> = matches
        .iter()
        .filter_map(|candidate| {
            catalog_hash_method(candidate, hashes).map(|method| (candidate, method))
        })
        .collect();
    let selected = retro_junk_core::matching::header_candidate_indices(
        &candidates,
        &identification.version,
        &identification.regions,
        None,
        |(candidate, _)| candidate_revision(candidate),
        |(candidate, _)| candidate.region.as_str(),
        |_| 0,
    );
    selected
        .first()
        .and_then(|index| candidates.get(*index).cloned())
}

/// Resolve every catalog match through one precedence-ordered decision path:
/// definitive hash, then serial plus header revision/region/size, then
/// ambiguity or no match. Header evidence never upgrades bytes to verified.
#[must_use]
pub fn resolve_catalog_match<'a>(
    matches: &'a [retro_junk_db::CatalogMediaMatch],
    identification: Option<&RomIdentification>,
    hashes: Option<&FileHashes>,
) -> CatalogMatchResolution<'a> {
    if matches.is_empty() {
        return CatalogMatchResolution::NotFound;
    }

    if let Some(hashes) = hashes {
        let selected = identification.map_or_else(
            || select_catalog_hash_match(matches, hashes, &[]),
            |identification| {
                select_catalog_hash_match_with_identification(matches, hashes, identification)
            },
        );
        if let Some((candidate, method)) = selected {
            return CatalogMatchResolution::Match { candidate, method };
        }
    }

    let Some(identification) =
        identification.filter(|identification| !identification.serial_number.is_empty())
    else {
        return CatalogMatchResolution::NotFound;
    };

    let candidate_indices = retro_junk_core::matching::header_candidate_indices(
        matches,
        &identification.version,
        &identification.regions,
        Some(identification.file_size),
        candidate_revision,
        |candidate| candidate.region.as_str(),
        |candidate| u64::try_from(candidate.media.file_size).unwrap_or(0),
    );
    let candidates: Vec<_> = candidate_indices
        .into_iter()
        .filter_map(|index| matches.get(index))
        .collect();

    let distinct: HashSet<_> = candidates
        .iter()
        .map(|candidate| candidate.media.dat_name.as_str())
        .collect();
    if distinct.len() > 1 {
        let mut names: Vec<_> = distinct.into_iter().map(str::to_owned).collect();
        names.sort();
        return CatalogMatchResolution::Ambiguous { candidates: names };
    }

    let Some(candidate) = candidates.first() else {
        return CatalogMatchResolution::NotFound;
    };

    CatalogMatchResolution::Match {
        candidate,
        method: MatchMethod::Serial,
    }
}

fn candidate_revision(candidate: &retro_junk_db::CatalogMediaMatch) -> &str {
    if candidate.release_revision.is_empty() {
        &candidate.media.revision
    } else {
        &candidate.release_revision
    }
}

/// Build the matcher used by explicit maintenance commands from their
/// authoritative `SQLite` connection. Raw DATs are import inputs only, and
/// this layer deliberately does not guess where the application's database
/// lives: the backend or CLI owns opening the configured catalog.
pub(crate) fn load_catalog_index(
    conn: &retro_junk_db::Connection,
    analyzer: &dyn RomAnalyzer,
) -> Result<DatIndex, DatError> {
    let releases = retro_junk_db::releases_for_platform(conn, analyzer.short_name())
        .map_err(|e| DatError::cache(format!("Cannot query catalog: {e}")))?;
    let mut games = Vec::new();
    for release in releases {
        let media = retro_junk_db::media_for_release(conn, &release.id)
            .map_err(|e| DatError::cache(format!("Cannot query catalog media: {e}")))?;
        let media_ids = media.iter().map(|item| item.id.clone()).collect::<Vec<_>>();
        let mut tracks_by_media = std::collections::HashMap::<String, Vec<_>>::new();
        for track in retro_junk_db::find_media_tracks_for_media_ids(conn, &media_ids)
            .map_err(|e| DatError::cache(format!("Cannot query catalog tracks: {e}")))?
        {
            tracks_by_media
                .entry(track.media_id.clone())
                .or_default()
                .push(track);
        }
        for item in media {
            let roms = tracks_by_media.remove(&item.id).map_or_else(
                || {
                    vec![DatRom {
                        name: item.rom_name.clone(),
                        size: u64::try_from(item.file_size).unwrap_or(0),
                        crc: item.crc32.clone(),
                        sha1: (!item.sha1.is_empty()).then_some(item.sha1.clone()),
                        md5: (!item.md5.is_empty()).then_some(item.md5.clone()),
                        serial: (!item.media_serial.is_empty())
                            .then_some(item.media_serial.clone()),
                    }]
                },
                |mut tracks| {
                    tracks.sort_by_key(|track| track.track_number);
                    tracks
                        .into_iter()
                        .map(|track| DatRom {
                            name: track.track_name,
                            size: u64::try_from(track.file_size).unwrap_or(0),
                            crc: track.crc32,
                            sha1: (!track.sha1.is_empty()).then_some(track.sha1),
                            md5: (!track.md5.is_empty()).then_some(track.md5),
                            serial: (!item.media_serial.is_empty())
                                .then_some(item.media_serial.clone()),
                        })
                        .collect()
                },
            );
            games.push(DatGame {
                name: item.dat_name,
                region: Some(release.region.clone()),
                serial: (!item.media_serial.is_empty()).then_some(item.media_serial.clone()),
                version: (!item.revision.is_empty()).then_some(item.revision),
                category: None,
                roms,
            });
        }
    }
    Ok(DatIndex::from_dat(DatFile {
        name: analyzer.platform_name().to_string(),
        description: "SQLite catalog projection".to_string(),
        version: String::new(),
        games,
    }))
}

#[cfg(test)]
mod tests {
    use super::{catalog_serial_key, load_catalog_index, prefer_content_candidates};

    #[test]
    fn content_candidates_always_outrank_fallback_evidence() {
        assert_eq!(prefer_content_candidates(vec![1, 2], vec![3]), vec![1, 2]);
        assert_eq!(
            prefer_content_candidates(Vec::<u8>::new(), vec![3]),
            vec![3]
        );
    }

    #[test]
    fn serial_key_uses_the_analyzers_dat_code() {
        let mut identification = retro_junk_core::RomIdentification::new();
        identification.serial_number = "NTR-ARME".into();
        assert_eq!(
            catalog_serial_key(&retro_junk_nintendo::DsAnalyzer, &identification).as_deref(),
            Some("ARME")
        );
    }

    #[test]
    fn maintenance_index_uses_the_supplied_catalog_connection() {
        let conn = retro_junk_db::open_memory().unwrap();
        conn.execute_batch(
            "INSERT INTO platforms(id,display_name,short_name,manufacturer,generation,media_type,release_year,description,core_platform)
             VALUES('nes','NES','NES','Nintendo',3,'cartridge',1983,'','Nes');
             INSERT INTO works(id,canonical_name) VALUES('work','Game');
             INSERT INTO releases(id,work_id,platform_id,region,title)
             VALUES('release','work','nes','usa','Game');
             INSERT INTO media(id,release_id,media_serial,dat_name,rom_name,dat_source,file_size,crc32,sha1)
             VALUES('media','release','NES-GAME','Game (USA)','Game (USA).nes','no-intro',4,'11223344','aabb');",
        )
        .unwrap();

        let index = load_catalog_index(&conn, &retro_junk_nintendo::NesAnalyzer).unwrap();
        assert_eq!(index.games.len(), 1);
        assert_eq!(index.games[0].name, "Game (USA)");
        assert_eq!(index.games[0].roms[0].crc, "11223344");
    }

    #[test]
    fn maintenance_index_retains_the_complete_catalog_track_set() {
        let conn = retro_junk_db::open_memory().unwrap();
        conn.execute_batch(
            "INSERT INTO platforms(id,display_name,short_name,manufacturer,generation,media_type,release_year,description,core_platform)
             VALUES('saturn','Saturn','Saturn','Sega',5,'disc',1994,'','Saturn');
             INSERT INTO works(id,canonical_name) VALUES('work','Disc Game');
             INSERT INTO releases(id,work_id,platform_id,region,title)
             VALUES('release','work','saturn','japan','Disc Game');
             INSERT INTO media(id,release_id,media_serial,dat_name,rom_name,dat_source,file_size,crc32,sha1)
             VALUES('media','release','GS-0001','Disc Game (Japan)','Disc Game (Japan) (Track 1).bin','redump',2352,'11111111','aaaa');
             INSERT INTO media_tracks(media_id,track_number,track_name,file_size,crc32,sha1)
             VALUES('media',1,'Disc Game (Japan) (Track 1).bin',2352,'11111111','aaaa'),
                   ('media',2,'Disc Game (Japan) (Track 2).bin',4704,'22222222','bbbb');",
        )
        .unwrap();

        let index = load_catalog_index(&conn, &retro_junk_sega::SaturnAnalyzer).unwrap();
        assert_eq!(index.games.len(), 1);
        assert_eq!(index.games[0].roms.len(), 2);
        assert_eq!(index.games[0].roms[1].sha1.as_deref(), Some("bbbb"));
    }
}
