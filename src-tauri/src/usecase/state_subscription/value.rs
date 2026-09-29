use crate::usecase::{
    agent_session::{AgentSessionHistoryPageDto, AgentSessionItemDto, AgentSessionProviderDto},
    code_dto::{ReviewFileViewDto, ReviewSnapshotDto},
    comment::ReviewThreadDto,
    git_host::IssueInfoDto,
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
use std::sync::Arc;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TerminalSurfaceStreamItemDto {
    Snapshot(TerminalSurfaceSnapshotDto),
    Output {
        session_key: String,
        data: Arc<str>,
        sequence: u64,
    },
    Resize {
        session_key: String,
        cols: u16,
        rows: u16,
        sequence: u64,
    },
    Exit {
        session_key: String,
        exit_code: Option<i32>,
        sequence: u64,
    },
}

impl From<crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem>
    for TerminalSurfaceStreamItemDto
{
    fn from(
        value: crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem,
    ) -> Self {
        use crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem as Item;
        match value {
            Item::Snapshot(surface) => Self::Snapshot(surface.into()),
            Item::Output {
                session_key,
                data,
                sequence,
            } => Self::Output {
                session_key,
                data,
                sequence,
            },
            Item::Resize {
                session_key,
                cols,
                rows,
                sequence,
            } => Self::Resize {
                session_key,
                cols,
                rows,
                sequence,
            },
            Item::Exit {
                session_key,
                exit_code,
                sequence,
            } => Self::Exit {
                session_key,
                exit_code,
                sequence,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum StateValue {
    Failures(crate::usecase::failure::FailurePageDto),
    Terminal(TerminalSurfaceStreamItemDto),
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
