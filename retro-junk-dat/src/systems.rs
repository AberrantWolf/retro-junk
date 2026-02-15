/// Rules for normalizing byte order before hashing.
///
/// Some platforms have multiple ROM dump formats with different byte orderings.
/// NoIntro DATs catalog a single canonical byte order, so we must normalize
/// before hashing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteOrderRule {
    /// No byte-order normalization needed
    None,
    /// Detect N64 byte order from the first 4 bytes and normalize to big-endian (.z64).
    /// - `80 37 12 40` → .z64 (big-endian), no swap
    /// - `37 80 40 12` → .v64, swap byte pairs
    /// - `40 12 37 80` → .n64, reverse 4-byte groups
    N64,
}

/// Rules for detecting and stripping ROM headers before hashing.
///
/// NoIntro DATs generally catalog headerless ROMs (the pure ROM data).
/// Some formats have copier headers or format headers that must be stripped
/// before hashing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderDetect {
    /// No header to strip (ROM data starts at byte 0)
    None,
    /// Always skip a fixed number of bytes (e.g., 16-byte iNES header)
    Fixed { skip: u64 },
    /// SNES copier headers: if `file_size % modulo` equals `remainder`,
    /// skip `skip` bytes from the start.
    SizeModulo {
        modulo: u64,
        remainder: u64,
        skip: u64,
    },
}

impl HeaderDetect {
    /// Calculate the number of bytes to skip for header stripping.
    pub fn header_size(&self, file_size: u64) -> u64 {
        match self {
            HeaderDetect::None => 0,
            HeaderDetect::Fixed { skip } => *skip,
            HeaderDetect::SizeModulo {
                modulo,
                remainder,
                skip,
            } => {
                if file_size % modulo == *remainder {
                    *skip
                } else {
                    0
                }
            }
        }
    }
}

/// Mapping from a retro-junk short_name to a NoIntro DAT file name and header rules.
#[derive(Debug, Clone)]
pub struct SystemMapping {
    /// retro-junk analyzer short_name (e.g., "nes", "snes")
    pub short_name: &'static str,
    /// NoIntro DAT name (used in the XML and for the GitHub mirror filename)
    pub dat_name: &'static str,
    /// Header detection/stripping rule for hash-based matching
    pub header: HeaderDetect,
    /// Byte-order normalization rule (e.g., N64 byteswap)
    pub byte_order: ByteOrderRule,
}

/// All supported system mappings for cartridge-based platforms.
pub static SYSTEMS: &[SystemMapping] = &[
    SystemMapping {
        short_name: "nes",
        dat_name: "Nintendo - Nintendo Entertainment System",
        // The GitHub mirror hosts the headerless DAT, so strip the 16-byte
        // iNES/NES 2.0 header before hashing. The header is always exactly
        // 16 bytes (no trainer consideration — trainer is part of ROM data
        // in headerless dumps, but iNES files in the wild always have the
        // 16-byte header prefix).
        header: HeaderDetect::Fixed { skip: 16 },
        byte_order: ByteOrderRule::None,
    },
    SystemMapping {
        short_name: "snes",
        dat_name: "Nintendo - Super Nintendo Entertainment System",
        header: HeaderDetect::SizeModulo {
            modulo: 1024,
            remainder: 512,
            skip: 512,
        },
        byte_order: ByteOrderRule::None,
    },
    SystemMapping {
        short_name: "n64",
        dat_name: "Nintendo - Nintendo 64",
        header: HeaderDetect::None,
        byte_order: ByteOrderRule::N64,
    },
    SystemMapping {
        short_name: "gb",
        dat_name: "Nintendo - Game Boy",
        header: HeaderDetect::None,
        byte_order: ByteOrderRule::None,
    },
    SystemMapping {
        short_name: "gbc",
        dat_name: "Nintendo - Game Boy Color",
        header: HeaderDetect::None,
        byte_order: ByteOrderRule::None,
    },
    SystemMapping {
        short_name: "gba",
        dat_name: "Nintendo - Game Boy Advance",
        header: HeaderDetect::None,
        byte_order: ByteOrderRule::None,
    },
    SystemMapping {
        short_name: "nds",
        dat_name: "Nintendo - Nintendo DS Decrypted",
        header: HeaderDetect::None,
        byte_order: ByteOrderRule::None,
    },
    SystemMapping {
        short_name: "genesis",
        dat_name: "Sega - Mega Drive - Genesis",
        header: HeaderDetect::None,
        byte_order: ByteOrderRule::None,
    },
    SystemMapping {
        short_name: "mastersystem",
        dat_name: "Sega - Master System - Mark III",
        header: HeaderDetect::None,
        byte_order: ByteOrderRule::None,
    },
    SystemMapping {
        short_name: "gamegear",
        dat_name: "Sega - Game Gear",
        header: HeaderDetect::None,
        byte_order: ByteOrderRule::None,
    },
    SystemMapping {
        short_name: "32x",
        dat_name: "Sega - 32X",
        header: HeaderDetect::None,
        byte_order: ByteOrderRule::None,
    },
];

/// Look up a system mapping by its short_name.
pub fn find_system(short_name: &str) -> Option<&'static SystemMapping> {
    SYSTEMS
        .iter()
        .find(|s| s.short_name.eq_ignore_ascii_case(short_name))
}

/// Get all supported short names.
pub fn supported_short_names() -> Vec<&'static str> {
    SYSTEMS.iter().map(|s| s.short_name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_system() {
        let nes = find_system("nes").unwrap();
        assert_eq!(
            nes.dat_name,
            "Nintendo - Nintendo Entertainment System"
        );
        assert_eq!(nes.header, HeaderDetect::Fixed { skip: 16 });
    }

    #[test]
    fn test_snes_header_detect() {
        let snes = find_system("snes").unwrap();
        // File with copier header: 512 + 1024*N → file_size % 1024 == 512
        assert_eq!(snes.header.header_size(524800), 512); // 512 + 512*1024
        // File without copier header: 1024*N → file_size % 1024 == 0
        assert_eq!(snes.header.header_size(524288), 0); // 512*1024
    }

    #[test]
    fn test_case_insensitive_lookup() {
        assert!(find_system("NES").is_some());
        assert!(find_system("Snes").is_some());
    }

    #[test]
    fn test_unknown_system() {
        assert!(find_system("turbografx").is_none());
    }
}
