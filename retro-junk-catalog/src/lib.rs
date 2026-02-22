//! Game catalog data model types, YAML I/O, and No-Intro name parsing.
//!
//! This crate defines the persistent data model for the game catalog without
//! any database dependencies. Consumers can use these types directly for
//! serialization, display, or passing to `retro-junk-db` for persistence.

pub mod name_parser;
pub mod types;
pub mod yaml;

pub use name_parser::{parse_dat_name, region_to_slug, DumpStatus, ParsedDatName};
pub use types::*;
pub use yaml::{load_catalog, load_companies, load_overrides, load_platforms};
