use std::collections::HashMap;

use tokio::sync::Mutex;
use tokio::time::Duration;

use crate::credentials::Credentials;
use crate::error::ScrapeError;
use crate::types::{JeuInfosResponse, UserInfo, UserInfoResponse, UserQuota};

const BASE_URL: &str = "https://api.screenscraper.fr/api2";
const MIN_REQUEST_INTERVAL: Duration = Duration::from_millis(1200);

/// Hard timeout for API requests (covers connect + headers + body read).
const API_TIMEOUT: Duration = Duration::from_secs(30);

/// Timeout for acquiring internal mutex locks (should be near-instant).
const LOCK_TIMEOUT: Duration = Duration::from_secs(5);

/// Maximum number of retries for transient server errors.
const MAX_RETRIES: u32 = 3;

/// Initial backoff duration before first retry (doubles each attempt).
const INITIAL_BACKOFF: Duration = Duration::from_secs(2);

/// Hard timeout for media file downloads.
const MEDIA_TIMEOUT: Duration = Duration::from_secs(120);

/// HTTP client for the ScreenScraper API with rate limiting and quota tracking.
///
/// Concurrency is controlled externally by the caller (e.g., worker pool count
/// or `buffer_unordered` limit). Each API call sleeps for `MIN_REQUEST_INTERVAL`
/// after completing, ensuring per-worker rate limiting.
pub struct ScreenScraperClient {
    http: reqwest::Client,
    creds: Credentials,
    quota: Mutex<Option<UserQuota>>,
}

impl ScreenScraperClient {
    /// Create a new client and validate credentials by calling ssuserInfos.php.
    ///
    /// Returns the client and user info (which includes max_threads for the
    /// caller to configure its own concurrency control).
    pub async fn new(creds: Credentials) -> Result<(Self, UserInfo), ScrapeError> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .read_timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(10)
            .pool_idle_timeout(Duration::from_secs(10))
            .tcp_keepalive(Duration::from_secs(30))
            .tcp_nodelay(true)
            .build()?;

        let client = Self {
            http,
            creds,
            quota: Mutex::new(None),
        };

        let user_info = client.get_user_info().await?;

        Ok((client, user_info))
    }

    /// Get user info and quota from ssuserInfos.php.
    async fn get_user_info(&self) -> Result<UserInfo, ScrapeError> {
        let mut params = self.base_params();
        params.insert("output", "json".to_string());

        let text = self
            .rate_limited_get(&format!("{}/ssuserInfos.php", BASE_URL), &params)
            .await?;

        let status_err = check_auth_status_from_text(&text);
        if let Some(e) = status_err {
            return Err(e);
        }

        let info: UserInfoResponse =
            serde_json::from_str(&text).map_err(|e| ScrapeError::Api(format!("Failed to parse user info: {e}. Response: {}", &text[..text.len().min(200)])))?;

        Ok(info.response.ssuser)
    }

    /// Look up a game by various parameters.
    pub async fn lookup_game(
        &self,
        params: HashMap<&str, String>,
    ) -> Result<JeuInfosResponse, ScrapeError> {
        let mut all_params = self.base_params();
        all_params.insert("output", "json".to_string());
        for (k, v) in params {
            all_params.insert(k, v);
        }

        let text = self
            .rate_limited_get(&format!("{}/jeuInfos.php", BASE_URL), &all_params)
            .await?;

        // Check for error patterns in the response text.
        // ScreenScraper returns HTTP 200 for everything and uses French text
        // to signal errors, so ordering matters here.

        // Empty response is a server glitch, not "game doesn't exist"
        if text.is_empty() {
            return Err(ScrapeError::ServerError {
                status: 200,
                message: "Empty response from API".to_string(),
            });
        }

        // Fatal conditions first — these contain "Erreur" too, so check before
        // the general error handler
        if text.contains("API fermé") || text.contains("API closed") {
            return Err(ScrapeError::ServerClosed(
                "ScreenScraper API is temporarily closed".to_string(),
            ));
        }
        if text.contains("Le quota de scrape journalier") {
            return Err(ScrapeError::QuotaExceeded { used: 0, max: 0 });
        }

        // "Not found" — ScreenScraper uses "non trouvé(e)" for games/ROMs
        // that genuinely don't exist in their database
        if text.contains("non trouvé") {
            return Err(ScrapeError::NotFound { warnings: vec![] });
        }

        // Other "Erreur" messages (login errors, server errors, etc.) are NOT
        // "not found" — treat as retryable server errors so they don't
        // permanently mark releases as missing
        if text.contains("Erreur") {
            return Err(ScrapeError::ServerError {
                status: 200,
                message: format!(
                    "ScreenScraper error: {}",
                    &text[..text.len().min(200)]
                ),
            });
        }

        let response: JeuInfosResponse = serde_json::from_str(&text).map_err(|e| {
            ScrapeError::Api(format!(
                "Failed to parse game info: {e}. Response: {}",
                &text[..text.len().min(200)]
            ))
        })?;

        // Update quota tracking
        if let Some(ref user) = response.response.ssuser {
            match tokio::time::timeout(LOCK_TIMEOUT, self.quota.lock()).await {
                Ok(mut guard) => *guard = Some(user.clone()),
                Err(_) => log::debug!("Quota lock timed out during update"),
            }
        }

        Ok(response)
    }

    /// Download a media file from a URL with a hard timeout.
    ///
    /// Media CDN downloads don't count against the API rate limit, so no
    /// rate limiting is applied here — but we still enforce a total timeout
    /// to prevent hangs when ScreenScraper stalls mid-transfer.
    pub async fn download_media(&self, url: &str) -> Result<Vec<u8>, ScrapeError> {
        tokio::time::timeout(MEDIA_TIMEOUT, async {
            let resp = self.http.get(url).send().await?;
            Ok::<_, reqwest::Error>(resp.bytes().await?.to_vec())
        })
        .await
        .map_err(|_| {
            ScrapeError::Api(format!(
                "Media download timed out after {}s",
                MEDIA_TIMEOUT.as_secs()
            ))
        })?
        .map_err(ScrapeError::from)
    }

    /// Get current quota info if available.
    pub async fn current_quota(&self) -> Option<UserQuota> {
        match tokio::time::timeout(LOCK_TIMEOUT, self.quota.lock()).await {
            Ok(guard) => guard.clone(),
            Err(_) => {
                log::debug!("Quota lock timed out during read");
                None
            }
        }
    }

    /// Perform a rate-limited HTTP GET request with retries for transient errors.
    ///
    /// After each request, sleeps for `MIN_REQUEST_INTERVAL` to enforce
    /// per-worker rate limiting. Concurrency is controlled externally by the
    /// caller (worker pool count or `buffer_unordered` limit).
    ///
    /// On retryable errors (5xx, timeouts, HTML-wrapped errors), backs off
    /// exponentially before the next attempt.
    async fn rate_limited_get(
        &self,
        url: &str,
        params: &HashMap<&str, String>,
    ) -> Result<String, ScrapeError> {
        let mut last_error: Option<ScrapeError> = None;
        let mut consecutive_timeouts: u32 = 0;

        for attempt in 0..=MAX_RETRIES {
            // Back off before retries (not before the first attempt)
            if attempt > 0 {
                let backoff = INITIAL_BACKOFF * 2u32.pow(attempt - 1);
                log::debug!(
                    "Retrying request (attempt {}/{}) after {}s backoff",
                    attempt + 1,
                    MAX_RETRIES + 1,
                    backoff.as_secs(),
                );
                tokio::time::sleep(backoff).await;
            }

            let result = tokio::time::timeout(API_TIMEOUT, async {
                let resp = self
                    .http
                    .get(url)
                    .query(params)
                    .send()
                    .await
                    .map_err(|e| ScrapeError::Api(redact_credentials(&e.to_string())))?;

                let status = resp.status();
                if status == reqwest::StatusCode::UNAUTHORIZED
                    || status == reqwest::StatusCode::FORBIDDEN
                {
                    return Err(ScrapeError::InvalidCredentials(
                        "Credentials rejected".to_string(),
                    ));
                }
                if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    return Err(ScrapeError::RateLimit);
                }
                if status.is_server_error() {
                    return Err(ScrapeError::ServerError {
                        status: status.as_u16(),
                        message: format!("Server returned HTTP {}", status.as_u16()),
                    });
                }

                let text = resp
                    .text()
                    .await
                    .map_err(|e| ScrapeError::ServerError {
                        status: 200,
                        message: format!("Failed to read response body: {}", redact_credentials(&e.to_string())),
                    })?;

                // Detect HTML error pages returned with 200 status (CDN/proxy errors)
                if looks_like_html_error(&text) {
                    return Err(ScrapeError::ServerError {
                        status: 200,
                        message: "Server returned HTML error page instead of JSON".to_string(),
                    });
                }

                Ok(text)
            })
            .await;

            // Rate limit: sleep after each request so this worker doesn't
            // fire another request too quickly.
            tokio::time::sleep(MIN_REQUEST_INTERVAL).await;

            match result {
                Ok(Ok(text)) => return Ok(text),
                Ok(Err(e)) if is_retryable(&e) => {
                    consecutive_timeouts = 0;
                    log::debug!("Transient error: {}", e);
                    last_error = Some(e);
                    continue;
                }
                Ok(Err(e)) => return Err(e),
                Err(_timeout) => {
                    consecutive_timeouts += 1;
                    let e = ScrapeError::Api(format!(
                        "API request timed out after {}s",
                        API_TIMEOUT.as_secs()
                    ));
                    log::debug!("Request timed out ({} consecutive)", consecutive_timeouts);
                    last_error = Some(e);
                    // After 2 consecutive timeouts, connections are likely stale
                    // (e.g., laptop woke from sleep). Stop retrying to recover faster.
                    if consecutive_timeouts >= 2 {
                        break;
                    }
                    continue;
                }
            }
        }

        Err(last_error.unwrap_or_else(|| ScrapeError::Api("All retries exhausted".to_string())))
    }

    fn base_params(&self) -> HashMap<&str, String> {
        let mut params = HashMap::new();
        params.insert("devid", self.creds.dev_id.clone());
        params.insert("devpassword", self.creds.dev_password.clone());
        params.insert("softname", self.creds.soft_name.clone());
        if let Some(ref id) = self.creds.user_id {
            params.insert("ssid", id.clone());
        }
        if let Some(ref pw) = self.creds.user_password {
            params.insert("sspassword", pw.clone());
        }
        params
    }
}

/// Check response text for auth-related error messages.
fn check_auth_status_from_text(text: &str) -> Option<ScrapeError> {
    if text.contains("Erreur de login") || text.contains("Identifiants") {
        Some(ScrapeError::InvalidCredentials(
            "Invalid developer or user credentials".to_string(),
        ))
    } else {
        None
    }
}

/// Check if a response body looks like an HTML error page rather than JSON.
///
/// CDN/proxy servers sometimes return 200 with an HTML error page (e.g., 502 Bad Gateway)
/// instead of a proper HTTP error status.
fn looks_like_html_error(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with("<!DOCTYPE") || trimmed.starts_with("<html") || trimmed.starts_with("<HTML")
}

/// Check if a ScrapeError is retryable (transient server issue).
fn is_retryable(e: &ScrapeError) -> bool {
    matches!(e, ScrapeError::ServerError { .. })
}

/// Redact credential query parameters from error messages that may contain URLs.
///
/// Replaces values for `devpassword`, `sspassword`, `devid`, and `ssid` with `[REDACTED]`.
fn redact_credentials(msg: &str) -> String {
    let mut result = msg.to_string();
    for param in &["devpassword", "sspassword", "devid", "ssid"] {
        // Match param=value where value ends at & or end of string/whitespace
        let prefix = format!("{}=", param);
        while let Some(start) = result.find(&prefix) {
            let value_start = start + prefix.len();
            let value_end = result[value_start..]
                .find(|c: char| c == '&' || c.is_whitespace() || c == '"' || c == '\'')
                .map(|i| value_start + i)
                .unwrap_or(result.len());
            result.replace_range(value_start..value_end, "[REDACTED]");
        }
    }
    result
}
