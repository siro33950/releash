use crate::domain::agent_session::aggregates::AgentSessionAvailability;
use crate::usecase::{
    agent_session::{AgentSessionHistoryPageDto, AgentSessionItemDto},
    provider_dto::AgentSessionProviderDto,
    repository_dto::BranchDto,
    workflow::{diagnostic_dto::DiagnosticReport, WorkspaceNodeDetailDto},
    workspace_tree::WorkspaceList,
};

#[derive(Debug, Clone, PartialEq)]
pub enum StateValue {
    DaemonInfo(crate::domain::daemon::DaemonInfo),
    Terminal(crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem),
    RepositoryGroupState(bool),
    Workspaces(WorkspaceList),
    /// 全 worktree の実行木の一覧と集約状態。
    ExecutionOverview(
        (
            Vec<crate::domain::workspace_tree::WorkspaceTree>,
            Vec<(String, crate::domain::workflow::WorkflowError)>,
        ),
    ),
    NodeDetail(Option<WorkspaceNodeDetailDto>),
    SessionTab {
        session: Option<(AgentSessionItemDto, AgentSessionAvailability)>,
        node: Option<Box<(crate::domain::workspace_tree::WorkspaceTreeNode, bool)>>,
    },
    SessionHistory(AgentSessionHistoryPageDto),
    Providers(Vec<AgentSessionProviderDto>),
    Branches(Vec<BranchDto>),
    /// ブランチと、その worktree があるか。
    BranchStatus(Vec<(crate::domain::git::Branch, bool)>),
    Issues(
        crate::usecase::fetched::Fetched<crate::usecase::git_host::git_host_usecase::IssueListing>,
    ),
    NotionTasks(
        crate::usecase::fetched::Fetched<
            crate::domain::notion::NotionTaskPage,
            crate::usecase::notion::error::NotionUsecaseError,
        >,
    ),
    WorkspaceState(
        crate::domain::local_event::StreamVersion,
        Option<crate::domain::workspace_state::WorkspaceState>,
        Vec<crate::usecase::workspace_tree::query_service::SessionNodeResolutionFailureDto>,
        Option<crate::domain::workspace_state::WorkspaceStateError>,
    ),
    Directory(Vec<crate::usecase::directory_query::DirectoryEntryDto>),
    FileTab(crate::usecase::code_dto::FileTabViewDto),
    FileThreads(Vec<crate::domain::comment::Thread>),
    ReviewThreads(Vec<crate::domain::comment::Thread>),
    WorkflowExecution(Option<crate::domain::workflow::ExecutionTree>),
    WorkflowOutput(Option<crate::usecase::workflow::WorkflowGetOutputResult>),
    ReviewSessionThreads(Option<Vec<crate::domain::comment::Thread>>),
    ReviewSessionThread(Option<crate::domain::comment::Thread>),
    ReviewSessionThreadHistory(Option<Vec<crate::domain::comment::ReviewHistoryEntry>>),
    Workflows(Vec<crate::usecase::workflow::WorkflowSummaryDto>),
    Workflow(Option<crate::domain::workflow::WorkflowDefinition>),
    Facets(Vec<crate::usecase::workflow::FacetSummaryDto>),
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
