//! Organizing multi-disc games into .m3u folders: plan on request, execute
//! after the frontend has shown the plan for confirmation.

use std::path::Path;

use retro_junk_core::Platform;
use retro_junk_io::ProgressUnit;
use retro_junk_lib::AnalysisContext;
use retro_junk_lib::organize::{
    OrganizeOptions, OrganizePlan, OrganizeProgress, execute_organize_job, plan_organize,
};

use super::OpCtx;

/// What executing a plan did: counts for the summary line plus every error.
pub struct OrganizeExecution {
    /// Jobs whose target folder was actually created.
    pub jobs_executed: usize,
    pub files_moved: usize,
    /// Files the plan could not group, carried over from the plan.
    pub unmatched: usize,
    pub errors: Vec<String>,
}

/// Plan the organize operation for one console folder. The plan is returned
/// for the frontend to preview; nothing is moved yet.
pub fn plan(
    context: &AnalysisContext,
    platform: Platform,
    folder_path: &Path,
    ctx: &OpCtx,
) -> Result<OrganizePlan, String> {
    let registered = context
        .get_by_platform(platform)
        .ok_or_else(|| "No analyzer registered for platform".to_string())?;

    let options = OrganizeOptions {
        dat_dir: None,
        limit: None,
        include_single_disc: false,
        hash_fallback: false,
    };

    let progress_callback = |progress: OrganizeProgress| match &progress {
        OrganizeProgress::Scanning { file_count } => {
            (ctx.progress)("Scanning files", ProgressUnit::Items, 0, *file_count as u64);
        }
        OrganizeProgress::ReadingDisc { index, total, .. } => {
            (ctx.progress)(
                "Reading discs",
                ProgressUnit::Items,
                *index as u64,
                *total as u64,
            );
        }
        _ => {}
    };

    plan_organize(
        folder_path,
        registered.analyzer.as_ref(),
        &options,
        &progress_callback,
    )
    .map_err(|e| format!("Failed to plan: {e}"))
}

/// Execute an already-planned organize operation (after user confirmation).
/// Stops early when cancelled; the counts reflect what actually happened.
pub fn execute_plan(plan: &OrganizePlan, ctx: &OpCtx) -> OrganizeExecution {
    let mut result = OrganizeExecution {
        jobs_executed: 0,
        files_moved: 0,
        unmatched: plan.unmatched.len(),
        errors: Vec::new(),
    };

    let total_jobs = plan.jobs.len() as u64;
    for (i, job) in plan.jobs.iter().enumerate() {
        if ctx.cancelled() {
            break;
        }
        (ctx.progress)(
            "Moving disc files",
            ProgressUnit::Items,
            i as u64,
            total_jobs,
        );
        let job_result = execute_organize_job(job);
        if job_result.folder_created {
            result.jobs_executed += 1;
        }
        result.files_moved += job_result.files_moved;
        result.errors.extend(job_result.errors);
    }
    result
}
