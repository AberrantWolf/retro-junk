#[cfg(test)]
#[path = "tests/credentials_tests.rs"]
mod tests;

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
    /// Personal ScreenScraper account username (empty = anonymous API access).
    pub user_id: String,
    /// Personal ScreenScraper account password (empty = anonymous API access).
    pub user_password: String,
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

/// Treat blank (empty or whitespace-only) values as unset, wherever they came
/// from — a `user_id = ""` line in the config file, an env var set to "".
/// A blank user_id must mean "use the anonymous API", not "log in with an
/// empty username", and a blank dev_id must fall through to the embedded
/// credentials instead of masking them.
fn non_blank(v: Option<String>) -> Option<String> {
    v.filter(|s| !s.trim().is_empty())
}

/// Resolve one field: env var > config file, skipping blank values at each level.
fn resolve(env_var: &str, file_value: Option<String>) -> Option<String> {
    non_blank(std::env::var(env_var).ok()).or_else(|| non_blank(file_value))
}

/// Provenance decision for one field, given already-fetched raw values.
/// Blank values don't count as set, mirroring [`resolve`].
fn source_from(
    env_var: &'static str,
    env_value: Option<String>,
    file_value: Option<&str>,
    fallback: CredentialSource,
) -> CredentialSource {
    if non_blank(env_value).is_some() {
        CredentialSource::EnvVar(env_var)
    } else if file_value.is_some_and(|s| !s.trim().is_empty()) {
        CredentialSource::ConfigFile
    } else {
        fallback
    }
}

impl Credentials {
    /// Load credentials from environment variables, config file, or embedded defaults.
    ///
    /// Priority: env vars > config file > embedded (compile-time).
    /// Blank values are treated as unset at every level.
    /// Required: dev_id, dev_password, soft_name.
    /// Optional: user_id, user_password.
    pub fn load() -> Result<Self, ScrapeError> {
        // Try config file first as base values
        let config = load_config_file();

        let dev_id = resolve(
            "SCREENSCRAPER_DEVID",
            config.as_ref().and_then(|c| c.dev_id.clone()),
        )
        .or_else(embedded_dev_id)
        .ok_or_else(|| {
            ScrapeError::Config(
                "Missing dev_id. Set SCREENSCRAPER_DEVID env var or add to config file".to_string(),
            )
        })?;

        let dev_password = resolve(
            "SCREENSCRAPER_DEVPASSWORD",
            config.as_ref().and_then(|c| c.dev_password.clone()),
        )
        .or_else(embedded_dev_password)
        .ok_or_else(|| {
            ScrapeError::Config(
                "Missing dev_password. Set SCREENSCRAPER_DEVPASSWORD env var or add to config file"
                    .to_string(),
            )
        })?;

        let soft_name = resolve(
            "SCREENSCRAPER_SOFTNAME",
            config.as_ref().and_then(|c| c.soft_name.clone()),
        )
        .unwrap_or_else(|| "retro-junk".to_string());

        let user_id = resolve(
            "SCREENSCRAPER_SSID",
            config.as_ref().and_then(|c| c.user_id.clone()),
        )
        .unwrap_or_default();

        let user_password = resolve(
            "SCREENSCRAPER_SSPASSWORD",
            config.as_ref().and_then(|c| c.user_password.clone()),
        )
        .unwrap_or_default();

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
            self.user_id = id;
        }
        if let Some(pw) = user_password {
            self.user_password = pw;
        }
        self
    }
}

/// Static description of one credential field: where it can be set and what
/// it is for. Single source of truth for CLI/GUI help text.
#[derive(Debug, Clone, Copy)]
pub struct CredentialFieldMeta {
    /// TOML key under `[screenscraper]` (also the canonical field name).
    pub key: &'static str,
    /// Human-readable label.
    pub label: &'static str,
    /// Environment variable that overrides the config file.
    pub env_var: &'static str,
    /// Whether the API cannot be used without this field.
    pub required: bool,
    /// What the value is for.
    pub description: &'static str,
    /// Where a user obtains the value.
    pub how_to_obtain: &'static str,
}

/// All ScreenScraper credential fields, in display order.
pub static CREDENTIAL_FIELDS: [CredentialFieldMeta; 5] = [
    CredentialFieldMeta {
        key: "dev_id",
        label: "Developer ID",
        env_var: "SCREENSCRAPER_DEVID",
        required: true,
        description: "ScreenScraper developer API ID. Identifies the application (not you) \
                      to the API; every request requires it. Official builds ship with one \
                      embedded, so you normally only need this when building from source.",
        how_to_obtain: "Request developer API access from the ScreenScraper team via the \
                        forums at https://www.screenscraper.fr (developer registration is \
                        manual and granted per application).",
    },
    CredentialFieldMeta {
        key: "dev_password",
        label: "Developer password",
        env_var: "SCREENSCRAPER_DEVPASSWORD",
        required: true,
        description: "Password paired with the developer ID. Required alongside it for \
                      every API request.",
        how_to_obtain: "Issued together with the developer ID when ScreenScraper grants \
                        developer API access.",
    },
    CredentialFieldMeta {
        key: "soft_name",
        label: "Software name",
        env_var: "SCREENSCRAPER_SOFTNAME",
        required: false,
        description: "Name this application reports to the ScreenScraper API. Defaults to \
                      \"retro-junk\"; there is rarely a reason to change it.",
        how_to_obtain: "Free-form — no registration needed. Leave unset to use the default.",
    },
    CredentialFieldMeta {
        key: "user_id",
        label: "User ID",
        env_var: "SCREENSCRAPER_SSID",
        required: false,
        description: "Your personal ScreenScraper account username. Optional, but raises \
                      your daily request quota and allowed download threads — recommended \
                      when scraping more than a handful of games.",
        how_to_obtain: "Create a free account at https://www.screenscraper.fr (Inscription). \
                        Donating members get higher quotas.",
    },
    CredentialFieldMeta {
        key: "user_password",
        label: "User password",
        env_var: "SCREENSCRAPER_SSPASSWORD",
        required: false,
        description: "Password for your personal ScreenScraper account, used together with \
                      the user ID.",
        how_to_obtain: "Chosen when you register your account at https://www.screenscraper.fr.",
    },
];

impl CredentialSources {
    /// Look up a field's provenance by its canonical key.
    pub fn by_key(&self, key: &str) -> Option<&CredentialSource> {
        match key {
            "dev_id" => Some(&self.dev_id),
            "dev_password" => Some(&self.dev_password),
            "soft_name" => Some(&self.soft_name),
            "user_id" => Some(&self.user_id),
            "user_password" => Some(&self.user_password),
            _ => None,
        }
    }
}

/// Starter contents written when creating a fresh credentials file.
///
/// Every key is present but commented out: an uncommented empty string would
/// count as "set in config file" during provenance checks, which is not what
/// a template should do.
const CONFIG_TEMPLATE: &str = r#"# retro-junk credentials
#
# Uncomment a line and fill in its value to set it. Environment variables
# take priority over this file:
#   SCREENSCRAPER_DEVID, SCREENSCRAPER_DEVPASSWORD, SCREENSCRAPER_SOFTNAME,
#   SCREENSCRAPER_SSID, SCREENSCRAPER_SSPASSWORD

[screenscraper]
# Developer API credentials. Official builds embed a set, so these are only
# needed when building from source. Granted by the ScreenScraper team via
# https://www.screenscraper.fr forums.
#dev_id = ""
#dev_password = ""

# Software name reported to the API (default: "retro-junk").
#soft_name = ""

# Your personal ScreenScraper account (optional; raises rate limits).
# Register free at https://www.screenscraper.fr
#user_id = ""
#user_password = ""
"#;

/// Ensure the credentials config file exists, writing a commented template if
/// it does not. Returns the path and whether a new file was created.
pub fn ensure_config_file() -> Result<(PathBuf, bool), ScrapeError> {
    let path = config_path()
        .ok_or_else(|| ScrapeError::Config("Could not determine config directory".to_string()))?;
    if path.exists() {
        return Ok((path, false));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, CONFIG_TEMPLATE)?;
    Ok((path, true))
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
    let path = config_path()
        .ok_or_else(|| ScrapeError::Config("Could not determine config directory".to_string()))?;

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
            user_id: non_blank(Some(creds.user_id.clone())),
            user_password: non_blank(Some(creds.user_password.clone())),
        }),
    };

    let toml_str = toml::to_string_pretty(&config)
        .map_err(|e| ScrapeError::Config(format!("Failed to serialize config: {}", e)))?;

    std::fs::write(&path, toml_str)?;
    Ok(path)
}

/// Determine where each credential field is coming from.
///
/// Uses the same blank-means-unset rules as [`Credentials::load`], so the
/// reported source always matches what `load()` would actually use.
pub fn credential_sources() -> CredentialSources {
    let config = load_config_file();

    let source = |env_var: &'static str,
                  file_value: Option<&String>,
                  fallback: CredentialSource|
     -> CredentialSource {
        source_from(
            env_var,
            std::env::var(env_var).ok(),
            file_value.map(String::as_str),
            fallback,
        )
    };

    let dev_fallback = || {
        if has_embedded_dev_credentials() {
            CredentialSource::Embedded
        } else {
            CredentialSource::Missing
        }
    };

    CredentialSources {
        dev_id: source(
            "SCREENSCRAPER_DEVID",
            config.as_ref().and_then(|c| c.dev_id.as_ref()),
            dev_fallback(),
        ),
        dev_password: source(
            "SCREENSCRAPER_DEVPASSWORD",
            config.as_ref().and_then(|c| c.dev_password.as_ref()),
            dev_fallback(),
        ),
        soft_name: source(
            "SCREENSCRAPER_SOFTNAME",
            config.as_ref().and_then(|c| c.soft_name.as_ref()),
            CredentialSource::Default,
        ),
        user_id: source(
            "SCREENSCRAPER_SSID",
            config.as_ref().and_then(|c| c.user_id.as_ref()),
            CredentialSource::Missing,
        ),
        user_password: source(
            "SCREENSCRAPER_SSPASSWORD",
            config.as_ref().and_then(|c| c.user_password.as_ref()),
            CredentialSource::Missing,
        ),
    }
}

fn load_config_file() -> Option<ScreenScraperConfig> {
    let path = config_path()?;
    let content = std::fs::read_to_string(&path).ok()?;
    let config: ConfigFile = toml::from_str(&content).ok()?;
    config.screenscraper
}
