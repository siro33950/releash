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
pub struct WorkspaceList {
    pub repositories: Vec<WorkspaceListRepository>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceListRepository {
    pub path: String,
    pub worktrees: Fetched<Vec<WorkspaceListWorktree>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceListWorktree {
    pub tracking: Fetched<Option<crate::domain::repository::BranchTracking>>,
    pub worktree: Worktree,
    pub deleting: bool,
    pub dirty_count: Fetched<usize>,
    /// PR の状態を合わせた merge 済み。
    pub merged: bool,
    pub pull_request: Option<PrInfo>,
    pub pull_request_loaded: bool,
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
    tracking: Fetched<Option<crate::domain::repository::BranchTracking>>,
    worktree: Worktree,
    deleting: bool,
    dirty_count: Fetched<usize>,
}

/// Workspaces の一覧。値は持たず、読むときに持ち主から集める。
#[derive(Clone)]
pub struct WorkspaceListUsecase {
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
                                tracking: match self
                                    .repository
                                    .branch_tracking(&path, &worktree.branch)
                                {
                                    Ok(value) => Fetched::ready(value),
                                    Err(error) => Fetched {
                                        value: None,
                                        error: Some(
                                            crate::domain::failure::WorkFailure::from_error(&error),
                                        ),
                                    },
                                },
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
    pub async fn refresh(&self) {
        let paths = self.repositories.get();
        futures_util::future::join_all(paths.iter().map(|path| async move {
            if let Err(error) = self.repository_state.rescan(path).await {
                log::warn!("workspace repository rescan failed for {path}: {error}");
            }
        }))
        .await;
        self.refresh_pull_requests().await;
    }

    pub async fn refresh_pull_requests(&self) {
        futures_util::future::join_all(self.repositories.get().into_iter().map(
            |path| async move {
                if let Err(error) = self.git_host.refresh_pr_status(&path).await {
                    log::warn!("workspace PR status refresh failed for {path}: {error}");
                }
            },
        ))
        .await;
    }

    /// 監視する Repository と worktree。
    pub fn watch_paths(
        &self,
    ) -> (
        Vec<String>,
        Vec<(String, crate::domain::failure::WorkFailure)>,
    ) {
        let mut failures = Vec::new();
        let mut paths = std::collections::HashSet::new();
        for path in self.repositories.get() {
            let worktrees = self.repository_state.worktrees(&path);
            if let Some(error) = worktrees.error {
                failures.push((path.clone(), error));
            }
            paths.extend(
                worktrees
                    .value
                    .into_iter()
                    .flatten()
                    .map(|worktree| worktree.path),
            );
            match self.repository_state.repository_root(&path) {
                Ok(root) => {
                    paths.insert(root);
                }
                Err(error) => failures.push((
                    path.clone(),
                    crate::domain::failure::WorkFailure::from_error(&error),
                )),
            }
            paths.insert(path);
        }
        (paths.into_iter().collect(), failures)
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
                                        tracking: values.tracking,
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
                                            .and_then(|prs| prs.for_branch(branch).cloned()),
                                        pull_request_loaded: pull_requests.loaded(),
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

#[cfg(feature = "test-support")]
impl WorkspaceListUsecase {
    pub fn test_replace_repositories(&mut self, repositories: Arc<RepoPathsUsecase>) {
        self.repositories = repositories;
    }

    pub fn test_replace_repository_state(&mut self, repository_state: Arc<RepositoryStateService>) {
        self.repository_state = repository_state;
    }
}
