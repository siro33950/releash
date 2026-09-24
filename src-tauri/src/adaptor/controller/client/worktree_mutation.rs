use super::dispatch::{invalid_request, required};
use crate::adaptor::controller::api::protocol::client as wire;
use crate::usecase::workflow::WorkflowRuntimeUsecase;
use crate::usecase::worktree_operation::WorktreeMutationGuard;

tokio::task_local! {
    static MUTATION_GUARDS: std::sync::Arc<Vec<WorktreeMutationGuard>>;
}

pub(super) async fn scope<T>(
    guards: Vec<WorktreeMutationGuard>,
    future: impl std::future::Future<Output = T>,
) -> T {
    MUTATION_GUARDS
        .scope(std::sync::Arc::new(guards), future)
        .await
}

pub(in crate::adaptor::controller) fn spawn_blocking<F, T>(
    operation: F,
) -> tokio::task::JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let guards = MUTATION_GUARDS.try_with(std::sync::Arc::clone).ok();
    tokio::task::spawn_blocking(move || {
        let _guards = guards;
        operation()
    })
}

pub(super) fn admit(
    runtime: Option<&WorkflowRuntimeUsecase>,
    command: &wire::command_request::Command,
) -> Result<Vec<WorktreeMutationGuard>, wire::CommandFailure> {
    use wire::command_request::Command as C;
    let path = match command {
        C::GitCreateBranch(a) => a.repo_path.as_deref(),
        C::GitStage(a) => a.repo_path.as_deref(),
        C::GitUnstage(a) => a.repo_path.as_deref(),
        C::GitStageReviewGroup(a) => a.input.as_ref().and_then(|a| a.worktree_path.as_deref()),
        C::GitUnstageReviewGroup(a) => a.input.as_ref().and_then(|a| a.worktree_path.as_deref()),
        C::SetBranchBase(a) => a.repo_path.as_deref(),
        C::SetReleashBase(a) => a.repo_path.as_deref(),
        C::SaveNotionConfig(a) => a.repo_path.as_deref(),
        C::DeleteNotionConfig(a) => a.repo_path.as_deref(),
        C::RenameWorkspaceSessionNode(a) => a.worktree_path.as_deref(),
        C::ApproveWorkspaceNode(a) => a.worktree_path.as_deref(),
        C::RetryWorkspaceNode(a) => a.worktree_path.as_deref(),
        C::ResumeWorkspaceSessionNode(a) => a.worktree_path.as_deref(),
        C::CreateReviewThread(a) => a.worktree_name.as_deref(),
        C::AppendReviewComment(a) => a.worktree_name.as_deref(),
        C::DeleteReviewThread(a) => a.worktree_name.as_deref(),
        C::ResolveReviewThread(a) => a.worktree_name.as_deref(),
        C::GetOrSpawnTerminalSurface(a) => terminal_workspace(a.owner.as_ref()),
        C::WriteTerminalSurface(a) => terminal_workspace(a.owner.as_ref()),
        C::WritePathsToTerminalSurface(a) => terminal_workspace(a.owner.as_ref()),
        C::ResizeTerminalSurface(a) => terminal_workspace(a.owner.as_ref()),
        C::KillTerminalSurface(a) => terminal_workspace(a.owner.as_ref()),
        C::CreateWorktree(a) => {
            let runtime =
                runtime.ok_or_else(|| invalid_request("Command dependency unavailable"))?;
            return runtime
                .begin_worktree_creation_mutation(
                    required(a.repo_path.as_deref(), "repoPath")?,
                    required(a.branch.as_deref(), "branch")?,
                )
                .map_err(mutation_error);
        }
        C::SaveWorkspaceState(a) => {
            let runtime =
                runtime.ok_or_else(|| invalid_request("Command dependency unavailable"))?;
            return runtime
                .begin_workspace_state_mutation(required(
                    a.worktree_name.as_deref(),
                    "worktreeName",
                )?)
                .map(|guard| vec![guard])
                .map_err(mutation_error);
        }
        _ => return Ok(Vec::new()),
    };
    let runtime = runtime.ok_or_else(|| invalid_request("Command dependency unavailable"))?;
    runtime
        .begin_worktree_mutation(required(path, "worktreePath")?)
        .map(|guard| vec![guard])
        .map_err(mutation_error)
}

fn terminal_workspace(owner: Option<&wire::TerminalSurfaceOwnerV1>) -> Option<&str> {
    use wire::terminal_surface_owner_v1::Variant;
    match owner?.variant.as_ref()? {
        Variant::Workspace(owner) => owner.workspace_path.as_deref(),
        Variant::Session(owner) => owner.workspace_path.as_deref(),
    }
}

fn mutation_error(error: crate::domain::workflow::WorkflowError) -> wire::CommandFailure {
    use crate::domain::failure::ClassifiedFailure;
    crate::other::AppError::coded(
        "WORKTREE_MUTATION_REJECTED",
        error.to_string(),
        error.failure_kind(),
    )
    .into()
}

#[cfg(test)]
#[path = "worktree_mutation_test.rs"]
mod worktree_mutation_tests;
