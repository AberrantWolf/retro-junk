//! CUE sheet repair: check `CDRWin`-style sheets for compatibility issues
//! and rewrite them in standard format, backing up the original.

use std::path::Path;

use retro_junk_disc::cue::{check_cue_compat, convert_cue_to_standard};
use retro_junk_io::ProgressUnit;

use super::OpCtx;

pub struct CueFixResult {
    pub file_name: String,
    pub outcome: CueFixOutcome,
}

pub enum CueFixOutcome {
    Fixed { summary: String },
    AlreadyStandard,
    Unfixable { reason: String },
    Error { message: String },
}

/// Fix each CUE file in `cue_files` (entry display name, cue path) in turn.
/// Stops early when cancelled; the results collected so far are returned.
pub fn fix_cues(cue_files: &[(String, std::path::PathBuf)], ctx: &OpCtx) -> Vec<CueFixResult> {
    let total = cue_files.len() as u64;
    let mut results: Vec<CueFixResult> = Vec::new();
    for (i, (entry_name, cue_path)) in cue_files.iter().enumerate() {
        if ctx.cancelled() {
            break;
        }
        (ctx.progress)("Fixing CUE files", ProgressUnit::Items, i as u64, total);
        let file_name = cue_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(entry_name)
            .to_string();
        results.push(CueFixResult {
            file_name,
            outcome: fix_one_cue(cue_path),
        });
    }
    results
}

/// Check one CUE file and, when fixable, back it up and rewrite it in
/// standard format. Returns the outcome for the results dialog.
fn fix_one_cue(cue_path: &Path) -> CueFixOutcome {
    let content = match std::fs::read_to_string(cue_path) {
        Ok(c) => c,
        Err(e) => {
            return CueFixOutcome::Error {
                message: format!("Cannot read: {e}"),
            };
        }
    };

    let report = check_cue_compat(&content);

    if report.is_standard() {
        return CueFixOutcome::AlreadyStandard;
    }

    if let Some(reason) = report.blocked_reason() {
        return CueFixOutcome::Unfixable {
            reason: reason.to_string(),
        };
    }

    let summary = report.summary();
    let cue_dir = cue_path.parent().unwrap_or(cue_path);

    let converted = match convert_cue_to_standard(&content, cue_dir) {
        Ok(c) => c,
        Err(e) => {
            return CueFixOutcome::Error {
                message: format!("Conversion failed: {e}"),
            };
        }
    };

    // Back up the original before rewriting it in place.
    let backup_path = cue_path.with_extension("cue.bak");
    if let Err(e) = std::fs::copy(cue_path, &backup_path) {
        return CueFixOutcome::Error {
            message: format!("Backup failed: {e}"),
        };
    }

    match std::fs::write(cue_path, &converted) {
        Ok(()) => CueFixOutcome::Fixed { summary },
        Err(e) => CueFixOutcome::Error {
            message: format!("Write failed: {e}"),
        },
    }
}
