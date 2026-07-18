//! Cross-analyzer invariants for the CHD compression/hashing feature.
//!
//! These check every analyzer registered in [`crate::create_default_context`]
//! rather than one specific console, so they live here (the crate that
//! depends on every platform crate) instead of in a single platform crate's
//! `tests/` directory.
//!
//! Background: `RomAnalyzer::chd_extensions()` (see retro-junk-core) lets an
//! analyzer declare which of its extensions chdman can losslessly convert
//! to `.chd`. Declaring that capability creates two obligations that are easy
//! to forget when wiring up a new platform (this bill actually came due for
//! Sega CD, Dreamcast, and PSP — see docs/chd-remediation-spec.md, C2):
//!
//! 1. The produced `.chd` file must be scannable (`file_extensions()` must
//!    list `"chd"`), or the conversion feature's own output vanishes from the
//!    library.
//! 2. Any analyzer that can encounter a `.chd` must override
//!    `compute_container_hashes` to decompress and hash the inner disc data.
//!    Per CLAUDE.md's compressed-format-hashing rule, DAT matching must never
//!    hash compressed container bytes directly — the default implementation
//!    of `compute_container_hashes` returns `Ok(None)` (defer to the plain
//!    streaming hasher), which for a CHD means hashing the compressed
//!    container itself.

use crate::create_default_context;
use retro_junk_core::{ChdExtensionRole, HashAlgorithms};

/// Any analyzer that declares a CHD-convertible source extension
/// (`chd_extensions()` contains a `Source(_)` entry) must also list `"chd"`
/// in `file_extensions()`. Otherwise the very file this analyzer's own CHD
/// conversion produces is invisible to library scans.
#[test]
fn chd_convertible_analyzers_declare_chd_extension() {
    let ctx = create_default_context();
    let violations: Vec<&'static str> = ctx
        .consoles()
        .map(|console| console.analyzer.as_ref())
        .filter(|analyzer| {
            analyzer
                .chd_extensions()
                .iter()
                .any(|(_, role)| matches!(role, ChdExtensionRole::Source(_)))
        })
        .filter(|analyzer| !analyzer.file_extensions().contains(&"chd"))
        .map(retro_junk_core::RomAnalyzer::platform_name)
        .collect();

    assert!(
        violations.is_empty(),
        "these analyzers declare a CHD-convertible source extension via \
         chd_extensions() but do not list \"chd\" in file_extensions() -- the \
         .chd file their own conversion feature produces would never be \
         scanned: {violations:?}",
    );
}

/// Any analyzer that lists `"chd"` in `file_extensions()` must override
/// `compute_container_hashes` to actually decompress and hash the CHD's
/// inner disc data, rather than falling through to the default.
///
/// Detection mechanism (there is no direct way to ask "is this method
/// overridden" through the trait object): the default implementation of
/// `compute_container_hashes` (retro-junk-core lib.rs) returns `Ok(None)`
/// unconditionally, without ever touching the reader. Every real override
/// in this codebase (`hash_disc_container` and friends) instead attempts to
/// parse disc/CHD structure from the reader and returns `Ok(Some(hashes))`
/// on success or `Err(_)` when the input doesn't parse (as an empty/garbage
/// cursor won't). So "not `Ok(None)`" on a garbage reader is a reliable
/// behavioral proxy for "this method was overridden with a real
/// implementation" -- it would fail to distinguish two different *real*
/// overrides, but that's not what this test is checking for.
#[test]
fn chd_capable_analyzers_override_compute_container_hashes() {
    let ctx = create_default_context();
    let violations: Vec<&'static str> = ctx
        .consoles()
        .map(|console| console.analyzer.as_ref())
        .filter(|analyzer| analyzer.file_extensions().contains(&"chd"))
        .filter(|analyzer| {
            let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
            let result = analyzer.compute_container_hashes(
                &mut cursor,
                HashAlgorithms::Crc32Sha1,
                None,
                None,
            );
            matches!(result, Ok(None))
        })
        .map(retro_junk_core::RomAnalyzer::platform_name)
        .collect();

    assert!(
        violations.is_empty(),
        "these analyzers list \"chd\" in file_extensions() but \
         compute_container_hashes() returned Ok(None) for a garbage/empty reader \
         -- this is the trait's default implementation, meaning no override \
         exists and .chd files for these platforms would be hashed as raw \
         compressed container bytes instead of decompressed disc data (violates \
         CLAUDE.md's compressed-format-hashing rule): {violations:?}",
    );
}
