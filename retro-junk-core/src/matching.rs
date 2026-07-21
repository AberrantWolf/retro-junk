//! Conservative, shared policy for comparing ROM-header identity fields with
//! catalog metadata.

use crate::Region;

/// Compare a version extracted from a ROM header with a DAT release revision.
///
/// Returns `None` when the two formats cannot be compared safely. Numeric
/// cartridge revisions use two common conventions: headers expose `v1` or
/// `1.1`, while No-Intro names the same revision `Rev 1`. An empty DAT
/// revision represents the original (`0`) release.
#[must_use]
pub fn header_version_matches_revision(
    header_version: &str,
    catalog_revision: &str,
) -> Option<bool> {
    let header = normalize_version(header_version);
    if header.is_empty() {
        return None;
    }

    let catalog = normalize_version(catalog_revision);
    if !catalog.is_empty() && header == catalog {
        return Some(true);
    }

    if !catalog.is_empty()
        && let (Some(header_parts), Some(catalog_parts)) = (
            numeric_version_parts(&header),
            numeric_version_parts(&catalog),
        )
    {
        return Some(header_parts == catalog_parts);
    }

    let header_ordinal = header_revision_ordinal(&header)?;
    let catalog_ordinal = catalog_revision_ordinal(&catalog)?;
    Some(header_ordinal == catalog_ordinal)
}

/// Determine whether decoded header regions are compatible with a catalog
/// region. Empty/unknown evidence is deliberately non-disqualifying.
#[must_use]
pub fn header_regions_match_catalog(detected: &[Region], catalog_region: &str) -> bool {
    let detected: Vec<_> = detected
        .iter()
        .copied()
        .filter(|region| *region != Region::Unknown)
        .collect();
    if detected.is_empty() || catalog_region.trim().is_empty() {
        return true;
    }

    let catalog = normalize_region(catalog_region);
    if catalog == "world" {
        return true;
    }
    detected.iter().any(|region| {
        normalize_region(region.name()) == catalog || normalize_region(region.code()) == catalog
    })
}

/// Rank a set of already-identified serial/hash candidates using every
/// trustworthy header field. A field only narrows the set when at least one
/// candidate agrees, so incomplete catalog metadata cannot erase an otherwise
/// useful identity match.
#[must_use]
pub fn header_candidate_indices<T>(
    candidates: &[T],
    header_version: &str,
    detected_regions: &[Region],
    detected_size: Option<u64>,
    revision: impl for<'a> Fn(&'a T) -> &'a str,
    region: impl for<'a> Fn(&'a T) -> &'a str,
    size: impl Fn(&T) -> u64,
) -> Vec<usize> {
    let mut indices: Vec<_> = (0..candidates.len()).collect();
    retain_if_any(&mut indices, |index| {
        header_version_matches_revision(header_version, revision(&candidates[*index])) == Some(true)
    });
    retain_if_any(&mut indices, |index| {
        header_regions_match_catalog(detected_regions, region(&candidates[*index]))
    });
    if let Some(detected_size) = detected_size.filter(|size| *size > 0) {
        retain_if_any(&mut indices, |index| {
            size(&candidates[*index]) == detected_size
        });
    }
    indices
}

fn retain_if_any<T>(candidates: &mut Vec<T>, predicate: impl Fn(&T) -> bool) {
    if candidates.iter().any(&predicate) {
        candidates.retain(predicate);
    }
}

fn normalize_region(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect()
}

fn numeric_version_parts(version: &str) -> Option<Vec<u32>> {
    version
        .strip_prefix('v')
        .unwrap_or(version)
        .split('.')
        .map(str::parse)
        .collect::<Result<Vec<_>, _>>()
        .ok()
}

fn normalize_version(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .collect()
}

fn header_revision_ordinal(version: &str) -> Option<u32> {
    let numeric = version.strip_prefix('v').unwrap_or(version);
    if let Ok(value) = numeric.parse() {
        return Some(value);
    }

    let mut components = numeric.split('.');
    let major: u32 = components.next()?.parse().ok()?;
    let minor: u32 = components.next()?.parse().ok()?;
    if components.next().is_none() && major == 1 {
        return Some(minor);
    }
    None
}

fn catalog_revision_ordinal(revision: &str) -> Option<u32> {
    if revision.is_empty() {
        return Some(0);
    }
    revision.strip_prefix("rev")?.parse().ok()
}

#[cfg(test)]
mod tests {
    use crate::Region;

    use super::{
        header_candidate_indices, header_regions_match_catalog, header_version_matches_revision,
    };

    struct Candidate {
        revision: &'static str,
        region: &'static str,
        size: u64,
    }

    #[test]
    fn numeric_header_revisions_match_no_intro_names() {
        assert_eq!(header_version_matches_revision("v0", ""), Some(true));
        assert_eq!(header_version_matches_revision("v1", "Rev 1"), Some(true));
        assert_eq!(header_version_matches_revision("v1.2", "Rev 2"), Some(true));
        assert_eq!(header_version_matches_revision("1.03", "Rev 3"), Some(true));
        assert_eq!(header_version_matches_revision("v1", ""), Some(false));
    }

    #[test]
    fn exact_versions_match_without_guessing_semantic_versions() {
        assert_eq!(
            header_version_matches_revision("V1.006", "v1.006"),
            Some(true)
        );
        assert_eq!(
            header_version_matches_revision("V1.006", "1.006"),
            Some(true)
        );
        assert_eq!(
            header_version_matches_revision("V1.006", "1.002"),
            Some(false)
        );
        assert_eq!(header_version_matches_revision("v1.1.0", "Rev 1"), None);
        assert_eq!(header_version_matches_revision("v1", "Rev A"), None);
    }

    #[test]
    fn region_slugs_match_header_regions_case_insensitively() {
        assert!(header_regions_match_catalog(&[Region::Usa], "usa"));
        assert!(header_regions_match_catalog(&[Region::Japan], "Japan"));
        assert!(!header_regions_match_catalog(&[Region::Europe], "usa"));
        assert!(header_regions_match_catalog(&[Region::Europe], "world"));
    }

    #[test]
    fn one_policy_ranks_revision_then_region_then_size() {
        let candidates = [
            Candidate {
                revision: "",
                region: "usa",
                size: 64,
            },
            Candidate {
                revision: "Rev 1",
                region: "usa",
                size: 32,
            },
            Candidate {
                revision: "Rev 1",
                region: "japan",
                size: 64,
            },
        ];
        assert_eq!(
            header_candidate_indices(
                &candidates,
                "v1",
                &[Region::Usa],
                Some(32),
                |candidate| candidate.revision,
                |candidate| candidate.region,
                |candidate| candidate.size,
            ),
            vec![1]
        );
    }

    #[test]
    fn missing_catalog_evidence_does_not_destroy_a_candidate_set() {
        let candidates = [Candidate {
            revision: "Rev A",
            region: "",
            size: 0,
        }];
        assert_eq!(
            header_candidate_indices(
                &candidates,
                "v1",
                &[Region::Usa],
                Some(32),
                |candidate| candidate.revision,
                |candidate| candidate.region,
                |candidate| candidate.size,
            ),
            vec![0]
        );
    }
}
