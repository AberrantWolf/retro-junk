//! CUE sheet compatibility fixing: detect CDRWin-format CUE files and convert
//! them to standard CUE format for wider emulator compatibility.

use std::fs;
use std::path::{Path, PathBuf};

use retro_junk_disc::cue::{CueCompatReport, check_cue_compat, convert_cue_to_standard};

/// Options for CUE fix operations.
pub struct CueFixOptions {
    /// Whether to create `.cue.bak` backup files before overwriting.
    pub create_backup: bool,
}

/// A planned fix for a single CUE file.
pub struct CueFixAction {
    /// Path to the CUE file to fix.
    pub cue_path: PathBuf,
    /// Compatibility report describing what was detected.
    pub report: CueCompatReport,
    /// The converted standard CUE content to write.
    pub converted: String,
}

/// Result of planning CUE fixes for a directory.
pub struct CueFixPlan {
    /// CUE files that can be converted to standard format.
    pub fixable: Vec<CueFixAction>,
    /// CUE files already in standard format.
    pub already_standard: Vec<PathBuf>,
    /// CUE files that cannot be auto-converted, with the reason.
    pub unfixable: Vec<(PathBuf, String)>,
    /// CUE files that encountered errors during analysis.
    pub errors: Vec<(PathBuf, String)>,
}

impl CueFixPlan {
    /// Whether this plan has any fix actions to perform.
    #[must_use]
    pub fn has_fixes(&self) -> bool {
        !self.fixable.is_empty()
    }
}

/// Progress updates during CUE fix planning.
pub enum CueFixProgress {
    /// Found N CUE files to check.
    Scanning { file_count: usize },
    /// Checking a specific CUE file.
    Checking {
        file_name: String,
        file_index: usize,
        total: usize,
    },
    /// Planning complete.
    Done,
}

/// Summary after executing CUE fixes.
pub struct CueFixSummary {
    /// Number of CUE files successfully fixed.
    pub fixed: usize,
    /// Number of backup files created.
    pub backups_created: usize,
    /// Errors encountered during execution.
    pub errors: Vec<String>,
}

/// Scan a directory for CUE files and plan compatibility fixes.
pub fn plan_cue_fixes(folder: &Path, progress: &dyn Fn(CueFixProgress)) -> CueFixPlan {
    let mut plan = CueFixPlan {
        fixable: Vec::new(),
        already_standard: Vec::new(),
        unfixable: Vec::new(),
        errors: Vec::new(),
    };

    // Collect all .cue files in the directory (non-recursive — CUE files
    // should be at the top level alongside their BIN files)
    let cue_files: Vec<PathBuf> = match fs::read_dir(folder) {
        Ok(entries) => entries
            .filter_map(std::result::Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("cue"))
            })
            .collect(),
        Err(e) => {
            plan.errors
                .push((folder.to_path_buf(), format!("Cannot read directory: {e}")));
            return plan;
        }
    };

    progress(CueFixProgress::Scanning {
        file_count: cue_files.len(),
    });

    for (i, cue_path) in cue_files.iter().enumerate() {
        let file_name = cue_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();

        progress(CueFixProgress::Checking {
            file_name: file_name.clone(),
            file_index: i,
            total: cue_files.len(),
        });

        // Read CUE content
        let content = match fs::read_to_string(cue_path) {
            Ok(c) => c,
            Err(e) => {
                plan.errors
                    .push((cue_path.clone(), format!("Cannot read file: {e}")));
                continue;
            }
        };

        // Check compatibility
        let report = check_cue_compat(&content);

        if report.is_standard() {
            plan.already_standard.push(cue_path.clone());
            continue;
        }

        if let Some(reason) = report.blocked_reason() {
            plan.unfixable.push((cue_path.clone(), reason.to_string()));
            continue;
        }

        // Try to convert
        let cue_dir = cue_path.parent().unwrap_or(folder);
        match convert_cue_to_standard(&content, cue_dir) {
            Ok(converted) => {
                plan.fixable.push(CueFixAction {
                    cue_path: cue_path.clone(),
                    report,
                    converted,
                });
            }
            Err(e) => {
                plan.errors
                    .push((cue_path.clone(), format!("Conversion failed: {e}")));
            }
        }
    }

    progress(CueFixProgress::Done);
    plan
}

/// Execute the planned CUE fixes, writing converted files to disk.
#[must_use]
pub fn execute_cue_fixes(plan: &CueFixPlan, create_backup: bool) -> CueFixSummary {
    let mut summary = CueFixSummary {
        fixed: 0,
        backups_created: 0,
        errors: Vec::new(),
    };

    for action in &plan.fixable {
        // Create backup if requested
        if create_backup {
            let backup_path = action.cue_path.with_extension("cue.bak");
            if let Err(e) = fs::copy(&action.cue_path, &backup_path) {
                summary.errors.push(format!(
                    "{}: backup failed: {e}",
                    action
                        .cue_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("?")
                ));
                continue;
            }
            summary.backups_created += 1;
        }

        // Write converted content
        if let Err(e) = fs::write(&action.cue_path, &action.converted) {
            summary.errors.push(format!(
                "{}: write failed: {e}",
                action
                    .cue_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?")
            ));
            continue;
        }

        summary.fixed += 1;
    }

    summary
}
