use retro_junk_core::RomAnalyzer;
use retro_junk_dat::{DatError, DatFile, DatGame, DatIndex, DatRom};

/// Build the matcher used by explicit maintenance commands from the SQLite
/// catalog. Raw DATs are import inputs only; maintenance never parses them.
pub(crate) fn load_catalog_index(analyzer: &dyn RomAnalyzer) -> Result<DatIndex, DatError> {
    let path = retro_junk_dat::cache::cache_dir()
        .map_err(|e| DatError::cache(e.to_string()))?
        .join("catalog.db");
    if !path.exists() {
        return Err(DatError::cache(format!(
            "Catalog database not found at {}. Import DATs into the catalog first.",
            path.display()
        )));
    }
    let conn = retro_junk_db::open_database(&path)
        .map_err(|e| DatError::cache(format!("Cannot open catalog: {e}")))?;
    let releases = retro_junk_db::releases_for_platform(&conn, analyzer.short_name())
        .map_err(|e| DatError::cache(format!("Cannot query catalog: {e}")))?;
    let mut games = Vec::new();
    for release in releases {
        let media = retro_junk_db::media_for_release(&conn, &release.id)
            .map_err(|e| DatError::cache(format!("Cannot query catalog media: {e}")))?;
        for item in media {
            games.push(DatGame {
                name: item.dat_name,
                region: Some(release.region.clone()),
                serial: (!item.media_serial.is_empty()).then_some(item.media_serial.clone()),
                version: (!item.revision.is_empty()).then_some(item.revision),
                category: None,
                roms: vec![DatRom {
                    name: item.rom_name,
                    size: u64::try_from(item.file_size).unwrap_or(0),
                    crc: item.crc32,
                    sha1: (!item.sha1.is_empty()).then_some(item.sha1),
                    md5: (!item.md5.is_empty()).then_some(item.md5),
                    serial: (!item.media_serial.is_empty()).then_some(item.media_serial),
                }],
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
