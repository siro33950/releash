use super::SubscriptionError;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum SubscriptionTarget {
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
    pub fn parse(raw: &str) -> Result<Self, SubscriptionError> {
        let (name, mut rest) = raw.split_once(':').unwrap_or((raw, ""));
        let mut args = Vec::new();
        while !rest.is_empty() {
            let (length, tail) = rest.split_once(':').ok_or(SubscriptionError::InvalidId)?;
            let length: usize = length.parse().map_err(|_| SubscriptionError::InvalidId)?;
            let (arg, tail) = tail
                .split_at_checked(length)
                .ok_or(SubscriptionError::InvalidId)?;
            args.push(arg);
            rest = tail;
        }
        let target = Self::from_parts(name, &args)?;
        if target.to_string() != raw {
            return Err(SubscriptionError::InvalidId);
        }
        Ok(target)
    }

    pub fn from_parts(name: &str, args: &[&str]) -> Result<Self, SubscriptionError> {
        if args
            .iter()
            .any(|arg| arg.trim().is_empty() || arg.contains('\0'))
        {
            return Err(SubscriptionError::InvalidId);
        }
        let target = match (name, args) {
            ("repository-paths", []) => Ok(Self::RepositoryPaths),
            ("workspaces", []) => Ok(Self::Workspaces),
            ("selection", [path, id]) => Ok(Self::Selection((*path).into(), (*id).into())),
            ("node-detail", [path, id]) => Ok(Self::NodeDetail((*path).into(), (*id).into())),
            ("agent-session", [id]) => Ok(Self::AgentSession((*id).into())),
            ("session-node", [path, id]) => Ok(Self::SessionNode((*path).into(), (*id).into())),
            ("session-history", [path, count]) => {
                let parsed: usize = count.parse().map_err(|_| SubscriptionError::InvalidId)?;
                if parsed == 0 || parsed.to_string() != *count {
                    return Err(SubscriptionError::InvalidId);
                }
                Ok(Self::SessionHistory((*path).into(), parsed))
            }
            ("providers", []) => Ok(Self::Providers),
            ("branches", [path]) => Ok(Self::Branches((*path).into(), None)),
            ("branches", [path, excluded]) => {
                Ok(Self::Branches((*path).into(), Some((*excluded).into())))
            }
            ("branch-base", [path, name]) => Ok(Self::BranchBase((*path).into(), (*name).into())),
            ("branch-status", [path]) => Ok(Self::BranchStatus((*path).into())),
            ("current-branch", [path]) => Ok(Self::CurrentBranch((*path).into())),
            ("issues", [path]) => Ok(Self::Issues((*path).into())),
            ("worktrees", [path]) => Ok(Self::Worktrees((*path).into())),
            ("repository-root", [path]) => Ok(Self::RepositoryRoot((*path).into())),
            ("startup-repository", []) => Ok(Self::StartupRepository),
            ("workspace-state", [name, path]) => {
                Ok(Self::WorkspaceState((*name).into(), (*path).into()))
            }
            _ => Err(SubscriptionError::UnknownTarget),
        }?;
        Ok(target)
    }

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

impl SubscriptionTarget {
    pub fn parts(&self) -> (&'static str, Vec<String>) {
        match self {
            Self::RepositoryPaths => ("repository-paths", vec![]),
            Self::Workspaces => ("workspaces", vec![]),
            Self::Selection(p, id) => ("selection", vec![p.clone(), id.clone()]),
            Self::NodeDetail(p, id) => ("node-detail", vec![p.clone(), id.clone()]),
            Self::AgentSession(id) => ("agent-session", vec![id.clone()]),
            Self::SessionNode(p, id) => ("session-node", vec![p.clone(), id.clone()]),
            Self::SessionHistory(p, count) => {
                ("session-history", vec![p.clone(), count.to_string()])
            }
            Self::Providers => ("providers", vec![]),
            Self::Branches(p, excluded) => (
                "branches",
                std::iter::once(p.clone())
                    .chain(excluded.iter().cloned())
                    .collect(),
            ),
            Self::BranchBase(p, n) => ("branch-base", vec![p.clone(), n.clone()]),
            Self::BranchStatus(p) => ("branch-status", vec![p.clone()]),
            Self::CurrentBranch(p) => ("current-branch", vec![p.clone()]),
            Self::Issues(p) => ("issues", vec![p.clone()]),
            Self::Worktrees(p) => ("worktrees", vec![p.clone()]),
            Self::RepositoryRoot(p) => ("repository-root", vec![p.clone()]),
            Self::StartupRepository => ("startup-repository", vec![]),
            Self::WorkspaceState(n, p) => ("workspace-state", vec![n.clone(), p.clone()]),
        }
    }
}

impl std::fmt::Display for SubscriptionTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (name, args) = self.parts();
        write!(f, "{name}")?;
        if !args.is_empty() {
            write!(f, ":")?;
        }
        for arg in args {
            write!(f, "{}:{arg}", arg.len())?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "subscription_target_test.rs"]
mod subscription_target_tests;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum StateChangeSource {
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
