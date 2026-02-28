use std::io::Write;

use indicatif::{ProgressBar, ProgressStyle};
use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;

use crate::CliError;

fn mask_value(s: &str) -> String {
    if s.len() <= 2 {
        "****".to_string()
    } else {
        format!("{}****", &s[..2])
    }
}

/// Show current credentials and their sources.
pub(crate) fn run_credentials_show() -> Result<(), CliError> {
    use retro_junk_scraper::CredentialSource;

    let path = retro_junk_scraper::config_path();
    let sources = retro_junk_scraper::credential_sources();

    log::info!(
        "{}",
        "ScreenScraper Configuration".if_supports_color(Stdout, |t| t.bold()),
    );
    crate::log_blank();

    // Config file status
    match &path {
        Some(p) if p.exists() => {
            log::info!(
                "  Config file: {} {}",
                p.display().if_supports_color(Stdout, |t| t.cyan()),
                "(exists)".if_supports_color(Stdout, |t| t.green()),
            );
        }
        Some(p) => {
            log::info!(
                "  Config file: {} {}",
                p.display().if_supports_color(Stdout, |t| t.cyan()),
                "(not found)".if_supports_color(Stdout, |t| t.dimmed()),
            );
        }
        None => {
            log::info!(
                "  Config file: {}",
                "could not determine path".if_supports_color(Stdout, |t| t.red()),
            );
        }
    }
    crate::log_blank();

    // Resolve values per-field (Credentials::load() would fail if required fields are missing)
    let creds = retro_junk_scraper::Credentials::load().ok();

    let get_value = |source: &CredentialSource,
                     from_creds: Option<String>,
                     is_secret: bool|
     -> Option<String> {
        match source {
            CredentialSource::Missing => None,
            CredentialSource::Default => Some("retro-junk".to_string()),
            CredentialSource::Embedded => {
                from_creds.map(|v| if is_secret { mask_value(&v) } else { v })
            }
            CredentialSource::EnvVar(var) => {
                let v = std::env::var(var).ok()?;
                Some(if is_secret { mask_value(&v) } else { v })
            }
            CredentialSource::ConfigFile => {
                from_creds.map(|v| if is_secret { mask_value(&v) } else { v })
            }
        }
    };

    let fields: &[(&str, &CredentialSource, Option<String>)] = &[
        (
            "dev_id",
            &sources.dev_id,
            get_value(
                &sources.dev_id,
                creds.as_ref().map(|c| c.dev_id.clone()),
                false,
            ),
        ),
        (
            "dev_password",
            &sources.dev_password,
            get_value(
                &sources.dev_password,
                creds.as_ref().map(|c| c.dev_password.clone()),
                true,
            ),
        ),
        (
            "soft_name",
            &sources.soft_name,
            get_value(
                &sources.soft_name,
                creds.as_ref().map(|c| c.soft_name.clone()),
                false,
            ),
        ),
        (
            "user_id",
            &sources.user_id,
            get_value(
                &sources.user_id,
                creds.as_ref().and_then(|c| c.user_id.clone()),
                false,
            ),
        ),
        (
            "user_password",
            &sources.user_password,
            get_value(
                &sources.user_password,
                creds.as_ref().and_then(|c| c.user_password.clone()),
                true,
            ),
        ),
    ];

    for (name, source, value) in fields {
        let source_str = format!("({})", source);
        match value {
            Some(v) => {
                log::info!(
                    "  {} {} {}",
                    format!("{}:", name).if_supports_color(Stdout, |t| t.cyan()),
                    v,
                    source_str.if_supports_color(Stdout, |t| t.dimmed()),
                );
            }
            None => {
                log::info!(
                    "  {} {} {}",
                    format!("{}:", name).if_supports_color(Stdout, |t| t.cyan()),
                    "not set".if_supports_color(Stdout, |t| t.yellow()),
                    source_str.if_supports_color(Stdout, |t| t.dimmed()),
                );
            }
        }
    }

    Ok(())
}

/// Interactively set up credentials.
pub(crate) fn run_credentials_setup() -> Result<(), CliError> {
    println!(
        "{}",
        "ScreenScraper Credential Setup".if_supports_color(Stdout, |t| t.bold()),
    );
    println!();

    // Load existing config as defaults
    let existing = retro_junk_scraper::Credentials::load().ok();

    let read_line = |prompt: &str, default: Option<&str>, required: bool| -> Option<String> {
        loop {
            if let Some(def) = default {
                print!("  {} [{}]: ", prompt, def);
            } else {
                print!("  {}: ", prompt);
            }
            let _ = std::io::stdout().flush();

            let mut input = String::new();
            let _ = std::io::stdin().read_line(&mut input);
            let trimmed = input.trim().to_string();

            if trimmed.is_empty() {
                if let Some(def) = default {
                    return Some(def.to_string());
                }
                if required {
                    println!(
                        "    {}",
                        "This field is required.".if_supports_color(Stdout, |t| t.yellow()),
                    );
                    continue;
                }
                return None;
            }
            return Some(trimmed);
        }
    };

    let has_embedded = retro_junk_scraper::has_embedded_dev_credentials();

    let (dev_id, dev_password) = if has_embedded {
        println!(
            "  {}",
            "Developer credentials: embedded in binary (no setup needed)"
                .if_supports_color(Stdout, |t| t.dimmed()),
        );
        // Use whatever load() resolved (embedded or overridden)
        let base = existing.as_ref();
        (
            base.map(|c| c.dev_id.clone())
                .unwrap_or_else(|| "embedded".to_string()),
            base.map(|c| c.dev_password.clone())
                .unwrap_or_else(|| "embedded".to_string()),
        )
    } else {
        println!(
            "  {}",
            "Developer credentials (required):".if_supports_color(Stdout, |t| t.dimmed()),
        );
        // SAFETY: read_line with required=true always returns Some
        let dev_id =
            read_line("dev_id", existing.as_ref().map(|c| c.dev_id.as_str()), true).unwrap();
        let dev_password = read_line(
            "dev_password",
            existing.as_ref().map(|c| c.dev_password.as_str()),
            true,
        )
        .unwrap();
        (dev_id, dev_password)
    };

    println!();
    println!(
        "  {}",
        "User credentials (optional, press Enter to skip):"
            .if_supports_color(Stdout, |t| t.dimmed()),
    );
    let user_id = read_line(
        "user_id",
        existing.as_ref().and_then(|c| c.user_id.as_deref()),
        false,
    );
    let user_password = read_line(
        "user_password",
        existing.as_ref().and_then(|c| c.user_password.as_deref()),
        false,
    );

    let creds = retro_junk_scraper::Credentials {
        dev_id,
        dev_password,
        soft_name: existing
            .map(|c| c.soft_name)
            .unwrap_or_else(|| "retro-junk".to_string()),
        user_id,
        user_password,
    };

    let path = retro_junk_scraper::save_to_file(&creds)
        .map_err(|e| CliError::config(format!("Failed to save credentials: {}", e)))?;

    println!();
    log::info!(
        "{} Credentials saved to {}",
        "\u{2714}".if_supports_color(Stdout, |t| t.green()),
        path.display().if_supports_color(Stdout, |t| t.cyan()),
    );

    Ok(())
}

/// Test credentials against the ScreenScraper API.
pub(crate) fn run_credentials_test(quiet: bool) -> Result<(), CliError> {
    let creds = match retro_junk_scraper::Credentials::load() {
        Ok(c) => c,
        Err(e) => {
            log::warn!(
                "{} Failed to load credentials: {}",
                "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                e,
            );
            log::warn!("");
            log::warn!("Run 'retro-junk credentials setup' to configure credentials.");
            return Ok(());
        }
    };

    log::info!("Testing credentials against ScreenScraper API...");

    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| CliError::runtime(format!("Failed to create tokio runtime: {}", e)))?;

    rt.block_on(async {
        let pb = if quiet {
            ProgressBar::hidden()
        } else {
            let pb = ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::with_template("  {spinner:.cyan} {msg}")
                    .expect("static pattern")
                    .tick_chars("/-\\|"),
            );
            pb.set_message("Connecting...");
            pb.enable_steady_tick(std::time::Duration::from_millis(100));
            pb
        };

        match retro_junk_scraper::ScreenScraperClient::new(creds).await {
            Ok((_client, user_info)) => {
                pb.finish_and_clear();
                log::info!(
                    "{} Credentials are valid!",
                    "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                );
                crate::log_blank();
                log::info!(
                    "  Requests today: {}/{}",
                    user_info.requests_today(),
                    user_info.max_requests_per_day(),
                );
                log::info!("  Max threads:    {}", user_info.max_threads());
            }
            Err(e) => {
                pb.finish_and_clear();
                log::warn!(
                    "{} Credential validation failed: {}",
                    "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                    e,
                );
            }
        }
    });

    Ok(())
}

/// Print the credentials file path.
pub(crate) fn run_credentials_path() -> Result<(), CliError> {
    match retro_junk_scraper::config_path() {
        Some(path) => log::info!("{}", path.display()),
        None => {
            return Err(CliError::config("Could not determine config directory"));
        }
    }

    Ok(())
}
