use crate::domain::code::{ReviewBase, ReviewSection};
use crate::domain::workflow::FacetKind;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SubscriptionTarget {
    Terminal(crate::domain::terminal_surface::TerminalSurfaceOwner),
    RepositoryPaths,
    Workspaces,
    Selection(String, String),
    NodeDetail(String, String),
    AgentSession(String),
    SessionHistory(String, usize),
    Providers,
    Branches(String, Option<String>),
    BranchBase(String, String),
    BranchStatus(String),
    CurrentBranch(String),
    Issues(String),
    NotionTasks(crate::usecase::notion::usecase::NotionTaskListRequest),
    NotionLabelOptions(String),
    Worktrees(String),
    StartupRepository,
    WorkspaceState(String, String),
    ReviewSnapshot(String, ReviewBase),
    ReviewFileView(String, String, ReviewSection, ReviewBase),
    ReviewThreads(String),
    Workflows,
    Workflow(String),
    WorkflowSource(String),
    Facets(FacetKind),
    Facet(FacetKind, String),
    Diagnostics,
    DesktopSettings,
    NotionConfig(String),
    ProviderAvailability,
    ExternalEditor,
    ReleashBase(String),
    WorkflowConfig,
    ProviderHookHealth,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WatchRequirement {
    Git(String),
    Files(String, StateChangeSource),
}

impl SubscriptionTarget {
    pub fn watches(
        &self,
        repositories: &[String],
        history_paths: &[String],
        review_comments_dir: &str,
        workflows_dir: &str,
        hook_health_markers: &str,
    ) -> Vec<WatchRequirement> {
        match self {
            Self::Workspaces => repositories
                .iter()
                .cloned()
                .map(WatchRequirement::Git)
                .collect(),
            Self::Branches(path, _)
            | Self::BranchBase(path, _)
            | Self::BranchStatus(path)
            | Self::CurrentBranch(path)
            | Self::Worktrees(path)
            | Self::ReviewSnapshot(path, _)
            | Self::ReviewFileView(path, _, _, _)
            | Self::ReleashBase(path) => vec![WatchRequirement::Git(path.clone())],
            Self::SessionHistory(_, _) => history_paths
                .iter()
                .cloned()
                .map(|path| WatchRequirement::Files(path, StateChangeSource::ProviderHistory))
                .collect(),
            Self::ReviewThreads(_) => vec![WatchRequirement::Files(
                review_comments_dir.into(),
                StateChangeSource::ReviewComments(None),
            )],
            Self::Workflows
            | Self::Workflow(_)
            | Self::WorkflowSource(_)
            | Self::Facets(_)
            | Self::Facet(_, _)
            | Self::Diagnostics => vec![WatchRequirement::Files(
                workflows_dir.to_string(),
                StateChangeSource::WorkflowDefinitions,
            )],
            Self::ProviderHookHealth => vec![WatchRequirement::Files(
                hook_health_markers.to_string(),
                StateChangeSource::ProviderHookHealth,
            )],
            _ => vec![],
        }
    }

    pub fn external_information(&self) -> bool {
        matches!(
            self,
            Self::Workspaces
                | Self::Issues(_)
                | Self::NotionTasks(..)
                | Self::NotionLabelOptions(_)
        )
    }
}

#[cfg(test)]
#[path = "target_test.rs"]
mod subscription_target_tests;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StateChangeSource {
    Repositories,
    Repository(Vec<String>),
    Worktree(String),
    WorkspaceList,
    WorkspaceState(String),
    Providers,
    Issues(String),
    ProviderHistory,
    /// review comment の変化。`None` は worktree を特定できない外部の書き込み。
    ReviewComments(Option<String>),
    WorkflowDefinitions,
    AppConfig,
    NotionConfig(String),
    ProviderHookHealth,
}

impl SubscriptionTarget {
    pub fn affected_by(&self, change: &StateChangeSource) -> bool {
        use StateChangeSource as C;
        match change {
            C::Repositories => matches!(self, Self::RepositoryPaths | Self::Workspaces),
            C::Repository(paths) => match self {
                Self::Workspaces => true,
                Self::Branches(p, _)
                | Self::BranchBase(p, _)
                | Self::BranchStatus(p)
                | Self::CurrentBranch(p)
                | Self::Worktrees(p)
                | Self::ReviewSnapshot(p, _)
                | Self::ReviewFileView(p, _, _, _)
                | Self::ReleashBase(p) => paths.contains(p),
                _ => false,
            },
            C::Worktree(path) => match self {
                Self::Workspaces | Self::AgentSession(_) | Self::Workflows => true,
                Self::Selection(p, _) | Self::NodeDetail(p, _) | Self::SessionHistory(p, _) => {
                    p == path
                }
                _ => false,
            },
            C::WorkspaceList => matches!(self, Self::Workspaces),
            C::WorkspaceState(name) => matches!(self, Self::WorkspaceState(n, _) if n == name),
            C::ProviderHistory => matches!(self, Self::SessionHistory(_, _)),
            C::Providers => matches!(self, Self::Providers | Self::ProviderAvailability),
            C::Issues(path) => matches!(self, Self::Issues(p) if p == path),
            C::ReviewComments(worktree) => {
                matches!(self, Self::ReviewThreads(name) if worktree.as_ref().is_none_or(|w| w == name))
            }
            C::WorkflowDefinitions => matches!(
                self,
                Self::Workflows
                    | Self::Workflow(_)
                    | Self::WorkflowSource(_)
                    | Self::Facets(_)
                    | Self::Facet(_, _)
                    | Self::Diagnostics
            ),
            C::NotionConfig(path) => match self {
                Self::NotionTasks(request) => request.path == *path,
                Self::NotionLabelOptions(p) => p == path,
                _ => false,
            },
            C::AppConfig => matches!(
                self,
                Self::DesktopSettings
                    | Self::NotionConfig(_)
                    | Self::ExternalEditor
                    | Self::WorkflowConfig
            ),
            C::ProviderHookHealth => matches!(self, Self::ProviderHookHealth),
        }
    }
}
