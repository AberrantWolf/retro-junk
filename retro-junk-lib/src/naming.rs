//! The one canonical-name rule.
//!
//! Everything that decides what a file should be called goes through here:
//! the playable builder naming its output, the library rename planner naming
//! a scanned ROM, the multi-disc playlist folder, and the conformance check
//! that notices a file whose name no longer matches. Three separate layers
//! used to answer this question, agreeing most of the time — and where they
//! disagreed, nothing noticed, because nothing compared them.
//!
//! The rule, in order of authority:
//!
//! 1. What the catalog calls the whole medium. A multi-track disc's catalog
//!    `rom_name` is its largest *track* file, so a container holding the whole
//!    disc takes the game name instead — otherwise a CHD of a Redump disc is
//!    called `… (Track 1).chd`, and the scraped artwork and frontend entry
//!    inherit that name too.
//! 2. Failing that, what the archive's own manifest says: title plus region,
//!    revision, and variant, in the shape a catalog would write them. This is
//!    a provisional name and is labelled as such — it is what an unidentified
//!    release is called until identification gives it a real one.
//!
//! A disc number is then appended when the release has more than one disc,
//! because that belongs to the physical carrier rather than to either name
//! source.

#[cfg(test)]
#[path = "tests/naming_tests.rs"]
mod tests;

use std::fmt::Write as _;

/// Everything the rule needs to name one file.
///
/// Catalog fields are empty when the carrier is not bound; the archive
/// manifest fields then decide the name.
#[derive(Debug, Clone, Default)]
pub struct NameInputs<'a> {
    /// The catalog's name for the game, e.g. `Crash Team Racing (USA)`.
    pub dat_name: &'a str,
    /// The catalog's filename for the medium (or its largest track).
    pub rom_name: &'a str,
    /// Whether the catalog stores this medium as separate track files.
    pub medium_has_tracks: bool,
    /// The archive manifest's title, used when the catalog says nothing.
    pub title: &'a str,
    /// Region slug as the manifest stores it (lowercased).
    pub region: &'a str,
    pub revision: &'a str,
    pub variant: &'a str,
    /// This carrier's disc number, or 0 when it is not a numbered disc.
    pub disc_number: u32,
    /// Discs the complete release is expected to have.
    pub disc_count: u32,
}

/// Where a canonical name came from — the caller may want to say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameSource {
    /// The catalog named it. This is a real identity.
    Catalog,
    /// The archive manifest named it. Provisional until identification.
    ArchiveManifest,
}

/// The canonical stem (no extension) for one file, and where it came from.
#[must_use]
pub fn canonical_stem(inputs: &NameInputs<'_>) -> (String, NameSource) {
    let (mut name, source) =
        if inputs.dat_name.trim().is_empty() && inputs.rom_name.trim().is_empty() {
            (archive_manifest_name(inputs), NameSource::ArchiveManifest)
        } else {
            (
                retro_junk_dat::tracks::whole_medium_stem(
                    inputs.dat_name,
                    inputs.rom_name,
                    inputs.medium_has_tracks,
                ),
                NameSource::Catalog,
            )
        };
    // A name that already states its disc keeps it: the catalog writes
    // `(Disc N)` itself when the DAT does.
    if inputs.disc_count > 1 && inputs.disc_number > 0 && !name.contains("(Disc ") {
        let _ = write!(name, " (Disc {})", inputs.disc_number);
    }
    (retro_junk_archive::safe_file_stem(&name), source)
}

/// The canonical filename, with the extension the container actually uses.
///
/// The extension is the caller's to decide — it depends on the format a
/// playable was built in, or on what a scanned file turned out to be — and
/// naming never overrides it.
#[must_use]
pub fn canonical_filename(inputs: &NameInputs<'_>, extension: &str) -> String {
    let (stem, _) = canonical_stem(inputs);
    if extension.is_empty() {
        stem
    } else {
        format!("{stem}.{extension}")
    }
}

/// The name of a multi-disc set's playlist folder: the release's name with no
/// disc suffix, since the folder holds every disc.
#[must_use]
pub fn canonical_release_stem(inputs: &NameInputs<'_>) -> String {
    let without_disc = NameInputs {
        disc_number: 0,
        disc_count: 0,
        ..inputs.clone()
    };
    canonical_stem(&without_disc).0
}

/// The archive's own name for a release, in the shape a catalog would write.
///
/// Region is stored lowercased for comparison and written back out the way a
/// catalog writes it — `(USA)`, not `(usa)`. An unrecognized region is passed
/// through rather than dropped: a name the tool does not understand is still
/// the user's.
fn archive_manifest_name(inputs: &NameInputs<'_>) -> String {
    let mut name = inputs.title.to_owned();
    let region = retro_junk_core::Region::from_slug(inputs.region).map_or_else(
        || inputs.region.to_owned(),
        |region| region.name().to_owned(),
    );
    for value in [region.as_str(), inputs.revision, inputs.variant] {
        if !value.is_empty() {
            let _ = write!(name, " ({value})");
        }
    }
    name
}

/// Whether a file's current name is the one the rule would give it.
///
/// Compared as stems, because the extension follows the container format
/// rather than the name: a disc converted from BIN+CUE to CHD is correctly
/// named even though its extension changed.
#[must_use]
pub fn name_conforms(current_file_name: &str, inputs: &NameInputs<'_>) -> bool {
    let current_stem = std::path::Path::new(current_file_name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(current_file_name);
    let (canonical, source) = canonical_stem(inputs);
    // A provisional name is not evidence of anything, so it is never used to
    // call an existing file wrong. Only the catalog may condemn a name.
    if source == NameSource::ArchiveManifest {
        return true;
    }
    current_stem == canonical
}
