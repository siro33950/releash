use crate::usecase::{
    agent_session::{AgentSessionHistoryPageDto, AgentSessionItemDto},
    code_dto::{ReviewFileViewDto, ReviewSnapshotDto},
    comment::ReviewThreadDto,
    git_host::IssueInfoDto,
    provider_dto::AgentSessionProviderDto,
    repository_dto::{BranchDto, WorktreeEntryDto},
    repository_state::snapshot::RepositoryBranchCardsSnapshotDto,
    workflow::{
        diagnostic_dto::DiagnosticReport,
        dto::{FacetSummaryDto, WorkflowDto, WorkflowSummaryDto},
        WorkspaceNodeDetailDto, WorkspaceTreeSelectionSnapshotDto,
    },
    workspace_state::dto::WorkspaceStateDto,
    workspace_tree::WorkspaceListSnapshotDto,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TerminalSurfaceSnapshotDto {
    pub session_key: String,
    pub replay: String,
    pub sequence: u64,
    pub cols: u16,
    pub rows: u16,
    pub is_exited: bool,
    pub exit_code: Option<i32>,
    pub label: Option<String>,
}

impl From<crate::domain::terminal_surface::entities::TerminalSurface>
    for TerminalSurfaceSnapshotDto
{
    fn from(surface: crate::domain::terminal_surface::entities::TerminalSurface) -> Self {
        Self {
            session_key: surface.session_key,
            replay: surface.checkpoint.replay,
            sequence: surface.checkpoint.sequence,
            cols: surface.checkpoint.cols,
            rows: surface.checkpoint.rows,
            is_exited: surface.process_state.is_exited(),
            exit_code: surface.process_state.exit_code(),
            label: surface.label,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum StateValue {
    Failures(crate::usecase::failure::FailurePage),
    Terminal(crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem),
    RepositoryPaths(Vec<String>),
    Workspaces(WorkspaceListSnapshotDto),
    Selection(WorkspaceTreeSelectionSnapshotDto),
    NodeDetail(Option<WorkspaceNodeDetailDto>),
    AgentSession(Option<AgentSessionItemDto>),
    SessionNode(Option<String>),
    SessionHistory(AgentSessionHistoryPageDto),
    Providers(Vec<AgentSessionProviderDto>),
    Branches(Vec<BranchDto>),
    BranchBase(Option<String>),
    BranchStatus(RepositoryBranchCardsSnapshotDto),
    CurrentBranch(String),
    Issues(Vec<IssueInfoDto>),
    Worktrees(Vec<WorktreeEntryDto>),
    RepositoryRoot(String),
    StartupRepository(String),
    WorkspaceState(Option<WorkspaceStateDto>),
    ReviewSnapshot(ReviewSnapshotDto),
    ReviewFileView(ReviewFileViewDto),
    ReviewThreads(Vec<ReviewThreadDto>),
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
    PerformanceSwitches(crate::usecase::telemetry::PerformanceSwitches),
    ProviderHookHealth(Vec<crate::usecase::provider_lifecycle::ProviderHookHealthWarningDto>),
    StartupOutcome(crate::usecase::application_startup::ApplicationStartupOutcome),
}
