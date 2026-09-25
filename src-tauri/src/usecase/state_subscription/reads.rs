use super::StateValue;
use crate::domain::{
    failure::{ClassifiedFailure, FailureKind},
    state_subscription::SubscriptionTarget,
    workspace_state::WorkspaceStateRepository,
};
use crate::usecase::{
    agent_session::{
        AgentSessionHistoryReadUsecase, AgentSessionHistoryRequest, AgentSessionProviderDto,
        AgentSessionReadUsecase, ProviderAvailabilityUsecase,
    },
    git_host::GitHostUsecase,
    repo_paths_usecase::RepoPathsUsecase,
    repository_state::RepositoryStateService,
    repository_usecase::RepositoryUsecase,
    workflow::WorkflowUsecase,
    workspace_tree::WorkspaceListUsecase,
};
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(crate) struct StateReadError {
    pub kind: FailureKind,
    pub message: String,
}
impl ClassifiedFailure for StateReadError {
    fn failure_kind(&self) -> FailureKind {
        self.kind
    }
}
fn error(e: impl ClassifiedFailure + std::fmt::Debug) -> StateReadError {
    StateReadError {
        kind: e.failure_kind(),
        message: format!("{e:?}"),
    }
}

#[derive(Clone)]
pub(crate) struct WorkspaceStateReads {
    pub queue: std::sync::Arc<crate::usecase::work_queue::WorkQueueUsecase>,
    pub repositories: Arc<RepoPathsUsecase>,
    pub repository: Arc<RepositoryUsecase>,
    pub repository_state: Arc<RepositoryStateService>,
    pub workflow: Arc<WorkflowUsecase>,
    pub workspaces: Arc<WorkspaceListUsecase>,
    pub sessions: Arc<AgentSessionReadUsecase>,
    pub history: Arc<AgentSessionHistoryReadUsecase>,
    pub providers: Arc<ProviderAvailabilityUsecase>,
    pub git_host: Arc<GitHostUsecase>,
    pub workspace_state: Arc<dyn WorkspaceStateRepository>,
}

impl WorkspaceStateReads {
    pub async fn read(&self, target: &SubscriptionTarget) -> Result<StateValue, StateReadError> {
        use SubscriptionTarget as T;
        match target {
            T::Failures(target, offset) => {
                return Ok(StateValue::Failures(
                    self.queue.records_page(target, *offset).await,
                ))
            }
            T::AgentSession(id) => {
                return self
                    .sessions
                    .get(id)
                    .await
                    .map(StateValue::AgentSession)
                    .map_err(error)
            }
            T::SessionHistory(path, count) => {
                return self
                    .history
                    .list(AgentSessionHistoryRequest {
                        worktree_path: path.clone(),
                        visible_count: *count,
                    })
                    .await
                    .map(StateValue::SessionHistory)
                    .map_err(error)
            }
            T::Selection(p, id) => {
                return Ok(StateValue::Selection(
                    self.workflow
                        .get_workspace_tree_selection_reconciliation(p, id)
                        .await
                        .map_err(error)?,
                ))
            }
            T::NodeDetail(p, id) => {
                return Ok(StateValue::NodeDetail(
                    self.workflow
                        .get_workspace_node_detail(p, id)
                        .await
                        .map_err(error)?,
                ))
            }
            T::SessionNode(p, id) => {
                return Ok(StateValue::SessionNode(
                    self.workflow
                        .get_workspace_session_node_id(p, id)
                        .await
                        .map_err(error)?,
                ))
            }
            _ => {}
        }
        let reads = self.clone();
        let target = target.clone();
        crate::other::operation_context::spawn_blocking(move || reads.read_blocking(&target))
            .await
            .map_err(|e| StateReadError {
                kind: FailureKind::Internal,
                message: e.to_string(),
            })?
    }

    fn read_blocking(&self, target: &SubscriptionTarget) -> Result<StateValue, StateReadError> {
        use SubscriptionTarget as T;
        Ok(match target {
            T::RepositoryPaths => StateValue::RepositoryPaths(self.repositories.get()),
            T::Workspaces => StateValue::Workspaces(self.workspaces.snapshot()),
            T::Providers => StateValue::Providers(
                self.providers
                    .available_providers()
                    .map_err(error)?
                    .into_iter()
                    .map(|provider| match provider {
                        crate::domain::provider_lifecycle::ProviderKind::Claude => {
                            AgentSessionProviderDto::Claude
                        }
                        crate::domain::provider_lifecycle::ProviderKind::Codex => {
                            AgentSessionProviderDto::Codex
                        }
                    })
                    .collect(),
            ),
            T::Branches(p, excluded) => StateValue::Branches(
                self.repository
                    .list_branches(p)
                    .map_err(error)?
                    .into_iter()
                    .filter(|branch| branch.is_base_candidate(excluded.as_deref()))
                    .map(Into::into)
                    .collect(),
            ),
            T::BranchBase(p, name) => {
                StateValue::BranchBase(self.repository.get_branch_base(p, name).map_err(error)?)
            }
            T::BranchStatus(p) => StateValue::BranchStatus(
                self.repository_state
                    .list_branches_with_status_snapshot(p)
                    .map_err(error)?,
            ),
            T::CurrentBranch(p) => {
                StateValue::CurrentBranch(self.repository.get_current_branch(p).map_err(error)?)
            }
            T::Issues(p) => StateValue::Issues(
                self.git_host
                    .get_cached_issues(p)
                    .map_err(error)?
                    .into_iter()
                    .map(Into::into)
                    .collect(),
            ),
            T::Worktrees(p) => {
                StateValue::Worktrees(self.repository.list_worktrees(p).map_err(error)?)
            }
            T::RepositoryRoot(p) => {
                StateValue::RepositoryRoot(self.repository.get_main_repo_path(p).map_err(error)?)
            }
            T::StartupRepository => StateValue::StartupRepository(
                self.repository
                    .get_main_repo_path(&self.repository.get_cwd().map_err(error)?)
                    .map_err(error)?,
            ),
            T::WorkspaceState(name, path) => StateValue::WorkspaceState(
                crate::usecase::workspace_state::usecase::load_workspace_state(
                    self.workspace_state.as_ref(),
                    name,
                    path,
                )
                .map(Into::into),
            ),
            T::Failures(..)
            | T::Terminal(_)
            | T::AgentSession(_)
            | T::SessionHistory(_, _)
            | T::Selection(_, _)
            | T::NodeDetail(_, _)
            | T::SessionNode(_, _) => {
                unreachable!("async reads handled above")
            }
        })
    }
}

#[async_trait::async_trait]
pub(crate) trait StateSubscriptionRead: Send + Sync {
    async fn read(&self, target: &SubscriptionTarget) -> Result<StateValue, StateReadError>;
    async fn refresh_external(&self, _target: &SubscriptionTarget) -> Result<(), StateReadError> {
        Ok(())
    }
    async fn refresh_workspaces(
        &self,
        source: Option<crate::domain::state_subscription::StateChangeSource>,
    );
    fn repositories(&self) -> Vec<String>;
}

#[async_trait::async_trait]
impl StateSubscriptionRead for WorkspaceStateReads {
    async fn read(&self, target: &SubscriptionTarget) -> Result<StateValue, StateReadError> {
        WorkspaceStateReads::read(self, target).await
    }
    async fn refresh_external(&self, target: &SubscriptionTarget) -> Result<(), StateReadError> {
        if let SubscriptionTarget::Issues(path) = target {
            let reads = self.clone();
            let path = path.clone();
            crate::other::operation_context::spawn_blocking(move || {
                reads.repository.get_main_repo_path(&path).map_err(error)?;
                reads.git_host.fetch_issues(&path).map_err(error)?;
                Ok(())
            })
            .await
            .map_err(|error| StateReadError {
                kind: FailureKind::Internal,
                message: error.to_string(),
            })??;
        }
        Ok(())
    }
    async fn refresh_workspaces(
        &self,
        source: Option<crate::domain::state_subscription::StateChangeSource>,
    ) {
        use crate::domain::state_subscription::StateChangeSource;
        match source {
            Some(StateChangeSource::WorkspaceList) => {}
            Some(StateChangeSource::Repository(paths)) => {
                let mut repositories = std::collections::HashSet::new();
                for path in paths {
                    match self.repository.get_main_repo_path(&path) {
                        Ok(path) => {
                            repositories.insert(path);
                        }
                        Err(error) => log::warn!("Repository resolution failed: {error}"),
                    }
                }
                for path in repositories {
                    self.workspaces.refresh_current_repository(&path).await;
                }
            }
            Some(StateChangeSource::Worktree(path)) => {
                self.workspaces.refresh_worktree(&path).await;
            }
            None => self.workspaces.refresh_external_information().await,
            _ => {
                self.workspaces.refresh().await;
            }
        }
    }
    fn repositories(&self) -> Vec<String> {
        self.workspaces.watch_paths()
    }
}

#[cfg(test)]
#[path = "reads_test.rs"]
pub(crate) mod reads_tests;
