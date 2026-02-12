//! Microsoft console disc image analyzers.
//!
//! This crate provides disc analysis implementations for Microsoft consoles:
//!
//! - Xbox (Original)
//! - Xbox 360

pub mod xbox;
pub mod xbox360;

pub use xbox::XboxAnalyzer;
pub use xbox360::Xbox360Analyzer;
