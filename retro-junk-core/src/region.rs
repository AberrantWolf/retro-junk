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
    /// Brazil
    Brazil,
    /// World / Region-free
    World,
    /// Unknown region
    Unknown,
}

impl Region {
    /// Returns the standard abbreviation for this region.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Japan => "JPN",
            Self::Usa => "USA",
            Self::Europe => "EUR",
            Self::Australia => "AUS",
            Self::Korea => "KOR",
            Self::China => "CHN",
            Self::Taiwan => "TWN",
            Self::Brazil => "BRA",
            Self::World => "WLD",
            Self::Unknown => "UNK",
        }
    }

    /// Returns the full name of this region.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Japan => "Japan",
            Self::Usa => "USA",
            Self::Europe => "Europe",
            Self::Australia => "Australia",
            Self::Korea => "Korea",
            Self::China => "China",
            Self::Taiwan => "Taiwan",
            Self::Brazil => "Brazil",
            Self::World => "World",
            Self::Unknown => "Unknown",
        }
    }

    /// Attempt to parse a region from a code character (common in serial numbers).
    pub fn from_code_char(c: char) -> Option<Self> {
        match c.to_ascii_uppercase() {
            'J' => Some(Self::Japan),
            'U' | 'E' => Some(Self::Usa), // E is sometimes used for "English/USA"
            'P' => Some(Self::Europe),    // PAL
            'A' => Some(Self::Australia),
            'K' => Some(Self::Korea),
            'C' => Some(Self::China),
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
