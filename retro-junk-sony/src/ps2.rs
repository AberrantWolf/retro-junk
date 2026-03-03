//! PlayStation 2 disc image analyzer.
//!
//! Supports:
//! - ISO images (2048 bytes/sector)
//! - BIN images (raw 2352 bytes/sector)
//! - CUE sheets (parses track layout, optionally opens referenced BIN)
//! - CHD compressed images
//!
//! PS2 discs are nearly identical to PS1 from a filesystem perspective (ISO 9660
//! with a SYSTEM.CNF boot descriptor). The key differentiator is `BOOT2` in
//! SYSTEM.CNF (vs PS1's `BOOT`). All disc parsing is shared via `sony_disc`.

use retro_junk_core::ReadSeek;
use std::io::{Seek, SeekFrom};

use retro_junk_core::{
    AnalysisError, AnalysisOptions, FileHashes, HashAlgorithms, Platform, RomAnalyzer,
    RomIdentification,
};

use crate::sony_disc::{self, BootKey, DiscFormat};

/// DVD-5 capacity threshold (4.7 GB = 4_700_000_000 bytes).
/// Files larger than this are likely DVD-9 (dual layer).
const DVD5_SIZE_THRESHOLD: u64 = 4_700_000_000;

/// Multi-disc PS2 games where the per-disc boot serial (from SYSTEM.CNF)
/// differs from the catalog serial used in the DAT.
///
/// Populated as specific multi-disc PS2 games are encountered.
const MULTI_DISC_SERIAL_FIXUPS: &[(&str, &str)] = &[];

/// Analyzer for PlayStation 2 disc images.
#[derive(Debug, Default)]
pub struct Ps2Analyzer;

impl Ps2Analyzer {
    pub fn new() -> Self {
        Self
    }
}

impl Ps2Analyzer {
    /// Analyze an ISO or raw BIN disc image.
    fn analyze_disc_image(
        &self,
        reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
        format: DiscFormat,
    ) -> Result<RomIdentification, AnalysisError> {
        let file_size = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;

        let pvd = sony_disc::read_pvd(reader, format)?;

        // Verify this is a PlayStation disc
        if !pvd.system_identifier.starts_with("PLAYSTATION") {
            return Err(AnalysisError::invalid_format(format!(
                "Not a PlayStation disc (system ID: '{}')",
                pvd.system_identifier
            )));
        }

        let mut id = RomIdentification::new().with_platform(Platform::Ps2);
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

        // Detect DVD layer type from file size
        detect_dvd_layer(file_size, &mut id);

        // Read SYSTEM.CNF for serial and region
        if let Ok(content) = sony_disc::find_file_in_root(reader, format, &pvd, "SYSTEM.CNF") {
            let text = String::from_utf8_lossy(&content);
            if let Ok(ref cnf) = sony_disc::parse_system_cnf(&text) {
                // Reject PS1 discs (BOOT) — let the PS1 analyzer handle them
                if cnf.boot_key == BootKey::Boot {
                    return Err(AnalysisError::invalid_format(
                        "PS1 disc (BOOT in SYSTEM.CNF) — not a PS2 disc",
                    ));
                }
                apply_system_cnf(cnf, &mut id);
            }
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

        let sheet = sony_disc::parse_cue(&cue_text)?;

        let mut id = RomIdentification::new().with_platform(Platform::Ps2);
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
        if let Some(ref file_path) = options.file_path
            && let Some(parent) = file_path.parent()
            && let Some(first_data_file) = sheet.files.iter().find(|f| {
                f.tracks
                    .iter()
                    .any(|t| t.mode.to_uppercase().contains("MODE"))
            })
        {
            let bin_path = parent.join(&first_data_file.filename);
            if bin_path.exists()
                && let Ok(mut bin_file) = std::fs::File::open(&bin_path)
                && let Ok(bin_format) = sony_disc::detect_disc_format(&mut bin_file)
            {
                let bin_format = match bin_format {
                    DiscFormat::RawSector2352 => DiscFormat::RawSector2352,
                    _ => DiscFormat::Iso2048,
                };

                // Detect DVD layer from the BIN file size
                if let Ok(bin_size) = bin_file.seek(SeekFrom::End(0)) {
                    detect_dvd_layer(bin_size, &mut id);
                    bin_file.seek(SeekFrom::Start(0)).ok();
                }

                if let Ok(pvd) = sony_disc::read_pvd(&mut bin_file, bin_format)
                    && pvd.system_identifier.starts_with("PLAYSTATION")
                {
                    if !pvd.volume_identifier.is_empty() {
                        id.internal_name = Some(pvd.volume_identifier.clone());
                    }
                    if let Ok(content) =
                        sony_disc::find_file_in_root(&mut bin_file, bin_format, &pvd, "SYSTEM.CNF")
                    {
                        let text = String::from_utf8_lossy(&content);
                        if let Ok(ref cnf) = sony_disc::parse_system_cnf(&text) {
                            apply_system_cnf(cnf, &mut id);
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

        let chd_info = sony_disc::read_chd_info(reader)?;

        let mut id = RomIdentification::new().with_platform(Platform::Ps2);
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

        // Detect DVD layer from CHD logical size
        detect_dvd_layer(chd_info.logical_size, &mut id);

        // Read SYSTEM.CNF from CHD
        match sony_disc::read_system_cnf_from_chd(reader) {
            Ok(content) => {
                let text = String::from_utf8_lossy(&content);
                if let Ok(ref cnf) = sony_disc::parse_system_cnf(&text) {
                    apply_system_cnf(cnf, &mut id);
                }
            }
            Err(_) => {
                // CHD might not be PS2, or SYSTEM.CNF not found
            }
        }

        Ok(id)
    }
}

impl RomAnalyzer for Ps2Analyzer {
    fn analyze(
        &self,
        reader: &mut dyn ReadSeek,
        options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        let format = sony_disc::detect_disc_format(reader)?;

        match format {
            DiscFormat::Iso2048 | DiscFormat::RawSector2352 => {
                self.analyze_disc_image(reader, options, format)
            }
            DiscFormat::Cue => self.analyze_cue(reader, options),
            DiscFormat::Chd => self.analyze_chd(reader, options),
        }
    }

    fn platform(&self) -> Platform {
        Platform::Ps2
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["iso", "bin", "chd"]
    }

    fn can_handle(&self, reader: &mut dyn ReadSeek) -> bool {
        let format = match sony_disc::detect_disc_format(reader) {
            Ok(f) => f,
            Err(_) => return false,
        };

        match format {
            DiscFormat::Iso2048 | DiscFormat::RawSector2352 => {
                // Verify PLAYSTATION system identifier in PVD
                let pvd = match sony_disc::read_pvd(reader, format) {
                    Ok(pvd) if pvd.system_identifier.starts_with("PLAYSTATION") => pvd,
                    _ => return false,
                };

                // PS2 discs use BOOT2 in SYSTEM.CNF
                if let Ok(content) =
                    sony_disc::find_file_in_root(reader, format, &pvd, "SYSTEM.CNF")
                {
                    let text = String::from_utf8_lossy(&content);
                    if let Ok(cnf) = sony_disc::parse_system_cnf(&text) {
                        return cnf.boot_key == BootKey::Boot2;
                    }
                }

                // No SYSTEM.CNF — not identifiable as PS2
                false
            }
            // CUE and CHD: can't cheaply verify without reading disc data
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
        _file_path: Option<&std::path::Path>,
    ) -> Result<Option<FileHashes>, AnalysisError> {
        let format = sony_disc::detect_disc_format(reader)?;

        match format {
            DiscFormat::Chd => {
                log::info!("PS2 compute_container_hashes: CHD detected");
                let hashes = sony_disc::hash_chd_raw_sectors(reader, algorithms)?;
                log::info!(
                    "PS2 compute_container_hashes: done, crc32={}, data_size={}",
                    hashes.crc32,
                    hashes.data_size
                );
                Ok(Some(hashes))
            }
            DiscFormat::RawSector2352 => {
                // Multi-track BIN files contain data + audio tracks concatenated.
                // Redump DATs hash only Track 1 (data), so detect the boundary.
                if let Some(data_size) = sony_disc::find_raw_bin_data_track_size(reader)? {
                    log::info!(
                        "PS2 compute_container_hashes: raw BIN, hashing Track 1 ({} bytes)",
                        data_size
                    );
                    let hashes = crate::ps1::hash_raw_bin_track1(reader, algorithms, data_size)?;
                    Ok(Some(hashes))
                } else {
                    Ok(None)
                }
            }
            _ => Ok(None),
        }
    }

    fn dat_names(&self) -> &'static [&'static str] {
        &["Sony - PlayStation 2"]
    }

    fn expects_serial(&self) -> bool {
        true
    }

    fn extract_dat_game_code(&self, serial: &str) -> Option<String> {
        // Check fixup table for multi-disc games
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

/// Apply parsed SYSTEM.CNF data to the identification.
fn apply_system_cnf(cnf: &sony_disc::SystemCnf, id: &mut RomIdentification) {
    id.extra.insert("boot_path".into(), cnf.boot_path.clone());
    if let Some(ref vmode) = cnf.vmode {
        id.extra.insert("vmode".into(), vmode.clone());
    }
    if let Some(serial) = sony_disc::extract_serial(&cnf.boot_path) {
        if let Some(region) = sony_disc::serial_to_region(&serial) {
            id.regions.push(region);
        }
        id.serial_number = Some(serial);
    }
}

/// Detect DVD layer type from file/image size and record it in extras.
fn detect_dvd_layer(size: u64, id: &mut RomIdentification) {
    let layer = if size > DVD5_SIZE_THRESHOLD {
        "DVD-9"
    } else {
        "DVD-5"
    };
    id.extra.insert("dvd_layer".into(), layer.into());
}

#[cfg(test)]
#[path = "tests/ps2_tests.rs"]
mod tests;
