//! Shared test helpers for constructing synthetic Sony disc images.
//!
//! Used by PS1, PS2, and other Sony disc analyzer tests.
//! Generic disc helpers (`make_iso`, `make_raw_bin`, etc.) are in retro-junk-disc.

pub use retro_junk_disc::test_helpers::*;

/// Build a full ISO with a root directory containing SYSTEM.CNF.
///
/// `boot_key` controls whether SYSTEM.CNF uses `BOOT` (PS1) or `BOOT2` (PS2).
/// `serial` is the boot executable filename (e.g., "`SLUS_012.34`").
pub fn make_iso_with_system_cnf(serial: &str, boot_key: &str) -> Vec<u8> {
    let cdrom_prefix = if boot_key == "BOOT2" {
        "cdrom0:"
    } else {
        "cdrom:"
    };
    let system_cnf_content = format!("{boot_key} = {cdrom_prefix}\\{serial};1\r\nVMODE = NTSC\r\n");

    make_iso_with_file("PLAYSTATION", "SYSTEM.CNF", system_cnf_content.as_bytes())
}
