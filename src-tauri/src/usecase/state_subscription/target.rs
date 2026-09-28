#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum SubscriptionTarget {
    Failures(String, usize),
    Terminal(crate::domain::terminal_surface::TerminalSurfaceOwner),
    RepositoryPaths,
    Workspaces,
    Selection(String, String),
    NodeDetail(String, String),
    AgentSession(String),
    SessionNode(String, String),
    SessionHistory(String, usize),
    Providers,
    Branches(String, Option<String>),
    BranchBase(String, String),
    BranchStatus(String),
    CurrentBranch(String),
    Issues(String),
    Worktrees(String),
    RepositoryRoot(String),
    StartupRepository,
    WorkspaceState(String, String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum WatchRequirement {
    Git(String),
    Files(String),
}

impl SubscriptionTarget {
    pub fn watches(
        &self,
        repositories: &[String],
        history_paths: &[String],
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
            | Self::RepositoryRoot(path) => vec![WatchRequirement::Git(path.clone())],
            Self::SessionHistory(_, _) => history_paths
                .iter()
                .cloned()
                .map(WatchRequirement::Files)
                .collect(),
            _ => vec![],
        }
    }

    pub fn external_information(&self) -> bool {
        matches!(self, Self::Workspaces | Self::Issues(_))
    }
}

#[cfg(test)]
#[path = "target_test.rs"]
mod subscription_target_tests;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum StateChangeSource {
    Failures(String),
    Repositories,
    Repository(Vec<String>),
    Worktree(String),
    WorkspaceList,
    WorkspaceState(String),
    Providers,
    Issues(String),
    ProviderHistory,
}

impl SubscriptionTarget {
    pub fn affected_by(&self, change: &StateChangeSource) -> bool {
        use StateChangeSource as C;
        match change {
            C::Failures(target) => {
                matches!(self, Self::Failures(id, _) if id == target || id == "*")
            }
            C::Repositories => matches!(self, Self::Workspaces),
            C::Repository(paths) => match self {
                Self::Workspaces => true,
                Self::Branches(p, _)
                | Self::BranchBase(p, _)
                | Self::BranchStatus(p)
                | Self::CurrentBranch(p)
                | Self::Worktrees(p)
                | Self::RepositoryRoot(p) => paths.contains(p),
                _ => false,
            },
            C::Worktree(path) => match self {
                Self::Workspaces | Self::AgentSession(_) => true,
                Self::Selection(p, _)
                | Self::NodeDetail(p, _)
                | Self::SessionNode(p, _)
                | Self::SessionHistory(p, _) => p == path,
                _ => false,
            },
            C::WorkspaceList => matches!(self, Self::Workspaces),
            C::WorkspaceState(name) => matches!(self, Self::WorkspaceState(n, _) if n == name),
            C::ProviderHistory => matches!(self, Self::SessionHistory(_, _)),
            C::Providers => matches!(self, Self::Providers),
            C::Issues(path) => matches!(self, Self::Issues(p) if p == path),
        }
    }
}
