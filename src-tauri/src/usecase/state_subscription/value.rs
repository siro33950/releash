use crate::usecase::{
    agent_session::{AgentSessionHistoryPageDto, AgentSessionItemDto, AgentSessionProviderDto},
    git_host::IssueInfoDto,
    repository_dto::{BranchDto, WorktreeEntryDto},
    repository_state::snapshot::RepositoryBranchCardsSnapshotDto,
    workflow::{WorkspaceNodeDetailDto, WorkspaceTreeSelectionSnapshotDto},
    workspace_state::dto::WorkspaceStateDto,
    workspace_tree::WorkspaceListSnapshotDto,
};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum StateValue {
    Failures(crate::usecase::work_queue::FailurePage),
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
}
