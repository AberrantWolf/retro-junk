/// Progress update sent during ROM analysis.
///
/// These updates are sent via MPSC channel for GUI applications
/// to display progress during analysis of large files.
#[derive(Debug, Clone)]
pub enum AnalysisProgress {
    /// Analysis has started
    Started {
        /// Total size of the file being analyzed (if known)
        total_bytes: Option<u64>,
    },

    /// Currently reading/processing data
    Reading {
        /// Bytes processed so far
        bytes_read: u64,
        /// Total bytes to process (if known)
        total_bytes: Option<u64>,
    },

    /// A specific analysis phase has started
    Phase {
        /// Name of the current phase
        name: String,
        /// Optional phase number (e.g., 1 of 3)
        current: Option<u32>,
        /// Optional total number of phases
        total: Option<u32>,
    },

    /// Intermediate finding during analysis
    Found {
        /// What was found
        description: String,
    },

    /// Analysis completed successfully
    Completed,

    /// Analysis failed with an error
    Failed {
        /// Error message
        message: String,
    },
}

impl AnalysisProgress {
    pub fn started(total_bytes: Option<u64>) -> Self {
        Self::Started { total_bytes }
    }

    pub fn reading(bytes_read: u64, total_bytes: Option<u64>) -> Self {
        Self::Reading {
            bytes_read,
            total_bytes,
        }
    }

    pub fn phase(name: impl Into<String>) -> Self {
        Self::Phase {
            name: name.into(),
            current: None,
            total: None,
        }
    }

    pub fn phase_numbered(name: impl Into<String>, current: u32, total: u32) -> Self {
        Self::Phase {
            name: name.into(),
            current: Some(current),
            total: Some(total),
        }
    }

    pub fn found(description: impl Into<String>) -> Self {
        Self::Found {
            description: description.into(),
        }
    }

    pub fn failed(message: impl Into<String>) -> Self {
        Self::Failed {
            message: message.into(),
        }
    }

    /// Returns the progress percentage (0.0 to 1.0) if calculable.
    pub fn percentage(&self) -> Option<f64> {
        match self {
            Self::Reading {
                bytes_read,
                total_bytes: Some(total),
            } if *total > 0 => Some(*bytes_read as f64 / *total as f64),
            Self::Completed => Some(1.0),
            _ => None,
        }
    }
}
