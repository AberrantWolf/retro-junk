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

/// Derive the base game name from a collection of DAT game names for a multi-disc set.
///
/// - 0 names → `""`
/// - 1 name → `strip_disc_tag()` (handles "(Disc N)"; no-ops on others)
/// - 2+ names → if `strip_disc_tag` changes the first name, use that (fast path for
///   numbered discs). Otherwise, compute the longest common prefix trimmed to a clean
///   parenthesized-group boundary (handles scenario-named discs like "Leon Hen"/"Claire Hen").
pub fn derive_base_game_name(names: &[&str]) -> String {
    match names.len() {
        0 => String::new(),
        1 => strip_disc_tag(names[0]),
        _ => {
            let stripped = strip_disc_tag(names[0]);
            if stripped != names[0] {
                // Fast path: numbered discs — strip_disc_tag handled it
                stripped
            } else {
                // Scenario discs: find the longest common prefix across all names
                let prefix = longest_common_prefix(names);
                trim_to_paren_boundary(&prefix)
            }
        }
    }
}

/// Compute the longest common prefix of a slice of strings.
fn longest_common_prefix(strings: &[&str]) -> String {
    if strings.is_empty() {
        return String::new();
    }
    let first = strings[0].as_bytes();
    let mut len = first.len();
    for s in &strings[1..] {
        len = len.min(s.len());
        for (i, &b) in first[..len].iter().enumerate() {
            if s.as_bytes()[i] != b {
                len = i;
                break;
            }
        }
    }
    strings[0][..len].to_string()
}

/// Trim a string to the last complete parenthesized group boundary.
///
/// Strips any trailing incomplete `" (..."` fragment and trailing whitespace.
/// - `"RE2 (JP) ("` → `"RE2 (JP)"`
/// - `"RE2 (JP) (Cla"` → `"RE2 (JP)"`
/// - `"RE2 (JP)"` → `"RE2 (JP)"` (already clean)
fn trim_to_paren_boundary(s: &str) -> String {
    let trimmed = s.trim_end();
    // Count open/close parens to check if balanced
    let open = trimmed.chars().filter(|&c| c == '(').count();
    let close = trimmed.chars().filter(|&c| c == ')').count();
    if open == close {
        // Already balanced — return as-is (trimmed)
        return trimmed.to_string();
    }
    // There's an incomplete trailing group — find the last " (" that starts it
    if let Some(pos) = trimmed.rfind(" (") {
        trimmed[..pos].trim_end().to_string()
    } else {
        trimmed.trim_end().to_string()
    }
}

#[cfg(test)]
#[path = "tests/disc_tests.rs"]
mod tests;
