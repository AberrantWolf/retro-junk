use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, Semaphore};
use tokio::time::Duration;

use crate::credentials::Credentials;
use crate::error::ScrapeError;
use crate::types::{JeuInfosResponse, UserInfo, UserInfoResponse, UserQuota};

const BASE_URL: &str = "https://api.screenscraper.fr/api2";
const MIN_REQUEST_INTERVAL: Duration = Duration::from_millis(1200);

/// Hard timeout for API requests (covers connect + headers + body read).
const API_TIMEOUT: Duration = Duration::from_secs(30);

/// Hard timeout for media file downloads.
const MEDIA_TIMEOUT: Duration = Duration::from_secs(120);

/// HTTP client for the ScreenScraper API with rate limiting and quota tracking.
pub struct ScreenScraperClient {
    http: reqwest::Client,
    creds: Credentials,
    /// Semaphore limiting concurrent API requests. Permits auto-release on drop,
    /// preventing slot leaks even on timeout or cancellation.
    rate_semaphore: Arc<Semaphore>,
    quota: Mutex<Option<UserQuota>>,
}

impl ScreenScraperClient {
    /// Create a new client and validate credentials by calling ssuserInfos.php.
    ///
    /// Starts with 1 slot for the auth request, then rebuilds with
    /// `user_info.max_threads()` slots.
    pub async fn new(creds: Credentials) -> Result<(Self, UserInfo), ScrapeError> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()?;

        // Start with 1 slot for the initial auth call
        let client = Self {
            http,
            creds,
            rate_semaphore: Arc::new(Semaphore::new(1)),
            quota: Mutex::new(None),
        };

        let user_info = client.get_user_info().await?;

        // Rebuild with the user's actual max_threads
        let max_threads = (user_info.max_threads() as usize).max(1);
        let client = Self {
            rate_semaphore: Arc::new(Semaphore::new(max_threads)),
            http: client.http,
            creds: client.creds,
            quota: client.quota,
        };

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

        // Check for common error patterns in the response text
        if text.contains("Erreur") || text.contains("Jeu non trouvé") || text.is_empty() {
            return Err(ScrapeError::NotFound { warnings: vec![] });
        }
        if text.contains("API fermé") || text.contains("API closed") {
            return Err(ScrapeError::ServerClosed(
                "ScreenScraper API is temporarily closed".to_string(),
            ));
        }
        if text.contains("Le quota de scrape journalier") {
            return Err(ScrapeError::QuotaExceeded { used: 0, max: 0 });
        }

        let response: JeuInfosResponse = serde_json::from_str(&text).map_err(|e| {
            ScrapeError::Api(format!(
                "Failed to parse game info: {e}. Response: {}",
                &text[..text.len().min(200)]
            ))
        })?;

        // Update quota tracking
        if let Some(ref user) = response.response.ssuser {
            *self.quota.lock().await = Some(user.clone());
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
        self.quota.lock().await.clone()
    }

    /// Perform a rate-limited HTTP GET request with a hard timeout.
    ///
    /// Acquires a semaphore permit (blocking if all slots are busy), makes the
    /// request within a hard timeout that covers the full response (including
    /// body read), then sleeps for the rate limit interval before releasing.
    /// The permit auto-releases on drop, preventing slot leaks.
    async fn rate_limited_get(
        &self,
        url: &str,
        params: &HashMap<&str, String>,
    ) -> Result<String, ScrapeError> {
        let _permit = self
            .rate_semaphore
            .acquire()
            .await
            .map_err(|_| ScrapeError::Api("Rate limiter closed".to_string()))?;

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

            let text = resp
                .text()
                .await
                .map_err(|e| ScrapeError::Api(redact_credentials(&e.to_string())))?;
            Ok(text)
        })
        .await
        .map_err(|_| {
            ScrapeError::Api(format!(
                "API request timed out after {}s",
                API_TIMEOUT.as_secs()
            ))
        })?;

        // Rate limit: sleep before releasing the permit so the next request
        // on this slot respects the minimum interval.
        tokio::time::sleep(MIN_REQUEST_INTERVAL).await;

        result
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
