mod plan;
mod ports;
mod request;
mod sweep;

#[cfg(test)]
mod test_fixtures;
#[cfg(test)]
mod tests;

pub(crate) use ports::gc_file_system_error;
pub(crate) use ports::{
    CurrentSessionState, CurrentWorkflowExecutionState, GcFileSystem, GcFileSystemError,
    GcRevalidationReader, RevalidationRead, WorkflowArchivePruner,
};
pub(crate) use request::{
    CacheGcRecord, GcWorktreePath, LiveWorktree, LiveWorktreeResolution, LiveWorktreeSet,
    ProcessRecord, ProcessRecordStatus, ReviewCommentGcRecord, RuntimeProtection, SessionBlobStore,
    SessionGcRecord, StartupGcRequest, WorkflowArchivePruneResult, WorkflowExecutionGcRecord,
    WorkspaceStateGcRecord,
};

use crate::domain::app_data_gc::GcReport;

use plan::{
    collect_cache_deletions, collect_legacy_comment_deletions, collect_orphan_blob_deletions,
    collect_session_deletions, collect_stale_process_deletions, collect_workflow_deletions,
    collect_workspace_keyed_deletions, DeletionPlan, SessionDeletionContext,
};
use sweep::sweep;

pub(crate) fn run_startup_gc(
    request: StartupGcRequest,
    fs: &dyn GcFileSystem,
    archive_pruner: &dyn WorkflowArchivePruner,
    revalidation_reader: &dyn GcRevalidationReader,
) -> GcReport {
    let mut plan = DeletionPlan::new(request.app_data_dir.clone());

    if let Some(live_worktree_resolution) = request.live_worktrees.as_ref() {
        let session_context = SessionDeletionContext {
            live_worktrees: live_worktree_resolution,
            session_records: &request.session_records,
            active_session_ids: &request.runtime_protection.active_session_ids,
            running_worktrees: &request.runtime_protection.running_worktrees,
            now_secs: request.now_secs,
            retention: request.retention,
        };
        collect_session_deletions(&session_context, &mut plan);
        collect_workflow_deletions(
            &request.workflow_executions,
            live_worktree_resolution,
            request.now_secs,
            request.retention,
            &mut plan,
        );
        collect_workspace_keyed_deletions(
            &request.workspace_state_records,
            &request.review_comment_records,
            &request.checkpoint_paths,
            live_worktree_resolution,
            &request.runtime_protection,
            &mut plan,
        );
    } else {
        log::info!(
            "app data gc skipped workspace-dependent rules because live worktrees were unavailable"
        );
    }

    collect_cache_deletions(
        &request.cache_records,
        request.now_secs,
        request.retention,
        &mut plan,
    );
    collect_legacy_comment_deletions(&request.legacy_comment_paths, &mut plan);
    collect_orphan_blob_deletions(&request.session_blob_stores, fs, &mut plan);
    collect_stale_process_deletions(&request.process_records, &mut plan);

    let report = sweep(plan, fs, archive_pruner, revalidation_reader, &request);
    log::info!("{}", report.log_summary());
    report
}
