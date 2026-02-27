//! Display helpers for formatting ROM identification data.
//!
//! Extracts pure data-transformation logic from CLI/GUI so both can share
//! the same size verdicts, key prettification, and hardware key ordering.

use retro_junk_core::util::format_bytes;

// ---------------------------------------------------------------------------
// Size verdict
// ---------------------------------------------------------------------------

/// Result of comparing a file's actual size to its expected size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SizeVerdict {
    /// Sizes match exactly.
    Ok,
    /// File is smaller — trailing data was stripped (still mostly complete,
    /// size is a power-of-2 boundary).
    Trimmed { missing: u64 },
    /// File is smaller — significant data is missing.
    Truncated { missing: u64 },
    /// File has exactly 512 extra bytes (likely a copier header).
    CopierHeader,
    /// File is larger than expected.
    Oversized { excess: u64 },
}

impl SizeVerdict {
    /// Plain-text description of the verdict (no ANSI colors).
    pub fn description(&self) -> String {
        match self {
            SizeVerdict::Ok => "OK".into(),
            SizeVerdict::Trimmed { missing } => {
                format!(
                    "TRIMMED (-{}, trailing data stripped)",
                    format_bytes(*missing)
                )
            }
            SizeVerdict::Truncated { missing } => {
                format!("TRUNCATED (missing {})", format_bytes(*missing))
            }
            SizeVerdict::CopierHeader => "OVERSIZED (+512 bytes, likely copier header)".into(),
            SizeVerdict::Oversized { excess } => {
                format!("OVERSIZED (+{})", format_bytes(*excess))
            }
        }
    }

    /// Whether this verdict represents a problem (not OK).
    pub fn is_problem(&self) -> bool {
        !matches!(self, SizeVerdict::Ok)
    }

    /// Whether this verdict is a warning (trimmed, copier header, oversized)
    /// rather than an error (truncated).
    pub fn is_warning(&self) -> bool {
        matches!(
            self,
            SizeVerdict::Trimmed { .. } | SizeVerdict::CopierHeader | SizeVerdict::Oversized { .. }
        )
    }

    /// Whether this verdict is an error (truncated — data missing).
    pub fn is_error(&self) -> bool {
        matches!(self, SizeVerdict::Truncated { .. })
    }
}

fn is_power_of_two(n: u64) -> bool {
    n > 0 && (n & (n - 1)) == 0
}

/// Compare actual vs expected file size and return a verdict.
pub fn compute_size_verdict(file_size: u64, expected_size: u64) -> SizeVerdict {
    if file_size == expected_size {
        return SizeVerdict::Ok;
    }

    if file_size < expected_size {
        let missing = expected_size - file_size;

        // Likely trimmed: file still has most data AND file size is a power of 2
        // OR the missing amount is a power-of-2 fraction of expected size
        let has_most_data = file_size >= expected_size / 2;
        let file_is_pow2 = is_power_of_two(file_size);
        let missing_is_pow2_fraction =
            is_power_of_two(missing) && is_power_of_two(expected_size) && missing < expected_size;

        if has_most_data && (file_is_pow2 || missing_is_pow2_fraction) {
            SizeVerdict::Trimmed { missing }
        } else {
            SizeVerdict::Truncated { missing }
        }
    } else {
        let excess = file_size - expected_size;
        if excess == 512 {
            SizeVerdict::CopierHeader
        } else {
            SizeVerdict::Oversized { excess }
        }
    }
}

// ---------------------------------------------------------------------------
// Key prettification
// ---------------------------------------------------------------------------

/// Known acronyms that should stay uppercase when prettifying keys.
pub const ACRONYMS: &[&str] = &[
    "PRG", "CHR", "RAM", "ROM", "SRAM", "NVRAM", "SGB", "CGB", "TV", "ID",
];

/// Convert a snake_case key to Title Case, keeping known acronyms uppercase.
///
/// Examples: `prg_rom_size` → `PRG ROM Size`, `tv_system` → `TV System`.
pub fn prettify_key(key: &str) -> String {
    key.split('_')
        .filter(|s| !s.is_empty())
        .map(|word| {
            let upper = word.to_uppercase();
            if ACRONYMS.contains(&upper.as_str()) {
                upper
            } else {
                let mut chars = word.chars();
                match chars.next() {
                    Some(c) => {
                        let mut s = c.to_uppercase().to_string();
                        s.extend(chars);
                        s
                    }
                    None => String::new(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------------------
// Hardware keys (ordered display)
// ---------------------------------------------------------------------------

/// Known hardware/technical extra keys, in display order.
///
/// These are the `extra` map keys that represent hardware characteristics
/// rather than identification data. Used by display code to group them
/// separately from identity/detail fields.
pub const HARDWARE_KEYS: &[&str] = &[
    "mapping",
    "speed",
    "chipset",
    "coprocessor",
    "mirroring",
    "cartridge_type",
    "rom_size",
    "prg_rom_size",
    "chr_rom_size",
    "sram_size",
    "ram_size",
    "prg_ram_size",
    "prg_nvram_size",
    "chr_ram_size",
    "chr_nvram_size",
    "expansion_ram",
    "expansion_device",
    "battery",
    "trainer",
    "sgb",
    "console_type",
    "tv_system",
    "copier_header",
    "checksum_complement_valid",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_size_verdict_ok() {
        assert_eq!(compute_size_verdict(1024, 1024), SizeVerdict::Ok);
    }

    #[test]
    fn test_size_verdict_truncated() {
        let v = compute_size_verdict(100, 1024);
        assert!(matches!(v, SizeVerdict::Truncated { missing: 924 }));
        assert!(v.is_error());
    }

    #[test]
    fn test_size_verdict_copier_header() {
        assert_eq!(
            compute_size_verdict(1024 + 512, 1024),
            SizeVerdict::CopierHeader
        );
    }

    #[test]
    fn test_prettify_key_basic() {
        assert_eq!(prettify_key("prg_rom_size"), "PRG ROM Size");
        assert_eq!(prettify_key("tv_system"), "TV System");
        assert_eq!(prettify_key("expansion_device"), "Expansion Device");
    }

    #[test]
    fn test_prettify_key_single_word() {
        assert_eq!(prettify_key("battery"), "Battery");
        assert_eq!(prettify_key("sgb"), "SGB");
    }
}
