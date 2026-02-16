use serde::Deserialize;

/// Top-level response wrapper from jeuInfos.php.
#[derive(Debug, Deserialize)]
pub struct JeuInfosResponse {
    pub response: JeuInfosData,
}

#[derive(Debug, Deserialize)]
pub struct JeuInfosData {
    #[serde(default)]
    pub ssuser: Option<UserQuota>,
    pub jeu: GameInfo,
}

/// Game info from ScreenScraper. Fields use nested arrays with typed objects.
#[derive(Debug, Deserialize, Clone)]
pub struct GameInfo {
    pub id: String,
    #[serde(default)]
    pub romid: Option<String>,
    #[serde(default)]
    pub notgame: Option<String>,
    #[serde(default)]
    pub noms: Vec<RegionText>,
    #[serde(default)]
    pub synopsis: Vec<LangueText>,
    #[serde(default)]
    pub dates: Vec<RegionText>,
    #[serde(default)]
    pub medias: Vec<Media>,
    #[serde(default)]
    pub editeur: Option<IdText>,
    #[serde(default)]
    pub developpeur: Option<IdText>,
    #[serde(default)]
    pub joueurs: Option<IdText>,
    #[serde(default)]
    pub note: Option<IdText>,
    #[serde(default)]
    pub genres: Vec<Genre>,
    #[serde(default)]
    pub systeme: Option<IdText>,
}

impl GameInfo {
    /// Get the game name for a preferred region, falling back to the first available.
    pub fn name_for_region(&self, preferred: &str) -> Option<&str> {
        self.noms
            .iter()
            .find(|n| n.region == preferred)
            .or_else(|| self.noms.iter().find(|n| n.region == "ss"))
            .or_else(|| self.noms.first())
            .map(|n| n.text.as_str())
    }

    /// Get the synopsis for a preferred language (exact match only).
    pub fn synopsis_for_language(&self, preferred: &str) -> Option<&str> {
        self.synopsis
            .iter()
            .find(|s| s.langue == preferred)
            .map(|s| s.text.as_str())
    }

    /// Get the release date for a preferred region.
    pub fn date_for_region(&self, preferred: &str) -> Option<&str> {
        self.dates
            .iter()
            .find(|d| d.region == preferred)
            .or_else(|| self.dates.first())
            .map(|d| d.text.as_str())
    }

    /// Get all media of a given type (e.g., "ss", "box-2D", "wheel").
    pub fn media_by_type(&self, media_type: &str) -> Vec<&Media> {
        self.medias
            .iter()
            .filter(|m| m.media_type == media_type)
            .collect()
    }

    /// Get a single media of a given type, preferring a specific region.
    pub fn media_for_region(&self, media_type: &str, preferred_region: &str) -> Option<&Media> {
        let matches: Vec<_> = self.media_by_type(media_type);
        matches
            .iter()
            .find(|m| m.region == preferred_region)
            .or_else(|| matches.iter().find(|m| m.region == "us"))
            .or_else(|| matches.iter().find(|m| m.region == "wor"))
            .or_else(|| matches.iter().find(|m| m.region == "ss"))
            .or_else(|| matches.first())
            .copied()
    }

    /// Get the genre name for a preferred language (exact match only).
    pub fn genre_for_language(&self, preferred: &str) -> Option<String> {
        let genres: Vec<String> = self
            .genres
            .iter()
            .filter_map(|g| {
                g.noms
                    .iter()
                    .find(|n| n.langue == preferred)
                    .map(|n| n.text.clone())
            })
            .collect();

        if genres.is_empty() {
            None
        } else {
            Some(genres.join(", "))
        }
    }

    /// Get the rating as a 0.0-1.0 float (ScreenScraper uses 0-20 scale).
    pub fn rating_normalized(&self) -> Option<f32> {
        self.note.as_ref().and_then(|n| {
            n.text
                .parse::<f32>()
                .ok()
                .map(|v| (v / 20.0).clamp(0.0, 1.0))
        })
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct RegionText {
    pub region: String,
    pub text: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LangueText {
    pub langue: String,
    pub text: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct IdText {
    #[serde(default)]
    pub id: Option<String>,
    pub text: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Media {
    #[serde(rename = "type")]
    pub media_type: String,
    pub url: String,
    #[serde(default)]
    pub region: String,
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub crc: Option<String>,
    #[serde(default)]
    pub size: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Genre {
    pub id: String,
    #[serde(default)]
    pub noms: Vec<LangueText>,
}

/// User info response from ssuserInfos.php.
#[derive(Debug, Deserialize)]
pub struct UserInfoResponse {
    pub response: UserInfoData,
}

#[derive(Debug, Deserialize)]
pub struct UserInfoData {
    pub ssuser: UserInfo,
}

#[derive(Debug, Deserialize, Clone)]
pub struct UserInfo {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub maxthreads: Option<String>,
    #[serde(default)]
    pub maxdownloadspeed: Option<String>,
    #[serde(default)]
    pub requeststoday: Option<String>,
    #[serde(default)]
    pub maxrequestspermin: Option<String>,
    #[serde(default)]
    pub maxrequestsperday: Option<String>,
    #[serde(default)]
    pub maxrequestskoperday: Option<String>,
}

impl UserInfo {
    pub fn requests_today(&self) -> u32 {
        self.requeststoday
            .as_ref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    }

    pub fn max_requests_per_day(&self) -> u32 {
        self.maxrequestsperday
            .as_ref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(20000)
    }

    pub fn max_threads(&self) -> u32 {
        self.maxthreads
            .as_ref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1)
    }
}

/// Embedded user quota info returned in game lookup responses.
#[derive(Debug, Deserialize, Clone)]
pub struct UserQuota {
    #[serde(default)]
    pub requeststoday: Option<String>,
    #[serde(default)]
    pub maxrequestsperday: Option<String>,
}

impl UserQuota {
    pub fn requests_today(&self) -> u32 {
        self.requeststoday
            .as_ref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    }

    pub fn max_requests_per_day(&self) -> u32 {
        self.maxrequestsperday
            .as_ref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(20000)
    }
}
