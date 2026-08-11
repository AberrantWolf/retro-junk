use serde::{Deserialize, Serialize};

/// Geographic regions for ROM releases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Region {
    /// Japan
    Japan,
    /// USA / North America
    Usa,
    /// Europe (PAL regions)
    Europe,
    /// Australia
    Australia,
    /// Korea
    Korea,
    /// China
    China,
    /// Taiwan / Hong Kong
    Taiwan,
    /// Asia (non-Japan, NTSC-compatible)
    Asia,
    /// Brazil
    Brazil,
    /// Latin America (non-Brazil)
    LatinAmerica,
    /// World / Region-free
    World,
    /// Unknown region
    Unknown,
}

impl Region {
    /// All user-selectable region variants (excludes `Unknown`).
    pub const ALL: &[Region] = &[
        Self::Japan,
        Self::Usa,
        Self::Europe,
        Self::Australia,
        Self::Korea,
        Self::China,
        Self::Taiwan,
        Self::Asia,
        Self::Brazil,
        Self::LatinAmerica,
        Self::World,
    ];

    /// Returns the standard abbreviation for this region.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Japan => "JPN",
            Self::Usa => "USA",
            Self::Europe => "EUR",
            Self::Australia => "AUS",
            Self::Korea => "KOR",
            Self::China => "CHN",
            Self::Taiwan => "TWN",
            Self::Asia => "ASI",
            Self::Brazil => "BRA",
            Self::LatinAmerica => "LAT",
            Self::World => "WLD",
            Self::Unknown => "UNK",
        }
    }

    /// Returns the full name of this region.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Japan => "Japan",
            Self::Usa => "USA",
            Self::Europe => "Europe",
            Self::Australia => "Australia",
            Self::Korea => "Korea",
            Self::China => "China",
            Self::Taiwan => "Taiwan",
            Self::Asia => "Asia",
            Self::Brazil => "Brazil",
            Self::LatinAmerica => "Latin America",
            Self::World => "World",
            Self::Unknown => "Unknown",
        }
    }

    /// Recover a region from the lowercase form projections and manifests
    /// store it in, so it can be written back out the way a DAT writes it.
    ///
    /// Records keep region as a lowercase token — `usa`, `jp`, `eur` — which
    /// is right for comparing and grouping and wrong for naming a file: DAT
    /// convention is `(USA)`, `(Japan)`, `(Europe)`, and a playable named
    /// `Game (usa).chd` matches neither the catalog nor what a frontend or a
    /// scraper expects to see. The aliases mirror the ones the archive
    /// importer already accepts, so both sides agree on what a token means.
    #[must_use]
    pub fn from_slug(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "japan" | "jp" | "jpn" => Some(Self::Japan),
            "usa" | "us" | "united states" | "united_states" | "canada" | "north america"
            | "north-america" => Some(Self::Usa),
            "europe" | "eur" | "eu" | "pal" => Some(Self::Europe),
            "australia" | "aus" => Some(Self::Australia),
            "korea" | "kor" => Some(Self::Korea),
            "china" | "chn" => Some(Self::China),
            "taiwan" | "twn" => Some(Self::Taiwan),
            "asia" | "asi" => Some(Self::Asia),
            "brazil" | "bra" => Some(Self::Brazil),
            "latin america" | "latin-america" | "latinamerica" | "latin_america" | "lat" => {
                Some(Self::LatinAmerica)
            }
            "world" | "wld" => Some(Self::World),
            _ => None,
        }
    }

    /// Attempt to parse a region from a code character (common in serial numbers).
    #[must_use]
    pub fn from_code_char(c: char) -> Option<Self> {
        match c.to_ascii_uppercase() {
            'J' => Some(Self::Japan),
            'U' | 'E' => Some(Self::Usa), // E is sometimes used for "English/USA"
            'P' => Some(Self::Europe),    // PAL
            'A' => Some(Self::Australia),
            'K' => Some(Self::Korea),
            'C' => Some(Self::China),
            'T' => Some(Self::Taiwan),
            'W' => Some(Self::World),
            _ => None,
        }
    }
}

impl std::fmt::Display for Region {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}
