//! Sega Saturn disc image analyzer.
//!
//! Supports:
//! - ISO images (2048 bytes/sector)
//! - BIN images (raw 2352 bytes/sector, Mode 1)
//! - CUE sheets (parses track layout, optionally opens referenced BIN)
//! - CHD compressed images
//!
//! Saturn discs use CD-ROM Mode 1 sectors. The boot header (IP.BIN) is at
//! sector 0 and contains the magic string "SEGA SEGASATURN " at offset 0.

use retro_junk_core::ReadSeek;

use retro_junk_core::{
    AnalysisError, AnalysisOptions, FileHashes, HashAlgorithms, Platform, Region, RomAnalyzer,
    RomIdentification,
};

use retro_junk_disc::chd;
use retro_junk_disc::cue;
use retro_junk_disc::format::{DiscFormat, detect_disc_format};
use retro_junk_disc::hash::hash_disc_container;
use retro_junk_disc::iso9660;
use retro_junk_disc::sector;

/// Saturn IP.BIN header magic (16 bytes, space-padded).
const SATURN_MAGIC: &[u8; 16] = b"SEGA SEGASATURN ";

/// Parsed Saturn IP.BIN header fields.
struct SaturnHeader {
    maker_id: String,
    serial: String,
    version: String,
    release_date: String,
    region_codes: String,
    game_name: String,
    device_info: String,
}

/// Parse the Saturn IP.BIN header from 2048 bytes of sector 0 user data.
fn parse_ip_bin(data: &[u8]) -> Option<SaturnHeader> {
    if data.len() < 0x0D0 {
        return None;
    }
    if &data[0..16] != SATURN_MAGIC {
        return None;
    }

    Some(SaturnHeader {
        maker_id: read_trimmed(&data[0x010..0x020]),
        serial: read_trimmed(&data[0x020..0x02A]),
        version: read_trimmed(&data[0x02A..0x030]),
        release_date: read_trimmed(&data[0x030..0x038]),
        device_info: read_trimmed(&data[0x038..0x040]),
        region_codes: read_trimmed(&data[0x040..0x04A]),
        game_name: read_trimmed(&data[0x060..0x0D0]),
    })
}

/// Read a fixed-size ASCII field and trim trailing spaces/nulls.
fn read_trimmed(bytes: &[u8]) -> String {
    let s = std::str::from_utf8(bytes).unwrap_or("");
    s.trim_end_matches(|c: char| c == ' ' || c == '\0')
        .to_string()
}

/// Map Saturn region code characters to Region enum values.
///
/// From the IP.BIN Compatible Area field:
/// J = Japan, T = Asia, U = North America, B = Brazil,
/// K = Korea, A = Asia (PAL), E = Europe, L = Latin America
fn saturn_region_codes(codes: &str) -> Vec<Region> {
    let mut regions = Vec::new();
    for ch in codes.chars() {
        match ch {
            'J' => regions.push(Region::Japan),
            'T' => regions.push(Region::Asia), // Asia (NTSC)
            'U' => regions.push(Region::Usa),
            'E' => regions.push(Region::Europe),
            'A' => regions.push(Region::Asia), // Asia (PAL)
            'B' => regions.push(Region::Brazil),
            'L' => regions.push(Region::LatinAmerica),
            'K' => regions.push(Region::Korea),
            _ => {}
        }
    }
    regions
}

/// Analyzer for Sega Saturn disc images.
#[derive(Debug, Default)]
pub struct SaturnAnalyzer;

impl SaturnAnalyzer {
    /// Read sector 0 from an ISO or raw BIN to get IP.BIN data.
    ///
    /// Saturn uses Mode 1 sectors. For raw BIN, user data starts at offset 16
    /// (12 sync + 4 header), not 24 (Mode 2 Form 1).
    fn read_ip_bin_sector(
        &self,
        reader: &mut dyn ReadSeek,
        format: DiscFormat,
    ) -> Result<[u8; 2048], AnalysisError> {
        match format {
            DiscFormat::Iso2048 => sector::read_sector_data(reader, 0, format),
            DiscFormat::RawSector2352 => sector::read_sector_data_mode1(reader, 0, format),
            _ => Err(AnalysisError::unsupported(
                "Cannot read IP.BIN sector from CUE/CHD directly",
            )),
        }
    }

    /// Analyze an ISO or raw BIN disc image.
    fn analyze_disc_image(
        &self,
        reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
        format: DiscFormat,
    ) -> Result<RomIdentification, AnalysisError> {
        let file_size = retro_junk_core::util::file_size(reader)?;

        let sector0 = self.read_ip_bin_sector(reader, format)?;
        let header = parse_ip_bin(&sector0).ok_or_else(|| {
            AnalysisError::invalid_format("Not a Saturn disc (IP.BIN magic mismatch)")
        })?;

        let mut id = self.build_identification(&header);
        id.file_size = Some(file_size);
        id.extra.insert("format".into(), format.name().into());
        id.extra
            .insert("detected_extension".into(), format.extension().into());

        // Optionally read PVD for volume identifier
        if let Ok(pvd) = iso9660::read_pvd(reader, format) {
            if !pvd.volume_identifier.is_empty() && id.internal_name.is_none() {
                id.internal_name = Some(pvd.volume_identifier);
            }
            id.expected_size =
                Some(pvd.volume_space_size as u64 * retro_junk_disc::ISO_SECTOR_SIZE);
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

        let mut cue_text = String::new();
        reader.read_to_string(&mut cue_text)?;
        let sheet = cue::parse_cue(&cue_text)?;

        let mut id = RomIdentification::new().with_platform(Platform::Saturn);
        id.file_size = Some(file_size);
        id.extra.insert("format".into(), "CUE Sheet".into());
        id.extra.insert("detected_extension".into(), "cue".into());

        // Count tracks
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

        // Open the first data track BIN to read IP.BIN
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
                && let Ok(bin_format) = detect_disc_format(&mut bin_file)
            {
                let bin_format = match bin_format {
                    DiscFormat::RawSector2352 => DiscFormat::RawSector2352,
                    _ => DiscFormat::Iso2048,
                };
                if let Ok(sector0) = self.read_ip_bin_sector(&mut bin_file, bin_format) {
                    if let Some(header) = parse_ip_bin(&sector0) {
                        let header_id = self.build_identification(&header);
                        id.serial_number = header_id.serial_number;
                        id.regions = header_id.regions;
                        id.internal_name = header_id.internal_name;
                        // Copy Saturn-specific extras
                        for (k, v) in &header_id.extra {
                            id.extra.insert(k.clone(), v.clone());
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

        let chd_info = chd::read_chd_info(reader)?;

        // Read sector 0 from CHD for IP.BIN.
        // Saturn uses Mode 1, so user data is at offset 16 within the raw sector.
        let sector0 = chd::read_chd_sector_mode1(reader, 0)?;

        let header = parse_ip_bin(&sector0).ok_or_else(|| {
            AnalysisError::invalid_format("Not a Saturn disc (IP.BIN magic mismatch in CHD)")
        })?;

        let mut id = self.build_identification(&header);
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

        // Try to read PVD for volume identifier
        if let Ok(pvd) = chd::read_pvd_from_chd(reader) {
            if !pvd.volume_identifier.is_empty() && id.internal_name.is_none() {
                id.internal_name = Some(pvd.volume_identifier);
            }
        }

        Ok(id)
    }

    /// Build a RomIdentification from a parsed Saturn header.
    fn build_identification(&self, header: &SaturnHeader) -> RomIdentification {
        let mut id = RomIdentification::new().with_platform(Platform::Saturn);

        if !header.game_name.is_empty() {
            id.internal_name = Some(header.game_name.clone());
        }

        if !header.serial.is_empty() {
            id.serial_number = Some(header.serial.clone());
        }

        id.regions = saturn_region_codes(&header.region_codes);

        if !header.maker_id.is_empty() {
            id.extra.insert("maker_id".into(), header.maker_id.clone());
        }
        if !header.version.is_empty() {
            id.extra.insert("version".into(), header.version.clone());
        }
        if !header.release_date.is_empty() {
            id.extra
                .insert("release_date".into(), header.release_date.clone());
        }
        if !header.device_info.is_empty() {
            id.extra
                .insert("device_info".into(), header.device_info.clone());
        }
        if !header.region_codes.is_empty() {
            id.extra
                .insert("region_codes".into(), header.region_codes.clone());
        }

        id
    }
}

impl RomAnalyzer for SaturnAnalyzer {
    fn analyze(
        &self,
        reader: &mut dyn ReadSeek,
        options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        let format = detect_disc_format(reader)?;

        match format {
            DiscFormat::Iso2048 | DiscFormat::RawSector2352 => {
                self.analyze_disc_image(reader, options, format)
            }
            DiscFormat::Cue => self.analyze_cue(reader, options),
            DiscFormat::Chd => self.analyze_chd(reader, options),
        }
    }

    fn platform(&self) -> Platform {
        Platform::Saturn
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["bin", "cue", "iso", "chd", "mdf", "mds"]
    }

    fn can_handle(&self, reader: &mut dyn ReadSeek) -> bool {
        let format = match detect_disc_format(reader) {
            Ok(f) => f,
            Err(_) => return false,
        };

        match format {
            DiscFormat::Iso2048 | DiscFormat::RawSector2352 => self
                .read_ip_bin_sector(reader, format)
                .map(|s| s[0..16] == *SATURN_MAGIC)
                .unwrap_or(false),
            DiscFormat::Chd => chd::read_chd_sector_mode1(reader, 0)
                .map(|s| s[0..16] == *SATURN_MAGIC)
                .unwrap_or(false),
            // CUE: can't cheaply verify without opening the referenced BIN
            DiscFormat::Cue => true,
        }
    }

    fn dat_source(&self) -> retro_junk_core::DatSource {
        retro_junk_core::DatSource::Redump
    }

    fn redump_slug(&self) -> Option<&'static str> {
        Some("ss")
    }

    fn dat_download_ids(&self) -> &'static [&'static str] {
        &["ss"]
    }

    fn compute_container_hashes(
        &self,
        reader: &mut dyn ReadSeek,
        algorithms: HashAlgorithms,
        file_path: Option<&std::path::Path>,
        on_progress: retro_junk_core::HashProgressFn<'_>,
    ) -> Result<Option<FileHashes>, AnalysisError> {
        hash_disc_container(reader, algorithms, file_path, "Saturn", on_progress)
    }

    fn dat_names(&self) -> &'static [&'static str] {
        &["Sega - Saturn"]
    }

    fn gdb_csv_names(&self) -> &'static [&'static str] {
        &["console_sega_saturn"]
    }

    fn expects_serial(&self) -> bool {
        true
    }

    fn extract_dat_game_code(&self, serial: &str) -> Option<String> {
        // Redump DATs use the full serial (e.g., "MK-81009")
        Some(serial.to_string())
    }
}

#[cfg(test)]
#[path = "tests/saturn_tests.rs"]
mod tests;
