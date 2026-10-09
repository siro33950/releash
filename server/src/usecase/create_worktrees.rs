use std::sync::Arc;

use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::repository::RepositoryError;
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::agent_session::{AgentSessionLaunchRequest, AgentSessionLaunchUsecase};
use crate::usecase::repository_error::UsecaseError;
use crate::usecase::repository_usecase::RepositoryUsecase;
use crate::usecase::workflow::{command::StartExecutionCommand, WorkflowRuntimeUsecase};

#[derive(Clone)]
pub enum LaunchAfterCreation {
    None,
    Session {
        provider: ProviderKind,
        rows: u16,
        cols: u16,
        request_id: String,
    },
    Workflow {
        name: String,
        request: Option<String>,
    },
}

#[async_trait::async_trait]
pub trait WorktreeLaunch: Send + Sync {
    fn begin_creation(
        &self,
        repo: &str,
        branch: &str,
    ) -> Result<Vec<crate::usecase::worktree_operation::WorktreeMutationGuard>, UsecaseError>;
    async fn launch(&self, path: &str, launch: &LaunchAfterCreation) -> Result<(), UsecaseError>;
}

#[async_trait::async_trait]
pub trait WorktreeSessionLaunch: Send + Sync {
    async fn launch_session(&self, request: AgentSessionLaunchRequest) -> Result<(), UsecaseError>;
}
#[async_trait::async_trait]
impl WorktreeSessionLaunch for Arc<AgentSessionLaunchUsecase> {
    async fn launch_session(&self, request: AgentSessionLaunchRequest) -> Result<(), UsecaseError> {
        self.clone().launch_standalone_selection(request).await?;
        Ok(())
    }
}
#[async_trait::async_trait]
pub trait WorktreeWorkflowLaunch: Send + Sync {
    fn begin_creation(
        &self,
        repo: &str,
        branch: &str,
    ) -> Result<Vec<crate::usecase::worktree_operation::WorktreeMutationGuard>, UsecaseError>;
    async fn launch_workflow(&self, command: StartExecutionCommand) -> Result<(), UsecaseError>;
}
#[async_trait::async_trait]
impl WorktreeWorkflowLaunch for WorkflowRuntimeUsecase {
    fn begin_creation(
        &self,
        repo: &str,
        branch: &str,
    ) -> Result<Vec<crate::usecase::worktree_operation::WorktreeMutationGuard>, UsecaseError> {
        Ok(self.begin_worktree_creation_mutation(repo, branch)?)
    }
    async fn launch_workflow(&self, command: StartExecutionCommand) -> Result<(), UsecaseError> {
        self.start_execution(command).await?;
        Ok(())
    }
}
pub struct WorktreeLauncher<S = Arc<AgentSessionLaunchUsecase>, W = WorkflowRuntimeUsecase> {
    pub sessions: S,
    pub workflows: Arc<W>,
}

#[async_trait::async_trait]
impl<S: WorktreeSessionLaunch, W: WorktreeWorkflowLaunch> WorktreeLaunch
    for WorktreeLauncher<S, W>
{
    fn begin_creation(
        &self,
        repo: &str,
        branch: &str,
    ) -> Result<Vec<crate::usecase::worktree_operation::WorktreeMutationGuard>, UsecaseError> {
        self.workflows.begin_creation(repo, branch)
    }
    async fn launch(&self, path: &str, launch: &LaunchAfterCreation) -> Result<(), UsecaseError> {
        match launch {
            LaunchAfterCreation::None => {}
            LaunchAfterCreation::Session {
                provider,
                rows,
                cols,
                request_id,
            } => {
                self.sessions
                    .launch_session(AgentSessionLaunchRequest {
                        workspace: WorkspaceIdentity::new(path),
                        worktree_path: path.into(),
                        provider: *provider,
                        rows: *rows,
                        cols: *cols,
                        caller_request_id: format!("{request_id}:{path}"),
                    })
                    .await?;
            }
            LaunchAfterCreation::Workflow { name, request } => {
                self.workflows
                    .launch_workflow(StartExecutionCommand {
                        workflow_name: name.clone(),
                        worktree_path: path.into(),
                        request: request.clone(),
                        created_from: crate::domain::workflow::ExecutionOrigin::DesktopUi,
                    })
                    .await?;
            }
        }
        Ok(())
    }
}

pub async fn create_worktrees(
    repository: Arc<RepositoryUsecase>,
    launcher: &dyn WorktreeLaunch,
    repo_path: String,
    branches: Vec<String>,
    base_branch: Option<String>,
    launch: LaunchAfterCreation,
) -> Result<Vec<String>, UsecaseError> {
    crate::domain::repository::validate_worktree_branches(&branches)?;
    let mut paths = Vec::with_capacity(branches.len());
    for branch in branches {
        let guards = launcher.begin_creation(&repo_path, &branch)?;
        let repository = repository.clone();
        let repo_path = repo_path.clone();
        let base_branch = base_branch.clone();
        let path = crate::common::operation_context::spawn_blocking(move || {
            let _guards = guards;
            let existing = repository.list_branches_with_worktree(&repo_path)?;
            let create_branch =
                crate::domain::repository::Branch::needs_creation(&branch, &existing);
            repository
                .create_worktree(&repo_path, &branch, create_branch, base_branch.as_deref())
                .map(|worktree| worktree.path)
        })
        .await
        .map_err(|error| RepositoryError::Technical(error.into()))??;
        launcher.launch(&path, &launch).await?;
        paths.push(path);
    }
    Ok(paths)
}

#[cfg(test)]
#[path = "create_worktrees_test.rs"]
mod create_worktrees_tests;
