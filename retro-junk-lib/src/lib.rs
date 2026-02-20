// Re-export everything from retro-junk-core for backwards compatibility.
// Note: AnalysisOptions is defined in retro-junk-core now, not in context.rs.
pub use retro_junk_core::*;

// Modules that still live in retro-junk-lib:
pub mod context;
pub mod hasher;
pub mod rename;
pub mod repair;
pub mod scanner;

// Re-export context items at crate root for backwards compatibility.
pub use context::{AnalysisContext, Console, ConsoleFolder, FolderScanResult, RegisteredConsole};
