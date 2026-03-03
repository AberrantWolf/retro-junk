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
pub mod settings;
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
    ctx.register(retro_junk_nintendo::NesAnalyzer);
    ctx.register(retro_junk_nintendo::SnesAnalyzer);
    ctx.register(retro_junk_nintendo::N64Analyzer);
    ctx.register(retro_junk_nintendo::GameCubeAnalyzer);
    ctx.register(retro_junk_nintendo::WiiAnalyzer);
    ctx.register(retro_junk_nintendo::WiiUAnalyzer);
    ctx.register(retro_junk_nintendo::GameBoyAnalyzer);
    ctx.register(retro_junk_nintendo::GbaAnalyzer);
    ctx.register(retro_junk_nintendo::DsAnalyzer);
    ctx.register(retro_junk_nintendo::N3dsAnalyzer);

    // Sony
    ctx.register(retro_junk_sony::Ps1Analyzer);
    ctx.register(retro_junk_sony::Ps2Analyzer);
    ctx.register(retro_junk_sony::Ps3Analyzer);
    ctx.register(retro_junk_sony::PspAnalyzer);
    ctx.register(retro_junk_sony::VitaAnalyzer);

    // Sega
    ctx.register(retro_junk_sega::Sg1000Analyzer);
    ctx.register(retro_junk_sega::MasterSystemAnalyzer);
    ctx.register(retro_junk_sega::GenesisAnalyzer);
    ctx.register(retro_junk_sega::SegaCdAnalyzer);
    ctx.register(retro_junk_sega::Sega32xAnalyzer);
    ctx.register(retro_junk_sega::SaturnAnalyzer);
    ctx.register(retro_junk_sega::DreamcastAnalyzer);
    ctx.register(retro_junk_sega::GameGearAnalyzer);

    // Microsoft
    ctx.register(retro_junk_microsoft::XboxAnalyzer);
    ctx.register(retro_junk_microsoft::Xbox360Analyzer);

    ctx
}
