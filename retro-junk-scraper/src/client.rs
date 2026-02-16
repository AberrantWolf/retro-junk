use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::time::{Duration, Instant};

use crate::credentials::Credentials;
use crate::error::ScrapeError;
use crate::types::{JeuInfosResponse, UserInfo, UserInfoResponse, UserQuota};

const BASE_URL: &str = "https://api.screenscraper.fr/api2";
const MIN_REQUEST_INTERVAL: Duration = Duration::from_millis(1200);

/// HTTP client for the ScreenScraper API with rate limiting and quota tracking.
pub struct ScreenScraperClient {
    http: reqwest::Client,
    creds: Credentials,
    last_request: Arc<Mutex<Instant>>,
    quota: Arc<Mutex<Option<UserQuota>>>,
}

impl ScreenScraperClient {
    /// Create a new client and validate credentials by calling ssuserInfos.php.
    pub async fn new(creds: Credentials) -> Result<(Self, UserInfo), ScrapeError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?;

        let client = Self {
            http,
            creds,
            last_request: Arc::new(Mutex::new(Instant::now() - MIN_REQUEST_INTERVAL)),
            quota: Arc::new(Mutex::new(None)),
        };

        let user_info = client.get_user_info().await?;
        Ok((client, user_info))
    }

    /// Get user info and quota from ssuserInfos.php.
    async fn get_user_info(&self) -> Result<UserInfo, ScrapeError> {
        let mut params = self.base_params();
        params.insert("output", "json".to_string());

        self.rate_limit().await;

        let resp = self
            .http
            .get(format!("{}/ssuserInfos.php", BASE_URL))
            .query(&params)
            .send()
            .await?;

        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(ScrapeError::InvalidCredentials(
                "Invalid developer or user credentials".to_string(),
            ));
        }

        let text = resp.text().await?;
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

        self.rate_limit().await;

        let resp = self
            .http
            .get(format!("{}/jeuInfos.php", BASE_URL))
            .query(&all_params)
            .send()
            .await?;

        let status = resp.status();
        let text = resp.text().await?;

        // Check for common error patterns
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(ScrapeError::InvalidCredentials(
                "Credentials rejected".to_string(),
            ));
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(ScrapeError::RateLimit);
        }

        // ScreenScraper returns 200 with error text for not-found
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

    /// Enforce rate limiting: wait until at least MIN_REQUEST_INTERVAL has
    /// passed since the last API request.
    async fn rate_limit(&self) {
        let mut last = self.last_request.lock().await;
        let elapsed = last.elapsed();
        if elapsed < MIN_REQUEST_INTERVAL {
            tokio::time::sleep(MIN_REQUEST_INTERVAL - elapsed).await;
        }
        *last = Instant::now();
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
