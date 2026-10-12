use crate::usecase::{
    agent_session::{AgentSessionHistoryPageDto, AgentSessionItemDto},
    code_dto::{ReviewFileViewDto, ReviewSnapshotDto},
    provider_dto::AgentSessionProviderDto,
    repository_dto::{BranchDto, WorktreeEntryDto},
    workflow::{
        diagnostic_dto::DiagnosticReport,
        dto::{FacetSummaryDto, WorkflowDto, WorkflowSummaryDto},
        WorkspaceNodeDetailDto,
    },
    workspace_tree::WorkspaceList,
};

#[derive(Debug, Clone, PartialEq)]
pub enum StateValue {
    DaemonInfo(crate::domain::daemon::DaemonInfo),
    Terminal(crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem),
    RepositoryPaths(Vec<String>),
    RepositoryGroupState(bool),
    Workspaces(WorkspaceList),
    /// 実行木と、選択している Node が画面に出す木にあるか。
    Selection(crate::domain::workspace_tree::WorkspaceTree, bool),
    NodeDetail(Option<WorkspaceNodeDetailDto>),
    AgentSession(Option<AgentSessionItemDto>),
    SessionHistory(AgentSessionHistoryPageDto),
    Providers(Vec<AgentSessionProviderDto>),
    Branches(Vec<BranchDto>),
    BranchBase(Option<String>),
    /// ブランチと、その worktree があるか。
    BranchStatus(Vec<(crate::domain::repository::Branch, bool)>),
    CurrentBranch(String),
    Issues(
        crate::usecase::fetched::Fetched<crate::usecase::git_host::git_host_usecase::IssueListing>,
    ),
    NotionTasks(
        crate::usecase::fetched::Fetched<
            crate::domain::notion::NotionTaskPage,
            crate::usecase::notion::error::NotionUsecaseError,
        >,
    ),
    NotionLabelOptions(
        crate::usecase::fetched::Fetched<
            Vec<crate::domain::notion::NotionLabelOption>,
            crate::usecase::notion::error::NotionUsecaseError,
        >,
    ),
    Worktrees(Vec<WorktreeEntryDto>),
    StartupRepository(Option<crate::usecase::repository_dto::StartupWorktree>),
    WorkspaceState(Option<crate::domain::workspace_state::WorkspaceState>),
    ReviewSnapshot(ReviewSnapshotDto),
    ReviewFileView(ReviewFileViewDto),
    ReviewThreads(Vec<crate::domain::comment::ReviewThread>),
    WorkflowExecution(Option<crate::domain::workflow::ExecutionTree>),
    WorkflowOutput(Option<crate::usecase::workflow::WorkflowGetOutputResult>),
    ReviewSessionThreads(Option<Vec<crate::domain::comment::ReviewThread>>),
    ReviewSessionThread(Option<crate::domain::comment::ReviewThread>),
    ReviewSessionThreadHistory(Option<Vec<crate::domain::comment::ReviewHistoryEntry>>),
    Workflows(Vec<WorkflowSummaryDto>),
    Workflow(Option<WorkflowDto>),
    WorkflowSource(Option<String>),
    Facets(Vec<FacetSummaryDto>),
    Facet(String),
    Diagnostics(DiagnosticReport),
    DesktopSettings(crate::usecase::app_config::query_service::DesktopSettingsDto),
    NotionConfig(Option<crate::usecase::notion::usecase::NotionRepoConfigDto>),
    ProviderAvailability(crate::usecase::agent_session::ProviderAvailabilitySnapshotDto),
    ExternalEditor(crate::usecase::external_editor::dto::ExternalEditorState),
    ReleashBase(Option<String>),
    WorkflowConfig(crate::usecase::app_config::query_service::WorkflowConfigDto),
    ProviderHookHealth(crate::usecase::provider_lifecycle::ProviderHookHealthReadResult),
}
