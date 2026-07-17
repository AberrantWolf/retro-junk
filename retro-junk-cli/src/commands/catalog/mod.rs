pub(crate) mod disagreements;
pub(crate) mod enrich;
pub(crate) mod enrich_gdb;
pub(crate) mod gaps;
pub(crate) mod import;
pub(crate) mod lookup;
pub(crate) mod reconcile;
pub(crate) mod reset;
pub(crate) mod scan;
pub(crate) mod stats;
pub(crate) mod unenrich;
pub(crate) mod verify;

use std::path::PathBuf;

use retro_junk_lib::util::format_bytes;

pub(crate) fn default_catalog_db_path() -> PathBuf {
    retro_junk_dat::cache::cache_dir()
        .unwrap_or_else(|_| PathBuf::from(".cache"))
        .join("catalog.db")
}

/// Default path for catalog YAML data.
pub(crate) fn default_catalog_dir() -> PathBuf {
    // Look for catalog/ relative to the current directory
    PathBuf::from("catalog")
}

/// Truncate a string to a maximum width, appending "..." if needed.
pub(crate) fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else if max > 3 {
        format!("{}...", &s[..max - 3])
    } else {
        s[..max].to_string()
    }
}

/// Format a file size in human-readable form.
pub(crate) fn format_file_size(bytes: i64) -> String {
    format_bytes(bytes as u64)
}

/// Substitute a placeholder for an empty catalog string (empty = absent).
pub(crate) fn or_str<'a>(s: &'a str, placeholder: &'a str) -> &'a str {
    if s.is_empty() { placeholder } else { s }
}

/// Format a file size, or a placeholder when unknown (0 = unknown).
pub(crate) fn format_file_size_or(bytes: i64, placeholder: &str) -> String {
    if bytes == 0 {
        placeholder.to_string()
    } else {
        format_file_size(bytes)
    }
}
