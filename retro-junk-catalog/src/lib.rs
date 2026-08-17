//! Game catalog data model types, YAML I/O, and No-Intro name parsing.
//!
//! This crate defines the persistent data model for the game catalog without
//! any database dependencies. Consumers can use these types directly for
//! serialization, display, or passing to `retro-junk-db` for persistence.

pub mod content_id;
pub mod name_parser;
pub mod types;
pub mod yaml;

pub use content_id::{ContentIdError, ContentPart};
pub use name_parser::{
    DumpStatus, ParsedDatName, is_carrier_only_flag, parse_dat_name, region_slug_to_display,
    region_to_slug,
};
pub use types::*;
pub use yaml::{load_catalog, load_companies, load_overrides, load_platforms};
