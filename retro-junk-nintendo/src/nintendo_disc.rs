//! Nintendo disc header parsing utilities.
//!
//! Shared by GameCube and Wii analyzers. Both consoles use the same
//! disc header layout (0x0000-0x043F, "boot.bin") with identical field
//! positions and big-endian byte order.
//!
//! The key difference is the magic word used for identification:
//! - GameCube: 0xC2339F3D at offset 0x001C
//! - Wii: 0x5D1C9EA3 at offset 0x0018
//!
//! Sources:
//! - Yet Another GameCube Documentation (YAGCD): https://www.gc-forever.com/yagcd/chap13.html
//! - Wiibrew disc format: https://wiibrew.org/wiki/Wii_disc

use std::io::SeekFrom;
use std::path::Path;

use retro_junk_core::{AnalysisError, Platform, ReadSeek, RomIdentification};

use crate::constants::region_from_game_code;
use crate::licensee::maker_code_name;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// GameCube magic word at offset 0x001C (big-endian).
pub(crate) const GC_MAGIC: u32 = 0xC2339F3D;

/// Wii magic word at offset 0x0018 (big-endian).
pub(crate) const WII_MAGIC: u32 = 0x5D1C9EA3;

/// Size of the disc header ("boot.bin"): 0x440 bytes.
pub(crate) const HEADER_SIZE: usize = 0x440;

/// Minimum bytes needed to check both magic words (through offset 0x001F).
pub(crate) const MAGIC_CHECK_SIZE: usize = 0x20;

// ---------------------------------------------------------------------------
// Header struct
// ---------------------------------------------------------------------------

/// Parsed Nintendo disc header (0x0000-0x043F).
///
/// This structure is identical for GameCube and Wii discs.
pub(crate) struct NintendoDiscHeader {
    /// 4-byte game code: [console_id, game_code_hi, game_code_lo, country]
    pub game_code: [u8; 4],
    /// 2-byte maker/publisher code
    pub maker_code: [u8; 2],
    /// Disc number (0 = first disc)
    pub disc_id: u8,
    /// Software version/revision
    pub version: u8,
    /// Audio streaming flag
    pub audio_streaming: bool,
    /// Stream buffer size
    #[allow(dead_code)]
    pub stream_buffer_size: u8,
    /// Wii magic word at offset 0x0018
    pub wii_magic: u32,
    /// GameCube magic word at offset 0x001C
    pub gc_magic: u32,
    /// Game name (null-terminated string from offset 0x0020, up to 992 bytes)
    pub game_name: String,
    /// Main executable DOL offset
    pub dol_offset: u32,
    /// File System Table offset
    pub fst_offset: u32,
    /// File System Table size
    pub fst_size: u32,
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Parse the disc header from a reader positioned at the start of the disc image.
///
/// Reads 0x440 bytes and parses all fields. All multi-byte integers are big-endian.
pub(crate) fn parse_disc_header(
    reader: &mut dyn ReadSeek,
) -> Result<NintendoDiscHeader, AnalysisError> {
    reader.seek(SeekFrom::Start(0))?;

    let mut buf = [0u8; HEADER_SIZE];
    reader.read_exact(&mut buf).map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            AnalysisError::invalid_format("File too small for Nintendo disc header")
        } else {
            AnalysisError::from(e)
        }
    })?;

    let game_code: [u8; 4] = buf[0x0000..0x0004].try_into().unwrap();
    let maker_code: [u8; 2] = buf[0x0004..0x0006].try_into().unwrap();
    let disc_id = buf[0x0006];
    let version = buf[0x0007];
    let audio_streaming = buf[0x0008] != 0;
    let stream_buffer_size = buf[0x0009];

    let wii_magic = u32::from_be_bytes(buf[0x0018..0x001C].try_into().unwrap());
    let gc_magic = u32::from_be_bytes(buf[0x001C..0x0020].try_into().unwrap());

    // Game name: null-terminated string in 992 bytes at offset 0x0020
    let name_bytes = &buf[0x0020..0x0400];
    let name_end = name_bytes
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(name_bytes.len());
    let game_name = String::from_utf8_lossy(&name_bytes[..name_end])
        .trim()
        .to_string();

    let dol_offset = u32::from_be_bytes(buf[0x0420..0x0424].try_into().unwrap());
    let fst_offset = u32::from_be_bytes(buf[0x0424..0x0428].try_into().unwrap());
    let fst_size = u32::from_be_bytes(buf[0x0428..0x042C].try_into().unwrap());

    Ok(NintendoDiscHeader {
        game_code,
        maker_code,
        disc_id,
        version,
        audio_streaming,
        stream_buffer_size,
        wii_magic,
        gc_magic,
        game_name,
        dol_offset,
        fst_offset,
        fst_size,
    })
}

// ---------------------------------------------------------------------------
// Identification helpers
// ---------------------------------------------------------------------------

/// Returns true if the header indicates a GameCube disc.
pub(crate) fn is_gamecube(header: &NintendoDiscHeader) -> bool {
    header.gc_magic == GC_MAGIC && header.wii_magic != WII_MAGIC
}

/// Returns true if the header indicates a Wii disc.
pub(crate) fn is_wii(header: &NintendoDiscHeader) -> bool {
    header.wii_magic == WII_MAGIC
}

/// Returns the 4-byte game code as a string (e.g., "GALE").
pub(crate) fn game_code_str(header: &NintendoDiscHeader) -> String {
    String::from_utf8_lossy(&header.game_code).to_string()
}

/// Returns the 2-byte maker code as a string (e.g., "01").
pub(crate) fn maker_code_str(header: &NintendoDiscHeader) -> String {
    String::from_utf8_lossy(&header.maker_code).to_string()
}

/// Build a `RomIdentification` from a parsed disc header.
///
/// Populates serial number, internal name, region, version, maker code,
/// and platform-specific extras. Caller should add `file_size` and any
/// platform-specific fields (e.g., expected_size, DVD layer) afterward.
pub(crate) fn build_identification(
    header: &NintendoDiscHeader,
    platform: Platform,
) -> RomIdentification {
    let code = game_code_str(header);
    let maker = maker_code_str(header);

    let mut id = RomIdentification::new()
        .with_platform(platform)
        .with_serial(&code)
        .with_internal_name(&header.game_name);

    // Region from the country byte (4th char of game code)
    if let Some(region) = region_from_game_code(&code) {
        id.regions.push(region);
    }

    // Version
    if header.version > 0 {
        id.version = Some(format!("1.{:02}", header.version));
    }

    // Maker code
    id.maker_code = Some(maker.clone());

    // Extras
    id.extra.insert("game_code".into(), code);
    id.extra.insert("maker_code".into(), maker.clone());
    if let Some(name) = maker_code_name(&maker) {
        id.extra.insert("maker_name".into(), name.into());
    }
    id.extra
        .insert("disc_id".into(), header.disc_id.to_string());
    id.extra
        .insert("disc_version".into(), header.version.to_string());
    id.extra
        .insert("dol_offset".into(), format!("0x{:08X}", header.dol_offset));
    id.extra
        .insert("fst_offset".into(), format!("0x{:08X}", header.fst_offset));
    id.extra
        .insert("fst_size".into(), format!("0x{:08X}", header.fst_size));

    if header.audio_streaming {
        id.extra.insert("audio_streaming".into(), "true".into());
    }

    id
}

/// Read the first 0x20 bytes and check magic words without full header parsing.
///
/// Returns `(gc_magic_matches, wii_magic_matches)`. Seeks back to start.
pub(crate) fn check_magic(reader: &mut dyn ReadSeek) -> Result<(bool, bool), AnalysisError> {
    reader.seek(SeekFrom::Start(0))?;
    let mut buf = [0u8; MAGIC_CHECK_SIZE];
    if reader.read(&mut buf)? < MAGIC_CHECK_SIZE {
        reader.seek(SeekFrom::Start(0))?;
        return Ok((false, false));
    }
    reader.seek(SeekFrom::Start(0))?;

    let wii_magic = u32::from_be_bytes(buf[0x0018..0x001C].try_into().unwrap());
    let gc_magic = u32::from_be_bytes(buf[0x001C..0x0020].try_into().unwrap());

    Ok((
        gc_magic == GC_MAGIC && wii_magic != WII_MAGIC,
        wii_magic == WII_MAGIC,
    ))
}

// ---------------------------------------------------------------------------
// Compressed disc format support (RVZ, WIA, WBFS, CISO, GCZ)
// ---------------------------------------------------------------------------

/// Returns `true` if the reader begins with a compressed Nintendo disc container.
///
/// Detects RVZ, WIA, WBFS, CISO, and GCZ by magic bytes using `nod::Disc::detect()`.
/// Returns `false` for raw ISO/GCM or unrecognized formats. Always seeks back to start.
pub(crate) fn is_compressed_disc(reader: &mut dyn ReadSeek) -> bool {
    reader.seek(SeekFrom::Start(0)).ok();
    let result = nod::Disc::detect(reader);
    reader.seek(SeekFrom::Start(0)).ok();

    matches!(
        result,
        Ok(Some(
            nod::Format::Rvz
                | nod::Format::Wia
                | nod::Format::Wbfs
                | nod::Format::Ciso
                | nod::Format::Gcz
        ))
    )
}

/// Returns a display name for a `nod::Format` variant.
fn nod_format_name(format: nod::Format) -> &'static str {
    match format {
        nod::Format::Iso => "ISO",
        nod::Format::Rvz => "RVZ",
        nod::Format::Wia => "WIA",
        nod::Format::Wbfs => "WBFS",
        nod::Format::Ciso => "CISO",
        nod::Format::Gcz => "GCZ",
        _ => "Compressed",
    }
}

/// Open a compressed disc image via `nod` and parse the Nintendo disc header.
///
/// Returns the parsed header, format name string, and uncompressed disc size.
/// The file path is required because `nod::Disc::new()` opens the file directly.
pub(crate) fn open_compressed_disc(
    path: &Path,
) -> Result<(NintendoDiscHeader, &'static str, u64), AnalysisError> {
    let mut disc = nod::Disc::new(path).map_err(|e| {
        AnalysisError::invalid_format(&format!("Failed to open compressed disc: {e}"))
    })?;

    let format_name = nod_format_name(disc.meta().format);
    let disc_size = disc.disc_size();

    // nod::Disc implements Read + Seek, so parse_disc_header works directly
    let header = parse_disc_header(&mut disc)?;

    Ok((header, format_name, disc_size))
}
