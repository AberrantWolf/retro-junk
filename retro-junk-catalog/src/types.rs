//! Data model types for the game catalog.
//!
//! These types represent the persistent catalog schema: platforms, companies,
//! works, releases, media, assets, collections, and import tracking.
//!
//! # Absence conventions
//!
//! Text fields use the empty string for "not set" and numeric fields use `0`
//! where that value cannot be legitimate (a generation, a year, a file size).
//! `Option` is reserved for cases where absence is semantically distinct from
//! empty: nullable foreign keys (`publisher_id`), enrichment sentinels
//! (`screenscraper_id`, where SQL `IS NULL` drives query logic), values whose
//! zero is meaningful (`rating`), and optional enums (`tag`).

use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! database_id {
    ($name:ident) => {
        /// Primary key of a row in the corresponding database table.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(transparent)]
        pub struct $name(pub u64);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl std::str::FromStr for $name {
            type Err = std::num::ParseIntError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                value.parse().map(Self)
            }
        }
    };
}

database_id!(MediaAssetId);
database_id!(CollectionId);
database_id!(ImportLogId);
database_id!(DisagreementId);

// ── Platform ────────────────────────────────────────────────────────────────

/// A game platform/console definition, loaded from YAML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogPlatform {
    pub id: String,
    pub display_name: String,
    pub short_name: String,
    pub manufacturer: String,
    /// Console generation (1-9). 0 = unknown.
    #[serde(default)]
    pub generation: u32,
    pub media_type: MediaType,
    /// First release year. 0 = unknown.
    #[serde(default)]
    pub release_year: u32,
    #[serde(default)]
    pub description: String,
    /// Links to retro-junk-core Platform enum variant name (e.g., "Nes", "Snes").
    /// Empty when the platform has no analyzer support.
    #[serde(default)]
    pub core_platform: String,
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
    pub release_date: String,
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
    pub country: String,
    #[serde(default)]
    pub aliases: Vec<String>,
}

// ── Work ────────────────────────────────────────────────────────────────────

/// An abstract game concept — the "work" that may have multiple regional releases.
#[derive(Debug, Clone)]
pub struct Work {
    pub id: String,
    pub canonical_name: String,
    pub tag: Option<CatalogTag>,
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
    /// Software revision identifier: "Rev A", "v1.0", etc. Empty for original.
    pub revision: String,
    /// Marketing/edition label: "Greatest Hits", "Player's Choice", etc. Empty for standard.
    pub variant: String,
    pub title: String,
    pub alt_title: String,
    /// Nullable foreign key into companies; empty string would violate the constraint.
    pub publisher_id: Option<String>,
    /// Nullable foreign key into companies.
    pub developer_id: Option<String>,
    pub release_date: String,
    pub game_serial: String,
    pub genre: String,
    pub players: String,
    /// `None` = never rated; `Some(0.0)` is a legitimate rating.
    pub rating: Option<f64>,
    pub description: String,
    pub screen_title: String,
    pub cover_title: String,
    /// Enrichment sentinel: SQL `IS NULL` marks releases still needing enrichment.
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
    pub media_serial: String,
    /// Disc number within a multi-disc set. 0 = not disc-numbered media.
    pub disc_number: i32,
    pub disc_label: String,
    pub revision: String,
    pub status: MediaStatus,
    pub tag: Option<CatalogTag>,
    pub dat_name: String,
    /// Primary ROM filename from the source DAT, including extension.
    pub rom_name: String,
    pub dat_source: String,
    /// Size in bytes. 0 = unknown (a real dump is never empty).
    pub file_size: i64,
    pub crc32: String,
    pub sha1: String,
    pub md5: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Status of a media dump.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MediaStatus {
    #[default]
    Verified,
    Bad,
    Overdump,
    Prototype,
    Beta,
    Sample,
}

impl MediaStatus {
    #[must_use]
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

    #[must_use]
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

/// A user-applied tag for homebrew or modded games.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CatalogTag {
    Homebrew,
    Modded,
}

impl CatalogTag {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Homebrew => "homebrew",
            Self::Modded => "modded",
        }
    }

    #[must_use]
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "homebrew" => Some(Self::Homebrew),
            "modded" => Some(Self::Modded),
            _ => None,
        }
    }
}

// ── Asset ────────────────────────────────────────────────────────────────────

/// The entity an asset belongs to: a release (most art) or one specific
/// media entry (e.g., a per-disc label scan).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetOwner {
    Release(String),
    Media(String),
}

impl AssetOwner {
    /// The `media_assets.release_id` column value.
    #[must_use]
    pub fn release_id(&self) -> Option<&str> {
        match self {
            Self::Release(id) => Some(id),
            Self::Media(_) => None,
        }
    }

    /// The `media_assets.media_id` column value.
    #[must_use]
    pub fn media_id(&self) -> Option<&str> {
        match self {
            Self::Release(_) => None,
            Self::Media(id) => Some(id),
        }
    }
}

/// An art/media asset associated with a release or specific media.
#[derive(Debug, Clone)]
pub struct Asset {
    pub id: MediaAssetId,
    pub owner: AssetOwner,
    pub asset_type: String,
    pub region: String,
    pub source: String,
    pub file_path: String,
    pub source_url: String,
    pub scraped: bool,
    pub file_hash: String,
    /// Pixel dimensions. 0 = unknown.
    pub width: i32,
    pub height: i32,
    pub created_at: String,
}

// ── Collection ──────────────────────────────────────────────────────────────

/// A user's ownership record for a specific media entry.
#[derive(Debug, Clone)]
pub struct CollectionEntry {
    pub id: CollectionId,
    pub media_id: String,
    pub user_id: String,
    pub owned: bool,
    pub condition: String,
    pub notes: String,
    pub date_acquired: String,
    pub rom_path: String,
    /// Timestamp of last hash verification. Empty = never verified.
    pub verified_at: String,
}

// ── Import Tracking ─────────────────────────────────────────────────────────

/// Log entry for a data import operation.
#[derive(Debug, Clone)]
pub struct ImportLog {
    pub id: ImportLogId,
    pub source_type: String,
    pub source_name: String,
    pub source_version: String,
    pub imported_at: String,
    pub records_created: i64,
    pub records_updated: i64,
    pub records_unchanged: i64,
    pub disagreements_found: i64,
}

/// A detected disagreement between two data sources.
#[derive(Debug, Clone)]
pub struct Disagreement {
    pub id: DisagreementId,
    pub entity_type: String,
    pub entity_id: String,
    pub field: String,
    pub source_a: String,
    pub value_a: String,
    pub source_b: String,
    pub value_b: String,
    pub resolved: bool,
    pub resolution: String,
    pub resolved_at: String,
    pub created_at: String,
}

// ── Overrides ───────────────────────────────────────────────────────────────

/// A human-curated override for known data corrections, loaded from YAML.
///
/// Targeting is by exact entity id, by platform, and/or by DAT-name pattern;
/// unused selectors stay empty.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Override {
    pub entity_type: String,
    #[serde(default)]
    pub entity_id: String,
    #[serde(default)]
    pub platform_id: String,
    #[serde(default)]
    pub dat_name_pattern: String,
    pub field: String,
    pub override_value: String,
    pub reason: String,
}
