use thiserror::Error;

/// Errors that can occur during CLI command execution.
#[derive(Debug, Error)]
pub(crate) enum CliError {
    /// I/O error
    #[error("{0}")]
    Io(#[from] std::io::Error),

    /// Database operation failed
    #[error("Database error: {0}")]
    Database(String),

    /// Unknown or unsupported system name
    #[error("Unknown system: {0}")]
    UnknownSystem(String),

    /// Configuration error
    #[error("Config error: {0}")]
    Config(String),

    /// Runtime creation or async error
    #[error("Runtime error: {0}")]
    Runtime(String),

    /// DAT file error
    #[error("DAT error: {0}")]
    DatError(String),

    /// Analysis error
    #[error("Analysis error: {0}")]
    Analysis(String),

    /// Catch-all for other errors
    #[error("{0}")]
    Other(String),
}

impl CliError {
    pub(crate) fn database(msg: impl Into<String>) -> Self {
        Self::Database(msg.into())
    }

    pub(crate) fn unknown_system(msg: impl Into<String>) -> Self {
        Self::UnknownSystem(msg.into())
    }

    pub(crate) fn config(msg: impl Into<String>) -> Self {
        Self::Config(msg.into())
    }

    pub(crate) fn runtime(msg: impl Into<String>) -> Self {
        Self::Runtime(msg.into())
    }

    pub(crate) fn dat_error(msg: impl Into<String>) -> Self {
        Self::DatError(msg.into())
    }

    pub(crate) fn analysis(msg: impl Into<String>) -> Self {
        Self::Analysis(msg.into())
    }

    pub(crate) fn other(msg: impl Into<String>) -> Self {
        Self::Other(msg.into())
    }
}
