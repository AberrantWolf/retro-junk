//! Nintendo console ROM analyzers.
//!
//! This crate provides ROM analysis implementations for Nintendo consoles:
//!
//! - NES (Famicom)
//! - SNES (Super Famicom)
//! - Nintendo 64
//! - GameCube
//! - Wii
//! - Wii U
//! - Game Boy / Game Boy Color
//! - Game Boy Advance
//! - Nintendo DS
//! - Nintendo 3DS

pub(crate) mod constants;
pub mod ds;
pub mod gameboy;
pub mod gamecube;
pub mod gba;
pub(crate) mod licensee;
pub mod n3ds;
pub mod n64;
pub(crate) mod n64_byteorder;
pub mod nes;
pub mod snes;
pub mod wii;
pub mod wiiu;

pub use ds::DsAnalyzer;
pub use gameboy::GameBoyAnalyzer;
pub use gamecube::GameCubeAnalyzer;
pub use gba::GbaAnalyzer;
pub use n3ds::N3dsAnalyzer;
pub use n64::N64Analyzer;
pub use nes::NesAnalyzer;
pub use snes::SnesAnalyzer;
pub use wii::WiiAnalyzer;
pub use wiiu::WiiUAnalyzer;
