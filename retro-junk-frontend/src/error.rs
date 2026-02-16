/// Errors that can occur during frontend metadata generation.
#[derive(Debug, thiserror::Error)]
pub enum FrontendError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("XML writing error: {0}")]
    Xml(String),

    #[error("Invalid metadata: {0}")]
    InvalidMetadata(String),
}
