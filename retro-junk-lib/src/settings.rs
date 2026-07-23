//! Shared application settings (library path, config file location).
//!
//! Both CLI and GUI use these functions so the settings file is always
//! `~/.config/retro-junk/settings.toml` and library-path resolution is
//! consistent across frontends.

use std::io;
use std::path::{Path, PathBuf};

/// Canonical path to the shared settings file: `~/.config/retro-junk/settings.toml`.
#[must_use]
pub fn settings_path() -> PathBuf {
    let config = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    config.join("retro-junk").join("settings.toml")
}

/// Durable catalog location. Catalog and archive-index rows are long-lived
/// application data, not disposable DAT cache files.
#[must_use]
pub fn catalog_database_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("retro-junk")
        .join("catalog.db")
}

/// Whether first-run preparation will copy the legacy cache-hosted catalog to
/// its durable application-data location.
pub fn catalog_database_needs_location_migration() -> io::Result<bool> {
    let target = catalog_database_path();
    if target.exists() {
        return Ok(false);
    }
    let legacy = retro_junk_dat::cache::cache_dir()
        .map_err(io::Error::other)?
        .join("catalog.db");
    Ok(legacy.is_file())
}

/// Copy and validate the legacy cache-hosted database on first 0.4 startup.
/// The legacy file is retained as the migration backup.
pub fn ensure_catalog_database_location() -> io::Result<PathBuf> {
    let target = catalog_database_path();
    if target.exists() {
        return Ok(target);
    }
    let parent = target
        .parent()
        .ok_or_else(|| io::Error::other("catalog database has no parent directory"))?;
    std::fs::create_dir_all(parent)?;
    let legacy = retro_junk_dat::cache::cache_dir()
        .map_err(io::Error::other)?
        .join("catalog.db");
    if !legacy.is_file() {
        return Ok(target);
    }
    let temp = parent.join(".catalog.db.migrating");
    if temp.exists() {
        std::fs::remove_file(&temp)?;
    }
    std::fs::copy(&legacy, &temp)?;
    {
        let connection = retro_junk_db::open_database(&temp).map_err(io::Error::other)?;
        let result: String = connection
            .query_row("PRAGMA quick_check", [], |row| row.get(0))
            .map_err(io::Error::other)?;
        if result != "ok" {
            return Err(io::Error::other(format!(
                "migrated catalog failed SQLite quick_check: {result}"
            )));
        }
    }
    std::fs::rename(&temp, &target)?;
    log::info!(
        "Migrated catalog database to {}; retained legacy backup at {}",
        target.display(),
        legacy.display()
    );
    Ok(target)
}

/// Resolve the library root path using a priority chain:
///
/// 1. CLI override (if `Some`)
/// 2. Saved `library.current_root` in `settings.toml`
/// 3. Current working directory
#[must_use]
pub fn resolve_library_path(cli_override: Option<PathBuf>) -> PathBuf {
    if let Some(p) = cli_override {
        return p;
    }
    if let Some(p) = load_library_path() {
        return p;
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Read `library.current_root` from `settings.toml`, if set.
fn load_library_path() -> Option<PathBuf> {
    let contents = std::fs::read_to_string(settings_path()).ok()?;
    let doc: toml::Value = contents.parse().ok()?;
    let root = doc.get("library")?.get("current_root")?.as_str()?;
    if root.is_empty() {
        None
    } else {
        Some(PathBuf::from(root))
    }
}

/// Save the library path in `settings.toml`.
///
/// Uses `toml::Value` for a surgical update so GUI-specific fields
/// (`recent_roots`, `auto_scan_on_open`, etc.) are preserved.
pub fn save_library_path(path: &Path) -> io::Result<()> {
    update_library_root(Some(path))
}

/// Remove the saved library path from `settings.toml`.
///
/// Preserves all other settings, same as [`save_library_path`].
pub fn clear_library_path() -> io::Result<()> {
    update_library_root(None)
}

/// Shared surgical update of `library.current_root`.
fn update_library_root(path: Option<&Path>) -> io::Result<()> {
    let settings = settings_path();
    let mut doc: toml::Value = if let Ok(contents) = std::fs::read_to_string(&settings) {
        contents
            .parse()
            .unwrap_or_else(|_| toml::Value::Table(toml::map::Map::default()))
    } else {
        toml::Value::Table(toml::map::Map::default())
    };

    // Ensure [library] table exists
    let table = doc
        .as_table_mut()
        .ok_or_else(|| io::Error::other("settings.toml root is not a table"))?;
    let library = table
        .entry("library")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::default()));
    let lib_table = library
        .as_table_mut()
        .ok_or_else(|| io::Error::other("[library] is not a table"))?;

    match path {
        Some(p) => {
            lib_table.insert(
                "current_root".to_string(),
                toml::Value::String(p.to_string_lossy().into_owned()),
            );
        }
        None => {
            lib_table.remove("current_root");
        }
    }

    // Write atomically
    if let Some(parent) = settings.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let serialized = toml::to_string_pretty(&doc).map_err(io::Error::other)?;
    let tmp = settings.with_extension("toml.tmp");
    std::fs::write(&tmp, &serialized)?;
    std::fs::rename(&tmp, &settings)?;

    Ok(())
}

/// Load the full settings file as a pretty-printed TOML string for display.
#[must_use]
pub fn load_settings_string() -> Option<String> {
    let contents = std::fs::read_to_string(settings_path()).ok()?;
    let doc: toml::Value = contents.parse().ok()?;
    toml::to_string_pretty(&doc).ok()
}
