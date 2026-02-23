/// Errors that can occur during scraping operations.
#[derive(Debug, thiserror::Error)]
pub enum ScrapeError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Rate limited by ScreenScraper API")]
    RateLimit,

    #[error("Daily quota exceeded ({used}/{max} requests)")]
    QuotaExceeded { used: u32, max: u32 },

    #[error("Game not found in ScreenScraper database")]
    NotFound { warnings: Vec<String> },

    #[error("Invalid credentials: {0}")]
    InvalidCredentials(String),

    #[error("ScreenScraper server is closed: {0}")]
    ServerClosed(String),

    #[error("Server error (HTTP {status}): {message}")]
    ServerError { status: u16, message: String },

    #[error("API error: {0}")]
    Api(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON parsing error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Analysis error: {0}")]
    Analysis(String),
}
