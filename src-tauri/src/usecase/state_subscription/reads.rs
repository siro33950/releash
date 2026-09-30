use super::StateValue;
use crate::domain::{failure::TechnicalFailure, workspace_state::WorkspaceStateRepository};
use crate::usecase::state_subscription::SubscriptionTarget;
use crate::usecase::{
    agent_session::{
        AgentSessionHistoryReadUsecase, AgentSessionHistoryRequest, AgentSessionReadUsecase,
        ProviderAvailabilityUsecase,
    },
    comment::{ReviewCommentUsecase, ReviewThreadDto},
    git_host::GitHostUsecase,
    provider_dto::AgentSessionProviderDto,
    repo_paths_usecase::RepoPathsUsecase,
    repository_usecase::RepositoryUsecase,
    review_usecase::ReviewUsecase,
    workflow::WorkflowUsecase,
    workspace_tree::WorkspaceListUsecase,
};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug)]
pub(crate) struct StateReadError {
    pub source: StateReadFailure,
    pub message: String,
}

#[derive(Debug)]
pub(crate) enum StateReadFailure {
    InvalidTerminalInput,
    TerminalSubscriptionEnded,
    Workflow(Box<crate::domain::workflow::WorkflowError>),
    Session(Box<crate::usecase::agent_session::AgentSessionReadUsecaseError>),
    History(Box<crate::usecase::agent_session::AgentSessionHistoryQueryError>),
    Providers(Box<crate::usecase::agent_session::ProviderAvailabilityUsecaseError>),
    Repository(Box<crate::usecase::repository_error::UsecaseError>),
    RepositoryState(Box<crate::usecase::repository_state::error::RepositoryStateError>),
    GitHost(Box<crate::domain::git_host::GitHostError>),
    Watcher(Box<crate::usecase::watcher::UsecaseError>),
    Subscription(Box<crate::usecase::state_subscription::SubscriptionError>),
    Code(Box<crate::usecase::code_error::CodeUsecaseError>),
    Review(Box<crate::domain::comment::ReviewError>),
    AppConfig(Box<crate::usecase::app_config::error::UsecaseError>),
    Notion(Box<crate::usecase::notion::error::NotionUsecaseError>),
    Editor(Box<crate::domain::external_editor::EditorError>),
    HookHealth(Box<crate::usecase::provider_lifecycle::ProviderHookHealthUsecaseError>),
    Technical(Box<TechnicalFailure>),
}
impl From<crate::usecase::code_error::CodeUsecaseError> for StateReadFailure {
    fn from(error: crate::usecase::code_error::CodeUsecaseError) -> Self {
        Self::Code(Box::new(error))
    }
}
impl From<crate::domain::comment::ReviewError> for StateReadFailure {
    fn from(error: crate::domain::comment::ReviewError) -> Self {
        Self::Review(Box::new(error))
    }
}
impl From<crate::usecase::app_config::error::UsecaseError> for StateReadFailure {
    fn from(error: crate::usecase::app_config::error::UsecaseError) -> Self {
        Self::AppConfig(Box::new(error))
    }
}
impl From<crate::usecase::notion::error::NotionUsecaseError> for StateReadFailure {
    fn from(error: crate::usecase::notion::error::NotionUsecaseError) -> Self {
        Self::Notion(Box::new(error))
    }
}
impl From<crate::domain::external_editor::EditorError> for StateReadFailure {
    fn from(error: crate::domain::external_editor::EditorError) -> Self {
        Self::Editor(Box::new(error))
    }
}
impl From<crate::usecase::provider_lifecycle::ProviderHookHealthUsecaseError> for StateReadFailure {
    fn from(error: crate::usecase::provider_lifecycle::ProviderHookHealthUsecaseError) -> Self {
        Self::HookHealth(Box::new(error))
    }
}
impl From<crate::domain::workflow::WorkflowError> for StateReadFailure {
    fn from(error: crate::domain::workflow::WorkflowError) -> Self {
        Self::Workflow(Box::new(error))
    }
}
impl From<crate::usecase::agent_session::AgentSessionReadUsecaseError> for StateReadFailure {
    fn from(error: crate::usecase::agent_session::AgentSessionReadUsecaseError) -> Self {
        Self::Session(Box::new(error))
    }
}
impl From<crate::usecase::agent_session::AgentSessionHistoryQueryError> for StateReadFailure {
    fn from(error: crate::usecase::agent_session::AgentSessionHistoryQueryError) -> Self {
        Self::History(Box::new(error))
    }
}
impl From<crate::usecase::agent_session::ProviderAvailabilityUsecaseError> for StateReadFailure {
    fn from(error: crate::usecase::agent_session::ProviderAvailabilityUsecaseError) -> Self {
        Self::Providers(Box::new(error))
    }
}
impl From<crate::usecase::repository_error::UsecaseError> for StateReadFailure {
    fn from(error: crate::usecase::repository_error::UsecaseError) -> Self {
        Self::Repository(Box::new(error))
    }
}
impl From<crate::usecase::repository_state::error::RepositoryStateError> for StateReadFailure {
    fn from(error: crate::usecase::repository_state::error::RepositoryStateError) -> Self {
        Self::RepositoryState(Box::new(error))
    }
}
impl From<crate::domain::git_host::GitHostError> for StateReadFailure {
    fn from(error: crate::domain::git_host::GitHostError) -> Self {
        Self::GitHost(Box::new(error))
    }
}
impl From<crate::usecase::watcher::UsecaseError> for StateReadFailure {
    fn from(error: crate::usecase::watcher::UsecaseError) -> Self {
        Self::Watcher(Box::new(error))
    }
}
impl From<crate::usecase::state_subscription::SubscriptionError> for StateReadFailure {
    fn from(error: crate::usecase::state_subscription::SubscriptionError) -> Self {
        Self::Subscription(Box::new(error))
    }
}
impl From<TechnicalFailure> for StateReadFailure {
    fn from(error: TechnicalFailure) -> Self {
        Self::Technical(Box::new(error))
    }
}
impl StateReadError {
    pub(crate) fn from_error<E: std::fmt::Display>(error: E) -> Self
    where
        StateReadFailure: From<E>,
    {
        let message = error.to_string();
        Self {
            source: error.into(),
            message,
        }
    }
}
fn error<E: std::fmt::Debug>(e: E) -> StateReadError
where
    StateReadFailure: From<E>,
{
    let message = format!("{e:?}");
    StateReadError {
        source: e.into(),
        message,
    }
}

#[derive(Clone)]
pub(crate) struct WorkspaceStateReads {
    pub repositories: Arc<RepoPathsUsecase>,
    pub repository: Arc<RepositoryUsecase>,
    pub workflow: Arc<WorkflowUsecase>,
    pub workspaces: Arc<WorkspaceListUsecase>,
    pub sessions: Arc<AgentSessionReadUsecase>,
    pub history: Arc<AgentSessionHistoryReadUsecase>,
    pub providers: Arc<ProviderAvailabilityUsecase>,
    pub git_host: Arc<GitHostUsecase>,
    pub workspace_state: Arc<dyn WorkspaceStateRepository>,
    pub review: Arc<ReviewUsecase>,
    pub comments: Arc<ReviewCommentUsecase>,
    pub data_dir: PathBuf,
    pub review_comments_dir: PathBuf,
    pub workflows_dir: PathBuf,
    pub app_config: Arc<crate::usecase::app_config::AppConfigUsecase>,
    pub notion: Arc<crate::usecase::notion::usecase::NotionUsecase>,
    pub editor_settings: Arc<dyn crate::domain::external_editor::EditorSettingsGateway>,
    pub editor_scanner: Arc<dyn crate::domain::external_editor::InstalledEditorGateway>,
    pub performance_switches: crate::usecase::telemetry::PerformanceSwitches,
    pub hook_health: Arc<crate::usecase::provider_lifecycle::ProviderHookHealthReadUsecase>,
    pub startup: Arc<crate::usecase::application_startup::ApplicationStartupAuthority>,
}

impl WorkspaceStateReads {
    pub async fn read(&self, target: &SubscriptionTarget) -> Result<StateValue, StateReadError> {
        use SubscriptionTarget as T;
        match target {
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
            T::Workspaces => return Ok(StateValue::Workspaces(self.workspaces.read().await)),
            T::Selection(p, id) => {
                let (tree, selected) = self
                    .workflow
                    .workspace_tree_selection(p, id)
                    .await
                    .map_err(error)?;
                return Ok(StateValue::Selection(tree, selected));
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
            T::Workflows => {
                return Ok(StateValue::Workflows(
                    self.workflow
                        .read_usecase()
                        .list_workflow_summaries()
                        .await
                        .map_err(error)?,
                ))
            }
            T::ProviderHookHealth => {
                return Ok(StateValue::ProviderHookHealth(
                    self.hook_health
                        .warnings()
                        .await
                        .map_err(error)?
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                ))
            }
            _ => {}
        }
        self.read_blocking(target)
    }

    fn read_blocking(&self, target: &SubscriptionTarget) -> Result<StateValue, StateReadError> {
        use SubscriptionTarget as T;
        Ok(match target {
            T::RepositoryPaths => StateValue::RepositoryPaths(self.repositories.get()),
            T::Providers => StateValue::Providers(
                self.providers
                    .available_providers()
                    .map_err(error)?
                    .into_iter()
                    .map(AgentSessionProviderDto::from)
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
                self.repository
                    .list_branches_with_worktree(p)
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
            T::ReviewSnapshot(path, base) => StateValue::ReviewSnapshot(
                self.review
                    .get_review_snapshot(path, base.as_str())
                    .map_err(error)?,
            ),
            T::ReviewFileView(path, file, section, base) => StateValue::ReviewFileView(
                self.review
                    .get_review_file_view(path, file, section.as_str(), base.as_str())
                    .map_err(error)?,
            ),
            T::ReviewThreads(name) => StateValue::ReviewThreads(
                self.comments
                    .list_threads(
                        &self.data_dir,
                        name,
                        None,
                        crate::domain::comment::ReviewActor::human(),
                    )
                    .map_err(error)?
                    .into_iter()
                    .map(ReviewThreadDto::from)
                    .collect(),
            ),
            T::Workflow(name) => StateValue::Workflow(self.workflow.get_workflow_dto(name)),
            T::WorkflowSource(name) => {
                StateValue::WorkflowSource(self.workflow.get_workflow_source(name).map_err(error)?)
            }
            T::Facets(kind) => StateValue::Facets(
                self.workflow
                    .list_facet_summaries(*kind)
                    .map_err(error)?
                    .into_iter()
                    .map(crate::usecase::workflow::dto::facet_summary_to_dto)
                    .collect(),
            ),
            T::Facet(kind, key) => {
                StateValue::Facet(self.workflow.get_facet(*kind, key).map_err(error)?)
            }
            T::Diagnostics => StateValue::Diagnostics(
                self.workflow
                    .diagnose_all(
                        crate::usecase::workflow::ports::WorkflowDiagnosticsTarget::AppliedConfigDirectory,
                    )
                    .map_err(error)?,
            ),
            T::DesktopSettings => {
                StateValue::DesktopSettings(self.app_config.desktop_settings().map_err(error)?)
            }
            T::NotionConfig(p) => {
                StateValue::NotionConfig(self.notion.get_config(p).map_err(error)?)
            }
            T::ProviderAvailability => {
                StateValue::ProviderAvailability(self.providers.snapshot_dto().map_err(error)?)
            }
            T::ExternalEditor => StateValue::ExternalEditor(
                crate::usecase::external_editor::dto::ExternalEditorState {
                    selected: crate::usecase::external_editor::open_usecase::get_external_editor(
                        self.editor_settings.as_ref(),
                    )
                    .map_err(error)?,
                    editors: crate::usecase::external_editor::detect_usecase::detect_editors(
                        self.editor_scanner.as_ref(),
                    )
                    .into_iter()
                    .map(Into::into)
                    .collect(),
                },
            ),
            T::ReleashBase(p) => {
                StateValue::ReleashBase(self.repository.get_releash_base(p).map_err(error)?)
            }
            T::WorkflowConfig => {
                StateValue::WorkflowConfig(self.app_config.get_workflow_config().map_err(error)?)
            }
            T::PerformanceSwitches => StateValue::PerformanceSwitches(self.performance_switches),
            T::StartupOutcome => StateValue::StartupOutcome(self.startup.outcome()),
            T::Terminal(_)
            | T::Workspaces
            | T::Workflows
            | T::AgentSession(_)
            | T::SessionHistory(_, _)
            | T::Selection(_, _)
            | T::NodeDetail(_, _)
            | T::SessionNode(_, _)
            | T::ProviderHookHealth => {
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
    fn repositories(&self) -> Vec<String>;
    fn review_comments_dir(&self) -> String {
        String::new()
    }
    fn workflows_dir(&self) -> String {
        String::new()
    }
}

#[async_trait::async_trait]
impl StateSubscriptionRead for WorkspaceStateReads {
    async fn read(&self, target: &SubscriptionTarget) -> Result<StateValue, StateReadError> {
        WorkspaceStateReads::read(self, target).await
    }
    async fn refresh_external(&self, target: &SubscriptionTarget) -> Result<(), StateReadError> {
        match target {
            SubscriptionTarget::Issues(path) => {
                self.repository.get_main_repo_path(path).map_err(error)?;
                self.git_host.fetch_issues(path).map_err(error)?;
            }
            SubscriptionTarget::Workspaces => self.workspaces.refresh_pull_requests(),
            _ => {}
        }
        Ok(())
    }
    fn repositories(&self) -> Vec<String> {
        self.workspaces.watch_paths()
    }
    fn review_comments_dir(&self) -> String {
        self.review_comments_dir.to_string_lossy().into_owned()
    }
    fn workflows_dir(&self) -> String {
        self.workflows_dir.to_string_lossy().into_owned()
    }
}

impl std::fmt::Display for StateReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for StateReadError {}

#[cfg(test)]
#[path = "reads_test.rs"]
mod reads_tests;
