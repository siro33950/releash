use std::sync::Arc;

use parking_lot::Mutex;
use serde::Serialize;

use super::list_query_service::WorkspaceListQueryService;
use crate::domain::workspace_tree::{WorkspaceListEntry, WorkspaceListRefresh};
use crate::usecase::{
    repository_dto::BranchCardDto,
    workflow::{WorkspaceTreeSnapshotDto, WorkspaceWorkflowHistoryItemDto},
};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceListStatusDto {
    pub loaded: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct WorkspaceBranchDto {
    #[serde(flatten)]
    pub branch: BranchCardDto,
    pub has_pr: bool,
    pub pr_number: Option<u64>,
    pub pr_url: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceWorktreeListDto {
    pub path: String,
    pub status: WorkspaceListStatusDto,
    pub snapshot: Option<WorkspaceTreeSnapshotDto>,
    pub workflow_history: Vec<WorkspaceWorkflowHistoryItemDto>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceRepositoryListDto {
    pub path: String,
    pub status: WorkspaceListStatusDto,
    pub branches: Vec<WorkspaceBranchDto>,
    pub worktrees: Vec<WorkspaceWorktreeListDto>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceListSnapshotDto {
    pub generation: u64,
    pub status: WorkspaceListStatusDto,
    pub repositories: Vec<WorkspaceRepositoryListDto>,
}

type WorkspaceLists = WorkspaceListRefresh<
    Vec<WorkspaceBranchDto>,
    (
        WorkspaceTreeSnapshotDto,
        Vec<WorkspaceWorkflowHistoryItemDto>,
    ),
>;

fn status<T>(list: &WorkspaceListEntry<T>) -> WorkspaceListStatusDto {
    WorkspaceListStatusDto {
        loaded: list.loaded(),
        error: list.error().map(str::to_owned),
    }
}

pub(crate) struct WorkspaceListUsecase {
    query: Arc<dyn WorkspaceListQueryService>,
    lists: Mutex<WorkspaceLists>,
}

impl WorkspaceListUsecase {
    pub fn new(query: Arc<dyn WorkspaceListQueryService>) -> Self {
        Self {
            query,
            lists: Mutex::new(WorkspaceLists::default()),
        }
    }

    pub async fn refresh(&self) -> WorkspaceListSnapshotDto {
        let generation = self.lists.lock().begin();
        let repositories = self.query.repositories().map_err(|error| error.to_string());
        let paths = self
            .lists
            .lock()
            .complete_repositories(generation, repositories);
        futures_util::future::join_all(
            paths
                .iter()
                .map(|path| self.refresh_branches(path, generation)),
        )
        .await;
        self.snapshot()
    }

    pub async fn refresh_repository(&self, path: &str) -> WorkspaceListSnapshotDto {
        let generation = self.lists.lock().begin_repository(path);
        if let Some(generation) = generation {
            self.refresh_branches(path, generation).await;
        }
        self.snapshot()
    }

    async fn refresh_branches(&self, path: &str, generation: u64) {
        let branches = self.query.branches(path).await.map(|branches| {
            let prs = self.query.pr_status(path);
            let branches: Vec<_> = branches
                .into_iter()
                .map(|mut branch| {
                    let pr = prs.open_prs.get(&branch.name);
                    branch.is_merged = prs.branch_is_merged(&branch.name, branch.is_merged);
                    WorkspaceBranchDto {
                        branch,
                        has_pr: pr.is_some(),
                        pr_number: pr.map(|pr| pr.number),
                        pr_url: pr.map(|pr| pr.url.clone()),
                    }
                })
                .collect();
            let paths = branches
                .iter()
                .filter_map(|branch| branch.branch.worktree_path.clone())
                .collect();
            (branches, paths)
        });
        let worktrees = self.lists.lock().complete_branches(
            path,
            generation,
            branches.map_err(|error| error.to_string()),
        );
        for path in worktrees {
            if !self.lists.lock().is_worktree_current(&path, generation) {
                continue;
            }
            self.refresh_nodes(&path, generation);
        }
    }

    pub fn refresh_worktree(&self, path: &str) -> WorkspaceListSnapshotDto {
        let generation = self.lists.lock().begin_worktree(path);
        if let Some(generation) = generation {
            self.refresh_nodes(path, generation);
        }
        self.snapshot()
    }

    fn refresh_nodes(&self, path: &str, generation: u64) {
        let result = self
            .query
            .nodes(path)
            .and_then(|nodes| self.query.history(path).map(|history| (nodes, history)));
        self.lists.lock().complete_worktree(
            path,
            generation,
            result.map_err(|error| error.to_string()),
        );
    }

    fn snapshot(&self) -> WorkspaceListSnapshotDto {
        let mut lists = self.lists.lock();
        let generation = lists.next_snapshot_generation();
        let paths = lists.repositories().value().cloned().unwrap_or_default();
        WorkspaceListSnapshotDto {
            generation,
            status: status(lists.repositories()),
            repositories: paths
                .into_iter()
                .map(|path| {
                    let empty = WorkspaceListEntry::default();
                    let branches = lists.branches(&path).unwrap_or(&empty);
                    WorkspaceRepositoryListDto {
                        path,
                        status: status(branches),
                        branches: branches
                            .value()
                            .map(|value| value.0.clone())
                            .unwrap_or_default(),
                        worktrees: branches
                            .value()
                            .map(|value| value.1.as_slice())
                            .unwrap_or_default()
                            .iter()
                            .map(|path| {
                                let empty = WorkspaceListEntry::default();
                                let nodes = lists.nodes(path).unwrap_or(&empty);
                                WorkspaceWorktreeListDto {
                                    path: path.clone(),
                                    status: status(nodes),
                                    snapshot: nodes.value().map(|value| value.0.clone()),
                                    workflow_history: nodes
                                        .value()
                                        .map(|value| value.1.clone())
                                        .unwrap_or_default(),
                                }
                            })
                            .collect(),
                    }
                })
                .collect(),
        }
    }
}

#[cfg(test)]
#[path = "list_test.rs"]
mod list_tests;
