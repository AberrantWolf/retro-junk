//! YAML loading for human-curated catalog data.
//!
//! Loads platform definitions, company profiles, and data overrides
//! from the `catalog/` directory.

use crate::types::{CatalogPlatform, Company, Override};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum YamlError {
    #[error("I/O error reading {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("YAML parse error in {path}: {source}")]
    Parse {
        path: String,
        source: serde_yml::Error,
    },
    #[error("Directory not found: {0}")]
    DirNotFound(String),
}

/// Result type for [`load_catalog`] containing all loaded catalog data.
pub type CatalogData = (Vec<CatalogPlatform>, Vec<Company>, Vec<Override>);

/// Load all platform definitions from YAML files in a directory.
///
/// Each `.yaml` file in the directory should contain a single `CatalogPlatform`.
pub fn load_platforms(dir: &Path) -> Result<Vec<CatalogPlatform>, YamlError> {
    load_yaml_dir(dir)
}

/// Load all company definitions from YAML files in a directory.
///
/// Each `.yaml` file should contain a single `Company`.
pub fn load_companies(dir: &Path) -> Result<Vec<Company>, YamlError> {
    load_yaml_dir(dir)
}

/// Load all override definitions from YAML files in a directory.
///
/// Each `.yaml` file should contain a YAML sequence (list) of `Override` entries.
pub fn load_overrides(dir: &Path) -> Result<Vec<Override>, YamlError> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    if !dir.is_dir() {
        return Err(YamlError::DirNotFound(dir.display().to_string()));
    }

    let mut all = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| YamlError::Io {
            path: dir.display().to_string(),
            source: e,
        })?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext == "yaml" || ext == "yml")
        })
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        let contents = std::fs::read_to_string(&path).map_err(|e| YamlError::Io {
            path: path.display().to_string(),
            source: e,
        })?;
        let overrides: Vec<Override> =
            serde_yml::from_str(&contents).map_err(|e| YamlError::Parse {
                path: path.display().to_string(),
                source: e,
            })?;
        all.extend(overrides);
    }

    Ok(all)
}

/// Load all catalog data from the standard directory layout.
///
/// Expected structure:
/// ```text
/// catalog_dir/
///   platforms/
///     nes.yaml
///     snes.yaml
///     ...
///   companies/
///     nintendo.yaml
///     capcom.yaml
///     ...
///   overrides/
///     psx-serials.yaml
///     ...
/// ```
pub fn load_catalog(catalog_dir: &Path) -> Result<CatalogData, YamlError> {
    let platforms = load_platforms(&catalog_dir.join("platforms"))?;
    let companies = load_companies(&catalog_dir.join("companies"))?;
    let overrides = load_overrides(&catalog_dir.join("overrides"))?;
    Ok((platforms, companies, overrides))
}

/// Generic helper: load all YAML files in a directory, each containing a single `T`.
fn load_yaml_dir<T: serde::de::DeserializeOwned>(dir: &Path) -> Result<Vec<T>, YamlError> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    if !dir.is_dir() {
        return Err(YamlError::DirNotFound(dir.display().to_string()));
    }

    let mut items = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| YamlError::Io {
            path: dir.display().to_string(),
            source: e,
        })?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext == "yaml" || ext == "yml")
        })
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        let contents = std::fs::read_to_string(&path).map_err(|e| YamlError::Io {
            path: path.display().to_string(),
            source: e,
        })?;
        let item: T = serde_yml::from_str(&contents).map_err(|e| YamlError::Parse {
            path: path.display().to_string(),
            source: e,
        })?;
        items.push(item);
    }

    Ok(items)
}
