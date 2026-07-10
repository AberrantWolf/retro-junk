//! Unit tests for the pure helpers in `catalog_ops`.

use super::{catalog_data_dir, parse_limit};
use std::path::PathBuf;

#[test]
fn parse_limit_handles_empty_and_invalid() {
    assert_eq!(parse_limit(""), None);
    assert_eq!(parse_limit("   "), None);
    assert_eq!(parse_limit("abc"), None);
    assert_eq!(parse_limit("10"), Some(10));
    assert_eq!(parse_limit("  25 "), Some(25));
}

#[test]
fn catalog_data_dir_falls_back_to_default() {
    assert_eq!(catalog_data_dir(""), PathBuf::from("catalog"));
    assert_eq!(catalog_data_dir("   "), PathBuf::from("catalog"));
    assert_eq!(
        catalog_data_dir("/data/catalog"),
        PathBuf::from("/data/catalog")
    );
}
