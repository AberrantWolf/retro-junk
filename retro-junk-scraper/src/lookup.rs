use std::collections::HashMap;

use retro_junk_core::Platform;
use tokio::time::Duration;

use crate::client::ScreenScraperClient;
use crate::error::ScrapeError;
use crate::systems::acceptable_system_ids;
use crate::types::GameInfo;

/// Maximum time for the entire multi-tier lookup (serial + filename + hash).
/// Each individual API call has a 30s timeout with retries, but the overall
/// lookup across all tiers should not exceed this. Covers ~2 full tiers with
/// retries; if the server is this slow, continuing is unlikely to help.
const LOOKUP_TIMEOUT: Duration = Duration::from_mins(1);

/// How a game was matched in `ScreenScraper`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookupMethod {
    /// Matched by serial number from ROM header
    Serial,
    /// Matched by filename (`NoIntro` name)
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

/// The full hash triple required for a `ScreenScraper` hash lookup.
///
/// The API's hash tier needs CRC32, MD5, and SHA1 together, so partial
/// combinations are unrepresentable by construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomHashes {
    /// CRC32 hash (uppercase hex)
    pub crc32: String,
    /// MD5 hash (lowercase hex)
    pub md5: String,
    /// SHA1 hash (lowercase hex)
    pub sha1: String,
}

impl RomHashes {
    /// The triple, or nothing at all.
    ///
    /// The hash tier matches on CRC32, MD5, and SHA-1 together, so a partial
    /// set is worth exactly as much as a filename — and offering one spends a
    /// request to be told so. Every caller that assembles hashes from a
    /// catalog row or a library entry asks this rather than restating it.
    #[must_use]
    pub fn complete(crc32: &str, md5: &str, sha1: &str) -> Option<Self> {
        (!crc32.is_empty() && !md5.is_empty() && !sha1.is_empty()).then(|| Self {
            crc32: crc32.to_owned(),
            md5: md5.to_owned(),
            sha1: sha1.to_owned(),
        })
    }
}

/// Information about a ROM file for lookup purposes.
#[derive(Debug, Clone)]
pub struct RomInfo {
    /// Serial number from ROM header analysis (empty = none)
    pub serial: String,
    /// Serial adapted for `ScreenScraper` lookups (from `extract_scraper_serial()`;
    /// empty = none)
    pub scraper_serial: String,
    /// ROM filename with extension
    pub filename: String,
    /// ROM file size in bytes
    pub file_size: u64,
    /// Hash triple for the hash lookup tier (None = hashes not computed)
    pub hashes: Option<RomHashes>,
    /// Platform identifier
    pub platform: Platform,
    /// Whether this platform's analyzer expects ROMs to have serials
    pub expects_serial: bool,
}

/// Look up a game using the tiered strategy.
///
/// 1. Serial match (preferred) — if serial was extracted from ROM header
/// 2. Hash match — if CRC32 + MD5 + SHA1 are available (e.g., from DAT entries).
///    Skipped naturally when hashes are None (e.g., scrape path for serial
///    consoles that skip hashing for speed).
/// 3. Filename match — last resort using `NoIntro` filename + system ID + file size
///
/// Each tier validates that the returned game belongs to the expected platform.
/// If a result comes back for the wrong platform (e.g., a serial collision),
/// it's treated as a failed lookup and the next tier is tried.
pub async fn lookup_game(
    client: &ScreenScraperClient,
    system_id: u32,
    rom_info: &RomInfo,
) -> Result<LookupResult, ScrapeError> {
    let filename = &rom_info.filename;
    log::debug!(
        "lookup_game: '{}' (system {}, timeout {}s)",
        filename,
        system_id,
        LOOKUP_TIMEOUT.as_secs(),
    );

    tokio::time::timeout(
        LOOKUP_TIMEOUT,
        lookup_game_tiers(client, system_id, rom_info),
    )
    .await
    .map_err(|_| {
        ScrapeError::Api(format!(
            "Lookup timed out after {}s",
            LOOKUP_TIMEOUT.as_secs()
        ))
    })?
}

/// Accept a tier's matched game unless it belongs to the wrong platform.
///
/// On a platform mismatch, records a warning (built from the mismatch text by
/// `mismatch_msg`) and returns `None` so the caller falls through to the next
/// tier. Otherwise returns the finished [`LookupResult`], taking ownership of
/// the accumulated warnings.
fn accept_tier_match(
    game: GameInfo,
    method: LookupMethod,
    system_id: u32,
    platform: Platform,
    warnings: &mut Vec<String>,
    mismatch_msg: impl FnOnce(&str) -> String,
) -> Option<LookupResult> {
    if let Some(warning) = check_platform_mismatch(&game, system_id, platform) {
        warnings.push(mismatch_msg(&warning));
        None
    } else {
        Some(LookupResult {
            game,
            method,
            warnings: std::mem::take(warnings),
        })
    }
}

/// Tier 1: Serial match — try scraper serial first, then raw serial.
///
/// Returns the accepted match, or `Ok(None)` (with warnings recorded) to fall
/// through to the next tier.
async fn serial_tier(
    client: &ScreenScraperClient,
    system_id: u32,
    rom_info: &RomInfo,
    warnings: &mut Vec<String>,
) -> Result<Option<LookupResult>, ScrapeError> {
    let attempts = serial_attempts(&rom_info.serial, &rom_info.scraper_serial);
    if attempts.is_empty() {
        if rom_info.expects_serial {
            warnings.push(format!(
                "Expected serial not found in ROM header for {}",
                rom_info.filename
            ));
        }
        return Ok(None);
    }

    log::debug!(
        "Tier 1 (serial): trying {} attempt(s) for '{}'",
        attempts.len(),
        rom_info.filename
    );
    for attempt in &attempts {
        match try_serial_lookup(client, system_id, attempt).await {
            Ok(game) => {
                if let Some(result) = accept_tier_match(
                    game,
                    LookupMethod::Serial,
                    system_id,
                    rom_info.platform,
                    warnings,
                    |warning| {
                        format!(
                            "Serial '{attempt}' matched wrong platform: {warning}; skipping to next lookup method",
                        )
                    },
                ) {
                    return Ok(Some(result));
                }
            }
            Err(ScrapeError::NotFound { .. }) => {
                warnings.push(format!("Serial '{attempt}' not found in ScreenScraper"));
            }
            Err(e) => return Err(e),
        }
    }

    // All serial attempts failed — add summary warning with ROM serial and tried values
    if !rom_info.serial.is_empty() {
        let tried: Vec<&str> = attempts.iter().map(std::string::String::as_str).collect();
        warnings.push(format!(
            "Serial lookup failed \u{2014} ROM serial: \"{}\", tried: [{}]",
            rom_info.serial,
            tried.join(", "),
        ));
    }
    Ok(None)
}

/// The tiered lookup body of [`lookup_game`], without the overall timeout.
async fn lookup_game_tiers(
    client: &ScreenScraperClient,
    system_id: u32,
    rom_info: &RomInfo,
) -> Result<LookupResult, ScrapeError> {
    let filename = &rom_info.filename;
    let mut warnings = Vec::new();

    if let Some(result) = serial_tier(client, system_id, rom_info, &mut warnings).await? {
        return Ok(result);
    }

    // Tier 2: Hash match — most reliable when hashes are available (e.g.,
    // from DAT entries). Skipped naturally when hashes are None (e.g., scrape
    // path for serial consoles that skip hashing for speed).
    if let Some(hashes) = &rom_info.hashes {
        log::debug!("Tier 2 (hash): trying for '{filename}'");
        match try_hash_lookup(
            client,
            system_id,
            hashes,
            &rom_info.filename,
            rom_info.file_size,
        )
        .await
        {
            Ok(game) => {
                if let Some(result) = accept_tier_match(
                    game,
                    LookupMethod::Hash,
                    system_id,
                    rom_info.platform,
                    &mut warnings,
                    |warning| {
                        format!(
                            "Hash matched wrong platform: {warning}; skipping to next lookup method"
                        )
                    },
                ) {
                    return Ok(result);
                }
            }
            Err(ScrapeError::NotFound { .. }) => {
                warnings.push("Hash not found in ScreenScraper".to_string());
            }
            Err(e) => return Err(e),
        }
    }

    // Tier 3: Filename match — last resort fallback when serial and hash
    // lookups fail or aren't available.
    log::debug!("Tier 3 (filename): trying for '{filename}'");
    match try_filename_lookup(client, system_id, &rom_info.filename, rom_info.file_size).await {
        Ok(game) => {
            if let Some(result) = accept_tier_match(
                game,
                LookupMethod::Filename,
                system_id,
                rom_info.platform,
                &mut warnings,
                |warning| {
                    format!(
                        "Filename '{}' matched wrong platform: {warning}; no more lookup methods available",
                        rom_info.filename,
                    )
                },
            ) {
                return Ok(result);
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
    insert_file_size(&mut params, file_size);
    params.insert("romtype", "rom".to_string());

    let resp = client.lookup_game(params).await?;
    Ok(resp.response.jeu)
}

/// Send the file size only when one is known.
///
/// `romtaille` narrows a name match to files of that size, which is exactly
/// wrong when the size belongs to a different file than the name does — a mod
/// asked about as its parent knows the parent's name but not its size. Sending
/// `0` would claim an empty file.
fn insert_file_size(params: &mut HashMap<&'static str, String>, file_size: u64) {
    if file_size > 0 {
        params.insert("romtaille", file_size.to_string());
    }
}

/// Build a deduped list of serial strings to try, scraper serial first.
///
/// Empty serials are skipped; returns an empty list if neither is available.
fn serial_attempts(serial: &str, scraper_serial: &str) -> Vec<String> {
    let mut attempts = Vec::new();

    // Scraper serial first (adapted form)
    if !scraper_serial.is_empty() {
        attempts.push(scraper_serial.to_string());
    }

    // Raw serial as fallback (if different from scraper serial)
    if !serial.is_empty() && !attempts.iter().any(|a| a == serial) {
        attempts.push(serial.to_string());
    }

    attempts
}

async fn try_hash_lookup(
    client: &ScreenScraperClient,
    system_id: u32,
    hashes: &RomHashes,
    filename: &str,
    file_size: u64,
) -> Result<GameInfo, ScrapeError> {
    let mut params = HashMap::new();
    params.insert("systemeid", system_id.to_string());
    params.insert("crc", hashes.crc32.to_uppercase());
    params.insert("md5", hashes.md5.clone());
    params.insert("sha1", hashes.sha1.clone());
    params.insert("romnom", filename.to_string());
    insert_file_size(&mut params, file_size);
    params.insert("romtype", "rom".to_string());

    let resp = client.lookup_game(params).await?;
    Ok(resp.response.jeu)
}

#[cfg(test)]
#[path = "tests/lookup_tests.rs"]
mod tests;
