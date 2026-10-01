use std::sync::Arc;

use crate::domain::git_host::{PrInfo, PrStatus};
use crate::domain::repository::Worktree;
use crate::domain::workspace_tree::WorkspaceTree;
use crate::usecase::{
    fetched::Fetched, git_host::GitHostUsecase, repo_paths_usecase::RepoPathsUsecase,
    repository_state::RepositoryStateService, repository_usecase::RepositoryUsecase,
    workflow::WorkflowUsecase,
};

/// Workspaces の購読で配信する値。持ち主から集めた値を、そのまま並べる。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WorkspaceList {
    pub repositories: Vec<WorkspaceListRepository>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WorkspaceListRepository {
    pub path: String,
    pub worktrees: Fetched<Vec<WorkspaceListWorktree>>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WorkspaceListWorktree {
    pub worktree: Worktree,
    pub deleting: bool,
    pub dirty_count: Fetched<usize>,
    /// PR の状態を合わせた merge 済み。
    pub merged: bool,
    pub pull_request: Option<PrInfo>,
    pub tree: Fetched<WorkspaceTree>,
    pub pull_request_error: Option<crate::domain::failure::WorkFailure>,
}

/// 1 つの Repository について、持ち主から集めた値。
struct RepositoryValues {
    path: String,
    worktrees: Fetched<Vec<WorktreeValues>>,
    pull_requests: Fetched<PrStatus>,
}

struct WorktreeValues {
    worktree: Worktree,
    deleting: bool,
    dirty_count: Fetched<usize>,
}

/// Workspaces の一覧。値は持たず、読むときに持ち主から集める。
#[derive(Clone)]
pub(crate) struct WorkspaceListUsecase {
    repositories: Arc<RepoPathsUsecase>,
    repository: Arc<RepositoryUsecase>,
    repository_state: Arc<RepositoryStateService>,
    workflow: Arc<WorkflowUsecase>,
    git_host: Arc<GitHostUsecase>,
}

impl WorkspaceListUsecase {
    pub fn new(
        repositories: Arc<RepoPathsUsecase>,
        repository: Arc<RepositoryUsecase>,
        repository_state: Arc<RepositoryStateService>,
        workflow: Arc<WorkflowUsecase>,
        git_host: Arc<GitHostUsecase>,
    ) -> Self {
        Self {
            repositories,
            repository,
            repository_state,
            workflow,
            git_host,
        }
    }

    pub async fn read(&self) -> Result<WorkspaceList, crate::domain::failure::TechnicalFailure> {
        let usecase = self.clone();
        let repositories =
            crate::common::operation_context::spawn_blocking(move || usecase.repository_values())
                .await
                .map_err(|error| crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: error.to_string(),
                })?;
        let worktree_paths = repositories
            .iter()
            .flat_map(|repository| repository.worktrees.value.iter().flatten())
            .map(|values| values.worktree.path.clone())
            .collect::<Vec<_>>();
        let trees = self
            .workflow
            .retained_workspace_trees(&worktree_paths)
            .await;
        Ok(compose(repositories, trees))
    }

    fn repository_values(&self) -> Vec<RepositoryValues> {
        self.repositories
            .get()
            .into_iter()
            .map(|path| {
                let scanned = self.repository_state.worktrees(&path);
                let root = self.repository_state.repository_root(&path).ok();
                let worktrees = Fetched {
                    value: scanned.value.zip(root).map(|(worktrees, root)| {
                        self.repository
                            .with_deleting_worktrees(&root, worktrees)
                            .into_iter()
                            .map(|(worktree, deleting)| WorktreeValues {
                                dirty_count: self.repository_state.dirty_count(&worktree.path),
                                worktree,
                                deleting,
                            })
                            .collect()
                    }),
                    error: scanned.error,
                };
                RepositoryValues {
                    pull_requests: self.git_host.pr_status_result(&path),
                    path,
                    worktrees,
                }
            })
            .collect()
    }

    /// 全 Repository の worktree の並びと変更の状態を読み直し、PR を取り直す。
    /// 走査の終わりまで待つ。PR は待たず、取れた時点で購読へ届く。
    pub async fn refresh(&self) {
        let paths = self.repositories.get();
        futures_util::future::join_all(paths.iter().map(|path| async move {
            if let Err(error) = self.repository_state.rescan(path).await {
                log::warn!("workspace repository rescan failed for {path}: {error}");
            }
        }))
        .await;
        self.refresh_pull_requests();
    }

    /// 全 Repository の PR を取り直す。取り終わるのを待たない。
    pub fn refresh_pull_requests(&self) {
        for path in self.repositories.get() {
            let git_host = self.git_host.clone();
            tokio::task::spawn_blocking(move || {
                if let Err(error) = git_host.refresh_pr_status(&path) {
                    log::warn!("workspace PR status refresh failed for {path}: {error}");
                }
            });
        }
    }

    /// 監視する Repository と worktree。
    pub fn watch_paths(&self) -> Vec<String> {
        let mut paths = std::collections::HashSet::new();
        for path in self.repositories.get() {
            paths.extend(
                self.repository_state
                    .worktrees(&path)
                    .value
                    .into_iter()
                    .flatten()
                    .map(|worktree| worktree.path),
            );
            // worktree の並びは root の監視が持つ。登録パスが root でないときも root を監視する。
            paths.extend(self.repository_state.repository_root(&path).ok());
            paths.insert(path);
        }
        paths.into_iter().collect()
    }
}

/// 集めた値を一覧に並べる。`trees` は、読めている worktree の並び順に対応する。
fn compose(
    repositories: Vec<RepositoryValues>,
    trees: Vec<Fetched<WorkspaceTree>>,
) -> WorkspaceList {
    let mut trees = trees.into_iter();
    WorkspaceList {
        repositories: repositories
            .into_iter()
            .map(|repository| {
                let pull_requests = repository.pull_requests;
                WorkspaceListRepository {
                    path: repository.path,
                    worktrees: Fetched {
                        error: repository.worktrees.error,
                        value: repository.worktrees.value.map(|worktrees| {
                            worktrees
                                .into_iter()
                                .map(|values| {
                                    let branch = values.worktree.branch.as_str();
                                    WorkspaceListWorktree {
                                        merged: pull_requests.value.as_ref().map_or(
                                            values.worktree.is_merged,
                                            |prs| {
                                                prs.branch_is_merged(
                                                    branch,
                                                    values.worktree.is_merged,
                                                )
                                            },
                                        ),
                                        pull_request: pull_requests
                                            .value
                                            .as_ref()
                                            .and_then(|prs| prs.open_prs.get(branch).cloned()),
                                        pull_request_error: pull_requests.error.clone(),
                                        tree: trees.next().unwrap_or_default(),
                                        deleting: values.deleting,
                                        dirty_count: values.dirty_count,
                                        worktree: values.worktree,
                                    }
                                })
                                .collect()
                        }),
                    },
                }
            })
            .collect(),
    }
}

#[cfg(test)]
#[path = "list_test.rs"]
mod list_tests;
