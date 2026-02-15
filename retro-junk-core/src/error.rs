use thiserror::Error;

/// Errors that can occur during ROM analysis.
#[derive(Debug, Error)]
pub enum AnalysisError {
    /// I/O error while reading the ROM
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The ROM format is not recognized or is invalid
    #[error("Invalid ROM format: {0}")]
    InvalidFormat(String),

    /// The ROM header is corrupted or incomplete
    #[error("Corrupted header: {0}")]
    CorruptedHeader(String),

    /// The ROM is too small to contain valid data
    #[error("ROM too small: expected at least {expected} bytes, got {actual}")]
    TooSmall { expected: u64, actual: u64 },

    /// Unsupported ROM variant or version
    #[error("Unsupported variant: {0}")]
    UnsupportedVariant(String),

    /// Checksum verification failed
    #[error("Checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },

    /// Progress channel disconnected
    #[error("Progress channel disconnected")]
    ChannelDisconnected,

    /// Generic analysis error with message
    #[error("{0}")]
    Other(String),
}

impl AnalysisError {
    pub fn invalid_format(msg: impl Into<String>) -> Self {
        Self::InvalidFormat(msg.into())
    }

    pub fn corrupted_header(msg: impl Into<String>) -> Self {
        Self::CorruptedHeader(msg.into())
    }

    pub fn unsupported(msg: impl Into<String>) -> Self {
        Self::UnsupportedVariant(msg.into())
    }

    pub fn other(msg: impl Into<String>) -> Self {
        Self::Other(msg.into())
    }
}
