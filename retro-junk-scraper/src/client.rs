use std::collections::HashMap;

use tokio::sync::Mutex;
use tokio::time::{Duration, Instant};

use crate::credentials::Credentials;
use crate::error::ScrapeError;
use crate::types::{JeuInfosResponse, UserInfo, UserInfoResponse, UserQuota};

const BASE_URL: &str = "https://api.screenscraper.fr/api2";
const MIN_REQUEST_INTERVAL: Duration = Duration::from_millis(1200);

/// A pool of N rate-limit tokens, each tracking its own 1200ms cooldown.
///
/// Pre-filled with N `Instant` values. `acquire()` takes a token (blocking if
/// all N slots are busy), sleeps until that slot's cooldown has elapsed, and
/// returns a guard. The guard's `release()` sends `Instant::now()` back.
struct RateLimitPool {
    send: tokio::sync::mpsc::Sender<Instant>,
    recv: Mutex<tokio::sync::mpsc::Receiver<Instant>>,
}

impl RateLimitPool {
    /// Create a pool with `n` concurrent slots.
    fn new(n: usize) -> Self {
        let (send, recv) = tokio::sync::mpsc::channel(n);
        let past = Instant::now() - MIN_REQUEST_INTERVAL;
        for _ in 0..n {
            send.try_send(past).expect("channel should have capacity");
        }
        Self {
            send,
            recv: Mutex::new(recv),
        }
    }

    /// Acquire a rate-limit slot. Blocks until a slot is available and its
    /// cooldown has elapsed. Returns the sender for releasing the slot.
    async fn acquire(&self) -> tokio::sync::mpsc::Sender<Instant> {
        let last = {
            let mut recv = self.recv.lock().await;
            recv.recv().await.expect("rate limit pool closed unexpectedly")
        };
        let elapsed = last.elapsed();
        if elapsed < MIN_REQUEST_INTERVAL {
            tokio::time::sleep(MIN_REQUEST_INTERVAL - elapsed).await;
        }
        self.send.clone()
    }
}

/// Release a rate-limit slot back to the pool.
async fn release_slot(sender: &tokio::sync::mpsc::Sender<Instant>) {
    let _ = sender.send(Instant::now()).await;
}

/// HTTP client for the ScreenScraper API with rate limiting and quota tracking.
pub struct ScreenScraperClient {
    http: reqwest::Client,
    creds: Credentials,
    rate_pool: RateLimitPool,
    quota: Mutex<Option<UserQuota>>,
}

impl ScreenScraperClient {
    /// Create a new client and validate credentials by calling ssuserInfos.php.
    ///
    /// Starts with 1 slot for the auth request, then rebuilds the pool with
    /// `user_info.max_threads()` slots.
    pub async fn new(creds: Credentials) -> Result<(Self, UserInfo), ScrapeError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?;

        // Start with 1 slot for the initial auth call
        let client = Self {
            http,
            creds,
            rate_pool: RateLimitPool::new(1),
            quota: Mutex::new(None),
        };

        let user_info = client.get_user_info().await?;

        // Rebuild with the user's actual max_threads
        let max_threads = (user_info.max_threads() as usize).max(1);
        let client = Self {
            rate_pool: RateLimitPool::new(max_threads),
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
            return Err(ScrapeError::NotFound);
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

    /// Download a media file from a URL. Media CDN downloads don't count against
    /// the API rate limit, so no rate limiting is applied here.
    pub async fn download_media(&self, url: &str) -> Result<Vec<u8>, ScrapeError> {
        let resp = self.http.get(url).send().await?;
        let bytes = resp.bytes().await?;
        Ok(bytes.to_vec())
    }

    /// Get current quota info if available.
    pub async fn current_quota(&self) -> Option<UserQuota> {
        self.quota.lock().await.clone()
    }

    /// Perform a rate-limited HTTP GET request. Acquires a slot from the pool,
    /// sends the request, and always releases the slot back (even on error).
    async fn rate_limited_get(
        &self,
        url: &str,
        params: &HashMap<&str, String>,
    ) -> Result<String, ScrapeError> {
        let slot = self.rate_pool.acquire().await;

        let result = async {
            let resp = self.http.get(url).query(params).send().await?;

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

            let text = resp.text().await?;
            Ok(text)
        }
        .await;

        release_slot(&slot).await;
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
