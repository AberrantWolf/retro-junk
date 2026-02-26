//! PlayStation (PS1/PSX) disc image analyzer.
//!
//! Supports:
//! - ISO images (2048 bytes/sector)
//! - BIN images (raw 2352 bytes/sector)
//! - CUE sheets (parses track layout, optionally opens referenced BIN)
//! - CHD compressed images

use retro_junk_core::ReadSeek;
use std::io::SeekFrom;
use std::sync::mpsc::Sender;

use retro_junk_core::{
    AnalysisError, AnalysisOptions, AnalysisProgress, FileHashes, HashAlgorithms, Platform,
    RomAnalyzer, RomIdentification,
};

use crate::ps1_disc::{self, DiscFormat};

/// Multi-disc PS1 games where the per-disc boot serial (from SYSTEM.CNF)
/// differs from the catalog serial used in the DAT. Maps boot serial to the
/// suffixed catalog serial in the LibRetro Redump DAT.
///
/// Only needed for discs whose boot serial is NOT the catalog serial (i.e.,
/// discs 2+ of sequential-serial games). Disc 1 is handled by the matcher's
/// "-0" suffix preference when the boot serial matches the catalog serial.
const MULTI_DISC_SERIAL_FIXUPS: &[(&str, &str)] = &[
    // Final Fantasy VII (USA) - Disc 2, Disc 3
    ("SCUS-94164", "SCUS-94163-1"),
    ("SCUS-94165", "SCUS-94163-2"),
    // Star Ocean - The Second Story (USA) - Disc 2
    ("SCUS-94422", "SCUS-94421-1"),
];

/// Analyzer for PlayStation disc images.
#[derive(Debug, Default)]
pub struct Ps1Analyzer;

impl Ps1Analyzer {
    pub fn new() -> Self {
        Self
    }
}

impl Ps1Analyzer {
    /// Analyze an ISO or raw BIN disc image.
    fn analyze_disc_image(
        &self,
        reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
        format: DiscFormat,
    ) -> Result<RomIdentification, AnalysisError> {
        let file_size = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;

        let pvd = ps1_disc::read_pvd(reader, format)?;

        // Verify this is a PlayStation disc
        if !pvd.system_identifier.starts_with("PLAYSTATION") {
            return Err(AnalysisError::invalid_format(format!(
                "Not a PlayStation disc (system ID: '{}')",
                pvd.system_identifier
            )));
        }

        let mut id = RomIdentification::new().with_platform("PlayStation");
        id.file_size = Some(file_size);
        id.extra.insert("format".into(), format.name().into());

        if !pvd.volume_identifier.is_empty() {
            id.internal_name = Some(pvd.volume_identifier.clone());
        }

        // Calculate expected size from PVD
        let sector_size = match format {
            DiscFormat::RawSector2352 => 2352u64,
            _ => 2048u64,
        };
        id.expected_size = Some(pvd.volume_space_size as u64 * sector_size);

        // Read SYSTEM.CNF for serial and region (fast: just 1-2 sector reads)
        if let Ok(content) = ps1_disc::find_file_in_root(reader, format, &pvd, "SYSTEM.CNF") {
            self.apply_system_cnf(&content, &mut id);
        }

        Ok(id)
    }

    /// Analyze a CUE sheet.
    fn analyze_cue(
        &self,
        reader: &mut dyn ReadSeek,
        options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        let file_size = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;

        // Read the CUE text
        let mut cue_text = String::new();
        reader.read_to_string(&mut cue_text)?;

        let sheet = ps1_disc::parse_cue(&cue_text)?;

        let mut id = RomIdentification::new().with_platform("PlayStation");
        id.file_size = Some(file_size);
        id.extra.insert("format".into(), "CUE Sheet".into());

        // Count data and audio tracks
        let total_tracks: usize = sheet.files.iter().map(|f| f.tracks.len()).sum();
        let data_tracks: usize = sheet
            .files
            .iter()
            .flat_map(|f| &f.tracks)
            .filter(|t| t.mode.to_uppercase().contains("MODE"))
            .count();
        let audio_tracks = total_tracks - data_tracks;

        id.extra
            .insert("total_tracks".into(), total_tracks.to_string());
        id.extra
            .insert("data_tracks".into(), data_tracks.to_string());
        id.extra
            .insert("audio_tracks".into(), audio_tracks.to_string());

        // Store referenced filenames
        let filenames: Vec<&str> = sheet.files.iter().map(|f| f.filename.as_str()).collect();
        if filenames.len() == 1 {
            id.extra.insert("bin_file".into(), filenames[0].to_string());
        } else {
            id.extra.insert("bin_files".into(), filenames.join(", "));
        }

        // Open the first data track BIN and extract serial/volume ID
        // (fast: just a few sector reads from the referenced BIN file)
        if let Some(ref file_path) = options.file_path
            && let Some(parent) = file_path.parent()
        {
            // Find the first file with a data track
            if let Some(first_data_file) = sheet.files.iter().find(|f| {
                f.tracks
                    .iter()
                    .any(|t| t.mode.to_uppercase().contains("MODE"))
            }) {
                let bin_path = parent.join(&first_data_file.filename);
                if bin_path.exists()
                    && let Ok(mut bin_file) = std::fs::File::open(&bin_path)
                {
                    // Detect format and analyze the BIN
                    if let Ok(bin_format) = ps1_disc::detect_disc_format(&mut bin_file) {
                        let bin_format = match bin_format {
                            DiscFormat::RawSector2352 => DiscFormat::RawSector2352,
                            _ => DiscFormat::Iso2048,
                        };
                        if let Ok(pvd) = ps1_disc::read_pvd(&mut bin_file, bin_format)
                            && pvd.system_identifier.starts_with("PLAYSTATION")
                        {
                            if !pvd.volume_identifier.is_empty() {
                                id.internal_name = Some(pvd.volume_identifier.clone());
                            }
                            if let Ok(content) = ps1_disc::find_file_in_root(
                                &mut bin_file,
                                bin_format,
                                &pvd,
                                "SYSTEM.CNF",
                            ) {
                                self.apply_system_cnf(&content, &mut id);
                            }
                        }
                    }
                }
            }
        }

        Ok(id)
    }

    /// Analyze a CHD compressed disc image.
    fn analyze_chd(
        &self,
        reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        let file_size = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;

        let chd_info = ps1_disc::read_chd_info(reader)?;

        let mut id = RomIdentification::new().with_platform("PlayStation");
        id.file_size = Some(file_size);
        id.extra.insert("format".into(), "CHD".into());
        id.extra
            .insert("chd_version".into(), format!("v{}", chd_info.version));
        id.extra
            .insert("chd_hunk_size".into(), format!("{}", chd_info.hunk_size));
        id.extra.insert(
            "chd_logical_size".into(),
            format!("{}", chd_info.logical_size),
        );

        // Read SYSTEM.CNF from CHD (decompresses 1-2 hunks — fast enough)
        match ps1_disc::read_system_cnf_from_chd(reader) {
            Ok(content) => {
                self.apply_system_cnf(&content, &mut id);
            }
            Err(_) => {
                // CHD might not be PS1, or SYSTEM.CNF not found
            }
        }

        Ok(id)
    }

    /// Parse SYSTEM.CNF content and apply serial/region to the identification.
    fn apply_system_cnf(&self, content: &[u8], id: &mut RomIdentification) {
        let text = String::from_utf8_lossy(content);
        if let Ok(cnf) = ps1_disc::parse_system_cnf(&text) {
            id.extra.insert("boot_path".into(), cnf.boot_path.clone());
            if let Some(ref vmode) = cnf.vmode {
                id.extra.insert("vmode".into(), vmode.clone());
            }
            if let Some(serial) = ps1_disc::extract_serial(&cnf.boot_path) {
                if let Some(region) = ps1_disc::serial_to_region(&serial) {
                    id.regions.push(region);
                }
                id.serial_number = Some(serial);
            }
        }
    }
}

impl RomAnalyzer for Ps1Analyzer {
    fn analyze(
        &self,
        reader: &mut dyn ReadSeek,
        options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        let format = ps1_disc::detect_disc_format(reader)?;

        match format {
            DiscFormat::Iso2048 | DiscFormat::RawSector2352 => {
                self.analyze_disc_image(reader, options, format)
            }
            DiscFormat::Cue => self.analyze_cue(reader, options),
            DiscFormat::Chd => self.analyze_chd(reader, options),
        }
    }

    fn analyze_with_progress(
        &self,
        reader: &mut dyn ReadSeek,
        options: &AnalysisOptions,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        self.analyze(reader, options)
    }

    fn platform(&self) -> Platform {
        Platform::Ps1
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["iso", "bin", "chd"]
    }

    fn can_handle(&self, reader: &mut dyn ReadSeek) -> bool {
        let format = match ps1_disc::detect_disc_format(reader) {
            Ok(f) => f,
            Err(_) => return false,
        };

        match format {
            DiscFormat::Iso2048 | DiscFormat::RawSector2352 => {
                // Verify PLAYSTATION system identifier in PVD
                if let Ok(pvd) = ps1_disc::read_pvd(reader, format) {
                    pvd.system_identifier.starts_with("PLAYSTATION")
                } else {
                    false
                }
            }
            // CUE and CHD: can't verify without reading disc data
            DiscFormat::Cue | DiscFormat::Chd => true,
        }
    }

    fn dat_source(&self) -> retro_junk_core::DatSource {
        retro_junk_core::DatSource::Redump
    }

    fn compute_container_hashes(
        &self,
        reader: &mut dyn ReadSeek,
        algorithms: HashAlgorithms,
    ) -> Result<Option<FileHashes>, AnalysisError> {
        let format = ps1_disc::detect_disc_format(reader)?;

        match format {
            ps1_disc::DiscFormat::Chd => {
                log::info!("PS1 compute_container_hashes: CHD detected");
                let hashes = ps1_disc::hash_chd_raw_sectors(reader, algorithms)?;
                log::info!(
                    "PS1 compute_container_hashes: done, crc32={}, data_size={}",
                    hashes.crc32,
                    hashes.data_size
                );
                Ok(Some(hashes))
            }
            ps1_disc::DiscFormat::RawSector2352 => {
                // Multi-track BIN files contain data + audio tracks concatenated.
                // Redump DATs hash only Track 1 (data), so detect the boundary.
                if let Some(data_size) = ps1_disc::find_raw_bin_data_track_size(reader)? {
                    log::info!(
                        "PS1 compute_container_hashes: raw BIN, hashing Track 1 ({} bytes)",
                        data_size
                    );
                    let hashes = hash_raw_bin_track1(reader, algorithms, data_size)?;
                    Ok(Some(hashes))
                } else {
                    // Single-track BIN — let the standard hasher handle it
                    Ok(None)
                }
            }
            // CUE sheets and ISOs: let the standard hasher handle them
            _ => Ok(None),
        }
    }

    fn dat_names(&self) -> &'static [&'static str] {
        &["Sony - PlayStation"]
    }

    fn expects_serial(&self) -> bool {
        true
    }

    fn extract_dat_game_code(&self, serial: &str) -> Option<String> {
        // Check fixup table for multi-disc games where the per-disc boot
        // serial differs from the catalog serial in the DAT
        if let Some((_, suffixed)) = MULTI_DISC_SERIAL_FIXUPS
            .iter()
            .find(|(boot, _)| *boot == serial)
        {
            return Some(suffixed.to_string());
        }
        // Redump DATs use the full serial (e.g., "SLUS-01234")
        Some(serial.to_string())
    }
}

/// Hash the first `data_size` bytes of a raw 2352-byte sector BIN file.
fn hash_raw_bin_track1(
    reader: &mut dyn ReadSeek,
    algorithms: HashAlgorithms,
    data_size: u64,
) -> Result<FileHashes, AnalysisError> {
    use sha1::Digest;

    reader.seek(SeekFrom::Start(0))?;

    let mut crc = if algorithms.crc32 {
        Some(crc32fast::Hasher::new())
    } else {
        None
    };
    let mut sha = if algorithms.sha1 {
        Some(sha1::Sha1::new())
    } else {
        None
    };
    let mut md5_ctx = if algorithms.md5 {
        Some(md5::Context::new())
    } else {
        None
    };

    let mut buf = [0u8; 64 * 1024];
    let mut remaining = data_size;

    while remaining > 0 {
        let to_read = remaining.min(buf.len() as u64) as usize;
        let n = reader.read(&mut buf[..to_read])?;
        if n == 0 {
            break;
        }
        if let Some(ref mut h) = crc {
            h.update(&buf[..n]);
        }
        if let Some(ref mut h) = sha {
            h.update(&buf[..n]);
        }
        if let Some(ref mut h) = md5_ctx {
            h.consume(&buf[..n]);
        }
        remaining -= n as u64;
    }

    Ok(FileHashes {
        crc32: crc
            .map(|h| format!("{:08x}", h.finalize()))
            .unwrap_or_default(),
        sha1: sha.map(|h| format!("{:x}", h.finalize())),
        md5: md5_ctx.map(|h| format!("{:x}", h.compute())),
        data_size,
    })
}

#[cfg(test)]
#[path = "tests/ps1_tests.rs"]
mod tests;
