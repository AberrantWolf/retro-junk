use std::collections::HashMap;

use crate::client::ScreenScraperClient;
use crate::error::ScrapeError;
use crate::systems;
use crate::types::GameInfo;

/// How a game was matched in ScreenScraper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookupMethod {
    /// Matched by serial number from ROM header
    Serial,
    /// Matched by filename (NoIntro name)
    Filename,
    /// Matched by hash (CRC32 + MD5 + SHA1)
    Hash,
}

impl std::fmt::Display for LookupMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LookupMethod::Serial => write!(f, "serial"),
            LookupMethod::Filename => write!(f, "filename"),
            LookupMethod::Hash => write!(f, "hash"),
        }
    }
}

/// Result of a successful game lookup.
#[derive(Debug, Clone)]
pub struct LookupResult {
    pub game: GameInfo,
    pub method: LookupMethod,
    pub warnings: Vec<String>,
}

/// Information about a ROM file for lookup purposes.
#[derive(Debug, Clone)]
pub struct RomInfo {
    /// Serial number from ROM header analysis (if any)
    pub serial: Option<String>,
    /// ROM filename with extension
    pub filename: String,
    /// ROM file size in bytes
    pub file_size: u64,
    /// CRC32 hash (uppercase hex, computed if needed)
    pub crc32: Option<String>,
    /// MD5 hash (lowercase hex, computed if needed)
    pub md5: Option<String>,
    /// SHA1 hash (lowercase hex, computed if needed)
    pub sha1: Option<String>,
    /// Short name of the console (e.g., "n64")
    pub short_name: String,
}

/// Look up a game using the tiered strategy.
///
/// 1. Serial match (preferred) — if serial was extracted from ROM header
/// 2. Filename match — using NoIntro filename + system ID + file size
/// 3. Hash match (conditional) — only for consoles without expected serials,
///    or if force_hash is true
pub async fn lookup_game(
    client: &ScreenScraperClient,
    system_id: u32,
    rom_info: &RomInfo,
    force_hash: bool,
) -> Result<LookupResult, ScrapeError> {
    let mut warnings = Vec::new();

    // Tier 1: Serial match
    if let Some(ref serial) = rom_info.serial {
        match try_serial_lookup(client, system_id, serial).await {
            Ok(game) => {
                return Ok(LookupResult {
                    game,
                    method: LookupMethod::Serial,
                    warnings,
                });
            }
            Err(ScrapeError::NotFound) => {
                warnings.push(format!("Serial '{}' not found in ScreenScraper", serial));
            }
            Err(e) => return Err(e),
        }
    } else if systems::expects_serial(&rom_info.short_name) {
        warnings.push(format!(
            "Expected serial not found in ROM header for {}",
            rom_info.filename
        ));
    }

    // Tier 2: Filename match
    match try_filename_lookup(client, system_id, &rom_info.filename, rom_info.file_size).await {
        Ok(game) => {
            return Ok(LookupResult {
                game,
                method: LookupMethod::Filename,
                warnings,
            });
        }
        Err(ScrapeError::NotFound) => {
            warnings.push(format!(
                "Filename '{}' not found in ScreenScraper",
                rom_info.filename
            ));
        }
        Err(e) => return Err(e),
    }

    // Tier 3: Hash match (conditional)
    let should_hash =
        !systems::expects_serial(&rom_info.short_name) || force_hash;

    if should_hash {
        if let (Some(crc), Some(md5), Some(sha1)) =
            (&rom_info.crc32, &rom_info.md5, &rom_info.sha1)
        {
            match try_hash_lookup(client, system_id, crc, md5, sha1, &rom_info.filename, rom_info.file_size).await {
                Ok(game) => {
                    return Ok(LookupResult {
                        game,
                        method: LookupMethod::Hash,
                        warnings,
                    });
                }
                Err(ScrapeError::NotFound) => {
                    warnings.push("Hash not found in ScreenScraper".to_string());
                }
                Err(e) => return Err(e),
            }
        }
    }

    Err(ScrapeError::NotFound)
}

async fn try_serial_lookup(
    client: &ScreenScraperClient,
    system_id: u32,
    serial: &str,
) -> Result<GameInfo, ScrapeError> {
    let mut params = HashMap::new();
    params.insert("systemeid", system_id.to_string());
    params.insert("serialnum", serial.to_string());
    params.insert("romtype", "rom".to_string());

    let resp = client.lookup_game(params).await?;
    Ok(resp.response.jeu)
}

async fn try_filename_lookup(
    client: &ScreenScraperClient,
    system_id: u32,
    filename: &str,
    file_size: u64,
) -> Result<GameInfo, ScrapeError> {
    let mut params = HashMap::new();
    params.insert("systemeid", system_id.to_string());
    params.insert("romnom", filename.to_string());
    params.insert("romtaille", file_size.to_string());
    params.insert("romtype", "rom".to_string());

    let resp = client.lookup_game(params).await?;
    Ok(resp.response.jeu)
}

async fn try_hash_lookup(
    client: &ScreenScraperClient,
    system_id: u32,
    crc32: &str,
    md5: &str,
    sha1: &str,
    filename: &str,
    file_size: u64,
) -> Result<GameInfo, ScrapeError> {
    let mut params = HashMap::new();
    params.insert("systemeid", system_id.to_string());
    params.insert("crc", crc32.to_uppercase());
    params.insert("md5", md5.to_string());
    params.insert("sha1", sha1.to_string());
    params.insert("romnom", filename.to_string());
    params.insert("romtaille", file_size.to_string());
    params.insert("romtype", "rom".to_string());

    let resp = client.lookup_game(params).await?;
    Ok(resp.response.jeu)
}
