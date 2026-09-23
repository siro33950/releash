use std::sync::Arc;

use parking_lot::Mutex;
use serde::Serialize;

use super::list_query_service::WorkspaceListQueryService;
use crate::domain::workspace_tree::{
    WorkspaceListEntry, WorkspaceListFailure, WorkspaceListRefresh, WorkspaceListState,
};
use crate::usecase::{
    repository_dto::BranchCardDto,
    workflow::{WorkspaceTreeSnapshotDto, WorkspaceWorkflowHistoryItemDto},
};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceListStatusDto {
    pub loaded: bool,
    pub state: &'static str,
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

fn status<T>(list: &WorkspaceListEntry<T>, empty: bool) -> WorkspaceListStatusDto {
    WorkspaceListStatusDto {
        loaded: list.loaded(),
        state: match list.state(empty) {
            WorkspaceListState::Loading => "loading",
            WorkspaceListState::InitialFailed => "initialFailed",
            WorkspaceListState::Empty => "empty",
            WorkspaceListState::Ready => "ready",
            WorkspaceListState::RefreshFailed => "refreshFailed",
        },
        error: list.error().map(str::to_owned),
    }
}

#[derive(Clone)]
pub(crate) struct WorkspaceListUsecase {
    query: Arc<dyn WorkspaceListQueryService>,
    lists: Arc<Mutex<WorkspaceLists>>,
    notify: Arc<dyn Fn() + Send + Sync>,
    completed: tokio::sync::watch::Sender<u64>,
}

impl WorkspaceListUsecase {
    pub fn new(query: Arc<dyn WorkspaceListQueryService>) -> Self {
        Self {
            query,
            lists: Arc::new(Mutex::new(WorkspaceLists::default())),
            notify: Arc::new(|| {}),
            completed: tokio::sync::watch::channel(0).0,
        }
    }

    pub fn with_notifier(mut self, notify: impl Fn() + Send + Sync + 'static) -> Self {
        self.notify = Arc::new(notify);
        self
    }

    pub async fn refresh(&self) -> WorkspaceListSnapshotDto {
        let mut completed = self.completed.subscribe();
        let (request, start) = {
            let mut lists = self.lists.lock();
            (lists.request_full(), lists.start_full())
        };
        if let Some(mut current) = start {
            let usecase = self.clone();
            tokio::spawn(async move {
                loop {
                    usecase.refresh_full(current.1).await;
                    let mut lists = usecase.lists.lock();
                    lists.complete_full(current.0);
                    usecase.completed.send_replace(current.0);
                    match lists.start_full() {
                        Some(next) => current = next,
                        None => break,
                    }
                }
            });
        }
        completed
            .wait_for(|done| *done >= request)
            .await
            .expect("refresh completion sender is owned by this usecase");
        self.snapshot()
    }

    async fn refresh_full(&self, generation: u64) {
        let repositories = self
            .query
            .repositories()
            .map_err(|error| WorkspaceListFailure::from(error.to_string()));
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
            let branches: Vec<_> = branches
                .into_iter()
                .map(|branch| WorkspaceBranchDto {
                    branch,
                    has_pr: false,
                    pr_number: None,
                    pr_url: None,
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
            branches.map_err(|error| WorkspaceListFailure::from(error.to_string())),
        );
        (self.notify)();
        let query = self.query.clone();
        let lists = self.lists.clone();
        let notify = self.notify.clone();
        let repository = path.to_owned();
        tokio::task::spawn_blocking(move || {
            let prs = query.pr_status(&repository);
            let updated = lists
                .lock()
                .update_branches(&repository, generation, |branches| {
                    for branch in branches {
                        let pr = prs.open_prs.get(&branch.branch.name);
                        branch.branch.is_merged =
                            prs.branch_is_merged(&branch.branch.name, branch.branch.is_merged);
                        branch.has_pr = pr.is_some();
                        branch.pr_number = pr.map(|pr| pr.number);
                        branch.pr_url = pr.map(|pr| pr.url.clone());
                    }
                });
            if updated {
                notify();
            }
        });
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
            result.map_err(|error| WorkspaceListFailure::from(error.to_string())),
        );
    }

    pub fn snapshot(&self) -> WorkspaceListSnapshotDto {
        let mut lists = self.lists.lock();
        let generation = lists.next_snapshot_generation();
        let paths = lists.repositories().value().cloned().unwrap_or_default();
        WorkspaceListSnapshotDto {
            generation,
            status: status(lists.repositories(), paths.is_empty()),
            repositories: paths
                .into_iter()
                .map(|path| {
                    let empty = WorkspaceListEntry::default();
                    let branches = lists.branches(&path).unwrap_or(&empty);
                    WorkspaceRepositoryListDto {
                        path,
                        status: status(
                            branches,
                            branches.value().is_none_or(|value| value.0.is_empty()),
                        ),
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
                                    status: status(
                                        nodes,
                                        nodes.value().is_none_or(|value| value.0.nodes.is_empty()),
                                    ),
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
