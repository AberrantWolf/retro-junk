//! Unified Nintendo licensee / maker code lookup tables.
//!
//! Nintendo used two generations of licensee codes across their platforms:
//!
//! - **Old (byte) codes**: A single byte identifying the publisher. Used by
//!   Game Boy (at 0x014B) and SNES (developer_id field). When the old code
//!   is 0x33 (GB) or developer_id == 0x33 (SNES), a 2-character "new" code
//!   is used instead.
//!
//! - **New (string) codes**: A 2-character ASCII code. Used across GB (new
//!   licensee at 0x0144-0x0145), SNES (extended header maker code), GBA
//!   (maker code at 0xB0-0xB1), DS (maker code at 0x010-0x011), and 3DS
//!   (maker code in SMDH / NCCH headers).
//!
//! This module merges all per-platform tables into a single canonical source.
//! Where two platforms assigned different names to the same code, the more
//! widely-used or more descriptive name is kept, with a comment noting the
//! alternative.
//!
//! Sources: Pan Docs (GB), GBATEK (GBA/DS), 3DBrew (3DS), fullsnes (SNES).

/// Look up a publisher/maker name from a 2-character ASCII licensee code.
///
/// This is the unified superset of the "new licensee" / "maker code" tables
/// from Game Boy, GBA, DS, 3DS, and SNES.
pub(crate) fn maker_code_name(code: &str) -> Option<&'static str> {
    match code {
        "00" => Some("None"),
        "01" => Some("Nintendo R&D1"),
        "08" => Some("Capcom"),
        "0A" => Some("Jaleco"), // SNES only
        "13" => Some("EA (Electronic Arts)"),
        "18" => Some("Hudson Soft"),
        "19" => Some("b-ai"),
        "1P" => Some("Creatures"),                  // SNES only
        "20" => Some("Destination Software / KSS"), // GB/GBA/DS: "kss"
        "22" => Some("pow"),
        "24" => Some("PCM Complete"),
        "25" => Some("san-x"),
        "28" => Some("Kemco Japan"),
        "29" => Some("seta"),
        "30" => Some("Viacom"),
        "31" => Some("Nintendo"),
        "32" => Some("Bandai"),
        "33" => Some("Ocean/Acclaim"),
        "34" => Some("Konami"),
        "35" => Some("Hector"),
        "37" => Some("Taito"),
        "38" => Some("Hudson"), // SNES: "Capcom"
        "39" => Some("Banpresto"),
        "41" => Some("Ubi Soft"),
        "42" => Some("Atlus"),
        "44" => Some("Malibu"),
        "46" => Some("angel"),
        "47" => Some("Bullet-Proof"), // SNES: "Spectrum Holobyte"
        "49" => Some("irem"),
        "50" => Some("Absolute"),
        "51" => Some("Acclaim"),
        "52" => Some("Activision"),
        "53" => Some("American sammy"),
        "54" => Some("Konami"), // SNES: "GameTek"
        "55" => Some("Hi tech entertainment"),
        "56" => Some("LJN"),
        "57" => Some("Matchbox"),
        "58" => Some("Mattel"),
        "59" => Some("Milton Bradley"),
        "5D" => Some("Midway"),  // SNES only
        "5G" => Some("Majesco"), // SNES only
        "60" => Some("Titus"),
        "61" => Some("Virgin"),
        "64" => Some("LucasArts"),
        "67" => Some("Ocean"),
        "69" => Some("EA (Electronic Arts)"),
        "6E" => Some("Elite Systems"),  // SNES only
        "6S" => Some("TDK Mediactive"), // SNES only
        "70" => Some("Infogrames"),
        "71" => Some("Interplay"),
        "72" => Some("Broderbund"),
        "73" => Some("sculptured"),
        "75" => Some("sci"), // SNES: "The Sales Curve"
        "78" => Some("THQ"),
        "79" => Some("Accolade"),
        "7D" => Some("Vivendi"), // SNES only
        "80" => Some("misawa"),
        "83" => Some("lozc"),
        "86" => Some("Tokuma Shoten"),
        "87" => Some("Tsukuda Original"),
        "8P" => Some("Sega"), // SNES only
        "91" => Some("Chunsoft"),
        "92" => Some("Video system"),
        "93" => Some("Ocean/Acclaim"),
        "95" => Some("Varie"),
        "96" => Some("Yonezawa/s'pal"),
        "97" => Some("Kaneko"),
        "99" => Some("Pack in soft"), // SNES: "Pack-In-Video"
        "9B" => Some("Tecmo"),        // SNES only
        "A4" => Some("Konami (Yu-Gi-Oh!)"),
        "AF" => Some("Namco"),                   // SNES only
        "B0" => Some("Acclaim"),                 // SNES only
        "B1" => Some("ASCII"),                   // SNES only
        "B2" => Some("Bandai"),                  // SNES only
        "B4" => Some("Enix"),                    // SNES only
        "B6" => Some("HAL Laboratory"),          // SNES only
        "BB" => Some("Sunsoft"),                 // SNES only
        "C0" => Some("Taito"),                   // SNES only
        "C3" => Some("Square"),                  // SNES only
        "C5" => Some("Data East"),               // SNES only
        "C8" => Some("Koei"),                    // SNES only
        "D1" => Some("Sofel"),                   // SNES only
        "E5" => Some("Epoch"),                   // SNES only
        "E7" => Some("Athena"),                  // SNES only
        "E9" => Some("Natsume"),                 // SNES only
        "EB" => Some("Atlus"),                   // SNES only
        "GR" => Some("Grasshopper Manufacture"), // 3DS only
        "GT" => Some("GUST"),                    // 3DS only
        "HB" => Some("Happinet"),                // 3DS only
        "KA" => Some("Kadokawa"),                // 3DS only
        "MR" => Some("Marvelous"),               // 3DS only
        "NB" => Some("Bandai Namco"),            // 3DS only
        "QH" => Some("D3 Publisher"),            // 3DS only
        "SQ" => Some("Square Enix"),             // 3DS only
        "XB" => Some("XSEED"),                   // 3DS only
        _ => None,
    }
}

/// Look up a publisher/maker name from an old-style single-byte licensee code.
///
/// This is the unified superset of the Game Boy old licensee table (0x014B)
/// and the SNES developer_id table. Where the same code maps to different
/// names across platforms, the more widely-used name is kept, with a comment
/// noting the alternative.
pub(crate) fn old_licensee_name(code: u8) -> Option<&'static str> {
    match code {
        0x00 => Some("None"),
        0x01 => Some("Nintendo"),
        0x08 => Some("Capcom"),
        0x09 => Some("Hot-B"), // GB only
        0x0A => Some("Jaleco"),
        0x0B => Some("Coconuts Japan"),
        0x0C => Some("Elite Systems"),        // GB only
        0x13 => Some("EA (Electronic Arts)"), // GB only
        0x18 => Some("Hudson Soft"),
        0x19 => Some("ITC Entertainment"),        // GB only
        0x1A => Some("Yanoman"),                  // GB only
        0x1D => Some("Japan Clary"),              // GB only; SNES: "Banpresto"
        0x1F => Some("Virgin Interactive"),       // GB only
        0x24 => Some("PCM Complete"),             // GB only
        0x25 => Some("San-X"),                    // GB only
        0x28 => Some("Kemco (Kotobuki Systems)"), // SNES: "Kemco Japan"
        0x29 => Some("SETA Corporation"),         // GB only
        0x30 => Some("Infogrames"),
        0x31 => Some("Nintendo"),
        0x32 => Some("Bandai"),        // GB only
        0x33 => Some("Ocean/Acclaim"), // SNES only (GB uses new licensee at 0x33)
        0x34 => Some("Konami"),
        0x35 => Some("HectorSoft"),
        0x38 => Some("Capcom"),
        0x39 => Some("Banpresto"),       // GB only
        0x3C => Some("Entertainment i"), // GB only
        0x3E => Some("Gremlin"),         // GB only
        0x41 => Some("Ubisoft"),
        0x42 => Some("Atlus"),
        0x44 => Some("Malibu"),
        0x46 => Some("Angel"),
        0x47 => Some("Spectrum Holobyte"), // GB only
        0x49 => Some("Irem"),              // GB only
        0x4A => Some("Virgin Interactive"),
        0x4D => Some("Malibu"), // GB only; SNES: "Tradewest"
        0x4F => Some("U.S. Gold"),
        0x50 => Some("Absolute"),
        0x51 => Some("Acclaim"),
        0x52 => Some("Activision"),
        0x53 => Some("American Sammy"),
        0x54 => Some("GameTek"),
        0x55 => Some("Park Place"),     // GB only
        0x56 => Some("LJN"),            // SNES: "Majesco"
        0x57 => Some("Matchbox"),       // GB only
        0x59 => Some("Milton Bradley"), // GB only
        0x5A => Some("Mindscape"),
        0x5B => Some("Romstar"),    // GB only
        0x5C => Some("Naxat Soft"), // GB only
        0x5D => Some("Tradewest"),  // GB only
        0x60 => Some("Titus Interactive"),
        0x61 => Some("Virgin Interactive"),
        0x67 => Some("Ocean Interactive"),
        0x69 => Some("EA (Electronic Arts)"), // SNES: "Electronic Arts"
        0x6E => Some("Elite Systems"),
        0x6F => Some("Electro Brain"),
        0x70 => Some("Infogrames"),
        0x71 => Some("Interplay"),
        0x72 => Some("Broderbund"),
        0x73 => Some("Sculptured Software"), // GB only
        0x75 => Some("The Sales Curve"),
        0x78 => Some("THQ"),
        0x79 => Some("Accolade"),
        0x7A => Some("Triffix Entertainment"), // GB only
        0x7C => Some("Microprose"),            // GB only
        0x7F => Some("Kemco"),
        0x80 => Some("Misawa Entertainment"),
        0x83 => Some("Lozc"), // SNES: "LOZC"
        0x86 => Some("Tokuma Shoten"),
        0x8B => Some("Bullet-Proof Software"),
        0x8C => Some("Vic Tokai"),
        0x8E => Some("Ape"),   // GB only; SNES: "Character Soft"
        0x8F => Some("I'Max"), // GB only
        0x91 => Some("Chunsoft"),
        0x92 => Some("Video System"),          // GB only
        0x93 => Some("Tsubaraya Productions"), // GB only; SNES: "Banpresto"
        0x95 => Some("Varie"),
        0x96 => Some("Yonezawa/s'pal"), // GB only
        0x97 => Some("Kaneko"),
        0x99 => Some("Arc"),          // GB only; SNES: "Pack-In-Video"
        0x9A => Some("Nihon Bussan"), // GB: "Nihon Bussan", SNES: "Nichibutsu"
        0x9B => Some("Tecmo"),
        0x9C => Some("Imagineer"),
        0x9D => Some("Banpresto"),     // GB only
        0x9F => Some("Nova"),          // GB only
        0xA0 => Some("Telenet"),       // SNES only
        0xA1 => Some("Hori Electric"), // GB only
        0xA2 => Some("Bandai"),        // GB only
        0xA4 => Some("Konami"),
        0xA6 => Some("Kawada"), // GB only
        0xA7 => Some("Takara"),
        0xA9 => Some("Technos Japan"), // GB only
        0xAA => Some("Broderbund"),    // GB only; SNES: "Culture Brain"
        0xAC => Some("Toei Animation"),
        0xAD => Some("Toho"), // GB only
        0xAF => Some("Namco"),
        0xB0 => Some("Acclaim"),
        0xB1 => Some("ASCII/Nexsoft"), // SNES: "ASCII / Nexoft"
        0xB2 => Some("Bandai"),
        0xB4 => Some("Square Enix"), // SNES: "Enix"
        0xB6 => Some("HAL Laboratory"),
        0xB7 => Some("SNK"),         // GB only
        0xB9 => Some("Pony Canyon"), // GB only
        0xBA => Some("Culture Brain"),
        0xBB => Some("Sunsoft"),
        0xBD => Some("Sony Imagesoft"),
        0xBF => Some("Sammy"),
        0xC0 => Some("Taito"),
        0xC2 => Some("Kemco"),
        0xC3 => Some("Square"),
        0xC4 => Some("Tokuma Shoten"),
        0xC5 => Some("Data East"),
        0xC6 => Some("Tonkinhouse"), // SNES: "Tonkin House"
        0xC8 => Some("Koei"),
        0xC9 => Some("UFL"),   // GB only
        0xCA => Some("Ultra"), // GB only; SNES: "Konami"
        0xCB => Some("Vap"),   // SNES: "Vapinc / NTVIC"
        0xCC => Some("Use Corporation"),
        0xCD => Some("Meldac"), // GB only
        0xCE => Some("Pony Canyon"),
        0xCF => Some("Angel"), // GB only
        0xD0 => Some("Taito"),
        0xD1 => Some("Sofel"),
        0xD2 => Some("Quest"),             // GB only; SNES: "Bothtec"
        0xD3 => Some("Sigma Enterprises"), // GB only
        0xD4 => Some("ASK Kodansha"),      // GB only
        0xD6 => Some("Naxat Soft"),
        0xD7 => Some("Copya System"), // GB only
        0xD9 => Some("Banpresto"),
        0xDA => Some("Tomy"),
        0xDB => Some("LJN"), // GB only; SNES: "Hiro"
        0xDD => Some("NCS"),
        0xDE => Some("Human"),
        0xDF => Some("Altron"),
        0xE0 => Some("Jaleco"), // GB only
        0xE1 => Some("Towa Chiki"),
        0xE2 => Some("Yutaka"),
        0xE3 => Some("Varie"), // GB only
        0xE5 => Some("Epoch"),
        0xE7 => Some("Athena"),
        0xE8 => Some("Asmik Ace"), // SNES: "Asmik"
        0xE9 => Some("Natsume"),
        0xEA => Some("King Records"),
        0xEB => Some("Atlus"),
        0xEC => Some("Epic/Sony Records"),
        0xEE => Some("IGS"),
        0xF0 => Some("A Wave"),                // SNES: "A-Wave"
        0xF3 => Some("Extreme Entertainment"), // GB only
        0xFF => Some("LJN"),                   // GB only
        _ => None,
    }
}
