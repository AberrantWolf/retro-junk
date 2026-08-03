//! Archive release overviews: identity, completion, and display fields for
//! lists and detail views.

use retro_junk_db::Connection;
use retro_junk_db::facts::ReleaseFacts;
use retro_junk_frontend::AssetSelection;

use crate::completion::Completion;

/// One archive release as every list, detail panel, and status line sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseOverview {
    pub archive_release_id: String,
    pub platform_id: String,
    pub title: String,
    pub region: String,
    pub revision: String,
    pub completion: Completion,
    /// The raw facts behind the completion, for views that show counts
    /// (carriers, copies) rather than status.
    pub facts: ReleaseFacts,
}

/// Every release in a profile, with completion computed by the one fold.
pub fn release_overviews(
    conn: &Connection,
    profile_id: &str,
    expected_assets: &AssetSelection,
) -> Result<Vec<ReleaseOverview>, String> {
    let all_facts = retro_junk_db::facts::release_completion_facts(conn, profile_id)
        .map_err(|error| error.to_string())?;
    Ok(all_facts
        .into_iter()
        .map(|facts| ReleaseOverview {
            archive_release_id: facts.archive_release_id.clone(),
            platform_id: facts.platform_id.clone(),
            title: facts.title.clone(),
            region: facts.region.clone(),
            revision: facts.revision.clone(),
            completion: Completion::for_release(&facts, expected_assets),
            facts,
        })
        .collect())
}
