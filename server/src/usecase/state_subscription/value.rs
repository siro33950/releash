use crate::domain::agent_session::aggregates::AgentSessionAvailability;
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
    AgentSession(Option<(AgentSessionItemDto, AgentSessionAvailability)>),
    SessionTab {
        session: Option<(AgentSessionItemDto, AgentSessionAvailability)>,
        node: Option<Box<(crate::domain::workspace_tree::WorkspaceTreeNode, bool)>>,
    },
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
    WorkspaceState(
        Option<crate::domain::workspace_state::WorkspaceState>,
        Vec<crate::usecase::workspace_tree::query_service::SessionNodeResolutionFailureDto>,
        Option<crate::domain::workspace_state::WorkspaceStateError>,
    ),
    Directory(Vec<crate::usecase::directory_query::DirectoryEntryDto>),
    FileTab(crate::usecase::code_dto::FileTabViewDto),
    ReviewSnapshot(ReviewSnapshotDto),
    ReviewFileView(ReviewFileViewDto),
    FileThreads(Vec<crate::domain::comment::Thread>, u32),
    ReviewThreads(Vec<crate::domain::comment::Thread>),
    WorkflowExecution(Option<crate::domain::workflow::ExecutionTree>),
    WorkflowOutput(Option<crate::usecase::workflow::WorkflowGetOutputResult>),
    ReviewSessionThreads(Option<Vec<crate::domain::comment::Thread>>),
    ReviewSessionThread(Option<crate::domain::comment::Thread>),
    ReviewSessionThreadHistory(Option<Vec<crate::domain::comment::ReviewHistoryEntry>>),
    Workflows(Vec<WorkflowSummaryDto>),
    Workflow(Option<WorkflowDto>),
    WorkflowSource(Option<String>),
    Facets(Vec<FacetSummaryDto>),
    Facet(String),
    Diagnostics(DiagnosticReport),
    DesktopSettings(crate::domain::app_config::AppConfig),
    NotionConfig(Option<crate::usecase::notion::usecase::NotionRepoConfigDto>),
    ProviderAvailability(crate::usecase::agent_session::ProviderAvailabilitySnapshotDto),
    ExternalEditor(crate::usecase::external_editor::dto::ExternalEditorState),
    ReleashBase(Option<String>),
    TerminalStartupCommand(String),
    WorkflowConfig(crate::domain::app_config::value_objects::WorkflowConfig),
    ProviderUsage(Vec<crate::usecase::provider_usage::ProviderUsageDto>),
    ProviderHookHealth(crate::usecase::provider_lifecycle::ProviderHookHealthReadResult),
}
