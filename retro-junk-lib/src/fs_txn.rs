//! Journaled filesystem transactions with preflight checks and rollback.
//!
//! A [`FsTransaction`] collects rename and file-write operations, validates
//! them all up front (sources exist, no target collisions), then executes
//! them while journaling each completed step. If any step fails, completed
//! steps are undone in reverse order, restoring the original state.
//!
//! Execution order: all renames run first (two-phase via temporary names
//! when a rename target is itself another rename's source, e.g. swapped
//! track files), then all file writes in insertion order. File writes
//! capture the original content for rollback.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// A single filesystem operation in a transaction.
#[derive(Debug, Clone)]
pub enum FsOp {
    /// Rename a file (or directory) to a new path.
    Rename { source: PathBuf, target: PathBuf },
    /// Overwrite (or create) a text file with new content.
    WriteFile { path: PathBuf, content: String },
}

/// Error from a failed transaction, including rollback status.
#[derive(Debug)]
pub struct TxnError {
    /// What went wrong (preflight violation or the failed operation).
    pub message: String,
    /// Errors encountered while rolling back. Empty means the rollback
    /// fully restored the original state.
    pub rollback_errors: Vec<String>,
}

impl TxnError {
    fn preflight(message: String) -> Self {
        Self {
            message,
            rollback_errors: Vec::new(),
        }
    }
}

impl std::fmt::Display for TxnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)?;
        if self.rollback_errors.is_empty() {
            write!(f, " (all changes rolled back)")
        } else {
            write!(
                f,
                " (ROLLBACK INCOMPLETE: {})",
                self.rollback_errors.join("; ")
            )
        }
    }
}

impl std::error::Error for TxnError {}

/// Journal entry describing how to undo one completed step.
enum UndoStep {
    /// A rename was performed from `from` to `to`; undo renames `to` back to `from`.
    Rename { from: PathBuf, to: PathBuf },
    /// A file was written at `path`; undo restores `original` (or deletes
    /// the file if it didn't exist before).
    Write {
        path: PathBuf,
        original: Option<String>,
    },
}

/// Summary of a successfully committed transaction.
#[derive(Debug, Default, Clone)]
pub struct TxnSummary {
    /// Number of rename operations performed (no-op renames excluded).
    pub renames: usize,
    /// Number of files written.
    pub writes: usize,
}

/// A set of filesystem operations that succeed or fail as a unit.
#[derive(Debug, Default)]
pub struct FsTransaction {
    ops: Vec<FsOp>,
}

impl FsTransaction {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue a rename. Renames where source equals target are skipped.
    pub fn rename(&mut self, source: impl Into<PathBuf>, target: impl Into<PathBuf>) {
        let source = source.into();
        let target = target.into();
        if source != target {
            self.ops.push(FsOp::Rename { source, target });
        }
    }

    /// Queue a file write (create or overwrite).
    pub fn write_file(&mut self, path: impl Into<PathBuf>, content: impl Into<String>) {
        self.ops.push(FsOp::WriteFile {
            path: path.into(),
            content: content.into(),
        });
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    pub fn ops(&self) -> &[FsOp] {
        &self.ops
    }

    /// Validate all operations without touching the filesystem.
    ///
    /// Checks:
    /// - every rename source exists
    /// - no two renames share a target
    /// - no rename target already exists on disk, unless it is itself the
    ///   source of another rename in this transaction (chain/swap, handled
    ///   via two-phase temp renames)
    /// - every write's parent directory exists
    pub fn preflight(&self) -> Result<(), TxnError> {
        let sources: HashSet<&Path> = self
            .ops
            .iter()
            .filter_map(|op| match op {
                FsOp::Rename { source, .. } => Some(source.as_path()),
                _ => None,
            })
            .collect();

        let mut targets: HashSet<&Path> = HashSet::new();
        for op in &self.ops {
            match op {
                FsOp::Rename { source, target } => {
                    if !source.exists() {
                        return Err(TxnError::preflight(format!(
                            "Rename source does not exist: {}",
                            source.display()
                        )));
                    }
                    if !targets.insert(target.as_path()) {
                        return Err(TxnError::preflight(format!(
                            "Multiple operations target: {}",
                            target.display()
                        )));
                    }
                    if target.exists() && !sources.contains(target.as_path()) {
                        return Err(TxnError::preflight(format!(
                            "Target already exists: {}",
                            target.display()
                        )));
                    }
                }
                FsOp::WriteFile { path, .. } => {
                    if let Some(parent) = path.parent()
                        && !parent.as_os_str().is_empty()
                        && !parent.is_dir()
                    {
                        return Err(TxnError::preflight(format!(
                            "Parent directory does not exist for write: {}",
                            path.display()
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    /// Preflight, then execute all operations. On any failure, undo every
    /// completed step in reverse order and return the error.
    pub fn commit(self) -> Result<TxnSummary, TxnError> {
        self.preflight()?;

        let (renames, writes): (Vec<_>, Vec<_>) = self
            .ops
            .into_iter()
            .partition(|op| matches!(op, FsOp::Rename { .. }));

        // Two-phase is needed when any rename target is another rename's source
        // (e.g., two track files whose canonical names are swapped).
        let sources: HashSet<PathBuf> = renames
            .iter()
            .filter_map(|op| match op {
                FsOp::Rename { source, .. } => Some(source.clone()),
                _ => None,
            })
            .collect();
        let two_phase = renames.iter().any(|op| match op {
            FsOp::Rename { target, .. } => sources.contains(target),
            _ => false,
        });

        let mut journal: Vec<UndoStep> = Vec::new();
        let mut summary = TxnSummary::default();

        let result = (|| -> Result<(), String> {
            if two_phase {
                // Phase 1: move every source aside to a unique temp name.
                let mut temps: Vec<(PathBuf, PathBuf)> = Vec::new(); // (temp, final target)
                for (i, op) in renames.iter().enumerate() {
                    let FsOp::Rename { source, target } = op else {
                        continue;
                    };
                    let temp = temp_path(target, i);
                    do_rename(source, &temp, &mut journal)?;
                    temps.push((temp, target.clone()));
                }
                // Phase 2: move temps to their final targets.
                for (temp, target) in temps {
                    do_rename(&temp, &target, &mut journal)?;
                    summary.renames += 1;
                }
            } else {
                for op in &renames {
                    let FsOp::Rename { source, target } = op else {
                        continue;
                    };
                    do_rename(source, target, &mut journal)?;
                    summary.renames += 1;
                }
            }

            for op in &writes {
                let FsOp::WriteFile { path, content } = op else {
                    continue;
                };
                let original = match fs::read_to_string(path) {
                    Ok(c) => Some(c),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                    Err(e) => {
                        return Err(format!(
                            "Failed to read original content of {}: {}",
                            path.display(),
                            e
                        ));
                    }
                };
                fs::write(path, content)
                    .map_err(|e| format!("Failed to write {}: {}", path.display(), e))?;
                journal.push(UndoStep::Write {
                    path: path.clone(),
                    original,
                });
                summary.writes += 1;
            }

            Ok(())
        })();

        match result {
            Ok(()) => Ok(summary),
            Err(message) => Err(TxnError {
                message,
                rollback_errors: rollback(journal),
            }),
        }
    }
}

/// Perform one rename and journal it.
fn do_rename(source: &Path, target: &Path, journal: &mut Vec<UndoStep>) -> Result<(), String> {
    fs::rename(source, target).map_err(|e| {
        format!(
            "Failed to rename {} -> {}: {}",
            source.display(),
            target.display(),
            e
        )
    })?;
    journal.push(UndoStep::Rename {
        from: source.to_path_buf(),
        to: target.to_path_buf(),
    });
    Ok(())
}

/// Undo completed steps in reverse order. Returns any errors encountered.
fn rollback(journal: Vec<UndoStep>) -> Vec<String> {
    let mut errors = Vec::new();
    for step in journal.into_iter().rev() {
        match step {
            UndoStep::Rename { from, to } => {
                if let Err(e) = fs::rename(&to, &from) {
                    errors.push(format!(
                        "Failed to restore {} -> {}: {}",
                        to.display(),
                        from.display(),
                        e
                    ));
                }
            }
            UndoStep::Write { path, original } => {
                let result = match original {
                    Some(content) => fs::write(&path, content),
                    None => fs::remove_file(&path),
                };
                if let Err(e) = result {
                    errors.push(format!("Failed to restore {}: {}", path.display(), e));
                }
            }
        }
    }
    errors
}

/// Build a temp path next to `target`, unique within this transaction.
fn temp_path(target: &Path, index: usize) -> PathBuf {
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    target.with_file_name(format!(".{name}.rjtxn{index}"))
}

#[cfg(test)]
#[path = "tests/fs_txn_tests.rs"]
mod tests;
