//! retro-junk CLI
//!
//! Command-line interface for analyzing retro game ROMs and disc images.

use retro_junk_lib::RomAnalyzer;

fn main() {
    println!("retro-junk CLI");
    println!("==============");
    println!();
    println!("Supported platforms:");
    println!();

    // Nintendo
    println!("Nintendo:");
    print_analyzer(&retro_junk_nintendo::NesAnalyzer::new());
    print_analyzer(&retro_junk_nintendo::SnesAnalyzer::new());
    print_analyzer(&retro_junk_nintendo::N64Analyzer::new());
    print_analyzer(&retro_junk_nintendo::GameCubeAnalyzer::new());
    print_analyzer(&retro_junk_nintendo::WiiAnalyzer::new());
    print_analyzer(&retro_junk_nintendo::WiiUAnalyzer::new());
    print_analyzer(&retro_junk_nintendo::GameBoyAnalyzer::new());
    print_analyzer(&retro_junk_nintendo::GbaAnalyzer::new());
    print_analyzer(&retro_junk_nintendo::DsAnalyzer::new());
    print_analyzer(&retro_junk_nintendo::N3dsAnalyzer::new());
    println!();

    // Sony
    println!("Sony:");
    print_analyzer(&retro_junk_sony::Ps1Analyzer::new());
    print_analyzer(&retro_junk_sony::Ps2Analyzer::new());
    print_analyzer(&retro_junk_sony::Ps3Analyzer::new());
    print_analyzer(&retro_junk_sony::PspAnalyzer::new());
    print_analyzer(&retro_junk_sony::VitaAnalyzer::new());
    println!();

    // Sega
    println!("Sega:");
    print_analyzer(&retro_junk_sega::Sg1000Analyzer::new());
    print_analyzer(&retro_junk_sega::MasterSystemAnalyzer::new());
    print_analyzer(&retro_junk_sega::GenesisAnalyzer::new());
    print_analyzer(&retro_junk_sega::SegaCdAnalyzer::new());
    print_analyzer(&retro_junk_sega::Sega32xAnalyzer::new());
    print_analyzer(&retro_junk_sega::SaturnAnalyzer::new());
    print_analyzer(&retro_junk_sega::DreamcastAnalyzer::new());
    print_analyzer(&retro_junk_sega::GameGearAnalyzer::new());
    println!();

    // Microsoft
    println!("Microsoft:");
    print_analyzer(&retro_junk_microsoft::XboxAnalyzer::new());
    print_analyzer(&retro_junk_microsoft::Xbox360Analyzer::new());
}

fn print_analyzer<A: RomAnalyzer>(analyzer: &A) {
    let extensions = analyzer.file_extensions().join(", ");
    println!("  - {} ({})", analyzer.platform_name(), extensions);
}
