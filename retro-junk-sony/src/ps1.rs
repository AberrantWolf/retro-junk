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
    AnalysisError, AnalysisOptions, AnalysisProgress, Platform, RomAnalyzer, RomIdentification,
};

use crate::ps1_disc::{self, DiscFormat};

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
        if let Ok(content) = ps1_disc::find_file_in_root(
            reader,
            format,
            &pvd,
            "SYSTEM.CNF",
        ) {
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
            id.extra
                .insert("bin_file".into(), filenames[0].to_string());
        } else {
            id.extra
                .insert("bin_files".into(), filenames.join(", "));
        }

        // Open the first data track BIN and extract serial/volume ID
        // (fast: just a few sector reads from the referenced BIN file)
        if let Some(ref file_path) = options.file_path {
            if let Some(parent) = file_path.parent() {
                // Find the first file with a data track
                if let Some(first_data_file) = sheet.files.iter().find(|f| {
                    f.tracks.iter().any(|t| t.mode.to_uppercase().contains("MODE"))
                }) {
                    let bin_path = parent.join(&first_data_file.filename);
                    if bin_path.exists() {
                        if let Ok(mut bin_file) = std::fs::File::open(&bin_path) {
                            // Detect format and analyze the BIN
                            if let Ok(bin_format) = ps1_disc::detect_disc_format(&mut bin_file) {
                                let bin_format = match bin_format {
                                    DiscFormat::RawSector2352 => DiscFormat::RawSector2352,
                                    _ => DiscFormat::Iso2048,
                                };
                                if let Ok(pvd) = ps1_disc::read_pvd(&mut bin_file, bin_format) {
                                    if pvd.system_identifier.starts_with("PLAYSTATION") {
                                        if !pvd.volume_identifier.is_empty() {
                                            id.internal_name =
                                                Some(pvd.volume_identifier.clone());
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
            id.extra
                .insert("boot_path".into(), cnf.boot_path.clone());
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
        &["iso", "bin", "cue", "img", "chd"]
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

    fn dat_names(&self) -> &'static [&'static str] {
        &["Sony - PlayStation"]
    }

    fn extract_dat_game_code(&self, serial: &str) -> Option<String> {
        // Redump DATs use the full serial (e.g., "SLUS-01234")
        Some(serial.to_string())
    }
}

#[cfg(test)]
#[path = "tests/ps1_tests.rs"]
mod tests;
