use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Component, Path};

use crate::domain::app_data_gc::{is_expired, GcCategory, GcReport};
use crate::usecase::agent_session::session::SessionState;

use super::plan::{DeletionCandidate, DeletionPlan, DeletionRevalidation};
use super::ports::{
    CurrentSessionState, CurrentWorkflowExecutionState, GcFileSystem, GcRevalidationReader,
    RevalidationRead, WorkflowArchivePruner,
};
use super::request::{LiveWorktreeResolution, RuntimeProtection, StartupGcRequest};

pub(super) fn sweep(
    plan: DeletionPlan,
    fs: &dyn GcFileSystem,
    archive_pruner: &dyn WorkflowArchivePruner,
    revalidation_reader: &dyn GcRevalidationReader,
    request: &StartupGcRequest,
) -> GcReport {
    let mut report = GcReport::default();
    let mut failed_workflow_execution_deletions: BTreeMap<GcCategory, HashSet<String>> =
        BTreeMap::new();
    let workflow_execution_ids = workflow_execution_revalidation_ids(&plan.candidates);
    let mut workflow_revalidation = revalidation_reader
        .workflow_execution_states(&request.app_data_dir, &workflow_execution_ids);
    let runtime_protection =
        revalidation_reader.runtime_protection(&request.app_data_dir, &request.process_records);
    for candidate in plan.candidates {
        if !candidate_is_contained(&plan.app_data_dir, &candidate.path) {
            mark_workflow_candidate_failed(&candidate, &mut failed_workflow_execution_deletions);
            log::info!(
                "app data gc skipped {} because deletion candidate is outside app data dir",
                candidate.path.display()
            );
            continue;
        }
        if !candidate_still_valid(
            &candidate,
            request,
            revalidation_reader,
            &runtime_protection,
            &mut workflow_revalidation,
        ) {
            mark_workflow_candidate_failed(&candidate, &mut failed_workflow_execution_deletions);
            log::info!(
                "app data gc skipped {} because deletion candidate is no longer eligible",
                candidate.path.display()
            );
            continue;
        }
        let reclaimed_bytes = if fs.exists(&candidate.path) {
            match fs.recursive_size(&candidate.path) {
                Ok(size) => size,
                Err(error) => {
                    log::warn!(
                        "app data gc could not measure {}: {error}",
                        candidate.path.display()
                    );
                    0
                }
            }
        } else {
            0
        };
        match fs.remove_path(&candidate.path) {
            Ok(true) => report.record_deleted(candidate.category, reclaimed_bytes),
            Ok(false) => {}
            Err(error) => {
                report.record_error();
                mark_workflow_candidate_failed(
                    &candidate,
                    &mut failed_workflow_execution_deletions,
                );
                log::warn!(
                    "app data gc failed to remove {}: {error}",
                    candidate.path.display()
                );
            }
        }
    }
    for (category, execution_ids) in plan.workflow_archive_records {
        let failed_execution_ids = failed_workflow_execution_deletions
            .get(&category)
            .cloned()
            .unwrap_or_default();
        let prune_execution_ids = execution_ids
            .difference(&failed_execution_ids)
            .cloned()
            .collect::<HashSet<_>>();
        if prune_execution_ids.is_empty() {
            continue;
        }
        match archive_pruner
            .prune_workflow_archive_records(&plan.app_data_dir, &prune_execution_ids)
        {
            Ok(result) => {
                for _ in 0..result.records_removed {
                    report.record_deleted(category, 0);
                }
                if result.reclaimed_bytes > 0 {
                    if let Some(stat) = report.categories.get_mut(&category) {
                        stat.reclaimed_bytes += result.reclaimed_bytes;
                    }
                    report.total_bytes += result.reclaimed_bytes;
                }
            }
            Err(error) => {
                report.record_error();
                log::warn!("app data gc failed to prune workflow archive records: {error}");
            }
        }
    }
    report
}

fn workflow_execution_revalidation_ids(candidates: &[DeletionCandidate]) -> HashSet<String> {
    candidates
        .iter()
        .filter_map(|candidate| match &candidate.revalidation {
            DeletionRevalidation::WorkflowExecutionMetadata { execution_id } => {
                Some(execution_id.clone())
            }
            DeletionRevalidation::None
            | DeletionRevalidation::Session { .. }
            | DeletionRevalidation::WorkspaceState { .. }
            | DeletionRevalidation::ReviewComment { .. } => None,
        })
        .collect()
}

fn candidate_is_contained(app_data_dir: &Path, path: &Path) -> bool {
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::Prefix(_) | Component::RootDir
        )
    }) && path.is_relative()
    {
        return false;
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return false;
    }
    path.starts_with(app_data_dir)
}

fn mark_workflow_candidate_failed(
    candidate: &DeletionCandidate,
    failed_workflow_execution_deletions: &mut BTreeMap<GcCategory, HashSet<String>>,
) {
    if let Some(execution_id) = candidate.workflow_execution_id.as_ref() {
        failed_workflow_execution_deletions
            .entry(candidate.category)
            .or_default()
            .insert(execution_id.clone());
    }
}

fn candidate_still_valid(
    candidate: &DeletionCandidate,
    request: &StartupGcRequest,
    reader: &dyn GcRevalidationReader,
    runtime: &RuntimeProtection,
    workflow_revalidation: &mut HashMap<String, RevalidationRead<CurrentWorkflowExecutionState>>,
) -> bool {
    match &candidate.revalidation {
        DeletionRevalidation::None => true,
        DeletionRevalidation::Session { session_id } => {
            session_candidate_still_valid(candidate.category, session_id, request, runtime, reader)
        }
        DeletionRevalidation::WorkflowExecutionMetadata { execution_id } => {
            let state = workflow_revalidation
                .entry(execution_id.clone())
                .or_insert_with(|| {
                    reader.workflow_execution_state(&request.app_data_dir, execution_id)
                });
            workflow_candidate_still_valid(
                candidate.category,
                execution_id,
                request,
                runtime,
                state,
            )
        }
        DeletionRevalidation::WorkspaceState { key } => {
            workspace_state_candidate_still_valid(key, request.live_worktrees.as_ref(), runtime)
        }
        DeletionRevalidation::ReviewComment { key } => {
            review_comment_candidate_still_valid(key, request.live_worktrees.as_ref(), runtime)
        }
    }
}

fn session_candidate_still_valid(
    category: GcCategory,
    session_id: &str,
    request: &StartupGcRequest,
    runtime: &RuntimeProtection,
    reader: &dyn GcRevalidationReader,
) -> bool {
    if runtime.active_session_ids.contains(session_id) {
        return false;
    }
    let Some(live_worktrees) = request.live_worktrees.as_ref() else {
        return false;
    };
    match reader.session_state(&request.app_data_dir, session_id) {
        RevalidationRead::Present(state) => {
            session_state_matches_category(category, &state, live_worktrees, runtime, request)
        }
        RevalidationRead::Missing => false,
        RevalidationRead::Unavailable(error) => {
            log::warn!("app data gc skipped session {session_id} during revalidation: {error}");
            false
        }
    }
}

fn session_state_matches_category(
    category: GcCategory,
    state: &CurrentSessionState,
    live_worktrees: &LiveWorktreeResolution,
    runtime: &RuntimeProtection,
    request: &StartupGcRequest,
) -> bool {
    let Some(worktree_path) = state.worktree_path.as_ref() else {
        return category == GcCategory::UnrecoverableSession;
    };
    if worktree_path.is_unresolved() || runtime.running_worktrees.contains(worktree_path.key()) {
        return false;
    }
    if !live_worktrees.contains_worktree_path(worktree_path) {
        return !live_worktrees.worktree_path_may_be_unresolved(worktree_path)
            && category == GcCategory::DeletedWorkspace;
    }
    category == GcCategory::RecoverableExpired
        && matches!(
            state.state.as_ref(),
            Some(SessionState::Archived | SessionState::Closed)
        )
        && state.updated_at.is_some_and(|updated_at| {
            is_expired(
                request.now_secs,
                updated_at,
                request.retention.archived_log_secs,
            )
        })
}

fn workflow_candidate_still_valid(
    category: GcCategory,
    execution_id: &str,
    request: &StartupGcRequest,
    runtime: &RuntimeProtection,
    state: &RevalidationRead<CurrentWorkflowExecutionState>,
) -> bool {
    let Some(live_worktrees) = request.live_worktrees.as_ref() else {
        return false;
    };
    match state {
        RevalidationRead::Present(state) => {
            workflow_state_matches_category(category, state, live_worktrees, runtime, request)
        }
        RevalidationRead::Missing => false,
        RevalidationRead::Unavailable(error) => {
            log::warn!(
                "app data gc skipped workflow execution {execution_id} during revalidation: {error}"
            );
            false
        }
    }
}

fn workflow_state_matches_category(
    category: GcCategory,
    state: &CurrentWorkflowExecutionState,
    live_worktrees: &LiveWorktreeResolution,
    runtime: &RuntimeProtection,
    request: &StartupGcRequest,
) -> bool {
    if !state.is_terminal
        || state.worktree_path.is_unresolved()
        || runtime
            .running_worktrees
            .contains(state.worktree_path.key())
    {
        return false;
    }
    if !live_worktrees.contains_worktree_path(&state.worktree_path) {
        return !live_worktrees.worktree_path_may_be_unresolved(&state.worktree_path)
            && category == GcCategory::DeletedWorkspace;
    }
    category == GcCategory::RecoverableExpired
        && state.manual_archived_at.is_some_and(|archived_at| {
            is_expired(
                request.now_secs,
                archived_at,
                request.retention.archived_log_secs,
            )
        })
}

fn workspace_state_candidate_still_valid(
    key: &str,
    live_worktrees: Option<&LiveWorktreeResolution>,
    runtime: &RuntimeProtection,
) -> bool {
    let Some(live_worktrees) = live_worktrees else {
        return false;
    };
    runtime.workspace_keyed_protection_complete
        && !live_worktrees.has_unresolved_repos()
        && !live_worktrees.contains_workspace_state_key(key)
        && !runtime
            .protected_worktrees
            .contains_workspace_state_key(key)
        && !live_worktrees.workspace_state_key_may_be_unresolved(key)
}

fn review_comment_candidate_still_valid(
    key: &str,
    live_worktrees: Option<&LiveWorktreeResolution>,
    runtime: &RuntimeProtection,
) -> bool {
    let Some(live_worktrees) = live_worktrees else {
        return false;
    };
    runtime.workspace_keyed_protection_complete
        && !live_worktrees.has_unresolved_repos()
        && !live_worktrees.contains_review_comment_key(key)
        && !runtime.protected_worktrees.contains_review_comment_key(key)
}
