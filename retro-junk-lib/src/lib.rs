// Re-export everything from retro-junk-core for backwards compatibility.
// Note: AnalysisOptions is defined in retro-junk-core now, not in context.rs.
pub use retro_junk_core::*;

// Modules that still live in retro-junk-lib:
pub mod archive_assets;
pub mod archive_ops;
pub mod async_util;
pub mod catalog_match;
pub mod chd_convert;
pub mod context;
pub mod cue_fix;
pub mod disc_hash;
pub mod disc_set;
pub mod display;
pub mod fs_txn;
pub mod hasher;
pub mod naming;
pub mod organize;
pub mod playable_build;
pub mod redumper_cache;
pub mod rename;
pub mod rename_playable;
pub mod repair;
pub mod scanner;
pub mod settings;
pub mod util;

// Re-export context items at crate root for backwards compatibility.
pub use context::{AnalysisContext, Console, ConsoleFolder, FolderScanResult, RegisteredConsole};

/// Create an `AnalysisContext` with all built-in console analyzers registered.
///
/// Registers all 25 analyzers: NES, SNES, N64, `GameCube`, Wii, Wii U, GB, GBA,
/// DS, 3DS, PS1, PS2, PS3, PSP, Vita, SG-1000, Master System, Genesis, Sega CD,
/// 32X, Saturn, Dreamcast, Game Gear, Xbox, Xbox 360.
#[must_use]
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
    ctx.register(PceCdAnalyzer);

    ctx
}

/// Redump-backed PC Engine CD / TurboGrafx-CD analyzer.
#[derive(Debug, Default)]
pub struct PceCdAnalyzer;

impl RomAnalyzer for PceCdAnalyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other(
            "PC Engine CD identification uses complete Redump track hashes",
        ))
    }

    fn platform(&self) -> Platform {
        Platform::PceCd
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["bin", "cue", "iso", "chd"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false
    }

    fn dat_source(&self) -> DatSource {
        DatSource::Redump
    }

    fn redump_slug(&self) -> Option<&'static str> {
        Some("pce")
    }

    fn dat_download_ids(&self) -> &'static [&'static str] {
        &["pce"]
    }

    fn dat_names(&self) -> &'static [&'static str] {
        &["NEC - PC Engine CD & TurboGrafx CD"]
    }

    fn chd_extensions(&self) -> &'static [(&'static str, ChdExtensionRole)] {
        &[("cue", ChdExtensionRole::Source(ChdMedia::Cd))]
    }

    fn compute_container_hashes(
        &self,
        reader: &mut dyn ReadSeek,
        algorithms: HashAlgorithms,
        file_path: Option<&std::path::Path>,
        on_progress: HashProgressFn<'_>,
    ) -> Result<Option<FileHashes>, AnalysisError> {
        retro_junk_disc::hash::hash_disc_container(
            reader,
            algorithms,
            file_path,
            "PC Engine CD",
            on_progress,
        )
    }
}

#[cfg(test)]
#[path = "tests/analyzer_invariants.rs"]
mod analyzer_invariants;
