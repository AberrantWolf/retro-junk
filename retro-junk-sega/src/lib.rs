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

pub mod sg1000;
pub mod master_system;
pub mod genesis;
pub mod sega_cd;
pub mod sega_32x;
pub mod saturn;
pub mod dreamcast;
pub mod game_gear;

pub use sg1000::Sg1000Analyzer;
pub use master_system::MasterSystemAnalyzer;
pub use genesis::GenesisAnalyzer;
pub use sega_cd::SegaCdAnalyzer;
pub use sega_32x::Sega32xAnalyzer;
pub use saturn::SaturnAnalyzer;
pub use dreamcast::DreamcastAnalyzer;
pub use game_gear::GameGearAnalyzer;
