//! Track structure of a DAT game, and the naming rule that follows from it.
//!
//! A Redump entry for a multi-track disc lists one ROM per *member file* — a
//! `.cue` sheet plus `Game (Track 1).bin`, `Game (Track 2).bin`, and so on.
//! Only the game name names the disc itself. Everything that has to name a
//! file holding the whole medium (a CHD, an ISO, a cue/bin set) needs that
//! distinction, so it lives here once rather than being re-derived by each
//! caller from filename shapes.

use crate::DatRom;

/// Whether a ROM entry is the set's `.cue` sheet.
#[must_use]
pub fn is_cue_name(name: &str) -> bool {
    std::path::Path::new(name)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("cue"))
}

/// Whether a game's ROM entries describe several member tracks.
///
/// Full Redump DATs list a CUE alongside per-track BINs, so the CUE does not
/// count: a single-track disc still has two ROM entries.
#[must_use]
pub fn is_multi_track(roms: &[DatRom]) -> bool {
    roms.iter().filter(|rom| !is_cue_name(&rom.name)).count() > 1
}

/// The track number in a Redump ROM name like `Game (Track 02).bin`.
///
/// `0` when the name carries no track tag — the caller is naming something
/// that is not a member track.
#[must_use]
pub fn track_number(name: &str) -> i32 {
    const PREFIX: &str = "(Track ";
    let Some(start) = name.find(PREFIX) else {
        return 0;
    };
    let after = &name[start + PREFIX.len()..];
    after
        .find(')')
        .and_then(|end| after[..end].trim().parse().ok())
        .unwrap_or(0)
}

/// Whether a ROM name is a member track of a multi-track set.
#[must_use]
pub fn is_track_member(name: &str) -> bool {
    !is_cue_name(name) && track_number(name) > 0
}

/// Remove a trailing ` (Track N)` tag from a name.
///
/// Redump forms each member filename by appending this tag to the game name,
/// so removing it recovers the disc-level name when the caller holds only the
/// ROM entry. Prefer the DAT game name where it is available.
#[must_use]
pub fn strip_track_tag(name: &str) -> String {
    const PREFIX: &str = " (Track ";
    let Some(start) = name.rfind(PREFIX) else {
        return name.to_owned();
    };
    let after = &name[start + PREFIX.len()..];
    let Some(close) = after.find(')') else {
        return name.to_owned();
    };
    let digits = after[..close].trim();
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return name.to_owned();
    }
    format!("{}{}", &name[..start], &after[close + 1..])
}

/// The stem a file holding the *whole* medium should carry.
///
/// For a multi-track disc this is the game name: a CHD, an ISO, or a cue/bin
/// set *is* the disc, and no member track's filename names it. Inheriting one
/// — which is what taking a track ROM's stem does — produces artifacts like
/// `Some Game (Japan) (1M) (Track 1).chd` for a container holding every track,
/// and propagates the same wrong stem to scraped media and frontend entries
/// derived from it.
///
/// Single-file media keep the ROM name, whose stem legitimately distinguishes
/// representations sharing one game name — N64 DATs carry separate `.z64` and
/// `.v64` records under a single entry.
///
/// `dat_name` may be empty for callers that only carry the matched ROM entry;
/// the track tag is then stripped instead. A DAT game name is not a filename —
/// it routinely ends in a parenthesized tag and may contain periods (`Dr. Mario
/// (USA)`) — so it is used verbatim. Only the ROM name, a real filename, gets
/// its extension stripped.
#[must_use]
pub fn whole_medium_stem(dat_name: &str, rom_name: &str, multi_track: bool) -> String {
    let rom_stem = std::path::Path::new(rom_name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(rom_name);
    if !multi_track && !rom_stem.trim().is_empty() {
        return rom_stem.to_owned();
    }
    if dat_name.trim().is_empty() {
        strip_track_tag(rom_stem)
    } else {
        dat_name.to_owned()
    }
}

#[cfg(test)]
#[path = "tests/tracks_tests.rs"]
mod tests;
