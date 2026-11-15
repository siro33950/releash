use crate::domain::code::ReviewBase;

use crate::domain::workflow::FacetKind;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SubscriptionTarget {
    DaemonInfo,
    Terminal(crate::domain::terminal_surface::TerminalSurfaceOwner),
    Workspaces,
    /// 全 worktree の実行木の一覧と集約状態。
    ExecutionOverview,
    NodeDetail(String, String),
    SessionTab(String),
    /// worktree の Provider の履歴。出す件数を持つ。
    /// 検索語があれば、題か最初の指示が検索語を含む履歴に絞る。
    SessionHistory(
        String,
        usize,
        Option<crate::domain::search_term::SearchTerm>,
    ),
    /// worktree の Archive 済みの Session と Workflow。
    /// 検索語があれば、Session は名前、Workflow はタイトルが検索語を含む行に絞る。
    ArchivedHistory(String, Option<crate::domain::search_term::SearchTerm>),
    Providers,
    Branches(String, Option<String>),
    /// worktree を作れるローカルのブランチ。検索語があれば、名前が検索語を含むブランチに絞る。
    AvailableBranches(String, Option<crate::domain::search_term::SearchTerm>),
    /// 新しい worktree のブランチを作る元にできるブランチと、デフォルトブランチ。
    WorktreeBases(String),
    Issues(String),
    FilteredIssues(String, crate::domain::git_host::IssueFilter),
    NotionTasks(crate::usecase::notion::usecase::NotionTaskListRequest),
    /// Repository の Notion の設定で Labels に選んだプロパティと、その選択肢。
    NotionLabelOptions(String),
    WorkspaceState(String, String),
    RepositoryGroupState(String),
    Directory(String, String),
    /// worktree の、条件で絞り込んだファイル一覧。
    FilteredFiles(
        String,
        crate::domain::code::value_objects::file_list::FileFilter,
    ),
    FileTab(String, String, ReviewBase),
    FileThreads(String, String),
    /// worktree の Thread の一覧。差分の比較の基準と、解決済みでも対象の行の差分を添える Thread の ID を持つ。
    ReviewThreads(String, ReviewBase, Vec<String>),
    WorkflowExecution(String),
    WorkflowOutput(String, String),
    NodeArtifact(String),
    ReviewSessionThreads(String, crate::domain::comment::ReviewThreadFilter),
    ReviewWorktreeThreads(String, crate::domain::comment::ReviewWorktreeFilter),
    ReviewSessionThread(String, String),
    ReviewSessionThreadHistory(String, String),
    Workflows,
    Workflow(String),
    Facets(FacetKind),
    Diagnostics,
    DesktopSettings,
    NotionConfig(String),
    ProviderAvailability,
    ExternalEditor,
    ReleashBase(String),
    WorkflowConfig,
    TerminalStartupCommand,
    ProviderHookHealth,
    ProviderUsage,
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
            Self::ExecutionOverview => vec![],
            Self::Directory(path, directory) => vec![
                WatchRequirement::Git(path.clone()),
                WatchRequirement::Files(
                    std::path::Path::new(path)
                        .join(directory)
                        .to_string_lossy()
                        .into_owned(),
                    StateChangeSource::Worktree(path.clone()),
                ),
            ],
            Self::FilteredFiles(path, _) => vec![
                WatchRequirement::Git(path.clone()),
                WatchRequirement::Files(path.clone(), StateChangeSource::Worktree(path.clone())),
            ],
            Self::FileTab(path, file, _) => vec![
                WatchRequirement::Git(path.clone()),
                WatchRequirement::Files(
                    std::path::Path::new(path)
                        .join(file)
                        .to_string_lossy()
                        .into_owned(),
                    StateChangeSource::Worktree(path.clone()),
                ),
            ],
            Self::Workspaces => repositories
                .iter()
                .cloned()
                .map(WatchRequirement::Git)
                .collect(),
            Self::Branches(path, _)
            | Self::AvailableBranches(path, _)
            | Self::WorktreeBases(path)
            | Self::ReleashBase(path) => {
                vec![WatchRequirement::Git(path.clone())]
            }
            Self::SessionHistory(..) => history_paths
                .iter()
                .cloned()
                .map(|path| WatchRequirement::Files(path, StateChangeSource::ProviderHistory))
                .collect(),
            Self::ReviewThreads(path, ..) => vec![
                WatchRequirement::Git(path.clone()),
                WatchRequirement::Files(path.clone(), StateChangeSource::Worktree(path.clone())),
                WatchRequirement::Files(
                    review_comments_dir.into(),
                    StateChangeSource::ReviewComments(None),
                ),
            ],
            Self::FileThreads(..)
            | Self::ReviewSessionThreads(..)
            | Self::ReviewWorktreeThreads(..)
            | Self::ReviewSessionThread(..)
            | Self::ReviewSessionThreadHistory(..) => vec![WatchRequirement::Files(
                review_comments_dir.into(),
                StateChangeSource::ReviewComments(None),
            )],
            Self::Workflows | Self::Workflow(_) | Self::Facets(_) | Self::Diagnostics => {
                vec![WatchRequirement::Files(
                    workflows_dir.to_string(),
                    StateChangeSource::WorkflowDefinitions,
                )]
            }
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
                | Self::FilteredIssues(..)
                | Self::NotionTasks(..)
                | Self::NotionLabelOptions(_)
        )
    }
}

#[cfg(test)]
#[path = "target_test.rs"]
mod target_tests;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StateChangeSource {
    Daemon,
    Repositories,
    Repository(Vec<String>),
    Worktree(String),
    WorkspaceList,
    WorkspaceState(String),
    RepositoryGroupState(String),
    Providers,
    Issues(String),
    ProviderHistory,
    /// review comment の変化。`None` は worktree を特定できない外部の書き込み。
    ReviewComments(Option<String>),
    WorkflowDefinitions,
    AppConfig,
    NotionConfig(String),
    ProviderHookHealth,
    ProviderUsage,
}

impl SubscriptionTarget {
    pub fn affected_by(&self, change: &StateChangeSource) -> bool {
        if matches!(self, Self::ExecutionOverview) {
            return matches!(
                change,
                StateChangeSource::Worktree(_) | StateChangeSource::WorkspaceList
            );
        }
        use StateChangeSource as C;
        match change {
            C::RepositoryGroupState(path) => {
                matches!(self, Self::RepositoryGroupState(p) if p == path)
            }
            C::Daemon => matches!(self, Self::DaemonInfo),
            C::Repositories => matches!(self, Self::Workspaces),
            C::Repository(paths) => match self {
                Self::Workspaces => true,
                Self::Branches(p, _)
                | Self::AvailableBranches(p, _)
                | Self::WorktreeBases(p)
                | Self::Directory(p, _)
                | Self::FilteredFiles(p, _)
                | Self::FileTab(p, _, _)
                | Self::ReviewThreads(p, ..)
                | Self::ReleashBase(p) => paths.contains(p),
                _ => false,
            },
            C::Worktree(path) => match self {
                Self::Workspaces
                | Self::SessionTab(_)
                | Self::Workflows
                | Self::WorkflowExecution(_)
                | Self::WorkflowOutput(..)
                | Self::NodeArtifact(_) => true,
                Self::Directory(p, _)
                | Self::FilteredFiles(p, _)
                | Self::FileTab(p, _, _)
                | Self::ReviewThreads(p, ..)
                | Self::NodeDetail(p, _)
                | Self::SessionHistory(p, ..)
                | Self::ArchivedHistory(p, _) => p == path,
                _ => false,
            },
            C::WorkspaceList => matches!(self, Self::Workspaces | Self::SessionTab(_)),
            C::WorkspaceState(name) => matches!(self, Self::WorkspaceState(n, _) if n == name),
            C::ProviderHistory => matches!(self, Self::SessionHistory(..)),
            C::Providers => matches!(self, Self::Providers | Self::ProviderAvailability),
            C::Issues(path) => {
                matches!(self, Self::Issues(p) | Self::FilteredIssues(p, _) if p == path)
            }
            C::ReviewComments(worktree) => {
                matches!(self, Self::FileThreads(path, _) if worktree.as_ref().is_none_or(|p| p == path))
                    || matches!(
                        self,
                        Self::ReviewSessionThreads(..)
                            | Self::ReviewWorktreeThreads(..)
                            | Self::ReviewSessionThread(..)
                            | Self::ReviewSessionThreadHistory(..)
                    )
                    || matches!(self, Self::ReviewThreads(name, ..) if worktree.as_ref().is_none_or(|w| w == name))
            }
            C::WorkflowDefinitions => matches!(
                self,
                Self::Workflows | Self::Workflow(_) | Self::Facets(_) | Self::Diagnostics
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
                    | Self::TerminalStartupCommand
            ),
            C::ProviderUsage => matches!(self, Self::ProviderUsage),
            C::ProviderHookHealth => matches!(self, Self::ProviderHookHealth),
        }
    }
}
