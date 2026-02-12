/// Checksum algorithms that ROMs may use for self-verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChecksumAlgorithm {
    /// CRC-16 (used by some older systems)
    Crc16,
    /// CRC-32 (common in many formats)
    Crc32,
    /// MD5 (128-bit)
    Md5,
    /// SHA-1 (160-bit)
    Sha1,
    /// SHA-256 (256-bit)
    Sha256,
    /// Simple additive checksum (platform-specific)
    Additive,
    /// Platform-specific checksum algorithm
    PlatformSpecific(&'static str),
}

impl ChecksumAlgorithm {
    pub fn name(&self) -> &str {
        match self {
            Self::Crc16 => "CRC-16",
            Self::Crc32 => "CRC-32",
            Self::Md5 => "MD5",
            Self::Sha1 => "SHA-1",
            Self::Sha256 => "SHA-256",
            Self::Additive => "Additive",
            Self::PlatformSpecific(name) => name,
        }
    }
}

/// A checksum value expected by the ROM for self-verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedChecksum {
    /// The algorithm used for this checksum
    pub algorithm: ChecksumAlgorithm,
    /// The expected checksum value as raw bytes
    pub value: Vec<u8>,
    /// Optional description of what this checksum covers
    pub description: Option<String>,
}

impl ExpectedChecksum {
    pub fn new(algorithm: ChecksumAlgorithm, value: Vec<u8>) -> Self {
        Self {
            algorithm,
            value,
            description: None,
        }
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Returns the checksum value as a hex string.
    pub fn hex_value(&self) -> String {
        self.value
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect()
    }
}
