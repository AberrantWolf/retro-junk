//! Shared test helpers for constructing synthetic disc images.
//!
//! Used by disc, Sony, Sega, and other disc analyzer tests.

use crate::sector::CD_SYNC_PATTERN;

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
pub fn make_raw_sector_mode2(user_data: &[u8; 2048]) -> [u8; 2352] {
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

/// Wrap 2048 bytes of user data into a raw 2352-byte Mode 1 sector.
pub fn make_raw_sector_mode1(user_data: &[u8; 2048]) -> [u8; 2352] {
    let mut sector = [0u8; 2352];
    // 12 bytes sync
    sector[0..12].copy_from_slice(&CD_SYNC_PATTERN);
    // 4 bytes header (MSF + mode) — just set mode to 1
    sector[15] = 0x01;
    // 2048 bytes user data at offset 16
    sector[16..16 + 2048].copy_from_slice(user_data);
    // Remaining bytes (EDC/ECC) left as zero
    sector
}

/// Build a raw BIN with Mode 2 Form 1 sectors: 16 raw empty sectors + raw PVD sector.
pub fn make_raw_bin(system_id: &str) -> Vec<u8> {
    let empty_user = [0u8; 2048];
    let mut data = Vec::new();
    for _ in 0..16 {
        data.extend_from_slice(&make_raw_sector_mode2(&empty_user));
    }
    let pvd = make_pvd_sector(system_id);
    data.extend_from_slice(&make_raw_sector_mode2(&pvd));
    data
}

/// Build a raw BIN with Mode 1 sectors: 16 raw empty sectors + raw PVD sector.
pub fn make_raw_bin_mode1(system_id: &str) -> Vec<u8> {
    let empty_user = [0u8; 2048];
    let mut data = Vec::new();
    for _ in 0..16 {
        data.extend_from_slice(&make_raw_sector_mode1(&empty_user));
    }
    let pvd = make_pvd_sector(system_id);
    data.extend_from_slice(&make_raw_sector_mode1(&pvd));
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

/// Build a full ISO with a root directory containing a named file.
///
/// The file contents are placed at sector 19, and the root directory
/// at sector 18 contains the dot entries plus a record for the file.
pub fn make_iso_with_file(system_id: &str, filename: &str, content: &[u8]) -> Vec<u8> {
    // Layout:
    // Sectors 0-15: empty padding
    // Sector 16: PVD (root dir at sector 18, 1 sector)
    // Sector 17: empty (VD terminator)
    // Sector 18: root directory
    // Sector 19: file content

    let mut data = vec![0u8; 16 * 2048]; // sectors 0-15

    // Sector 16: PVD
    let mut pvd = make_pvd_sector(system_id);
    pvd[158..162].copy_from_slice(&18u32.to_le_bytes());
    pvd[166..170].copy_from_slice(&2048u32.to_le_bytes());
    data.extend_from_slice(&pvd);

    // Sector 17: empty
    data.extend_from_slice(&[0u8; 2048]);

    // Sector 18: root directory
    let mut dir_sector = [0u8; 2048];
    let mut pos = 0;

    let dot_record = make_dir_record("\0", 18, 2048);
    dir_sector[pos..pos + dot_record.len()].copy_from_slice(&dot_record);
    pos += dot_record.len();

    let dotdot_record = make_dir_record("\x01", 18, 2048);
    dir_sector[pos..pos + dotdot_record.len()].copy_from_slice(&dotdot_record);
    pos += dotdot_record.len();

    let file_record = make_dir_record(&format!("{};1", filename), 19, content.len() as u32);
    dir_sector[pos..pos + file_record.len()].copy_from_slice(&file_record);

    data.extend_from_slice(&dir_sector);

    // Sector 19: file content
    let mut content_sector = [0u8; 2048];
    let copy_len = content.len().min(2048);
    content_sector[..copy_len].copy_from_slice(&content[..copy_len]);
    data.extend_from_slice(&content_sector);

    data
}
