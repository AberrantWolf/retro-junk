use std::collections::HashMap;
use std::time::SystemTime;

use tokio::sync::Mutex;
use tokio::time::Duration;

use crate::credentials::Credentials;
use crate::error::ScrapeError;
use crate::types::{JeuInfosResponse, UserInfo, UserInfoResponse, UserQuota};

use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

const BASE_URL: &str = "https://api.screenscraper.fr/api2";
const FALLBACK_REQUEST_INTERVAL: Duration = Duration::from_millis(1200);

/// Hard timeout for API requests (covers connect + headers + body read).
const API_TIMEOUT: Duration = Duration::from_secs(30);

/// Timeout for acquiring internal mutex locks (should be near-instant).
const LOCK_TIMEOUT: Duration = Duration::from_secs(5);

/// Maximum number of retries for transient server errors.
const MAX_RETRIES: u32 = 3;

/// Initial backoff duration before first retry (doubles each attempt).
const INITIAL_BACKOFF: Duration = Duration::from_secs(2);

/// Ceiling on a rate-limit wait. `Retry-After` is normally seconds; a server
/// that asks for longer than this is telling us to come back later, and the
/// caller's quota handling is the right place for that.
const MAX_RATE_LIMIT_BACKOFF: Duration = Duration::from_mins(1);

/// Hard timeout for media file downloads.
const MEDIA_TIMEOUT: Duration = Duration::from_mins(2);

/// HTTP client for the `ScreenScraper` API with rate limiting and quota tracking.
///
/// Concurrency is controlled externally by the caller (e.g., worker pool count
/// or `buffer_unordered` limit). Each API call sleeps for `MIN_REQUEST_INTERVAL`
/// after completing, ensuring per-worker rate limiting.
pub struct ScreenScraperClient {
    http: reqwest::Client,
    creds: Credentials,
    quota: Mutex<Option<UserQuota>>,
    /// Global start-time gate derived from `ScreenScraper`'s per-minute quota.
    /// The API's thread allowance controls in-flight concurrency separately.
    next_api_request: Mutex<tokio::time::Instant>,
    request_interval_ms: AtomicU64,
    next_download_byte: Mutex<tokio::time::Instant>,
    download_bytes_per_second: AtomicU64,
    /// Monotonic request counter for correlating log lines.
    request_counter: AtomicU64,
}

impl ScreenScraperClient {
    /// Create a new client and validate credentials by calling ssuserInfos.php.
    ///
    /// Returns the client and user info (which includes `max_threads` for the
    /// caller to configure its own concurrency control).
    pub async fn new(creds: Credentials) -> Result<(Self, UserInfo), ScrapeError> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(10)
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Duration::from_secs(30))
            .tcp_nodelay(true)
            .build()?;

        let client = Self {
            http,
            creds,
            quota: Mutex::new(None),
            next_api_request: Mutex::new(tokio::time::Instant::now()),
            request_interval_ms: AtomicU64::new(FALLBACK_REQUEST_INTERVAL.as_millis() as u64),
            next_download_byte: Mutex::new(tokio::time::Instant::now()),
            download_bytes_per_second: AtomicU64::new(0),
            request_counter: AtomicU64::new(0),
        };

        let user_info = client.get_user_info().await?;
        client.set_request_rate(user_info.max_requests_per_minute());
        client.set_download_rate(user_info.max_download_speed_kbps());

        Ok((client, user_info))
    }

    /// Get user info and quota from ssuserInfos.php.
    async fn get_user_info(&self) -> Result<UserInfo, ScrapeError> {
        let mut params = self.base_params();
        params.insert("output", "json".to_string());

        let text = self
            .rate_limited_get(&format!("{BASE_URL}/ssuserInfos.php"), &params)
            .await?;

        let status_err = check_auth_status_from_text(&text);
        if let Some(e) = status_err {
            return Err(e);
        }

        let info: UserInfoResponse = serde_json::from_str(&text).map_err(|e| {
            ScrapeError::Api(format!(
                "Failed to parse user info: {e}. Response: {}",
                &text[..text.len().min(200)]
            ))
        })?;

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
            .rate_limited_get(&format!("{BASE_URL}/jeuInfos.php"), &all_params)
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
            // Report the numbers the last response gave us. Zeroes here made
            // the error read "0/0 requests" on every surface that shows it.
            let (used, max) = self.current_quota().await.map_or((0, 0), |quota| {
                (quota.requests_today(), quota.max_requests_per_day())
            });
            return Err(ScrapeError::QuotaExceeded { used, max });
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
                message: format!("ScreenScraper error: {}", &text[..text.len().min(200)]),
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
    /// to prevent hangs when `ScreenScraper` stalls mid-transfer.
    pub async fn download_media(&self, url: &str) -> Result<Vec<u8>, ScrapeError> {
        tokio::time::timeout(MEDIA_TIMEOUT, async {
            let mut resp = self.http.get(url).send().await?.error_for_status()?;
            let mut bytes = Vec::with_capacity(
                resp.content_length()
                    .and_then(|length| usize::try_from(length).ok())
                    .unwrap_or_default(),
            );
            while let Some(chunk) = resp.chunk().await? {
                self.wait_for_download_bytes(chunk.len()).await;
                bytes.extend_from_slice(&chunk);
            }
            Ok::<_, reqwest::Error>(bytes)
        })
        .await
        .map_err(|_| {
            ScrapeError::Api(format!(
                "Media download timed out after {}s",
                MEDIA_TIMEOUT.as_secs()
            ))
        })?
        .map_err(ScrapeError::from)
        .and_then(|bytes| {
            let prefix = &bytes[..bytes.len().min(256)];
            let text = String::from_utf8_lossy(prefix);
            if bytes.is_empty() || looks_like_html_error(&text) {
                Err(ScrapeError::Api(
                    "Media server returned an empty or HTML response".to_owned(),
                ))
            } else {
                Ok(bytes)
            }
        })
    }

    /// A client that has never contacted `ScreenScraper`, for exercising
    /// orchestration paths that must not reach the network.
    #[cfg(test)]
    pub(crate) fn offline_for_tests() -> Self {
        Self {
            http: reqwest::Client::new(),
            creds: Credentials {
                dev_id: "dev".to_owned(),
                dev_password: "dev".to_owned(),
                soft_name: "retro-junk-tests".to_owned(),
                user_id: String::new(),
                user_password: String::new(),
            },
            quota: Mutex::new(None),
            next_api_request: Mutex::new(tokio::time::Instant::now()),
            request_interval_ms: AtomicU64::new(FALLBACK_REQUEST_INTERVAL.as_millis() as u64),
            next_download_byte: Mutex::new(tokio::time::Instant::now()),
            download_bytes_per_second: AtomicU64::new(0),
            request_counter: AtomicU64::new(0),
        }
    }

    /// Get current quota info if available.
    pub async fn current_quota(&self) -> Option<UserQuota> {
        if let Ok(guard) = tokio::time::timeout(LOCK_TIMEOUT, self.quota.lock()).await {
            guard.clone()
        } else {
            log::debug!("Quota lock timed out during read");
            None
        }
    }

    /// Perform a single HTTP GET attempt, mapping HTTP-level failures to
    /// `ScrapeError`s (credential rejection, rate limit, server errors, and
    /// HTML error pages returned with a 200 status).
    async fn attempt_get(
        &self,
        url: &str,
        params: &HashMap<&str, String>,
    ) -> Result<String, ScrapeError> {
        let resp = self
            .http
            .get(url)
            .query(params)
            .send()
            .await
            .map_err(|e| ScrapeError::Api(redact_credentials(&e.to_string())))?;

        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(ScrapeError::InvalidCredentials(
                "Credentials rejected".to_string(),
            ));
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(ScrapeError::RateLimit {
                retry_after: resp
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.trim().parse::<u64>().ok()),
            });
        }
        if status.is_server_error() {
            return Err(ScrapeError::ServerError {
                status: status.as_u16(),
                message: format!("Server returned HTTP {}", status.as_u16()),
            });
        }

        let text = resp.text().await.map_err(|e| ScrapeError::ServerError {
            status: 200,
            message: format!(
                "Failed to read response body: {}",
                redact_credentials(&e.to_string())
            ),
        })?;

        // Detect HTML error pages returned with 200 status (CDN/proxy errors)
        if looks_like_html_error(&text) {
            return Err(ScrapeError::ServerError {
                status: 200,
                message: "Server returned HTML error page instead of JSON".to_string(),
            });
        }

        Ok(text)
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
        let req_id = self.request_counter.fetch_add(1, AtomicOrdering::Relaxed);
        let endpoint = extract_endpoint(url);
        let mut last_error: Option<ScrapeError> = None;
        let mut consecutive_timeouts: u32 = 0;

        log::debug!(
            "[req:{}] {} starting (params: {})",
            req_id,
            endpoint,
            summarize_params(params),
        );

        let request_start = tokio::time::Instant::now();

        for attempt in 0..=MAX_RETRIES {
            backoff_before_retry(req_id, endpoint, attempt).await;
            self.wait_for_request_slot().await;

            let attempt_start = tokio::time::Instant::now();
            let wall_start = SystemTime::now();

            let result = tokio::time::timeout(API_TIMEOUT, self.attempt_get(url, params)).await;

            let attempt_elapsed = attempt_start.elapsed();
            warn_on_clock_drift(req_id, endpoint, attempt_elapsed, wall_start);

            match result {
                Ok(Ok(text)) => {
                    let total_elapsed = request_start.elapsed();
                    log::debug!(
                        "[req:{}] {} OK (attempt took {}ms, total {}ms, {}B)",
                        req_id,
                        endpoint,
                        attempt_elapsed.as_millis(),
                        total_elapsed.as_millis(),
                        text.len(),
                    );
                    return Ok(text);
                }
                Ok(Err(e)) if is_retryable(&e) => {
                    consecutive_timeouts = 0;
                    log::info!(
                        "[req:{}] {} transient error after {}ms: {}",
                        req_id,
                        endpoint,
                        attempt_elapsed.as_millis(),
                        e,
                    );
                    // A 429 carries the server's own wait; hold the global
                    // request gate for it so every worker slows down, not just
                    // this one.
                    if let Some(wait) = rate_limit_backoff(&e, attempt) {
                        log::info!(
                            "[req:{req_id}] {endpoint} rate limited; pausing all requests for {}s",
                            wait.as_secs(),
                        );
                        self.hold_request_slot(wait).await;
                    }
                    last_error = Some(e);
                }
                Ok(Err(e)) => {
                    log::debug!(
                        "[req:{}] {} non-retryable error after {}ms: {}",
                        req_id,
                        endpoint,
                        attempt_elapsed.as_millis(),
                        e,
                    );
                    return Err(e);
                }
                Err(_timeout) => {
                    consecutive_timeouts += 1;
                    let e = ScrapeError::Api(format!(
                        "API request timed out after {}s",
                        API_TIMEOUT.as_secs()
                    ));
                    log::warn!(
                        "[req:{}] {} TIMEOUT after {}ms ({} consecutive)",
                        req_id,
                        endpoint,
                        attempt_elapsed.as_millis(),
                        consecutive_timeouts,
                    );
                    last_error = Some(e);
                    // After 2 consecutive timeouts, connections are likely stale
                    // (e.g., laptop woke from sleep). Stop retrying to recover faster.
                    if consecutive_timeouts >= 2 {
                        log::warn!(
                            "[req:{req_id}] {endpoint} aborting after {consecutive_timeouts} consecutive timeouts (stale connections?)",
                        );
                        break;
                    }
                }
            }
        }

        let total_elapsed = request_start.elapsed();
        log::warn!(
            "[req:{}] {} FAILED after {}ms (all retries exhausted)",
            req_id,
            endpoint,
            total_elapsed.as_millis(),
        );
        Err(last_error.unwrap_or_else(|| ScrapeError::Api("All retries exhausted".to_string())))
    }

    fn set_request_rate(&self, requests_per_minute: u32) {
        let requests_per_minute = requests_per_minute.max(1);
        let interval_ms = (60_000_u64 / u64::from(requests_per_minute)).max(1);
        self.request_interval_ms
            .store(interval_ms, AtomicOrdering::Relaxed);
        log::info!("ScreenScraper API pacing: up to {requests_per_minute} request(s)/minute");
    }

    async fn wait_for_request_slot(&self) {
        let mut next = self.next_api_request.lock().await;
        let now = tokio::time::Instant::now();
        if *next > now {
            tokio::time::sleep_until(*next).await;
        }
        let interval =
            Duration::from_millis(self.request_interval_ms.load(AtomicOrdering::Relaxed));
        *next = tokio::time::Instant::now() + interval;
    }

    /// Push the global request gate out by `wait`, so every worker backs off
    /// together. Rate limiting is per account, not per connection: slowing
    /// down only the worker that saw the 429 just moves the next one into it.
    async fn hold_request_slot(&self, wait: Duration) {
        let mut next = self.next_api_request.lock().await;
        let resume = tokio::time::Instant::now() + wait;
        if resume > *next {
            *next = resume;
        }
    }

    /// Requests left in the account's daily budget, and the budget itself.
    ///
    /// `None` until a response has reported quota — the caller cannot make a
    /// reserve decision from a number it does not have.
    pub async fn quota_headroom(&self) -> Option<(u32, u32)> {
        let quota = self.current_quota().await?;
        let (used, max) = (quota.requests_today(), quota.max_requests_per_day());
        Some((max.saturating_sub(used), max))
    }

    fn set_download_rate(&self, kilobytes_per_second: u32) {
        self.download_bytes_per_second.store(
            u64::from(kilobytes_per_second).saturating_mul(1024),
            AtomicOrdering::Relaxed,
        );
        log::info!("ScreenScraper media pacing: up to {kilobytes_per_second} KB/s aggregate");
    }

    async fn wait_for_download_bytes(&self, byte_count: usize) {
        let bytes_per_second = self.download_bytes_per_second.load(AtomicOrdering::Relaxed);
        if bytes_per_second == 0 || byte_count == 0 {
            return;
        }
        let mut next = self.next_download_byte.lock().await;
        let now = tokio::time::Instant::now();
        if *next > now {
            tokio::time::sleep_until(*next).await;
        }
        let nanos = (byte_count as u128)
            .saturating_mul(1_000_000_000)
            .checked_div(u128::from(bytes_per_second))
            .unwrap_or_default()
            .min(u128::from(u64::MAX)) as u64;
        *next = tokio::time::Instant::now() + Duration::from_nanos(nanos);
    }

    fn base_params(&self) -> HashMap<&str, String> {
        let mut params = HashMap::new();
        params.insert("devid", self.creds.dev_id.clone());
        params.insert("devpassword", self.creds.dev_password.clone());
        params.insert("softname", self.creds.soft_name.clone());
        if !self.creds.user_id.is_empty() {
            params.insert("ssid", self.creds.user_id.clone());
        }
        if !self.creds.user_password.is_empty() {
            params.insert("sspassword", self.creds.user_password.clone());
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

/// Check if a `ScrapeError` is retryable (transient server issue).
fn is_retryable(e: &ScrapeError) -> bool {
    // 429 is the one error the server explicitly tells us to retry. Failing it
    // outright turned a "slow down" into a per-game failure and, for the
    // daemon, into a six-hour error backoff on a target that was fine.
    matches!(
        e,
        ScrapeError::ServerError { .. } | ScrapeError::RateLimit { .. }
    )
}

/// How long to wait before the next attempt at a rate-limited request.
///
/// A `Retry-After` is the server telling us exactly when it will serve us
/// again; guessing shorter just earns another 429.
fn rate_limit_backoff(error: &ScrapeError, attempt: u32) -> Option<Duration> {
    match error {
        ScrapeError::RateLimit { retry_after } => Some(
            retry_after
                .map_or_else(|| INITIAL_BACKOFF * 2u32.pow(attempt), Duration::from_secs)
                .min(MAX_RATE_LIMIT_BACKOFF),
        ),
        _ => None,
    }
}

/// Load credentials and create a connected `ScreenScraper` client.
///
/// Returns the client and the maximum number of worker threads to use,
/// computed from the server-granted thread limit and the optional user
/// override. Scraping is network-bound, so CPU parallelism is not a useful
/// additional cap.
pub async fn create_client(
    threads: Option<usize>,
) -> Result<(std::sync::Arc<ScreenScraperClient>, usize), ScrapeError> {
    let creds = Credentials::load()
        .map_err(|e| ScrapeError::Api(format!("Failed to load credentials: {e}")))?;

    let (client, user_info) = ScreenScraperClient::new(creds).await?;

    let ss_max = user_info.max_threads() as usize;
    let max_workers = threads
        .map_or(ss_max, |requested| requested.min(ss_max))
        .max(1);
    log::info!("ScreenScraper concurrency: using {max_workers} of {ss_max} granted thread(s)");

    // Seed the quota tracker with data from the initial user info response
    // so callers can read it immediately without waiting for a lookup.
    {
        let mut guard = client.quota.lock().await;
        *guard = Some(UserQuota {
            requeststoday: user_info.requeststoday.clone(),
            maxrequestsperday: user_info.maxrequestsperday.clone(),
        });
    }

    Ok((std::sync::Arc::new(client), max_workers))
}

/// Extract the API endpoint name from a full URL (e.g., "jeuInfos" from ".../jeuInfos.php").
/// Sleep with exponential backoff before a retry attempt.
///
/// No-op on the first attempt (`attempt == 0`).
async fn backoff_before_retry(req_id: u64, endpoint: &str, attempt: u32) {
    if attempt == 0 {
        return;
    }
    let backoff = INITIAL_BACKOFF * 2u32.pow(attempt - 1);
    log::info!(
        "[req:{}] {} retry {}/{} after {}s backoff",
        req_id,
        endpoint,
        attempt + 1,
        MAX_RETRIES + 1,
        backoff.as_secs(),
    );
    tokio::time::sleep(backoff).await;
}

/// Detect machine sleep: if wall-clock time advanced much more than the
/// tokio Instant-based elapsed, the machine likely slept.
fn warn_on_clock_drift(
    req_id: u64,
    endpoint: &str,
    attempt_elapsed: std::time::Duration,
    wall_start: SystemTime,
) {
    let wall_elapsed = wall_start.elapsed().unwrap_or_default();
    if wall_elapsed > attempt_elapsed + std::time::Duration::from_secs(10) {
        log::warn!(
            "[req:{}] {} clock drift: wall={}s vs tokio={}ms (machine likely slept)",
            req_id,
            endpoint,
            wall_elapsed.as_secs(),
            attempt_elapsed.as_millis(),
        );
    }
}

fn extract_endpoint(url: &str) -> &str {
    url.rsplit('/')
        .next()
        .and_then(|s| s.strip_suffix(".php"))
        .unwrap_or("unknown")
}

/// Summarize query params for logging, excluding credentials.
fn summarize_params(params: &HashMap<&str, String>) -> String {
    let mut parts: Vec<String> = params
        .iter()
        .filter(|(k, _)| {
            !matches!(
                **k,
                "devid" | "devpassword" | "ssid" | "sspassword" | "softname" | "output"
            )
        })
        .map(|(k, v)| {
            // Truncate long values (hashes, filenames)
            if v.len() > 40 {
                format!("{}={}...", k, &v[..37])
            } else {
                format!("{k}={v}")
            }
        })
        .collect();
    parts.sort();
    parts.join(", ")
}

/// Redact credential query parameters from error messages that may contain URLs.
///
/// Replaces values for `devpassword`, `sspassword`, `devid`, and `ssid` with `[REDACTED]`.
fn redact_credentials(msg: &str) -> String {
    let mut result = msg.to_string();
    for param in &["devpassword", "sspassword", "devid", "ssid"] {
        // Match param=value where value ends at & or end of string/whitespace
        let prefix = format!("{param}=");
        while let Some(start) = result.find(&prefix) {
            let value_start = start + prefix.len();
            let value_end = result[value_start..]
                .find(|c: char| c == '&' || c.is_whitespace() || c == '"' || c == '\'')
                .map_or(result.len(), |i| value_start + i);
            result.replace_range(value_start..value_end, "[REDACTED]");
        }
    }
    result
}

#[cfg(test)]
#[path = "tests/client_tests.rs"]
mod tests;
