use std::path::PathBuf;

use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;

use crate::CliError;

/// Show all saved settings.
pub(crate) fn run_config_show() -> Result<(), CliError> {
    let path = retro_junk_lib::settings::settings_path();

    log::info!(
        "{}",
        "Application Settings".if_supports_color(Stdout, |t| t.bold()),
    );
    crate::log_blank();
    log::info!(
        "  Settings file: {}",
        path.display().if_supports_color(Stdout, |t| t.cyan()),
    );
    crate::log_blank();

    match retro_junk_lib::settings::load_settings_string() {
        Some(pretty) => {
            for line in pretty.lines() {
                log::info!("  {}", line);
            }
        }
        None => {
            log::info!(
                "  {}",
                "(no settings file found)".if_supports_color(Stdout, |t| t.dimmed()),
            );
        }
    }

    Ok(())
}

/// Show or set the library path.
pub(crate) fn run_config_library_path(
    new_path: Option<PathBuf>,
    clear: bool,
) -> Result<(), CliError> {
    if clear {
        retro_junk_lib::settings::save_library_path(None)
            .map_err(|e| CliError::config(format!("Failed to clear library path: {}", e)))?;
        log::info!(
            "{} Library path cleared",
            "\u{2714}".if_supports_color(Stdout, |t| t.green()),
        );
        return Ok(());
    }

    if let Some(path) = new_path {
        let canonical = path.canonicalize().unwrap_or(path);
        retro_junk_lib::settings::save_library_path(Some(&canonical))
            .map_err(|e| CliError::config(format!("Failed to save library path: {}", e)))?;
        log::info!(
            "{} Library path set to: {}",
            "\u{2714}".if_supports_color(Stdout, |t| t.green()),
            canonical.display().if_supports_color(Stdout, |t| t.cyan()),
        );
    } else {
        // Display current library path
        let resolved = retro_junk_lib::settings::resolve_library_path(None);
        log::info!("{}", resolved.display());
    }

    Ok(())
}
