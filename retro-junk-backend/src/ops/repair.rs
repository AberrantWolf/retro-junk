//! Byte-level ROM repair: restore a file whose bytes drifted from what the
//! catalog says they should be.
//!
//! Some dumps differ from the catalog only in padding — trailing bytes added
//! or trimmed by whatever tool wrote them. The content is intact, so the file
//! plays, but it will never match a DAT and so can never be verified. Repair
//! restores the exact expected bytes and confirms the result by hash.
//!
//! Planning and execution are separate on purpose: a repair rewrites files in
//! place, so the caller shows the plan and gets an answer before anything is
//! touched.

use std::path::{Path, PathBuf};

use retro_junk_io::ProgressUnit;
use retro_junk_lib::AnalysisContext;
use retro_junk_lib::repair::{RepairOptions, RepairPlan, RepairProgress, RepairSummary};

use super::OpCtx;

/// One console folder's repair plan, with the folder it belongs to.
pub struct ConsoleRepairPlan {
    pub folder_name: String,
    pub plan: RepairPlan,
}

/// What a planning pass found across every requested console.
#[derive(Default)]
pub struct RepairPlanReport {
    pub consoles: Vec<ConsoleRepairPlan>,
    /// Consoles that could not be planned, with the reason — usually "this
    /// platform has no DAT support", which is a fact about the platform
    /// rather than a failure of the run.
    pub skipped: Vec<String>,
}

impl RepairPlanReport {
    /// Files across every console that repair would rewrite.
    #[must_use]
    pub fn repairable_count(&self) -> usize {
        self.consoles
            .iter()
            .map(|console| console.plan.repairable.len())
            .sum()
    }
}

/// Plan repairs for the given console folders without changing anything.
#[must_use]
pub fn plan(
    context: &AnalysisContext,
    root: &Path,
    consoles: &[(String, String)],
    options: &RepairOptions,
    ctx: &OpCtx,
) -> RepairPlanReport {
    let mut report = RepairPlanReport::default();
    let total = consoles.len() as u64;
    for (index, (folder_name, platform_short_name)) in consoles.iter().enumerate() {
        if ctx.cancelled() {
            break;
        }
        (ctx.progress)(
            &format!("Planning repairs for {folder_name}"),
            ProgressUnit::Items,
            index as u64,
            total,
        );
        let Some(console) = context.get_by_short_name(platform_short_name) else {
            report
                .skipped
                .push(format!("{folder_name}: no analyzer for this platform"));
            continue;
        };
        let folder = root.join(folder_name);
        // Planning reads and hashes every ROM in the folder, so its own
        // per-file progress is forwarded rather than leaving the caller
        // watching one unchanging console counter.
        let per_file = |progress: RepairProgress| match progress {
            RepairProgress::Scanning { file_count } => (ctx.progress)(
                &format!("Scanning {folder_name} ({file_count} files)"),
                ProgressUnit::Items,
                0,
                file_count as u64,
            ),
            RepairProgress::Checking {
                ref file_name,
                file_index,
                total,
            } => (ctx.progress)(
                &format!("Checking {file_name}"),
                ProgressUnit::Items,
                file_index as u64 + 1,
                total as u64,
            ),
            RepairProgress::TryingRepair {
                ref file_name,
                ref strategy_desc,
            } => (ctx.progress)(
                &format!("{file_name}: {strategy_desc}"),
                ProgressUnit::Items,
                0,
                0,
            ),
            RepairProgress::Done => {}
        };
        match retro_junk_lib::repair::plan_repairs(
            &folder,
            console.analyzer.as_ref(),
            options,
            &per_file,
        ) {
            Ok(plan) => report.consoles.push(ConsoleRepairPlan {
                folder_name: folder_name.clone(),
                plan,
            }),
            Err(error) => report.skipped.push(format!("{folder_name}: {error}")),
        }
    }
    report
}

/// One console's repair outcome.
pub struct ConsoleRepairSummary {
    pub folder_name: String,
    pub summary: RepairSummary,
}

/// Carry out a plan. Files are rewritten in place; `create_backup` keeps a
/// `.bak` beside each one first.
#[must_use]
pub fn execute(
    planned: &RepairPlanReport,
    create_backup: bool,
    ctx: &OpCtx,
) -> Vec<ConsoleRepairSummary> {
    let mut summaries = Vec::new();
    let total = planned.consoles.len() as u64;
    for (index, console) in planned.consoles.iter().enumerate() {
        if ctx.cancelled() {
            break;
        }
        (ctx.progress)(
            &format!("Repairing {}", console.folder_name),
            ProgressUnit::Items,
            index as u64,
            total,
        );
        if !console.plan.has_actions() {
            continue;
        }
        summaries.push(ConsoleRepairSummary {
            folder_name: console.folder_name.clone(),
            summary: retro_junk_lib::repair::execute_repairs(&console.plan, create_backup),
        });
    }
    summaries
}

/// Where a repaired file's backup is written, for callers that want to say so.
#[must_use]
pub fn backup_path(file: &Path) -> PathBuf {
    let mut name = file.as_os_str().to_os_string();
    name.push(".bak");
    PathBuf::from(name)
}
