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

    /// Unknown or unsupported system name. The message names the system and
    /// how to see the valid ones, so it is printed as given rather than
    /// prefixed again.
    #[error("{0}")]
    UnknownSystem(String),

    /// Configuration error
    #[error("Config error: {0}")]
    Config(String),

    /// Runtime creation or async error
    #[error("Runtime error: {0}")]
    Runtime(String),

    /// Catch-all for other errors
    #[error("{0}")]
    Other(String),

    /// An external tool required by the command is missing or unusable
    /// (e.g. chdman). Distinct from `UnknownSystem`, which concerns
    /// unrecognized *platforms*, not missing binaries.
    #[error("{0}")]
    ExternalTool(String),
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

    pub(crate) fn other(msg: impl Into<String>) -> Self {
        Self::Other(msg.into())
    }

    pub(crate) fn external_tool(msg: impl Into<String>) -> Self {
        Self::ExternalTool(msg.into())
    }
}
