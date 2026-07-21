//! `PlayStation` 2 disc image analyzer.
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
    AnalysisError, AnalysisOptions, ChdExtensionRole, ChdMedia, FileHashes, HashAlgorithms,
    Platform, RomAnalyzer, RomIdentification,
};

use crate::sony_disc::{self, BootKey, DiscFormat};

/// DVD-5 capacity threshold (4.7 GB = `4_700_000_000` bytes).
/// Files larger than this are likely DVD-9 (dual layer).
const DVD5_SIZE_THRESHOLD: u64 = 4_700_000_000;

// Multi-disc PS2 games with shared serials resolve via hash fallback
// after serial ambiguity. No fixup tables needed with full Redump DATs.

/// Analyzer for `PlayStation` 2 disc images.
#[derive(Debug, Default)]
pub struct Ps2Analyzer;

impl Ps2Analyzer {
    /// Analyze an ISO or raw BIN disc image.
    fn analyze_disc_image(
        reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
        format: DiscFormat,
    ) -> Result<RomIdentification, AnalysisError> {
        let file_size = retro_junk_core::util::file_size(reader)?;

        let pvd = sony_disc::read_pvd(reader, format)?;

        // Verify this is a PlayStation disc
        if !pvd.system_identifier.starts_with("PLAYSTATION") {
            return Err(AnalysisError::invalid_format(format!(
                "Not a PlayStation disc (system ID: '{}')",
                pvd.system_identifier
            )));
        }

        let mut id = RomIdentification::new();
        id.file_size = file_size;
        id.extra.insert("format".into(), format.name().into());
        id.extra
            .insert("detected_extension".into(), format.extension().into());

        if !pvd.volume_identifier.is_empty() {
            id.internal_name.clone_from(&pvd.volume_identifier);
        }

        // Calculate expected size from PVD
        let sector_size = match format {
            DiscFormat::RawSector2352 => retro_junk_disc::RAW_SECTOR_SIZE,
            _ => retro_junk_disc::ISO_SECTOR_SIZE,
        };
        id.expected_size = u64::from(pvd.volume_space_size) * sector_size;

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
        reader: &mut dyn ReadSeek,
        options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        let file_size = retro_junk_core::util::file_size(reader)?;

        // Read the CUE text
        let mut cue_text = String::new();
        reader.read_to_string(&mut cue_text)?;

        let sheet = sony_disc::parse_cue(&cue_text)?;

        let mut id = RomIdentification::new();
        id.file_size = file_size;
        id.extra.insert("format".into(), "CUE Sheet".into());
        id.extra.insert("detected_extension".into(), "cue".into());

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
                        id.internal_name.clone_from(&pvd.volume_identifier);
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
        reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        let file_size = retro_junk_core::util::file_size(reader)?;

        let chd_info = sony_disc::read_chd_info(reader)?;

        let mut id = RomIdentification::new();
        id.file_size = file_size;
        id.extra.insert("format".into(), "CHD".into());
        id.extra.insert("detected_extension".into(), "chd".into());
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
        if let Ok(content) = sony_disc::read_system_cnf_from_chd(reader) {
            let text = String::from_utf8_lossy(&content);
            if let Ok(ref cnf) = sony_disc::parse_system_cnf(&text) {
                apply_system_cnf(cnf, &mut id);
            }
        } else {
            // CHD might not be PS2, or SYSTEM.CNF not found
        }

        Ok(id)
    }
}

impl RomAnalyzer for Ps2Analyzer {
    fn identification_capabilities(&self) -> retro_junk_core::IdentificationCapabilities {
        retro_junk_core::IdentificationCapabilities::ALL
    }

    fn analyze(
        &self,
        reader: &mut dyn ReadSeek,
        options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        let format = sony_disc::detect_disc_format(reader)?;

        match format {
            DiscFormat::Iso2048 | DiscFormat::RawSector2352 => {
                Self::analyze_disc_image(reader, options, format)
            }
            DiscFormat::Cue => Self::analyze_cue(reader, options),
            DiscFormat::Chd => Self::analyze_chd(reader, options),
        }
    }

    fn platform(&self) -> Platform {
        Platform::Ps2
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["iso", "bin", "img", "cue", "chd"]
    }

    fn can_handle(&self, reader: &mut dyn ReadSeek) -> bool {
        let Ok(format) = sony_disc::detect_disc_format(reader) else {
            return false;
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

    fn redump_slug(&self) -> Option<&'static str> {
        Some("ps2")
    }

    fn chd_extensions(&self) -> &'static [(&'static str, ChdExtensionRole)] {
        // PS2 CD games (blue discs) are dumped as cue/bin; DVD games as iso.
        &[
            ("cue", ChdExtensionRole::Source(ChdMedia::Cd)),
            ("iso", ChdExtensionRole::Source(ChdMedia::Dvd)),
        ]
    }

    fn dat_download_ids(&self) -> &'static [&'static str] {
        &["ps2"]
    }

    fn compute_container_hashes(
        &self,
        reader: &mut dyn ReadSeek,
        algorithms: HashAlgorithms,
        file_path: Option<&std::path::Path>,
        on_progress: retro_junk_core::HashProgressFn<'_>,
    ) -> Result<Option<FileHashes>, AnalysisError> {
        sony_disc::hash_disc_container(reader, algorithms, file_path, "PS2", on_progress)
    }

    fn dat_names(&self) -> &'static [&'static str] {
        &["Sony - PlayStation 2"]
    }

    fn expects_serial(&self) -> bool {
        true
    }

    fn extract_dat_game_code(&self, serial: &str) -> Option<String> {
        // Full Redump DATs use real serials. Multi-disc games with shared
        // serials resolve via hash fallback after serial ambiguity.
        Some(serial.to_string())
    }
}

/// Apply parsed SYSTEM.CNF data to the identification.
fn apply_system_cnf(cnf: &sony_disc::SystemCnf, id: &mut RomIdentification) {
    id.extra.insert("boot_path".into(), cnf.boot_path.clone());
    if !cnf.vmode.is_empty() {
        id.extra.insert("vmode".into(), cnf.vmode.clone());
    }
    if let Some(serial) = sony_disc::extract_serial(&cnf.boot_path) {
        if let Some(region) = sony_disc::serial_to_region(&serial) {
            id.regions.push(region);
        }
        id.serial_number = serial;
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
