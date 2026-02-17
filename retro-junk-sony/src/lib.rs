//! Sony console disc image and ROM analyzers.
//!
//! This crate provides ROM/disc analysis implementations for Sony consoles:
//!
//! - PlayStation (PS1/PSX)
//! - PlayStation 2 (PS2)
//! - PlayStation 3 (PS3)
//! - PlayStation Portable (PSP)
//! - PlayStation Vita

pub mod ps1;
mod ps1_disc;
pub mod ps2;
pub mod ps3;
pub mod psp;
pub mod vita;

pub use ps1::Ps1Analyzer;
pub use ps2::Ps2Analyzer;
pub use ps3::Ps3Analyzer;
pub use psp::PspAnalyzer;
pub use vita::VitaAnalyzer;
