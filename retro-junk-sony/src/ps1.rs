//! PlayStation (PS1/PSX) disc image analyzer.
//!
//! Supports:
//! - ISO images (2048 bytes/sector)
//! - BIN images (raw 2352 bytes/sector)
//! - CUE sheets (parses track layout, optionally opens referenced BIN)
//! - CHD compressed images

use retro_junk_core::ReadSeek;

use retro_junk_core::{
    AnalysisError, AnalysisOptions, ChdExtensionRole, ChdMedia, FileHashes, HashAlgorithms,
    Platform, RomAnalyzer, RomIdentification,
};

use crate::sony_disc::{self, DiscFormat};

// Multi-disc PS1 games with shared serials (e.g., all FF7 USA discs share
// SCUS-94163) resolve via hash fallback after serial ambiguity. No fixup
// tables needed with full Redump DATs — the LibRetro-invented suffixed
// serials (SCUS-94163-0/1/2) don't exist in real Redump data.

/// Analyzer for PlayStation disc images.
#[derive(Debug, Default)]
pub struct Ps1Analyzer;

impl Ps1Analyzer {
    /// Analyze an ISO or raw BIN disc image.
    fn analyze_disc_image(
        &self,
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

        let mut id = RomIdentification::new().with_platform(Platform::Ps1);
        id.file_size = Some(file_size);
        id.extra.insert("format".into(), format.name().into());
        id.extra
            .insert("detected_extension".into(), format.extension().into());

        if !pvd.volume_identifier.is_empty() {
            id.internal_name = Some(pvd.volume_identifier.clone());
        }

        // Calculate expected size from PVD
        let sector_size = match format {
            DiscFormat::RawSector2352 => retro_junk_disc::RAW_SECTOR_SIZE,
            _ => retro_junk_disc::ISO_SECTOR_SIZE,
        };
        id.expected_size = Some(pvd.volume_space_size as u64 * sector_size);

        // Read SYSTEM.CNF for serial and region (fast: just 1-2 sector reads)
        if let Ok(content) = sony_disc::find_file_in_root(reader, format, &pvd, "SYSTEM.CNF") {
            let text = String::from_utf8_lossy(&content);
            if let Ok(ref cnf) = sony_disc::parse_system_cnf(&text) {
                // Reject PS2 discs (BOOT2) — let the PS2 analyzer handle them
                if cnf.boot_key == sony_disc::BootKey::Boot2 {
                    return Err(AnalysisError::invalid_format(
                        "PS2 disc (BOOT2 in SYSTEM.CNF) — not a PS1 disc",
                    ));
                }
                self.apply_system_cnf_parsed(cnf, &mut id);
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
        let file_size = retro_junk_core::util::file_size(reader)?;

        // Read the CUE text
        let mut cue_text = String::new();
        reader.read_to_string(&mut cue_text)?;

        let sheet = sony_disc::parse_cue(&cue_text)?;

        let mut id = RomIdentification::new().with_platform(Platform::Ps1);
        id.file_size = Some(file_size);
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
                // Try the referenced filename first; if it doesn't exist
                // (CDRWin DATAFILE with virtual name), find any existing BIN.
                let bin_path = parent.join(&first_data_file.filename);
                let resolved = if bin_path.exists() {
                    Some(bin_path)
                } else {
                    sheet
                        .files
                        .iter()
                        .map(|f| parent.join(&f.filename))
                        .find(|p| p.exists())
                };
                if let Some(bin_path) = resolved
                    && let Ok(mut bin_file) = std::fs::File::open(&bin_path)
                {
                    // Detect format and analyze the BIN
                    if let Ok(bin_format) = sony_disc::detect_disc_format(&mut bin_file) {
                        let bin_format = match bin_format {
                            DiscFormat::RawSector2352 => DiscFormat::RawSector2352,
                            _ => DiscFormat::Iso2048,
                        };
                        if let Ok(pvd) = sony_disc::read_pvd(&mut bin_file, bin_format)
                            && pvd.system_identifier.starts_with("PLAYSTATION")
                        {
                            if !pvd.volume_identifier.is_empty() {
                                id.internal_name = Some(pvd.volume_identifier.clone());
                            }
                            if let Ok(content) = sony_disc::find_file_in_root(
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
        let file_size = retro_junk_core::util::file_size(reader)?;

        let chd_info = sony_disc::read_chd_info(reader)?;

        let mut id = RomIdentification::new().with_platform(Platform::Ps1);
        id.file_size = Some(file_size);
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

        // Read SYSTEM.CNF from CHD (decompresses 1-2 hunks — fast enough)
        match sony_disc::read_system_cnf_from_chd(reader) {
            Ok(content) => {
                self.apply_system_cnf(&content, &mut id);
            }
            Err(_) => {
                // CHD might not be PS1, or SYSTEM.CNF not found
            }
        }

        Ok(id)
    }

    /// Parse raw SYSTEM.CNF bytes and apply serial/region to the identification.
    fn apply_system_cnf(&self, content: &[u8], id: &mut RomIdentification) {
        let text = String::from_utf8_lossy(content);
        if let Ok(ref cnf) = sony_disc::parse_system_cnf(&text) {
            self.apply_system_cnf_parsed(cnf, id);
        }
    }

    /// Apply parsed SYSTEM.CNF data to the identification.
    fn apply_system_cnf_parsed(&self, cnf: &sony_disc::SystemCnf, id: &mut RomIdentification) {
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
}

impl RomAnalyzer for Ps1Analyzer {
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
        Platform::Ps1
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["iso", "bin", "img", "cue", "chd"]
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

                // Differentiate PS1 from PS2 by checking SYSTEM.CNF boot key.
                if let Ok(content) =
                    sony_disc::find_file_in_root(reader, format, &pvd, "SYSTEM.CNF")
                {
                    let text = String::from_utf8_lossy(&content);
                    if let Ok(cnf) = sony_disc::parse_system_cnf(&text) {
                        return cnf.boot_key == sony_disc::BootKey::Boot;
                    }
                }

                // No SYSTEM.CNF or unparseable — accept as PS1 (best guess)
                true
            }
            // CUE and CHD: can't verify without reading disc data
            DiscFormat::Cue | DiscFormat::Chd => true,
        }
    }

    fn dat_source(&self) -> retro_junk_core::DatSource {
        retro_junk_core::DatSource::Redump
    }

    fn redump_slug(&self) -> Option<&'static str> {
        Some("psx")
    }

    fn chd_extensions(&self) -> &'static [(&'static str, ChdExtensionRole)] {
        // PS1 discs are CD-ROM; only cue/bin sets carry the full track layout
        // chdman needs. Loose bin/img/iso rips lack a table of contents.
        &[("cue", ChdExtensionRole::Source(ChdMedia::Cd))]
    }

    fn dat_download_ids(&self) -> &'static [&'static str] {
        &["psx"]
    }

    fn compute_container_hashes(
        &self,
        reader: &mut dyn ReadSeek,
        algorithms: HashAlgorithms,
        file_path: Option<&std::path::Path>,
        on_progress: retro_junk_core::HashProgressFn<'_>,
    ) -> Result<Option<FileHashes>, AnalysisError> {
        sony_disc::hash_disc_container(reader, algorithms, file_path, "PS1", on_progress)
    }

    fn dat_names(&self) -> &'static [&'static str] {
        &["Sony - PlayStation"]
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

#[cfg(test)]
#[path = "tests/ps1_tests.rs"]
mod tests;
