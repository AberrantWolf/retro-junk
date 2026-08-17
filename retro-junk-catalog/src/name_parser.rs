//! Parser for No-Intro and Redump naming conventions.
//!
//! DAT entries follow a structured naming convention that encodes metadata:
//! ```text
//! Game Name (Region1, Region2) (Rev X) (En,Fr,De) [!] [b]
//! ```
//!
//! This parser extracts the base title, regions, revision, languages, flags,
//! and status information from these names.

/// Parsed components of a No-Intro/Redump filename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedDatName {
    /// Base game title without any parenthetical or bracketed tags.
    pub title: String,
    /// Region strings as they appear in the name (e.g., "USA", "Japan", "Europe").
    pub regions: Vec<String>,
    /// Revision string if present (e.g., "Rev A", "Rev 1", "Rev 1.1").
    pub revision: Option<String>,
    /// Language codes if present (e.g., "En", "Fr", "De").
    pub languages: Vec<String>,
    /// Flags from parenthesized tags (e.g., "Unl", "Proto", "Beta", "Sample", "Demo").
    pub flags: Vec<String>,
    /// Disc number for multi-disc games.
    pub disc_number: Option<u32>,
    /// Disc designator exactly as catalogued (for example `0`, `2`, or `A`).
    /// Unlike `disc_number`, this distinguishes Disc 0 from unnumbered media
    /// and preserves alphabetic Redump positions.
    pub disc_designator: Option<String>,
    /// Disc label (e.g., "Disc 1 - The Beginning").
    pub disc_label: Option<String>,
    /// Status from bracketed tags: verified, bad dump, overdump.
    pub status: DumpStatus,
    /// Version string if present (e.g., "v1.0", "v1.1").
    pub version: Option<String>,
}

/// Dump verification status from bracketed tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DumpStatus {
    /// No status tag or [!] verified tag.
    #[default]
    Verified,
    /// [b] bad dump.
    BadDump,
    /// [o] overdump.
    Overdump,
}

/// Parse a No-Intro/Redump DAT name into its components.
///
/// # Examples
///
/// ```
/// use retro_junk_catalog::name_parser::parse_dat_name;
///
/// let parsed = parse_dat_name("Super Mario Bros. (USA)");
/// assert_eq!(parsed.title, "Super Mario Bros.");
/// assert_eq!(parsed.regions, vec!["USA"]);
///
/// let parsed = parse_dat_name("Final Fantasy VII (USA) (Disc 1)");
/// assert_eq!(parsed.title, "Final Fantasy VII");
/// assert_eq!(parsed.disc_number, Some(1));
///
/// let parsed = parse_dat_name("Zelda no Densetsu (Japan) (Rev A) (En,Fr)");
/// assert_eq!(parsed.revision, Some("Rev A".to_string()));
/// assert_eq!(parsed.languages, vec!["En", "Fr"]);
/// ```
#[must_use]
pub fn parse_dat_name(name: &str) -> ParsedDatName {
    let mut result = ParsedDatName {
        title: String::new(),
        regions: Vec::new(),
        revision: None,
        languages: Vec::new(),
        flags: Vec::new(),
        disc_number: None,
        disc_designator: None,
        disc_label: None,
        status: DumpStatus::Verified,
        version: None,
    };

    let (title, tags) = extract_title_and_tags(name);
    result.title = title;

    for tag in &tags {
        match tag {
            Tag::Paren(content) => classify_paren_tag(content, &mut result),
            Tag::Bracket(content) => classify_bracket_tag(content, &mut result),
        }
    }

    result
}

// ── Internal parsing ────────────────────────────────────────────────────────

#[derive(Debug)]
enum Tag {
    Paren(String),
    Bracket(String),
}

/// Split a DAT name into the base title and a sequence of (parenthesized) and [bracketed] tags.
fn extract_title_and_tags(name: &str) -> (String, Vec<Tag>) {
    let mut tags = Vec::new();
    let mut title_end = None;
    let mut chars = name.char_indices().peekable();

    while let Some((i, ch)) = chars.next() {
        let (open, close, make_tag): (char, char, fn(String) -> Tag) = match ch {
            '(' => ('(', ')', Tag::Paren),
            '[' => ('[', ']', Tag::Bracket),
            _ => continue,
        };

        if title_end.is_none() {
            title_end = Some(i);
        }

        let mut depth = 1u32;
        let start = i + open.len_utf8();
        let mut end = start;

        for (j, c) in chars.by_ref() {
            if c == open {
                depth += 1;
            } else if c == close {
                depth -= 1;
                if depth == 0 {
                    end = j;
                    break;
                }
            }
        }

        let content = name[start..end].to_string();
        // Skip if content is empty
        if !content.is_empty() {
            tags.push(make_tag(content));
        }
    }

    let title = match title_end {
        Some(pos) => name[..pos].trim_end().to_string(),
        None => name.trim().to_string(),
    };

    (title, tags)
}

/// Known region strings in No-Intro/Redump naming.
const KNOWN_REGIONS: &[&str] = &[
    "USA",
    "Japan",
    "Europe",
    "World",
    "Australia",
    "Korea",
    "China",
    "Taiwan",
    "Brazil",
    "France",
    "Germany",
    "Spain",
    "Italy",
    "Netherlands",
    "Sweden",
    "Norway",
    "Denmark",
    "Finland",
    "Portugal",
    "Russia",
    "Hong Kong",
    "Asia",
    "Canada",
    "Mexico",
    "Argentina",
    "Chile",
    "Colombia",
    "India",
    "South Africa",
    "United Kingdom",
    "New Zealand",
    "Poland",
    "Czech Republic",
    "Hungary",
    "Greece",
    "Turkey",
    "Israel",
    "Saudi Arabia",
    "UAE",
    "Scandinavia",
    "Latin America",
];

fn is_region_string(s: &str) -> bool {
    // Check if every comma-separated part is a known region
    s.split(',').all(|part| {
        let trimmed = part.trim();
        KNOWN_REGIONS
            .iter()
            .any(|r| r.eq_ignore_ascii_case(trimmed))
    })
}

/// Classify a parenthesized tag and update the result accordingly.
fn classify_paren_tag(content: &str, result: &mut ParsedDatName) {
    let trimmed = content.trim();

    // Region tag: "USA", "Japan", "USA, Europe", etc.
    if is_region_string(trimmed) {
        for part in trimmed.split(',') {
            let region = part.trim().to_string();
            if !result.regions.contains(&region) {
                result.regions.push(region);
            }
        }
        return;
    }

    // Revision: "Rev A", "Rev 1", "Rev 1.1"
    if let Some(rev) = trimmed.strip_prefix("Rev ") {
        result.revision = Some(format!("Rev {rev}"));
        return;
    }

    // Version: "v1.0", "v1.1", "V1.2"
    if (trimmed.starts_with('v') || trimmed.starts_with('V'))
        && trimmed.len() > 1
        && trimmed.as_bytes()[1].is_ascii_digit()
    {
        result.version = Some(trimmed.to_string());
        return;
    }

    // Disc: "Disc 0", "Disc 2", "Disc A", "Disc 1 - The Beginning"
    if let Some(disc_rest) = trimmed.strip_prefix("Disc ") {
        // Parse "Disc N" / "Disc A" and an optional role label.
        let parts: Vec<&str> = disc_rest.splitn(2, " - ").collect();
        let designator = parts[0].trim();
        if let Ok(n) = designator.parse::<u32>() {
            result.disc_number = Some(n);
            result.disc_designator = Some(designator.to_owned());
            if parts.len() > 1 {
                result.disc_label = Some(parts[1].trim().to_string());
            }
        } else if designator.len() == 1
            && designator
                .chars()
                .all(|character| character.is_ascii_alphabetic())
        {
            let letter = designator.to_ascii_uppercase();
            result.disc_number = letter.bytes().next().map(|byte| u32::from(byte - b'A') + 1);
            result.disc_designator = Some(letter);
            if parts.len() > 1 {
                result.disc_label = Some(parts[1].trim().to_string());
            }
        }
        return;
    }

    // Language list: "En,Fr,De" — 2-letter codes separated by commas
    if looks_like_language_list(trimmed) {
        for lang in trimmed.split(',') {
            result.languages.push(lang.trim().to_string());
        }
        return;
    }

    // Known flags
    let lower = trimmed.to_lowercase();
    match lower.as_str() {
        "unl" | "unlicensed" | "proto" | "prototype" | "beta" | "sample" | "demo" | "kiosk"
        | "debug" | "pirate" | "promo" | "virtual console" | "switch online" | "aftermarket"
        | "homebrew" | "test program" => {
            result.flags.push(trimmed.to_string());
            return;
        }
        _ => {}
    }

    // Anything else is also stored as a flag (e.g., alt version labels, compilation names)
    result.flags.push(trimmed.to_string());
}

/// Whether a Redump parenthetical describes one physical carrier rather than
/// a retail/software edition. Such tags must not split a logical multi-disc
/// release merely because different discs were pressed from different masters.
#[must_use]
pub fn is_carrier_only_flag(flag: &str) -> bool {
    let value = flag.trim();
    let upper = value.to_ascii_uppercase();

    // Saturn mastering/ring-code groups used by Redump: 1M, 2MB1, 32B, etc.
    let mastering = if let Some(first_non_digit) = upper.find(|c: char| !c.is_ascii_digit()) {
        first_non_digit > 0
            && upper[first_non_digit..]
                .chars()
                .all(|c| c.is_ascii_alphanumeric())
            && (upper[first_non_digit..].starts_with('M')
                || (upper[first_non_digit..].len() == 1
                    && upper[first_non_digit..]
                        .chars()
                        .all(|c| c.is_ascii_alphabetic())))
    } else {
        false
    };
    if mastering {
        return true;
    }

    let lower = value.to_ascii_lowercase();
    matches!(lower.as_str(), "gdi" | "nod")
        || lower.ends_with(" disc")
        || lower.ends_with(" disk")
        || lower.ends_with(" cd")
        || lower.ends_with(" cd-rom")
}

/// Check if a string looks like a language list (comma-separated 2-3 letter codes).
fn looks_like_language_list(s: &str) -> bool {
    let parts: Vec<&str> = s.split(',').collect();
    // Must have at least 2 parts to be a language list (single codes are ambiguous)
    if parts.len() < 2 {
        return false;
    }
    parts.iter().all(|p| {
        let t = p.trim();
        (2..=3).contains(&t.len())
            && t.chars().next().is_some_and(|c| c.is_ascii_uppercase())
            && t.chars().skip(1).all(|c| c.is_ascii_lowercase())
    })
}

/// Classify a bracketed tag and update the result accordingly.
fn classify_bracket_tag(content: &str, result: &mut ParsedDatName) {
    match content.trim() {
        "!" => result.status = DumpStatus::Verified,
        "b" => result.status = DumpStatus::BadDump,
        "o" => result.status = DumpStatus::Overdump,
        _ => {
            // Unknown bracket tags are stored as flags
            result.flags.push(format!("[{}]", content.trim()));
        }
    }
}

/// Map a catalog region slug back to its Redump display format.
///
/// Returns the display string (e.g., "USA", "Japan", "Europe").
/// Unknown slugs are returned title-cased.
#[must_use]
pub fn region_slug_to_display(slug: &str) -> String {
    match slug {
        "usa" => "USA".to_string(),
        "japan" => "Japan".to_string(),
        "europe" => "Europe".to_string(),
        "world" => "World".to_string(),
        "australia" => "Australia".to_string(),
        "korea" => "Korea".to_string(),
        "china" => "China".to_string(),
        "taiwan" => "Taiwan".to_string(),
        "brazil" => "Brazil".to_string(),
        "france" => "France".to_string(),
        "germany" => "Germany".to_string(),
        "spain" => "Spain".to_string(),
        "italy" => "Italy".to_string(),
        "netherlands" => "Netherlands".to_string(),
        "sweden" => "Sweden".to_string(),
        "norway" => "Norway".to_string(),
        "denmark" => "Denmark".to_string(),
        "finland" => "Finland".to_string(),
        "portugal" => "Portugal".to_string(),
        "russia" => "Russia".to_string(),
        "hong-kong" => "Hong Kong".to_string(),
        "asia" => "Asia".to_string(),
        "canada" => "Canada".to_string(),
        "united-kingdom" => "United Kingdom".to_string(),
        "scandinavia" => "Scandinavia".to_string(),
        "latin-america" => "Latin America".to_string(),
        other => {
            // Title-case: uppercase first char of each word
            other
                .split('-')
                .map(|word| {
                    let mut chars = word.chars();
                    match chars.next() {
                        Some(c) => {
                            let upper: String = c.to_uppercase().collect();
                            format!("{upper}{}", chars.as_str())
                        }
                        None => String::new(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        }
    }
}

/// Map a No-Intro/Redump region string to a lowercase slug used in the catalog.
///
/// Returns the canonical region slug (e.g., "usa", "japan", "europe").
#[must_use]
pub fn region_to_slug(region: &str) -> &'static str {
    match region.to_lowercase().as_str() {
        "usa" | "us" | "united states" => "usa",
        "japan" | "jp" | "jpn" => "japan",
        "europe" | "eu" | "eur" => "europe",
        "world" | "wld" => "world",
        "australia" | "aus" => "australia",
        "korea" | "kor" | "kr" => "korea",
        "china" | "chn" | "cn" => "china",
        "taiwan" | "twn" | "tw" => "taiwan",
        "brazil" | "bra" | "br" => "brazil",
        "france" | "fra" | "fr" => "france",
        "germany" | "ger" | "de" | "deu" => "germany",
        "spain" | "esp" | "es" => "spain",
        "italy" | "ita" | "it" => "italy",
        "netherlands" | "ned" | "nl" | "nld" | "holland" => "netherlands",
        "sweden" | "swe" | "se" => "sweden",
        "norway" | "nor" | "no" => "norway",
        "denmark" | "den" | "dk" | "dnk" => "denmark",
        "finland" | "fin" | "fi" => "finland",
        "portugal" | "por" | "pt" | "prt" => "portugal",
        "russia" | "rus" | "ru" => "russia",
        "hong kong" | "hk" | "hkg" => "hong-kong",
        "asia" => "asia",
        "canada" | "can" | "ca" => "canada",
        "united kingdom" | "uk" | "gbr" | "gb" => "united-kingdom",
        "scandinavia" => "scandinavia",
        "latin america" => "latin-america",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disc_zero_and_alphabetic_positions_are_explicit() {
        let zero = parse_dat_name("Enemy Zero (Japan) (Disc 0) (Opening Disc)");
        assert_eq!(zero.disc_number, Some(0));
        assert_eq!(zero.disc_designator.as_deref(), Some("0"));

        let alpha = parse_dat_name("Game (Japan) (Disc B)");
        assert_eq!(alpha.disc_number, Some(2));
        assert_eq!(alpha.disc_designator.as_deref(), Some("B"));
    }

    #[test]
    fn saturn_mastering_and_disc_roles_are_carrier_metadata() {
        for flag in ["1M", "2MB1", "32B", "Opening Disc", "GDI", "NOD"] {
            assert!(is_carrier_only_flag(flag), "{flag}");
        }
        for flag in ["Satakore", "Beta", "Shokai Tokutenban"] {
            assert!(!is_carrier_only_flag(flag), "{flag}");
        }
    }
}
