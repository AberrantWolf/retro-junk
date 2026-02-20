use std::path::PathBuf;

use crate::error::ScrapeError;

// XOR-obfuscated dev credentials embedded at compile time.
// Set SCREENSCRAPER_DEVID and SCREENSCRAPER_DEVPASSWORD env vars when building.
include!(concat!(env!("OUT_DIR"), "/embedded_credentials.rs"));

fn deobfuscate(data: &[u8]) -> String {
    let decoded: Vec<u8> = data
        .iter()
        .enumerate()
        .map(|(i, b)| b ^ OBFUSCATION_KEY[i % OBFUSCATION_KEY.len()])
        .collect();
    String::from_utf8(decoded).expect("embedded credentials must be valid UTF-8")
}

fn embedded_dev_id() -> Option<String> {
    EMBEDDED_DEV_ID.map(deobfuscate)
}

fn embedded_dev_password() -> Option<String> {
    EMBEDDED_DEV_PASSWORD.map(deobfuscate)
}

/// Returns true if dev credentials were embedded at compile time.
pub fn has_embedded_dev_credentials() -> bool {
    EMBEDDED_DEV_ID.is_some() && EMBEDDED_DEV_PASSWORD.is_some()
}

/// Credentials for authenticating with the ScreenScraper API.
#[derive(Debug, Clone)]
pub struct Credentials {
    pub dev_id: String,
    pub dev_password: String,
    pub soft_name: String,
    pub user_id: Option<String>,
    pub user_password: Option<String>,
}

/// Where a credential field's value came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialSource {
    /// Loaded from an environment variable.
    EnvVar(&'static str),
    /// Loaded from the config file.
    ConfigFile,
    /// Embedded at compile time.
    Embedded,
    /// Hard-coded default value.
    Default,
    /// Not set anywhere.
    Missing,
}

impl std::fmt::Display for CredentialSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EnvVar(var) => write!(f, "env ${}", var),
            Self::ConfigFile => write!(f, "config file"),
            Self::Embedded => write!(f, "embedded"),
            Self::Default => write!(f, "default"),
            Self::Missing => write!(f, "not set"),
        }
    }
}

/// Provenance of each credential field.
#[derive(Debug)]
pub struct CredentialSources {
    pub dev_id: CredentialSource,
    pub dev_password: CredentialSource,
    pub soft_name: CredentialSource,
    pub user_id: CredentialSource,
    pub user_password: CredentialSource,
}

/// TOML config file format.
#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct ConfigFile {
    screenscraper: Option<ScreenScraperConfig>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct ScreenScraperConfig {
    dev_id: Option<String>,
    dev_password: Option<String>,
    soft_name: Option<String>,
    user_id: Option<String>,
    user_password: Option<String>,
}

impl Credentials {
    /// Load credentials from environment variables, config file, or embedded defaults.
    ///
    /// Priority: env vars > config file > embedded (compile-time).
    /// Required: dev_id, dev_password, soft_name.
    /// Optional: user_id, user_password.
    pub fn load() -> Result<Self, ScrapeError> {
        // Try config file first as base values
        let config = load_config_file();

        let dev_id = std::env::var("SCREENSCRAPER_DEVID")
            .ok()
            .or_else(|| config.as_ref().and_then(|c| c.dev_id.clone()))
            .or_else(embedded_dev_id)
            .ok_or_else(|| {
                ScrapeError::Config(
                    "Missing dev_id. Set SCREENSCRAPER_DEVID env var or add to config file"
                        .to_string(),
                )
            })?;

        let dev_password = std::env::var("SCREENSCRAPER_DEVPASSWORD")
            .ok()
            .or_else(|| config.as_ref().and_then(|c| c.dev_password.clone()))
            .or_else(embedded_dev_password)
            .ok_or_else(|| {
                ScrapeError::Config(
                    "Missing dev_password. Set SCREENSCRAPER_DEVPASSWORD env var or add to config file"
                        .to_string(),
                )
            })?;

        let soft_name = std::env::var("SCREENSCRAPER_SOFTNAME")
            .ok()
            .or_else(|| config.as_ref().and_then(|c| c.soft_name.clone()))
            .unwrap_or_else(|| "retro-junk".to_string());

        let user_id = std::env::var("SCREENSCRAPER_SSID")
            .ok()
            .or_else(|| config.as_ref().and_then(|c| c.user_id.clone()));

        let user_password = std::env::var("SCREENSCRAPER_SSPASSWORD")
            .ok()
            .or_else(|| config.as_ref().and_then(|c| c.user_password.clone()));

        Ok(Self {
            dev_id,
            dev_password,
            soft_name,
            user_id,
            user_password,
        })
    }

    /// Create credentials with explicit values (e.g., from CLI args).
    pub fn with_overrides(
        mut self,
        dev_id: Option<String>,
        dev_password: Option<String>,
        user_id: Option<String>,
        user_password: Option<String>,
    ) -> Self {
        if let Some(id) = dev_id {
            self.dev_id = id;
        }
        if let Some(pw) = dev_password {
            self.dev_password = pw;
        }
        if let Some(id) = user_id {
            self.user_id = Some(id);
        }
        if let Some(pw) = user_password {
            self.user_password = Some(pw);
        }
        self
    }
}

/// Return the path to the credentials config file.
pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("retro-junk").join("credentials.toml"))
}

/// Save credentials to the config file, creating parent directories as needed.
///
/// Dev credentials are omitted from the file if they match the embedded values
/// (no point persisting what's already in the binary).
/// Returns the path the file was written to.
pub fn save_to_file(creds: &Credentials) -> Result<PathBuf, ScrapeError> {
    let path = config_path().ok_or_else(|| {
        ScrapeError::Config("Could not determine config directory".to_string())
    })?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Only persist dev credentials if they differ from the embedded defaults
    let embedded_id = embedded_dev_id();
    let embedded_pw = embedded_dev_password();
    let dev_id_differs = embedded_id.as_ref() != Some(&creds.dev_id);
    let dev_pw_differs = embedded_pw.as_ref() != Some(&creds.dev_password);
    let save_dev = dev_id_differs || dev_pw_differs;

    let config = ConfigFile {
        screenscraper: Some(ScreenScraperConfig {
            dev_id: if save_dev {
                Some(creds.dev_id.clone())
            } else {
                None
            },
            dev_password: if save_dev {
                Some(creds.dev_password.clone())
            } else {
                None
            },
            soft_name: if creds.soft_name == "retro-junk" {
                None
            } else {
                Some(creds.soft_name.clone())
            },
            user_id: creds.user_id.clone(),
            user_password: creds.user_password.clone(),
        }),
    };

    let toml_str = toml::to_string_pretty(&config)
        .map_err(|e| ScrapeError::Config(format!("Failed to serialize config: {}", e)))?;

    std::fs::write(&path, toml_str)?;
    Ok(path)
}

/// Determine where each credential field is coming from.
pub fn credential_sources() -> CredentialSources {
    let config = load_config_file();

    let dev_id = if std::env::var("SCREENSCRAPER_DEVID").is_ok() {
        CredentialSource::EnvVar("SCREENSCRAPER_DEVID")
    } else if config.as_ref().and_then(|c| c.dev_id.as_ref()).is_some() {
        CredentialSource::ConfigFile
    } else if has_embedded_dev_credentials() {
        CredentialSource::Embedded
    } else {
        CredentialSource::Missing
    };

    let dev_password = if std::env::var("SCREENSCRAPER_DEVPASSWORD").is_ok() {
        CredentialSource::EnvVar("SCREENSCRAPER_DEVPASSWORD")
    } else if config
        .as_ref()
        .and_then(|c| c.dev_password.as_ref())
        .is_some()
    {
        CredentialSource::ConfigFile
    } else if has_embedded_dev_credentials() {
        CredentialSource::Embedded
    } else {
        CredentialSource::Missing
    };

    let soft_name = if std::env::var("SCREENSCRAPER_SOFTNAME").is_ok() {
        CredentialSource::EnvVar("SCREENSCRAPER_SOFTNAME")
    } else if config
        .as_ref()
        .and_then(|c| c.soft_name.as_ref())
        .is_some()
    {
        CredentialSource::ConfigFile
    } else {
        CredentialSource::Default
    };

    let user_id = if std::env::var("SCREENSCRAPER_SSID").is_ok() {
        CredentialSource::EnvVar("SCREENSCRAPER_SSID")
    } else if config.as_ref().and_then(|c| c.user_id.as_ref()).is_some() {
        CredentialSource::ConfigFile
    } else {
        CredentialSource::Missing
    };

    let user_password = if std::env::var("SCREENSCRAPER_SSPASSWORD").is_ok() {
        CredentialSource::EnvVar("SCREENSCRAPER_SSPASSWORD")
    } else if config
        .as_ref()
        .and_then(|c| c.user_password.as_ref())
        .is_some()
    {
        CredentialSource::ConfigFile
    } else {
        CredentialSource::Missing
    };

    CredentialSources {
        dev_id,
        dev_password,
        soft_name,
        user_id,
        user_password,
    }
}

fn load_config_file() -> Option<ScreenScraperConfig> {
    let path = config_path()?;
    let content = std::fs::read_to_string(&path).ok()?;
    let config: ConfigFile = toml::from_str(&content).ok()?;
    config.screenscraper
}
