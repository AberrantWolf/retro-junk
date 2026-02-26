use std::io::{BufRead, Read};

use quick_xml::events::Event;
use quick_xml::reader::Reader;

use crate::error::DatError;

/// A parsed NoIntro DAT file (supports both Logiqx XML and ClrMamePro formats).
#[derive(Debug, Clone)]
pub struct DatFile {
    pub name: String,
    pub description: String,
    pub version: String,
    pub games: Vec<DatGame>,
}

/// A single game entry from a DAT file.
#[derive(Debug, Clone)]
pub struct DatGame {
    pub name: String,
    /// Region string (e.g., "USA", "Japan"), if present (LibRetro enhanced DATs).
    pub region: Option<String>,
    pub roms: Vec<DatRom>,
}

/// A single ROM entry within a game.
#[derive(Debug, Clone)]
pub struct DatRom {
    pub name: String,
    pub size: u64,
    /// CRC32 checksum (lowercase hex)
    pub crc: String,
    /// SHA1 checksum (lowercase hex), if present
    pub sha1: Option<String>,
    /// MD5 checksum (lowercase hex), if present
    pub md5: Option<String>,
    /// Serial number, if present
    pub serial: Option<String>,
}

/// Parse a DAT file, auto-detecting format (XML or ClrMamePro).
pub fn parse_dat<R: BufRead>(mut reader: R) -> Result<DatFile, DatError> {
    // Peek at the first non-whitespace content to detect format
    let mut first_bytes = Vec::new();
    let mut buf = [0u8; 1];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            return Err(DatError::invalid_dat("Empty DAT file"));
        }
        first_bytes.push(buf[0]);
        if !buf[0].is_ascii_whitespace() {
            break;
        }
    }

    // Build a chained reader with the peeked bytes + remaining data
    let chain = std::io::Cursor::new(first_bytes).chain(reader);
    let buffered = std::io::BufReader::new(chain);

    if buf[0] == b'<' {
        parse_xml(buffered)
    } else {
        parse_clrmamepro(buffered)
    }
}

/// Parse a DAT file from a file path.
pub fn parse_dat_file(path: &std::path::Path) -> Result<DatFile, DatError> {
    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    parse_dat(reader)
}

// ---------------------------------------------------------------------------
// Logiqx XML parser
// ---------------------------------------------------------------------------

fn parse_xml<R: BufRead>(reader: R) -> Result<DatFile, DatError> {
    let mut xml = Reader::from_reader(reader);
    xml.config_mut().trim_text(true);

    let mut buf = Vec::new();
    let mut dat = DatFile {
        name: String::new(),
        description: String::new(),
        version: String::new(),
        games: Vec::new(),
    };

    let mut in_header = false;
    let mut current_tag = String::new();
    let mut current_game: Option<DatGame> = None;
    let mut game_serial: Option<String> = None;

    loop {
        match xml.read_event_into(&mut buf)? {
            Event::Start(ref e) => {
                let tag_name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match tag_name.as_str() {
                    "header" => in_header = true,
                    "game" => {
                        let mut name = String::new();
                        for attr in e.attributes() {
                            let attr = attr?;
                            if attr.key.as_ref() == b"name" {
                                name = String::from_utf8_lossy(&attr.value).to_string();
                            }
                        }
                        current_game = Some(DatGame {
                            name,
                            region: None,
                            roms: Vec::new(),
                        });
                        game_serial = None;
                    }
                    _ => current_tag = tag_name,
                }
            }
            Event::Empty(ref e) => {
                let tag_name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if tag_name == "rom"
                    && let Some(ref mut game) = current_game
                {
                    let rom = parse_xml_rom_attributes(e)?;
                    game.roms.push(rom);
                }
            }
            Event::Text(ref e) => {
                let text = e.unescape()?.to_string();
                if in_header {
                    match current_tag.as_str() {
                        "name" => dat.name = text,
                        "description" => dat.description = text,
                        "version" => dat.version = text,
                        _ => {}
                    }
                } else if current_game.is_some() && current_tag.as_str() == "serial" {
                    game_serial = Some(text)
                }
            }
            Event::End(ref e) => {
                let tag_name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match tag_name.as_str() {
                    "header" => in_header = false,
                    "game" => {
                        if let Some(mut game) = current_game.take() {
                            // Propagate game-level serial to ROMs that lack one
                            if let Some(ref serial) = game_serial {
                                for rom in &mut game.roms {
                                    if rom.serial.is_none() {
                                        rom.serial = Some(serial.clone());
                                    }
                                }
                            }
                            game_serial = None;
                            dat.games.push(game);
                        }
                    }
                    _ => current_tag.clear(),
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    if dat.name.is_empty() && dat.games.is_empty() {
        return Err(DatError::invalid_dat(
            "No header or games found in XML DAT file",
        ));
    }

    Ok(dat)
}

fn parse_xml_rom_attributes(e: &quick_xml::events::BytesStart<'_>) -> Result<DatRom, DatError> {
    let mut rom = DatRom {
        name: String::new(),
        size: 0,
        crc: String::new(),
        sha1: None,
        md5: None,
        serial: None,
    };

    for attr in e.attributes() {
        let attr = attr?;
        let value = String::from_utf8_lossy(&attr.value).to_string();
        match attr.key.as_ref() {
            b"name" => rom.name = value,
            b"size" => {
                rom.size = value
                    .parse()
                    .map_err(|_| DatError::invalid_dat(format!("Invalid ROM size: {value}")))?;
            }
            b"crc" => rom.crc = value.to_lowercase(),
            b"sha1" => rom.sha1 = Some(value.to_lowercase()),
            b"md5" => rom.md5 = Some(value.to_lowercase()),
            b"serial" => rom.serial = Some(value),
            _ => {}
        }
    }

    Ok(rom)
}

// ---------------------------------------------------------------------------
// ClrMamePro DAT parser
// ---------------------------------------------------------------------------

/// Parse a ClrMamePro format DAT file.
///
/// Format:
/// ```text
/// clrmamepro (
///     name "System Name"
///     version 20240101-000000
/// )
///
/// game (
///     name "Game Name (Region)"
///     rom ( name "Game Name (Region).ext" size 12345 crc AABBCCDD sha1 ... )
/// )
/// ```
fn parse_clrmamepro<R: BufRead>(reader: R) -> Result<DatFile, DatError> {
    let mut dat = DatFile {
        name: String::new(),
        description: String::new(),
        version: String::new(),
        games: Vec::new(),
    };

    let mut in_block: Option<String> = None; // "clrmamepro" or "game"
    let mut current_game: Option<DatGame> = None;
    let mut game_serial: Option<String> = None;

    for line_result in reader.lines() {
        let line = line_result?;
        let trimmed = line.trim();

        if trimmed.is_empty() {
            continue;
        }

        // Detect block start: "blocktype ("
        if in_block.is_none() {
            if let Some(block_type) = detect_block_start(trimmed) {
                if block_type.as_str() == "game" {
                    current_game = Some(DatGame {
                        name: String::new(),
                        region: None,
                        roms: Vec::new(),
                    });
                }
                in_block = Some(block_type);
                continue;
            }
            continue;
        }

        // Detect block end: ")"
        if trimmed == ")" {
            let block_type = in_block.take().unwrap();
            if block_type.as_str() == "game"
                && let Some(mut game) = current_game.take()
            {
                // Propagate game-level serial to ROMs that lack one
                if let Some(ref serial) = game_serial {
                    for rom in &mut game.roms {
                        if rom.serial.is_none() {
                            rom.serial = Some(serial.clone());
                        }
                    }
                }
                game_serial = None;
                dat.games.push(game);
            }
            continue;
        }

        // Parse key-value pairs inside a block
        let block_type = in_block.as_ref().unwrap();
        if let Some((key, value)) = parse_kv(trimmed) {
            match block_type.as_str() {
                "clrmamepro" => match key.as_str() {
                    "name" => dat.name = value,
                    "description" => dat.description = value,
                    "version" => dat.version = value,
                    _ => {}
                },
                "game" => {
                    if let Some(ref mut game) = current_game {
                        match key.as_str() {
                            "name" => game.name = value,
                            "region" => game.region = Some(value),
                            "serial" => {
                                // Store game-level serial to propagate to ROMs later
                                game_serial = Some(value);
                            }
                            "rom" => {
                                if let Some(rom) = parse_clr_rom_inline(&value) {
                                    game.roms.push(rom);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    if dat.name.is_empty() && dat.games.is_empty() {
        return Err(DatError::invalid_dat(
            "No header or games found in ClrMamePro DAT file",
        ));
    }

    Ok(dat)
}

/// Detect a block start like `clrmamepro (` or `game (`.
fn detect_block_start(line: &str) -> Option<String> {
    let stripped = line.trim_end();
    if let Some(without_paren) = stripped.strip_suffix('(') {
        let block_type = without_paren.trim();
        if !block_type.is_empty() && block_type.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Some(block_type.to_lowercase());
        }
    }
    None
}

/// Parse a key-value line like `name "Some Value"` or `version 20240101`.
/// For `rom ( ... )` lines, the value is the content inside outer parens.
fn parse_kv(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();

    // Handle "rom ( ... )" specially — the key is "rom" and value is the inner content
    if let Some(after_rom) = trimmed.strip_prefix("rom") {
        let rest = after_rom.trim();
        if rest.starts_with('(') && rest.ends_with(')') {
            let inner = rest[1..rest.len() - 1].trim();
            return Some(("rom".to_string(), inner.to_string()));
        }
    }

    // Split on first whitespace
    let mut parts = trimmed.splitn(2, |c: char| c.is_ascii_whitespace());
    let key = parts.next()?.trim().to_string();
    let raw_value = parts.next()?.trim();

    // Strip surrounding quotes if present
    let value = if raw_value.starts_with('"') && raw_value.ends_with('"') && raw_value.len() >= 2 {
        raw_value[1..raw_value.len() - 1].to_string()
    } else {
        raw_value.to_string()
    };

    Some((key, value))
}

/// Parse an inline ROM entry like:
/// `name "Game (Region).ext" size 12345 crc AABBCCDD md5 ... sha1 ...`
fn parse_clr_rom_inline(inner: &str) -> Option<DatRom> {
    let tokens = tokenize_rom_line(inner);
    let mut rom = DatRom {
        name: String::new(),
        size: 0,
        crc: String::new(),
        sha1: None,
        md5: None,
        serial: None,
    };

    let mut i = 0;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "name" => {
                i += 1;
                if i < tokens.len() {
                    rom.name = tokens[i].clone();
                }
            }
            "size" => {
                i += 1;
                if i < tokens.len() {
                    rom.size = tokens[i].parse().unwrap_or(0);
                }
            }
            "crc" => {
                i += 1;
                if i < tokens.len() {
                    rom.crc = tokens[i].to_lowercase();
                }
            }
            "sha1" => {
                i += 1;
                if i < tokens.len() {
                    rom.sha1 = Some(tokens[i].to_lowercase());
                }
            }
            "md5" => {
                i += 1;
                if i < tokens.len() {
                    rom.md5 = Some(tokens[i].to_lowercase());
                }
            }
            "serial" => {
                i += 1;
                if i < tokens.len() {
                    rom.serial = Some(tokens[i].clone());
                }
            }
            _ => {}
        }
        i += 1;
    }

    if rom.name.is_empty() {
        return None;
    }
    Some(rom)
}

/// Tokenize a ROM line, respecting quoted strings.
/// `name "Game (Region).ext" size 12345 crc AB` → ["name", "Game (Region).ext", "size", "12345", "crc", "AB"]
fn tokenize_rom_line(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();

    loop {
        // Skip whitespace
        while chars.peek().is_some_and(|c| c.is_ascii_whitespace()) {
            chars.next();
        }

        if chars.peek().is_none() {
            break;
        }

        if chars.peek() == Some(&'"') {
            // Quoted string
            chars.next(); // consume opening quote
            let mut token = String::new();
            while let Some(&c) = chars.peek() {
                if c == '"' {
                    chars.next(); // consume closing quote
                    break;
                }
                token.push(c);
                chars.next();
            }
            tokens.push(token);
        } else {
            // Unquoted token
            let mut token = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_ascii_whitespace() {
                    break;
                }
                token.push(c);
                chars.next();
            }
            tokens.push(token);
        }
    }

    tokens
}

#[cfg(test)]
#[path = "tests/dat_tests.rs"]
mod tests;
