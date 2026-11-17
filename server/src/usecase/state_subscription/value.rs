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
    /// Archive 済みの実行。時刻の新しい順に並ぶ。
    ArchivedHistory(Vec<crate::domain::workspace_tree::ArchivedExecution>),
    Providers(Vec<AgentSessionProviderDto>),
    Branches(Vec<BranchDto>),
    /// ブランチと、その worktree があるか。
    BranchStatus(Vec<(crate::domain::git::Branch, bool)>),
    /// 新しい worktree のブランチを作る元にできるブランチと、デフォルトブランチ。
    WorktreeBases(Vec<crate::domain::git::Branch>, Option<String>),
    Issues(
        crate::usecase::fetched::Fetched<crate::usecase::git_host::git_host_usecase::IssueListing>,
    ),
    /// worktree のブランチの PR。
    PullRequest(crate::usecase::fetched::Fetched<crate::domain::git_host::BranchPullRequest>),
    /// PR の base に選べるブランチ。
    PullRequestBases(crate::domain::git_host::PullRequestBases),
    /// GitHub Actions の job のログの全体。
    CheckLog(crate::usecase::fetched::Fetched<String>),
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
    WorkspaceState(
        crate::domain::local_event::StreamVersion,
        Option<crate::domain::workspace_state::WorkspaceState>,
        Vec<crate::usecase::workspace_tree::query_service::SessionNodeResolutionFailureDto>,
        Option<crate::domain::workspace_state::WorkspaceStateError>,
    ),
    Directory(Vec<crate::usecase::directory_query::DirectoryEntryDto>),
    FilteredFiles(crate::domain::code::value_objects::file_list::FilteredFiles),
    FileTab(crate::usecase::code_dto::FileTabViewDto),
    /// worktree の Git の状態と、表示に使う変更の行数。
    SourceControl(
        crate::domain::git::WorktreeStatus,
        crate::domain::git::LineChanges,
    ),
    FileThreads(Vec<crate::domain::comment::Thread>),
    ReviewThreads(Vec<crate::domain::comment::Thread>),
    /// Thread と、その対象の行の差分。差分を読めなかった Thread は、その失敗を持つ。
    ThreadExcerpts(
        Vec<(
            crate::domain::comment::Thread,
            Result<Vec<crate::domain::comment::ExcerptLine>, crate::domain::failure::WorkFailure>,
        )>,
    ),
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
