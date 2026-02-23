//! Data model types for the game catalog.
//!
//! These types represent the persistent catalog schema: platforms, companies,
//! works, releases, media, assets, collections, and import tracking.

use serde::{Deserialize, Serialize};

// ── Platform ────────────────────────────────────────────────────────────────

/// A game platform/console definition, loaded from YAML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogPlatform {
    pub id: String,
    pub display_name: String,
    pub short_name: String,
    pub manufacturer: String,
    #[serde(default)]
    pub generation: Option<u32>,
    pub media_type: MediaType,
    #[serde(default)]
    pub release_year: Option<u32>,
    #[serde(default)]
    pub description: Option<String>,
    /// Links to retro-junk-core Platform enum variant name (e.g., "Nes", "Snes").
    #[serde(default)]
    pub core_platform: Option<String>,
    #[serde(default)]
    pub regions: Vec<PlatformRegion>,
    #[serde(default)]
    pub relationships: Vec<PlatformRelationshipEntry>,
}

/// Physical media type for a platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaType {
    Cartridge,
    Disc,
    Card,
    Digital,
}

/// A platform's regional release info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformRegion {
    pub region: String,
    #[serde(default)]
    pub release_date: Option<String>,
}

/// A relationship entry in platform YAML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformRelationshipEntry {
    pub platform: String,
    #[serde(rename = "type")]
    pub relationship_type: PlatformRelationship,
}

/// Types of relationships between platforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformRelationship {
    RegionalVariant,
    Successor,
    Addon,
    Compatible,
}

// ── Company ─────────────────────────────────────────────────────────────────

/// A company (publisher/developer) definition, loaded from YAML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Company {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
}

// ── Work ────────────────────────────────────────────────────────────────────

/// An abstract game concept — the "work" that may have multiple regional releases.
#[derive(Debug, Clone)]
pub struct Work {
    pub id: String,
    pub canonical_name: String,
    pub created_at: String,
    pub updated_at: String,
}

/// A relationship between two works.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkRelationship {
    Sequel,
    Prequel,
    Remake,
    Port,
    Remaster,
    Dlc,
}

// ── Release ─────────────────────────────────────────────────────────────────

/// A regional release of a work on a specific platform.
#[derive(Debug, Clone)]
pub struct Release {
    pub id: String,
    pub work_id: String,
    pub platform_id: String,
    pub region: String,
    /// Software revision identifier: "Rev A", "v1.0", etc. Empty string for original.
    pub revision: String,
    /// Marketing/edition label: "Greatest Hits", "Player's Choice", etc. Empty string for standard.
    pub variant: String,
    pub title: String,
    pub alt_title: Option<String>,
    pub publisher_id: Option<String>,
    pub developer_id: Option<String>,
    pub release_date: Option<String>,
    pub game_serial: Option<String>,
    pub genre: Option<String>,
    pub players: Option<String>,
    pub rating: Option<f64>,
    pub description: Option<String>,
    pub screenscraper_id: Option<String>,
    pub scraper_not_found: bool,
    pub created_at: String,
    pub updated_at: String,
}

// ── Media ───────────────────────────────────────────────────────────────────

/// A physical or digital media artifact (ROM, disc, cartridge).
#[derive(Debug, Clone)]
pub struct Media {
    pub id: String,
    pub release_id: String,
    pub media_serial: Option<String>,
    pub disc_number: Option<i32>,
    pub disc_label: Option<String>,
    pub revision: Option<String>,
    pub status: MediaStatus,
    pub dat_name: Option<String>,
    pub dat_source: Option<String>,
    pub file_size: Option<i64>,
    pub crc32: Option<String>,
    pub sha1: Option<String>,
    pub md5: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Status of a media dump.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaStatus {
    Verified,
    Bad,
    Overdump,
    Prototype,
    Beta,
    Sample,
}

impl Default for MediaStatus {
    fn default() -> Self {
        Self::Verified
    }
}

impl MediaStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Bad => "bad",
            Self::Overdump => "overdump",
            Self::Prototype => "prototype",
            Self::Beta => "beta",
            Self::Sample => "sample",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "bad" => Self::Bad,
            "overdump" => Self::Overdump,
            "prototype" | "proto" => Self::Prototype,
            "beta" => Self::Beta,
            "sample" => Self::Sample,
            _ => Self::Verified,
        }
    }
}

// ── Media Asset ─────────────────────────────────────────────────────────────

/// An art/media asset associated with a release or specific media.
#[derive(Debug, Clone)]
pub struct MediaAsset {
    pub id: i64,
    pub release_id: Option<String>,
    pub media_id: Option<String>,
    pub asset_type: String,
    pub region: Option<String>,
    pub source: String,
    pub file_path: Option<String>,
    pub source_url: Option<String>,
    pub scraped: bool,
    pub file_hash: Option<String>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub created_at: String,
}

// ── Collection ──────────────────────────────────────────────────────────────

/// A user's ownership record for a specific media entry.
#[derive(Debug, Clone)]
pub struct CollectionEntry {
    pub id: i64,
    pub media_id: String,
    pub user_id: String,
    pub owned: bool,
    pub condition: Option<String>,
    pub notes: Option<String>,
    pub date_acquired: Option<String>,
    pub rom_path: Option<String>,
    pub verified_at: Option<String>,
}

// ── Import Tracking ─────────────────────────────────────────────────────────

/// Log entry for a data import operation.
#[derive(Debug, Clone)]
pub struct ImportLog {
    pub id: i64,
    pub source_type: String,
    pub source_name: String,
    pub source_version: Option<String>,
    pub imported_at: String,
    pub records_created: i64,
    pub records_updated: i64,
    pub records_unchanged: i64,
    pub disagreements_found: i64,
}

/// A detected disagreement between two data sources.
#[derive(Debug, Clone)]
pub struct Disagreement {
    pub id: i64,
    pub entity_type: String,
    pub entity_id: String,
    pub field: String,
    pub source_a: String,
    pub value_a: Option<String>,
    pub source_b: String,
    pub value_b: Option<String>,
    pub resolved: bool,
    pub resolution: Option<String>,
    pub resolved_at: Option<String>,
    pub created_at: String,
}

// ── Overrides ───────────────────────────────────────────────────────────────

/// A human-curated override for known data corrections, loaded from YAML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Override {
    pub entity_type: String,
    #[serde(default)]
    pub entity_id: Option<String>,
    #[serde(default)]
    pub platform_id: Option<String>,
    #[serde(default)]
    pub dat_name_pattern: Option<String>,
    pub field: String,
    pub override_value: String,
    pub reason: String,
}
