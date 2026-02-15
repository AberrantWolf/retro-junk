//! Sega console ROM/disc analyzers.
//!
//! This crate provides ROM/disc analysis implementations for Sega consoles:
//!
//! - SG-1000
//! - Master System
//! - Genesis / Mega Drive
//! - Sega CD / Mega CD
//! - 32X
//! - Saturn
//! - Dreamcast
//! - Game Gear

pub mod dreamcast;
pub mod game_gear;
pub mod genesis;
pub mod master_system;
pub mod saturn;
pub mod sega_32x;
pub mod sega_cd;
pub mod sg1000;

pub use dreamcast::DreamcastAnalyzer;
pub use game_gear::GameGearAnalyzer;
pub use genesis::GenesisAnalyzer;
pub use master_system::MasterSystemAnalyzer;
pub use saturn::SaturnAnalyzer;
pub use sega_32x::Sega32xAnalyzer;
pub use sega_cd::SegaCdAnalyzer;
pub use sg1000::Sg1000Analyzer;
