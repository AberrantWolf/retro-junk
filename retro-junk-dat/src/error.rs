/// Errors that can occur during DAT file operations.
#[derive(Debug, thiserror::Error)]
pub enum DatError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("XML parse error: {0}")]
    XmlParse(#[from] quick_xml::Error),

    #[error("XML attribute error: {0}")]
    XmlAttribute(#[from] quick_xml::events::attributes::AttrError),

    #[error("Invalid DAT file: {0}")]
    InvalidDat(String),

    #[error("Cache error: {0}")]
    Cache(String),

    #[error("Download failed: {0}")]
    Download(String),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

impl DatError {
    pub fn invalid_dat(msg: impl Into<String>) -> Self {
        Self::InvalidDat(msg.into())
    }

    pub fn cache(msg: impl Into<String>) -> Self {
        Self::Cache(msg.into())
    }

    pub fn download(msg: impl Into<String>) -> Self {
        Self::Download(msg.into())
    }
}
