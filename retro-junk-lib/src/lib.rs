// Re-export everything from retro-junk-core for backwards compatibility.
// Note: AnalysisOptions is defined in retro-junk-core now, not in context.rs.
pub use retro_junk_core::*;

// Modules that still live in retro-junk-lib:
pub mod async_util;
pub mod context;
pub mod display;
pub mod hasher;
pub mod rename;
pub mod repair;
pub mod scanner;
pub mod util;

// Re-export context items at crate root for backwards compatibility.
pub use context::{AnalysisContext, Console, ConsoleFolder, FolderScanResult, RegisteredConsole};

/// Create an `AnalysisContext` with all built-in console analyzers registered.
///
/// Registers all 25 analyzers: NES, SNES, N64, GameCube, Wii, Wii U, GB, GBA,
/// DS, 3DS, PS1, PS2, PS3, PSP, Vita, SG-1000, Master System, Genesis, Sega CD,
/// 32X, Saturn, Dreamcast, Game Gear, Xbox, Xbox 360.
pub fn create_default_context() -> AnalysisContext {
    let mut ctx = AnalysisContext::new();

    // Nintendo
    ctx.register(retro_junk_nintendo::NesAnalyzer::new());
    ctx.register(retro_junk_nintendo::SnesAnalyzer::new());
    ctx.register(retro_junk_nintendo::N64Analyzer::new());
    ctx.register(retro_junk_nintendo::GameCubeAnalyzer::new());
    ctx.register(retro_junk_nintendo::WiiAnalyzer::new());
    ctx.register(retro_junk_nintendo::WiiUAnalyzer::new());
    ctx.register(retro_junk_nintendo::GameBoyAnalyzer::new());
    ctx.register(retro_junk_nintendo::GbaAnalyzer::new());
    ctx.register(retro_junk_nintendo::DsAnalyzer::new());
    ctx.register(retro_junk_nintendo::N3dsAnalyzer::new());

    // Sony
    ctx.register(retro_junk_sony::Ps1Analyzer::new());
    ctx.register(retro_junk_sony::Ps2Analyzer::new());
    ctx.register(retro_junk_sony::Ps3Analyzer::new());
    ctx.register(retro_junk_sony::PspAnalyzer::new());
    ctx.register(retro_junk_sony::VitaAnalyzer::new());

    // Sega
    ctx.register(retro_junk_sega::Sg1000Analyzer::new());
    ctx.register(retro_junk_sega::MasterSystemAnalyzer::new());
    ctx.register(retro_junk_sega::GenesisAnalyzer::new());
    ctx.register(retro_junk_sega::SegaCdAnalyzer::new());
    ctx.register(retro_junk_sega::Sega32xAnalyzer::new());
    ctx.register(retro_junk_sega::SaturnAnalyzer::new());
    ctx.register(retro_junk_sega::DreamcastAnalyzer::new());
    ctx.register(retro_junk_sega::GameGearAnalyzer::new());

    // Microsoft
    ctx.register(retro_junk_microsoft::XboxAnalyzer::new());
    ctx.register(retro_junk_microsoft::Xbox360Analyzer::new());

    ctx
}
