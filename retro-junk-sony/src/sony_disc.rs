//! PlayStation-specific disc utilities.
//!
//! SYSTEM.CNF parsing, serial extraction, and region mapping.
//! Generic disc utilities (ISO 9660, CUE, CHD, hashing) live in `retro-junk-disc`.

use retro_junk_core::{AnalysisError, Region};

// Re-export disc types used by PS1/PS2 analyzers for convenience.
pub use retro_junk_disc::chd::{find_file_in_chd, read_chd_info};
pub use retro_junk_disc::cue::parse_cue;
pub use retro_junk_disc::format::{DiscFormat, detect_disc_format};
pub use retro_junk_disc::hash::hash_disc_container;
pub use retro_junk_disc::iso9660::{find_file_in_root, read_pvd};

// ---------------------------------------------------------------------------
// SYSTEM.CNF parsing
// ---------------------------------------------------------------------------

/// Which key was used for the boot path in SYSTEM.CNF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootKey {
    /// PS1: `BOOT = cdrom:\...`
    Boot,
    /// PS2: `BOOT2 = cdrom0:\...`
    Boot2,
}

/// Parsed SYSTEM.CNF contents.
#[derive(Debug, Clone)]
pub struct SystemCnf {
    /// Boot executable path, e.g. "cdrom:\SLUS_012.34;1"
    pub boot_path: String,
    /// Which key was used (`BOOT` for PS1, `BOOT2` for PS2).
    pub boot_key: BootKey,
    /// Video mode from VMODE key. Empty when absent.
    pub vmode: String,
}

/// Parse the contents of a SYSTEM.CNF file.
pub fn parse_system_cnf(content: &str) -> Result<SystemCnf, AnalysisError> {
    let mut boot_path = None;
    let mut boot_key = None;
    let mut vmode = String::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim().to_uppercase();
            let value = value.trim();

            match key.as_str() {
                "BOOT2" => {
                    // BOOT2 is more specific (PS2); prefer it if both are present
                    boot_path = Some(value.to_string());
                    boot_key = Some(BootKey::Boot2);
                }
                "BOOT" => {
                    if boot_path.is_none() {
                        boot_path = Some(value.to_string());
                        boot_key = Some(BootKey::Boot);
                    }
                }
                "VMODE" => {
                    vmode = value.to_string();
                }
                _ => {}
            }
        }
    }

    match (boot_path, boot_key) {
        (Some(path), Some(key)) => Ok(SystemCnf {
            boot_path: path,
            boot_key: key,
            vmode,
        }),
        _ => Err(AnalysisError::corrupted_header(
            "SYSTEM.CNF missing BOOT= line",
        )),
    }
}

// ---------------------------------------------------------------------------
// Serial extraction and region mapping
// ---------------------------------------------------------------------------

/// Extract a normalized serial from a SYSTEM.CNF boot path.
///
/// Input: `"cdrom:\SLUS_012.34;1"` or `"cdrom:\\SLUS_012.34;1"` or `"cdrom:SLUS_006.91;1"`
/// Output: `"SLUS-01234"`
pub fn extract_serial(boot_path: &str) -> Option<String> {
    // Find the filename part (after last \, /, or : to handle all SYSTEM.CNF variants)
    let filename = boot_path.rsplit(['\\', '/', ':']).next()?;

    // Strip version suffix (";1")
    let filename = filename.split(';').next().unwrap_or(filename);

    // Match pattern like "SLUS_012.34" or "SLUS_01234" or "SCUS_012.34"
    let filename = filename.trim();
    if filename.len() < 8 {
        return None;
    }

    let prefix = &filename[..4];
    if !is_sony_serial_prefix(prefix) {
        return None;
    }

    // Extract digits after the prefix+separator
    let rest = &filename[4..];
    let digits: String = rest.chars().filter(|c| c.is_ascii_digit()).collect();

    if digits.len() >= 5 {
        Some(format!("{}-{}", prefix.to_uppercase(), digits))
    } else {
        None
    }
}

/// Check if a 4-character prefix is a known Sony serial prefix.
fn is_sony_serial_prefix(prefix: &str) -> bool {
    let upper = prefix.to_uppercase();
    matches!(
        upper.as_str(),
        "SLUS"
            | "SCUS"
            | "SLPS"
            | "SCPS"
            | "SLPM"
            | "SLES"
            | "SCES"
            | "SCED"
            | "SLKA"
            | "SCKA"
            | "PAPX"
            | "PCPX"
            | "SIPS"
    )
}

/// Map a PS1/PS2 serial prefix to a region.
pub fn serial_to_region(serial: &str) -> Option<Region> {
    if serial.len() < 4 {
        return None;
    }
    let prefix = serial[..4].to_uppercase();
    match prefix.as_str() {
        "SLUS" | "SCUS" => Some(Region::Usa),
        "SLPS" | "SCPS" | "SLPM" | "SIPS" => Some(Region::Japan),
        "SLES" | "SCES" | "SCED" => Some(Region::Europe),
        "SLKA" | "SCKA" => Some(Region::Korea),
        "PAPX" | "PCPX" => Some(Region::Japan), // dev/promo discs, usually Japanese
        _ => None,
    }
}

/// Find and read SYSTEM.CNF from a CHD disc image.
///
/// Thin wrapper around `retro_junk_disc::chd::find_file_in_chd` that
/// validates the "PLAYSTATION" system identifier.
pub fn read_system_cnf_from_chd(
    reader: &mut dyn retro_junk_core::ReadSeek,
) -> Result<Vec<u8>, AnalysisError> {
    let (pvd, content) = find_file_in_chd(reader, "SYSTEM.CNF")?;

    if !pvd.system_identifier.starts_with("PLAYSTATION") {
        return Err(AnalysisError::invalid_format(format!(
            "Not a PlayStation disc (system ID: '{}')",
            pvd.system_identifier,
        )));
    }

    Ok(content)
}

#[cfg(test)]
#[path = "tests/sony_disc_tests.rs"]
mod tests;
