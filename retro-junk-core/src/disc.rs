//! Disc-related filename utilities.
//!
//! Functions for parsing "(Disc N)" tags from game filenames and grouping
//! multi-disc entries. Used by both the rename and scraper systems.

use std::collections::HashMap;

/// Remove " (Disc N)" from a game name, preserving other parenthesized tags.
///
/// Examples:
/// - `"Final Fantasy VII (Disc 1) (USA)"` → `"Final Fantasy VII (USA)"`
/// - `"Crash Bandicoot (USA)"` → `"Crash Bandicoot (USA)"` (unchanged)
pub fn strip_disc_tag(name: &str) -> String {
    const PREFIX: &str = " (Disc ";
    if let Some(start) = name.find(PREFIX) {
        let after = &name[start + PREFIX.len()..];
        // Find closing ')' after the digits
        if let Some(close) = after.find(')') {
            let digits = &after[..close];
            if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
                let mut result = String::with_capacity(name.len());
                result.push_str(&name[..start]);
                result.push_str(&after[close + 1..]);
                return result;
            }
        }
    }
    name.to_string()
}

/// Extract disc number from a filename or game name for sorting.
///
/// Examples:
/// - `"Final Fantasy VII (Disc 2) (USA).chd"` → `Some(2)`
/// - `"Crash Bandicoot (USA).chd"` → `None`
pub fn extract_disc_number(name: &str) -> Option<u32> {
    const PREFIX: &str = "(Disc ";
    let start = name.find(PREFIX)?;
    let after = &name[start + PREFIX.len()..];
    let close = after.find(')')?;
    after[..close].parse().ok()
}

/// Info about a group of entries belonging to the same multi-disc game.
#[derive(Debug, Clone)]
pub struct DiscGroup {
    /// Base game name with disc tag stripped (e.g., "Final Fantasy VII (USA)")
    pub base_name: String,
    /// Index of the primary disc (lowest disc number) in the input list
    pub primary_index: usize,
    /// All indices in this group (including primary), sorted by disc number
    pub member_indices: Vec<usize>,
}

/// Given a list of (index, filename_stem) pairs, detect disc groups.
///
/// Only entries containing "(Disc N)" are considered. Groups with a single
/// entry are excluded (a lone "Disc 1" with no other discs isn't a group).
pub fn detect_disc_groups(entries: &[(usize, &str)]) -> Vec<DiscGroup> {
    // Group by base name (disc tag stripped)
    let mut groups: HashMap<String, Vec<(usize, u32)>> = HashMap::new();

    for &(index, stem) in entries {
        if let Some(disc_num) = extract_disc_number(stem) {
            let base = strip_disc_tag(stem);
            groups.entry(base).or_default().push((index, disc_num));
        }
    }

    let mut result: Vec<DiscGroup> = groups
        .into_iter()
        .filter(|(_, members)| members.len() >= 2)
        .map(|(base_name, mut members)| {
            members.sort_by_key(|&(_, disc_num)| disc_num);
            let primary_index = members[0].0;
            let member_indices = members.iter().map(|&(idx, _)| idx).collect();
            DiscGroup {
                base_name,
                primary_index,
                member_indices,
            }
        })
        .collect();

    // Sort groups by primary index for deterministic output
    result.sort_by_key(|g| g.primary_index);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_disc_tag_removes_disc_number() {
        assert_eq!(
            strip_disc_tag("Final Fantasy VII (Disc 1) (USA)"),
            "Final Fantasy VII (USA)"
        );
    }

    #[test]
    fn strip_disc_tag_preserves_non_disc_names() {
        assert_eq!(
            strip_disc_tag("Crash Bandicoot (USA)"),
            "Crash Bandicoot (USA)"
        );
    }

    #[test]
    fn strip_disc_tag_multi_digit() {
        assert_eq!(
            strip_disc_tag("Some Game (Disc 12) (USA)"),
            "Some Game (USA)"
        );
    }

    #[test]
    fn strip_disc_tag_disc_at_end() {
        assert_eq!(strip_disc_tag("Some Game (Disc 1)"), "Some Game");
    }

    #[test]
    fn extract_disc_number_found() {
        assert_eq!(
            extract_disc_number("Final Fantasy VII (Disc 2) (USA).chd"),
            Some(2)
        );
    }

    #[test]
    fn extract_disc_number_not_found() {
        assert_eq!(extract_disc_number("Crash Bandicoot (USA).chd"), None);
    }

    #[test]
    fn extract_disc_number_from_stem() {
        assert_eq!(
            extract_disc_number("Metal Gear Solid (Disc 1) (USA)"),
            Some(1)
        );
    }

    #[test]
    fn detect_disc_groups_basic() {
        let entries = vec![
            (0, "Final Fantasy VII (USA) (Disc 1)"),
            (1, "Final Fantasy VII (USA) (Disc 2)"),
            (2, "Final Fantasy VII (USA) (Disc 3)"),
            (3, "Crash Bandicoot (USA)"),
        ];
        let groups = detect_disc_groups(&entries);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].base_name, "Final Fantasy VII (USA)");
        assert_eq!(groups[0].primary_index, 0);
        assert_eq!(groups[0].member_indices, vec![0, 1, 2]);
    }

    #[test]
    fn detect_disc_groups_ignores_single_disc() {
        let entries = vec![(0, "Some Game (Disc 1)"), (1, "Another Game (USA)")];
        let groups = detect_disc_groups(&entries);
        assert!(groups.is_empty());
    }

    #[test]
    fn detect_disc_groups_multiple_games() {
        let entries = vec![
            (0, "FF7 (USA) (Disc 1)"),
            (1, "FF7 (USA) (Disc 2)"),
            (2, "MGS (USA) (Disc 1)"),
            (3, "MGS (USA) (Disc 2)"),
            (4, "Crash (USA)"),
        ];
        let groups = detect_disc_groups(&entries);
        assert_eq!(groups.len(), 2);

        let ff7 = groups.iter().find(|g| g.base_name == "FF7 (USA)").unwrap();
        assert_eq!(ff7.primary_index, 0);
        assert_eq!(ff7.member_indices, vec![0, 1]);

        let mgs = groups.iter().find(|g| g.base_name == "MGS (USA)").unwrap();
        assert_eq!(mgs.primary_index, 2);
        assert_eq!(mgs.member_indices, vec![2, 3]);
    }

    #[test]
    fn detect_disc_groups_out_of_order_discs() {
        let entries = vec![
            (0, "Game (USA) (Disc 3)"),
            (1, "Game (USA) (Disc 1)"),
            (2, "Game (USA) (Disc 2)"),
        ];
        let groups = detect_disc_groups(&entries);
        assert_eq!(groups.len(), 1);
        // Primary should be the entry with disc 1 (index 1)
        assert_eq!(groups[0].primary_index, 1);
        // Members sorted by disc number: disc 1 (idx 1), disc 2 (idx 2), disc 3 (idx 0)
        assert_eq!(groups[0].member_indices, vec![1, 2, 0]);
    }
}
