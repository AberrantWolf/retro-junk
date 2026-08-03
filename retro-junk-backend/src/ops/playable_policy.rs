//! Choosing which playable format a platform should build by default.
//!
//! The root manifest is the authority; the projection carries a copy so the
//! library view can show the choice without reading the manifest. Both are
//! updated together here so they cannot drift apart.

use std::path::Path;

use retro_junk_archive::{
    ArchiveRootManifest, CollectionProfile, DesiredPlayablePolicy, RepresentationFormat,
};

/// Whether two platform identifiers name the same platform.
///
/// Profiles and manifests can spell a platform differently (`ps1` vs
/// `playstation`), so fall back to parsing both before deciding they differ.
fn same_platform(left: &str, right: &str) -> bool {
    if left.eq_ignore_ascii_case(right) {
        return true;
    }
    match (
        left.parse::<retro_junk_core::Platform>(),
        right.parse::<retro_junk_core::Platform>(),
    ) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

/// Find a platform's existing default, preferring an exact identifier match
/// over an equivalent spelling.
#[must_use]
pub fn existing_default<'a>(
    defaults: &'a [retro_junk_archive::PlatformPlayableDefault],
    platform_id: &str,
) -> Option<&'a retro_junk_archive::PlatformPlayableDefault> {
    defaults
        .iter()
        .find(|default| default.platform_id.eq_ignore_ascii_case(platform_id))
        .or_else(|| {
            defaults
                .iter()
                .find(|default| same_platform(&default.platform_id, platform_id))
        })
}

/// Set (or clear, with `None`) the preferred playable format for one platform,
/// then update the projection to match the manifest that was just written.
///
/// Everything about the existing policy other than the format is preserved, so
/// choosing a new format does not silently reset options the user set earlier.
pub fn set_preferred_format(
    profile: &CollectionProfile,
    db_path: &Path,
    platform_id: &str,
    format: Option<RepresentationFormat>,
) -> Result<ArchiveRootManifest, String> {
    let policy = format.map(|format| {
        let mut policy = existing_default(&profile.platform_defaults, platform_id).map_or(
            DesiredPlayablePolicy {
                format: format.clone(),
                retain_canonical_intermediate: false,
                allow_unverified: false,
                options: std::collections::BTreeMap::new(),
            },
            |default| default.policy.clone(),
        );
        policy.format = format;
        policy
    });
    let manifest = retro_junk_archive::set_platform_playable_default(
        &profile.archive_root,
        platform_id,
        policy,
    )
    .map_err(|error| error.to_string())?;
    let projected_policy =
        existing_default(&manifest.platform_defaults, platform_id).map(|default| &default.policy);
    // The projection records which manifest it was built from, so hash the
    // file we just wrote rather than trusting an earlier reading of it.
    let (_, manifest_sha256) = retro_junk_archive::sha256_file(
        &retro_junk_archive::root_manifest_path(&profile.archive_root),
        &std::sync::atomic::AtomicBool::new(false),
    )
    .map_err(|error| error.to_string())?;
    let mut connection = crate::queries::open_catalog(db_path)?;
    retro_junk_db::update_projected_platform_policy(
        &mut connection,
        &profile.profile_id.to_string(),
        platform_id,
        projected_policy,
        &manifest_sha256,
    )
    .map_err(|error| error.to_string())?;
    Ok(manifest)
}
