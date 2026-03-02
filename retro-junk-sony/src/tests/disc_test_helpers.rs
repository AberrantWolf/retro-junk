//! Shared test helpers for constructing synthetic Sony disc images.
//!
//! Used by PS1, PS2, and other Sony disc analyzer tests.

use crate::sony_disc::CD_SYNC_PATTERN;

/// Build a minimal 2048-byte PVD sector with a given system identifier.
pub fn make_pvd_sector(system_id: &str) -> [u8; 2048] {
    let mut sector = [0u8; 2048];
    sector[0] = 0x01; // PVD type
    sector[1..6].copy_from_slice(b"CD001"); // standard identifier
    sector[6] = 0x01; // version

    // System identifier at offset 8, 32 bytes padded with spaces
    let id_bytes = system_id.as_bytes();
    let len = id_bytes.len().min(32);
    sector[8..8 + len].copy_from_slice(&id_bytes[..len]);
    for i in len..32 {
        sector[8 + i] = b' ';
    }

    // Volume identifier at offset 40, 32 bytes
    let vol = b"TEST_VOLUME";
    sector[40..40 + vol.len()].copy_from_slice(vol);
    for i in vol.len()..32 {
        sector[40 + i] = b' ';
    }

    // Volume space size at offset 80 (LE) — say 200 sectors
    sector[80..84].copy_from_slice(&200u32.to_le_bytes());
    sector[84..88].copy_from_slice(&200u32.to_be_bytes());

    // Root directory record at offset 156 (34 bytes)
    sector[156] = 34; // record length
    // extent LBA at record+2 (LE) — sector 18
    sector[158..162].copy_from_slice(&18u32.to_le_bytes());
    // data length at record+10 (LE) — 2048 bytes (1 sector)
    sector[166..170].copy_from_slice(&2048u32.to_le_bytes());

    sector
}

/// Build a minimal ISO: 16 sectors of padding + PVD at sector 16.
pub fn make_iso(system_id: &str) -> Vec<u8> {
    let mut data = vec![0u8; 16 * 2048]; // 16 empty sectors
    let pvd = make_pvd_sector(system_id);
    data.extend_from_slice(&pvd);
    data
}

/// Wrap 2048 bytes of user data into a raw 2352-byte Mode 2 Form 1 sector.
pub fn make_raw_sector(user_data: &[u8; 2048]) -> [u8; 2352] {
    let mut sector = [0u8; 2352];
    // 12 bytes sync
    sector[0..12].copy_from_slice(&CD_SYNC_PATTERN);
    // 4 bytes header (MSF + mode) — just set mode to 2
    sector[15] = 0x02;
    // 8 bytes subheader — zeros are fine
    // 2048 bytes user data at offset 24
    sector[24..24 + 2048].copy_from_slice(user_data);
    // Remaining bytes (EDC/ECC) left as zero
    sector
}

/// Build a raw BIN: 16 raw empty sectors + raw PVD sector.
pub fn make_raw_bin(system_id: &str) -> Vec<u8> {
    let empty_user = [0u8; 2048];
    let mut data = Vec::new();
    for _ in 0..16 {
        data.extend_from_slice(&make_raw_sector(&empty_user));
    }
    let pvd = make_pvd_sector(system_id);
    data.extend_from_slice(&make_raw_sector(&pvd));
    data
}

/// Build a directory record for a file.
pub fn make_dir_record(filename: &str, extent_lba: u32, data_length: u32) -> Vec<u8> {
    let id_bytes = filename.as_bytes();
    let id_len = id_bytes.len();
    let record_len = 33 + id_len + (id_len % 2); // pad to even
    let mut record = vec![0u8; record_len];
    record[0] = record_len as u8;
    record[2..6].copy_from_slice(&extent_lba.to_le_bytes());
    record[10..14].copy_from_slice(&data_length.to_le_bytes());
    record[25] = 0; // file flags (regular file)
    record[32] = id_len as u8;
    record[33..33 + id_len].copy_from_slice(id_bytes);
    record
}

/// Build a full ISO with a root directory containing SYSTEM.CNF.
///
/// `boot_key` controls whether SYSTEM.CNF uses `BOOT` (PS1) or `BOOT2` (PS2).
/// `serial` is the boot executable filename (e.g., "SLUS_012.34").
pub fn make_iso_with_system_cnf(serial: &str, boot_key: &str) -> Vec<u8> {
    let cdrom_prefix = if boot_key == "BOOT2" {
        "cdrom0:"
    } else {
        "cdrom:"
    };
    let system_cnf_content = format!(
        "{} = {}\\{};1\r\nVMODE = NTSC\r\n",
        boot_key, cdrom_prefix, serial
    );
    let cnf_bytes = system_cnf_content.as_bytes();

    // Layout:
    // Sectors 0-15: empty padding
    // Sector 16: PVD (root dir at sector 18, 1 sector)
    // Sector 17: empty (VD terminator)
    // Sector 18: root directory (with "." ".." and "SYSTEM.CNF;1" entries)
    // Sector 19: SYSTEM.CNF content

    let mut data = vec![0u8; 16 * 2048]; // sectors 0-15

    // Sector 16: PVD
    let mut pvd = make_pvd_sector("PLAYSTATION");
    // Point root dir to sector 18, size 2048
    pvd[158..162].copy_from_slice(&18u32.to_le_bytes());
    pvd[166..170].copy_from_slice(&2048u32.to_le_bytes());
    data.extend_from_slice(&pvd);

    // Sector 17: empty (VD set terminator would go here)
    data.extend_from_slice(&[0u8; 2048]);

    // Sector 18: root directory
    let mut dir_sector = [0u8; 2048];
    let mut pos = 0;

    // "." entry (current directory)
    let dot_record = make_dir_record("\0", 18, 2048);
    dir_sector[pos..pos + dot_record.len()].copy_from_slice(&dot_record);
    pos += dot_record.len();

    // ".." entry (parent directory)
    let dotdot_record = make_dir_record("\x01", 18, 2048);
    dir_sector[pos..pos + dotdot_record.len()].copy_from_slice(&dotdot_record);
    pos += dotdot_record.len();

    // SYSTEM.CNF entry pointing to sector 19
    let cnf_record = make_dir_record("SYSTEM.CNF;1", 19, cnf_bytes.len() as u32);
    dir_sector[pos..pos + cnf_record.len()].copy_from_slice(&cnf_record);

    data.extend_from_slice(&dir_sector);

    // Sector 19: SYSTEM.CNF content
    let mut cnf_sector = [0u8; 2048];
    cnf_sector[..cnf_bytes.len()].copy_from_slice(cnf_bytes);
    data.extend_from_slice(&cnf_sector);

    data
}
