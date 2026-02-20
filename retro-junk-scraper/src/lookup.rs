use std::collections::HashMap;

use retro_junk_core::Platform;

use crate::client::ScreenScraperClient;
use crate::error::ScrapeError;
use crate::systems::{self, acceptable_system_ids};
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
    /// Serial adapted for ScreenScraper lookups (from `extract_scraper_serial()`)
    pub scraper_serial: Option<String>,
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
    /// Platform identifier
    pub platform: Platform,
    /// Whether this platform's analyzer expects ROMs to have serials
    pub expects_serial: bool,
}

/// Look up a game using the tiered strategy.
///
/// 1. Serial match (preferred) — if serial was extracted from ROM header
/// 2. Filename match — using NoIntro filename + system ID + file size
/// 3. Hash match (conditional) — only for consoles without expected serials,
///    or if force_hash is true
///
/// Each tier validates that the returned game belongs to the expected platform.
/// If a result comes back for the wrong platform (e.g., a serial collision),
/// it's treated as a failed lookup and the next tier is tried.
pub async fn lookup_game(
    client: &ScreenScraperClient,
    system_id: u32,
    rom_info: &RomInfo,
    force_hash: bool,
) -> Result<LookupResult, ScrapeError> {
    let mut warnings = Vec::new();

    // Tier 1: Serial match — try scraper serial first, then raw serial
    let attempts = serial_attempts(&rom_info.serial, &rom_info.scraper_serial);
    if !attempts.is_empty() {
        for attempt in &attempts {
            match try_serial_lookup(client, system_id, attempt).await {
                Ok(game) => {
                    if let Some(warning) = check_platform_mismatch(&game, system_id, rom_info.platform) {
                        warnings.push(format!(
                            "Serial '{}' matched wrong platform: {}; skipping to next lookup method",
                            attempt, warning,
                        ));
                    } else {
                        return Ok(LookupResult {
                            game,
                            method: LookupMethod::Serial,
                            warnings,
                        });
                    }
                }
                Err(ScrapeError::NotFound { .. }) => {
                    warnings.push(format!("Serial '{}' not found in ScreenScraper", attempt));
                }
                Err(e) => return Err(e),
            }
        }

        // All serial attempts failed — add summary warning with ROM serial and tried values
        if let Some(raw) = &rom_info.serial {
            let tried: Vec<&str> = attempts.iter().map(|s| s.as_str()).collect();
            warnings.push(format!(
                "Serial lookup failed \u{2014} ROM serial: \"{}\", tried: [{}]",
                raw,
                tried.join(", "),
            ));
        }
    } else if rom_info.expects_serial {
        warnings.push(format!(
            "Expected serial not found in ROM header for {}",
            rom_info.filename
        ));
    }

    // Tier 2: Filename match
    match try_filename_lookup(client, system_id, &rom_info.filename, rom_info.file_size).await {
        Ok(game) => {
            if let Some(warning) = check_platform_mismatch(&game, system_id, rom_info.platform) {
                warnings.push(format!(
                    "Filename '{}' matched wrong platform: {}; skipping to next lookup method",
                    rom_info.filename, warning,
                ));
            } else {
                return Ok(LookupResult {
                    game,
                    method: LookupMethod::Filename,
                    warnings,
                });
            }
        }
        Err(ScrapeError::NotFound { .. }) => {
            warnings.push(format!(
                "Filename '{}' not found in ScreenScraper",
                rom_info.filename
            ));
        }
        Err(e) => return Err(e),
    }

    // Tier 3: Hash match (conditional)
    let should_hash =
        !systems::expects_serial(rom_info.platform) || force_hash;

    if should_hash {
        if let (Some(crc), Some(md5), Some(sha1)) =
            (&rom_info.crc32, &rom_info.md5, &rom_info.sha1)
        {
            match try_hash_lookup(client, system_id, crc, md5, sha1, &rom_info.filename, rom_info.file_size).await {
                Ok(game) => {
                    if let Some(warning) = check_platform_mismatch(&game, system_id, rom_info.platform) {
                        warnings.push(format!(
                            "Hash matched wrong platform: {}; no more lookup methods available",
                            warning,
                        ));
                    } else {
                        return Ok(LookupResult {
                            game,
                            method: LookupMethod::Hash,
                            warnings,
                        });
                    }
                }
                Err(ScrapeError::NotFound { .. }) => {
                    warnings.push("Hash not found in ScreenScraper".to_string());
                }
                Err(e) => return Err(e),
            }
        }
    }

    Err(ScrapeError::NotFound { warnings })
}

/// Check if the returned game's platform matches the expected system ID.
///
/// Returns `None` if the platform matches (or can't be determined), or a
/// descriptive message if there's a mismatch. Serial numbers can collide
/// across platforms, so this catches cases where e.g. an N64 serial happens
/// to match a SNES game.
fn check_platform_mismatch(
    game: &GameInfo,
    expected_system_id: u32,
    expected_platform: Platform,
) -> Option<String> {
    let systeme = game.systeme.as_ref()?;
    let id_str = systeme.id.as_ref()?;
    let returned_id = id_str.parse::<u32>().ok()?;

    if returned_id != expected_system_id
        && !acceptable_system_ids(expected_platform).contains(&returned_id)
    {
        Some(format!(
            "expected {} (system {}) but got '{}' (system {})",
            expected_platform.display_name(),
            expected_system_id,
            systeme.text,
            returned_id,
        ))
    } else {
        None
    }
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

/// Build a deduped list of serial strings to try, scraper serial first.
///
/// Returns an empty list if neither serial is available.
fn serial_attempts(serial: &Option<String>, scraper_serial: &Option<String>) -> Vec<String> {
    let mut attempts = Vec::new();

    // Scraper serial first (adapted form)
    if let Some(ss) = scraper_serial {
        attempts.push(ss.clone());
    }

    // Raw serial as fallback (if different from scraper serial)
    if let Some(s) = serial {
        if !attempts.iter().any(|a| a == s) {
            attempts.push(s.clone());
        }
    }

    attempts
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serial_attempts_both_different() {
        let serial = Some("NUS-NSME-USA".to_string());
        let scraper = Some("NSME".to_string());
        let attempts = serial_attempts(&serial, &scraper);
        assert_eq!(attempts, vec!["NSME", "NUS-NSME-USA"]);
    }

    #[test]
    fn test_serial_attempts_same_value() {
        let serial = Some("SLUS-01234".to_string());
        let scraper = Some("SLUS-01234".to_string());
        let attempts = serial_attempts(&serial, &scraper);
        assert_eq!(attempts, vec!["SLUS-01234"]);
    }

    #[test]
    fn test_serial_attempts_no_scraper_serial() {
        let serial = Some("NUS-NSME-USA".to_string());
        let scraper = None;
        let attempts = serial_attempts(&serial, &scraper);
        assert_eq!(attempts, vec!["NUS-NSME-USA"]);
    }

    #[test]
    fn test_serial_attempts_no_serial_at_all() {
        let serial: Option<String> = None;
        let scraper: Option<String> = None;
        let attempts = serial_attempts(&serial, &scraper);
        assert!(attempts.is_empty());
    }
}
