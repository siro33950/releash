use super::super::list_query_service::WorkspaceListUsecaseError;
use super::*;
use crate::domain::git_host::{PrInfo, PrStatus};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

type NodesHook = Arc<dyn Fn(&str) + Send + Sync>;

struct FakeQuery {
    paths: Mutex<Result<Vec<String>, String>>,
    branches: Mutex<HashMap<String, Result<Vec<BranchCardDto>, String>>>,
    nodes: Mutex<HashMap<String, Result<WorkspaceTreeSnapshotDto, String>>>,
    history: Mutex<HashMap<String, Result<Vec<WorkspaceWorkflowHistoryItemDto>, String>>>,
    prs: Mutex<HashMap<String, Result<PrStatus, String>>>,
    node_reads: Mutex<Vec<String>>,
    on_nodes: Mutex<Option<NodesHook>>,
    on_pr: Mutex<Option<NodesHook>>,
    scans: AtomicUsize,
    delayed_scan: AtomicUsize,
    started: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

fn branch(path: &str, name: &str) -> BranchCardDto {
    BranchCardDto {
        name: name.into(),
        is_deleting: false,
        is_main_worktree: true,
        worktree_path: Some(path.into()),
        dirty_count: 0,
        is_merged: false,
        ahead: 0,
        behind: 0,
        has_upstream: false,
        base_ahead: 0,
    }
}

fn nodes(id: &str) -> WorkspaceTreeSnapshotDto {
    WorkspaceTreeSnapshotDto {
        nodes: vec![],
        archived_sessions: vec![],
        preferred_node_id: Some(id.into()),
    }
}

fn history(path: &str, id: &str) -> Vec<WorkspaceWorkflowHistoryItemDto> {
    vec![WorkspaceWorkflowHistoryItemDto {
        execution_id: id.into(),
        worktree_path: path.into(),
        title: id.into(),
        status: "completed".into(),
        updated_at: 1.0,
        archived_at: 2.0,
        archive_reason: "completed".into(),
    }]
}

impl FakeQuery {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            paths: Mutex::new(Ok(vec!["/a".into(), "/b".into()])),
            branches: Mutex::new(HashMap::from([
                ("/a".into(), Ok(vec![branch("/a", "a")])),
                ("/b".into(), Ok(vec![branch("/b", "b")])),
            ])),
            nodes: Mutex::new(HashMap::from([
                ("/a".into(), Ok(nodes("a"))),
                ("/b".into(), Ok(nodes("b"))),
            ])),
            history: Mutex::new(HashMap::from([
                ("/a".into(), Ok(history("/a", "workflow-a"))),
                ("/b".into(), Ok(history("/b", "workflow-b"))),
            ])),
            prs: Mutex::new(HashMap::new()),
            node_reads: Mutex::new(Vec::new()),
            on_nodes: Mutex::new(None),
            on_pr: Mutex::new(None),
            scans: AtomicUsize::new(0),
            delayed_scan: AtomicUsize::new(usize::MAX),
            started: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        })
    }
}

#[async_trait::async_trait]
impl WorkspaceListQueryService for FakeQuery {
    fn repositories(&self) -> Result<Vec<String>, WorkspaceListUsecaseError> {
        self.paths.lock().clone().map_err(WorkspaceListUsecaseError)
    }
    async fn branches(&self, path: &str) -> Result<Vec<BranchCardDto>, WorkspaceListUsecaseError> {
        let result = self.branches.lock()[path].clone();
        let scan = self.scans.fetch_add(1, Ordering::SeqCst);
        if scan == self.delayed_scan.load(Ordering::SeqCst) {
            self.started.notify_one();
            self.release.notified().await;
        }
        result.map_err(WorkspaceListUsecaseError)
    }
    fn pr_status(&self, path: &str, _force: bool) -> Result<PrStatus, WorkspaceListUsecaseError> {
        let hook = self.on_pr.lock().clone();
        if let Some(hook) = hook {
            hook(path);
        }
        self.prs
            .lock()
            .get(path)
            .cloned()
            .unwrap_or_else(|| Ok(PrStatus::default()))
            .map_err(WorkspaceListUsecaseError)
    }
    async fn nodes(
        &self,
        path: &str,
    ) -> Result<WorkspaceTreeSnapshotDto, WorkspaceListUsecaseError> {
        self.node_reads.lock().push(path.into());
        let result = self.nodes.lock()[path].clone();
        let hook = self.on_nodes.lock().clone();
        if let Some(hook) = hook {
            let path = path.to_string();
            tokio::task::spawn_blocking(move || hook(&path))
                .await
                .unwrap();
        }
        result.map_err(WorkspaceListUsecaseError)
    }
    async fn history(
        &self,
        path: &str,
    ) -> Result<Vec<WorkspaceWorkflowHistoryItemDto>, WorkspaceListUsecaseError> {
        self.history.lock()[path]
            .clone()
            .map_err(WorkspaceListUsecaseError)
    }
}

#[tokio::test]
async fn test_全体更新_毎回全repositoryと子一覧を再取得する() {
    // Given
    let query = FakeQuery::new();
    let usecase = WorkspaceListUsecase::new(query.clone());
    let previous = usecase.refresh().await;
    for repository in &previous.repositories {
        assert_eq!(
            repository.worktrees[0].workflow_history,
            history(
                &repository.path,
                &format!("workflow-{}", &repository.path[1..])
            )
        );
        query.history.lock().insert(
            repository.path.clone(),
            Ok(history(&repository.path, "updated-workflow")),
        );
    }
    query.nodes.lock().insert("/b".into(), Ok(nodes("updated")));
    // When
    let result = usecase.refresh().await;
    // Then
    assert_eq!(query.scans.load(Ordering::SeqCst), 4);
    assert_eq!(result.repositories.len(), 2);
    for repository in &result.repositories {
        assert_eq!(
            repository.worktrees[0].workflow_history,
            history(&repository.path, "updated-workflow")
        );
    }
    assert_eq!(
        result.repositories[1].worktrees[0]
            .snapshot
            .as_ref()
            .unwrap()
            .preferred_node_id
            .as_deref(),
        Some("updated")
    );
}

#[tokio::test]
async fn test_部分失敗_成功範囲を更新し失敗範囲の一覧を残して復旧する() {
    // Given
    let query = FakeQuery::new();
    let usecase = WorkspaceListUsecase::new(query.clone());
    usecase.refresh().await;
    query
        .branches
        .lock()
        .insert("/a".into(), Err("scan failed".into()));
    query
        .branches
        .lock()
        .insert("/b".into(), Ok(vec![branch("/b", "new")]));
    query
        .nodes
        .lock()
        .insert("/b".into(), Err("nodes failed".into()));
    // When
    let result = usecase.refresh().await;
    // Then
    assert_eq!(result.repositories[0].branches[0].branch.name, "a");
    assert_eq!(result.repositories[0].status.state, "refreshFailed");
    assert_eq!(result.repositories[1].status.state, "ready");
    assert_eq!(
        result.repositories[1].worktrees[0].status.state,
        "refreshFailed"
    );
    assert!(result.repositories[0].status.loaded);
    assert_eq!(
        result.repositories[0].status.error.as_deref(),
        Some("scan failed")
    );
    assert_eq!(result.repositories[1].branches[0].branch.name, "new");
    assert!(result.repositories[1].worktrees[0].status.loaded);
    assert_eq!(
        result.repositories[1].worktrees[0]
            .snapshot
            .as_ref()
            .unwrap()
            .preferred_node_id
            .as_deref(),
        Some("b")
    );
    assert_eq!(
        result.repositories[1].worktrees[0].status.error.as_deref(),
        Some("nodes failed")
    );
    // When
    query
        .branches
        .lock()
        .insert("/a".into(), Ok(vec![branch("/a", "recovered")]));
    query
        .nodes
        .lock()
        .insert("/b".into(), Ok(nodes("recovered")));
    let result = usecase.refresh().await;
    // Then
    assert!(result.repositories[0].status.error.is_none());
    assert_eq!(result.repositories[0].branches[0].branch.name, "recovered");
    assert!(result.repositories[1].worktrees[0].status.error.is_none());
}

#[tokio::test]
async fn test_全件失敗_登録一覧と各一覧を保持する() {
    // Given
    let query = FakeQuery::new();
    let usecase = WorkspaceListUsecase::new(query.clone());
    usecase.refresh().await;
    *query.paths.lock() = Err("paths failed".into());
    for result in query.branches.lock().values_mut() {
        *result = Err("scan failed".into());
    }
    for result in query.nodes.lock().values_mut() {
        *result = Err("nodes failed".into());
    }
    // When
    let result = usecase.refresh().await;
    // Then
    assert_eq!(result.status.state, "refreshFailed");
    assert_eq!(result.status.error.as_deref(), Some("paths failed"));
    assert!(result.status.loaded);
    assert_eq!(result.repositories.len(), 2);
    for repo in result.repositories {
        assert_eq!(repo.branches.len(), 1);
        assert!(repo.status.loaded);
        assert!(repo.status.error.is_some());
        assert!(repo.worktrees[0].snapshot.is_some());
    }
}

#[tokio::test]
async fn test_初回失敗_未取得と正常な空を区別する() {
    // Given
    let query = FakeQuery::new();
    *query.paths.lock() = Err("offline".into());
    let usecase = WorkspaceListUsecase::new(query.clone());
    assert_eq!(usecase.snapshot().status.state, "loading");
    assert!(!usecase.snapshot().status.loaded);
    assert!(usecase.snapshot().status.error.is_none());
    // When / Then
    let result = usecase.refresh().await;
    assert_eq!(result.status.state, "initialFailed");
    assert!(!result.status.loaded);
    assert_eq!(result.status.error.as_deref(), Some("offline"));
    *query.paths.lock() = Ok(vec!["/a".into(), "/b".into()]);
    query
        .branches
        .lock()
        .insert("/a".into(), Err("first scan".into()));
    query
        .nodes
        .lock()
        .insert("/b".into(), Err("first nodes".into()));
    let result = usecase.refresh().await;
    assert_eq!(result.repositories[0].status.state, "initialFailed");
    assert_eq!(
        result.repositories[1].worktrees[0].status.state,
        "initialFailed"
    );
    assert!(!result.repositories[0].status.loaded);
    assert!(!result.repositories[1].worktrees[0].status.loaded);
    assert!(result.repositories[1].worktrees[0].snapshot.is_none());
    *query.paths.lock() = Ok(vec![]);
    let result = usecase.refresh().await;
    assert!(result.status.loaded);
    assert!(result.status.error.is_none());
    assert!(result.repositories.is_empty());
    assert_eq!(result.status.state, "empty");
}

#[tokio::test]
async fn test_正常な空と削除_各階層へ反映して保持データを解放する() {
    // Given
    let query = FakeQuery::new();
    let usecase = WorkspaceListUsecase::new(query.clone());
    usecase.refresh().await;
    query.nodes.lock().insert(
        "/b".into(),
        Ok(WorkspaceTreeSnapshotDto {
            nodes: vec![],
            archived_sessions: vec![],
            preferred_node_id: None,
        }),
    );
    query.branches.lock().insert("/a".into(), Ok(vec![]));
    // When
    let result = usecase.refresh().await;
    // Then
    assert!(result.repositories[0].branches.is_empty());
    assert_eq!(result.repositories[0].status.state, "empty");
    assert_eq!(result.repositories[1].worktrees[0].status.state, "empty");
    assert!(result.repositories[1].worktrees[0]
        .snapshot
        .as_ref()
        .unwrap()
        .preferred_node_id
        .is_none());
    assert!(usecase.lists.lock().nodes("/a").is_none());
    // When
    *query.paths.lock() = Ok(vec![]);
    assert!(usecase.refresh().await.repositories.is_empty());
    // Then
    assert!(usecase.lists.lock().branches("/a").is_none());
    assert!(usecase.lists.lock().branches("/b").is_none());
    assert!(usecase.lists.lock().nodes("/b").is_none());
}

#[tokio::test]
async fn test_競合更新_古い走査結果で新しい一覧を上書きしない() {
    // Given
    let query = FakeQuery::new();
    *query.paths.lock() = Ok(vec!["/a".into()]);
    query.delayed_scan.store(0, Ordering::SeqCst);
    let usecase = Arc::new(WorkspaceListUsecase::new(query.clone()));
    let old = {
        let usecase = usecase.clone();
        tokio::spawn(async move { usecase.refresh().await })
    };
    query.started.notified().await;
    query
        .branches
        .lock()
        .insert("/a".into(), Ok(vec![branch("/a", "new")]));
    // When
    let new = usecase.refresh_repository("/a").await;
    query.release.notify_one();
    let old = old.await.unwrap();
    // Then
    assert_eq!(new.repositories[0].branches[0].branch.name, "new");
    assert_eq!(old.repositories[0].branches[0].branch.name, "new");
    assert!(old.generation > new.generation);
}

#[tokio::test]
async fn test_履歴取得失敗_前回情報を残し成功時に解消する() {
    // Given
    let query = FakeQuery::new();
    let usecase = WorkspaceListUsecase::new(query.clone());
    usecase.refresh().await;
    query
        .history
        .lock()
        .insert("/a".into(), Err("history failed".into()));
    // When / Then
    let result = usecase.refresh().await;
    assert!(result.repositories[0].worktrees[0].snapshot.is_some());
    assert_eq!(
        result.repositories[0].worktrees[0].workflow_history,
        history("/a", "workflow-a")
    );
    assert_eq!(
        result.repositories[0].worktrees[0].status.error.as_deref(),
        Some("history failed")
    );
    query
        .history
        .lock()
        .insert("/a".into(), Ok(history("/a", "recovered")));
    let recovered = usecase.refresh().await;
    assert!(recovered.repositories[0].worktrees[0]
        .status
        .error
        .is_none());
    assert_eq!(
        recovered.repositories[0].worktrees[0].workflow_history,
        history("/a", "recovered")
    );
}

#[tokio::test]
async fn test_一覧の世代_同じ走査中のsnapshotと完了後を区別する() {
    // Given
    let query = FakeQuery::new();
    *query.paths.lock() = Ok(vec!["/a".into()]);
    let usecase = Arc::new(WorkspaceListUsecase::new(query.clone()));
    let previous = usecase.refresh().await;
    query.delayed_scan.store(1, Ordering::SeqCst);
    query
        .branches
        .lock()
        .insert("/a".into(), Ok(vec![branch("/a", "new")]));
    let pending = {
        let usecase = usecase.clone();
        tokio::spawn(async move { usecase.refresh().await })
    };
    query.started.notified().await;
    // When
    let in_progress = usecase.snapshot();
    query.release.notify_one();
    let completed = pending.await.unwrap();
    // Then
    assert!(in_progress.generation > previous.generation);
    assert!(completed.generation > in_progress.generation);
    assert_eq!(in_progress.repositories[0].branches[0].branch.name, "a");
    assert_eq!(completed.repositories[0].branches[0].branch.name, "new");
}

#[tokio::test]
async fn test_競合更新_古い失敗で新しい成功のエラーを復活させない() {
    // Given
    let query = FakeQuery::new();
    *query.paths.lock() = Ok(vec!["/a".into()]);
    query
        .branches
        .lock()
        .insert("/a".into(), Err("old failure".into()));
    query.delayed_scan.store(0, Ordering::SeqCst);
    let usecase = Arc::new(WorkspaceListUsecase::new(query.clone()));
    let old = {
        let usecase = usecase.clone();
        tokio::spawn(async move { usecase.refresh().await })
    };
    query.started.notified().await;
    query
        .branches
        .lock()
        .insert("/a".into(), Ok(vec![branch("/a", "new")]));
    // When
    usecase.refresh_repository("/a").await;
    query.release.notify_one();
    let result = old.await.unwrap();
    // Then
    assert!(result.repositories[0].status.error.is_none());
    assert_eq!(result.repositories[0].branches[0].branch.name, "new");
}

#[tokio::test]
async fn test_branch一覧合成_pr情報とworktreeの有無を反映する() {
    // Given
    let query = FakeQuery::new();
    *query.paths.lock() = Ok(vec!["/a".into()]);
    let mut without_worktree = branch("/unused", "without-worktree");
    without_worktree.worktree_path = None;
    let mut git_merged = branch("/a/git", "git-merged");
    git_merged.is_merged = true;
    query.branches.lock().insert(
        "/a".into(),
        Ok(vec![
            branch("/a", "open"),
            branch("/a/merged", "merged"),
            git_merged,
            without_worktree,
        ]),
    );
    for path in ["/a/merged", "/a/git"] {
        query.nodes.lock().insert(path.into(), Ok(nodes(path)));
        query.history.lock().insert(path.into(), Ok(vec![]));
    }
    query.prs.lock().insert(
        "/a".into(),
        Ok(PrStatus {
            open_prs: HashMap::from([(
                "open".into(),
                PrInfo {
                    number: 42,
                    url: "https://example.test/pull/42".into(),
                },
            )]),
            merged_branches: vec!["open".into(), "merged".into()],
        }),
    );
    let usecase = WorkspaceListUsecase::new(query.clone());
    // When
    usecase.refresh().await;
    // Then
    let snapshot = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let snapshot = usecase.snapshot();
            if snapshot.repositories[0].branches[0].has_pr {
                break snapshot;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let repo = &snapshot.repositories[0];
    let open = &repo.branches[0];
    assert!(open.has_pr);
    assert_eq!(open.pr_number, Some(42));
    assert_eq!(open.pr_url.as_deref(), Some("https://example.test/pull/42"));
    assert!(!open.branch.is_merged);
    let merged = &repo.branches[1];
    assert!(merged.branch.is_merged);
    assert!(!merged.has_pr);
    assert_eq!(merged.pr_number, None);
    assert_eq!(merged.pr_url, None);
    assert!(repo.branches[2].branch.is_merged);
    assert_eq!(repo.branches[3].branch.worktree_path, None);
    assert_eq!(repo.worktrees.len(), 3);
    assert_eq!(*query.node_reads.lock(), ["/a", "/a/merged", "/a/git"]);
}

#[tokio::test]
async fn test_局所再読込_指定worktreeの子一覧だけを更新する() {
    // Given
    let query = FakeQuery::new();
    let usecase = WorkspaceListUsecase::new(query.clone());
    usecase.refresh().await;
    query.node_reads.lock().clear();
    query.nodes.lock().insert("/a".into(), Ok(nodes("local")));
    query
        .history
        .lock()
        .insert("/a".into(), Ok(history("/a", "local")));
    *query.paths.lock() = Err("must not read repositories".into());
    // When
    let snapshot = usecase.refresh_worktree("/a").await;
    // Then
    assert_eq!(query.scans.load(Ordering::SeqCst), 2);
    assert!(snapshot.status.error.is_none());
    assert_eq!(*query.node_reads.lock(), ["/a"]);
    assert_eq!(
        snapshot.repositories[0].worktrees[0]
            .snapshot
            .as_ref()
            .unwrap()
            .preferred_node_id
            .as_deref(),
        Some("local")
    );
    assert_eq!(
        snapshot.repositories[0].worktrees[0].workflow_history,
        history("/a", "local")
    );
    assert_eq!(
        snapshot.repositories[1].worktrees[0]
            .snapshot
            .as_ref()
            .unwrap()
            .preferred_node_id
            .as_deref(),
        Some("b")
    );
    // When
    query
        .nodes
        .lock()
        .insert("/a".into(), Err("local failure".into()));
    let failed = usecase.refresh_worktree("/a").await;
    // Then
    assert_eq!(
        failed.repositories[0].worktrees[0].status.error.as_deref(),
        Some("local failure")
    );
    assert_eq!(
        failed.repositories[0].worktrees[0]
            .snapshot
            .as_ref()
            .unwrap()
            .preferred_node_id
            .as_deref(),
        Some("local")
    );
    // When
    query
        .nodes
        .lock()
        .insert("/a".into(), Ok(nodes("recovered")));
    let recovered = usecase.refresh_worktree("/a").await;
    // Then
    assert!(recovered.repositories[0].worktrees[0]
        .status
        .error
        .is_none());
}

fn pause_nodes(query: &Arc<FakeQuery>, path: &'static str) -> std::sync::mpsc::Sender<()> {
    let (send, receive) = std::sync::mpsc::channel();
    let receive = Mutex::new(receive);
    let once = std::sync::atomic::AtomicBool::new(false);
    let weak = Arc::downgrade(query);
    *query.on_nodes.lock() = Some(Arc::new(move |reading| {
        if reading == path && !once.swap(true, Ordering::SeqCst) {
            weak.upgrade().unwrap().started.notify_one();
            receive.lock().recv_timeout(Duration::from_secs(5)).unwrap();
        }
    }));
    send
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_複数worktreeの途中失効_先行結果を残りに適用せず後続だけを返す() {
    // Given
    let query = FakeQuery::new();
    *query.paths.lock() = Ok(vec!["/a".into()]);
    let paths = ["/a", "/a/second", "/a/third"];
    query.branches.lock().insert(
        "/a".into(),
        Ok(paths.iter().map(|path| branch(path, path)).collect()),
    );
    for path in paths {
        query.nodes.lock().insert(path.into(), Ok(nodes("old")));
        query
            .history
            .lock()
            .insert(path.into(), Ok(history(path, "old")));
    }
    let release = pause_nodes(&query, "/a/second");
    let usecase = Arc::new(WorkspaceListUsecase::new(query.clone()));
    let old = {
        let usecase = usecase.clone();
        tokio::spawn(async move { usecase.refresh().await })
    };
    query.started.notified().await;
    assert_eq!(
        usecase.snapshot().repositories[0].worktrees[0]
            .snapshot
            .as_ref()
            .unwrap()
            .preferred_node_id
            .as_deref(),
        Some("old")
    );
    for path in paths {
        query.nodes.lock().insert(path.into(), Ok(nodes("new")));
        query
            .history
            .lock()
            .insert(path.into(), Ok(history(path, "new")));
    }
    // When
    let new = usecase.refresh_repository("/a").await;
    release.send(()).unwrap();
    let old = old.await.unwrap();
    // Then
    for snapshot in [new, old, usecase.snapshot()] {
        for worktree in &snapshot.repositories[0].worktrees {
            assert_eq!(
                worktree
                    .snapshot
                    .as_ref()
                    .unwrap()
                    .preferred_node_id
                    .as_deref(),
                Some("new")
            );
            assert_eq!(worktree.workflow_history, history(&worktree.path, "new"));
        }
    }
    assert_eq!(
        query
            .node_reads
            .lock()
            .iter()
            .filter(|path| *path == "/a/third")
            .count(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_局所と全体の競合_開始順がどちらでも後続の子一覧を保持する() {
    for local_first in [true, false] {
        // Given
        let query = FakeQuery::new();
        *query.paths.lock() = Ok(vec!["/a".into()]);
        let usecase = Arc::new(WorkspaceListUsecase::new(query.clone()));
        usecase.refresh().await;
        let release = pause_nodes(&query, "/a");
        let old = {
            let usecase = usecase.clone();
            tokio::spawn(async move {
                if local_first {
                    usecase.refresh_worktree("/a").await
                } else {
                    usecase.refresh().await
                }
            })
        };
        query.started.notified().await;
        query.nodes.lock().insert("/a".into(), Ok(nodes("new")));
        // When
        let new = if local_first {
            usecase.refresh().await
        } else {
            usecase.refresh_worktree("/a").await
        };
        release.send(()).unwrap();
        let old = old.await.unwrap();
        // Then
        for snapshot in [new, old] {
            assert_eq!(
                snapshot.repositories[0].worktrees[0]
                    .snapshot
                    .as_ref()
                    .unwrap()
                    .preferred_node_id
                    .as_deref(),
                Some("new")
            );
        }
    }
}

#[tokio::test]
async fn test_repository再読込_対象だけを走査し失敗保持と復旧と削除を反映する() {
    // Given
    let query = FakeQuery::new();
    let usecase = WorkspaceListUsecase::new(query.clone());
    usecase.refresh().await;
    query.node_reads.lock().clear();
    *query.paths.lock() = Err("must not read repositories".into());
    query
        .branches
        .lock()
        .insert("/b".into(), Err("must not scan other repository".into()));
    query
        .branches
        .lock()
        .insert("/a".into(), Err("scan failed".into()));
    // When
    let failed = usecase.refresh_repository("/a").await;
    // Then
    assert_eq!(query.scans.load(Ordering::SeqCst), 3);
    assert_eq!(*query.node_reads.lock(), ["/a"]);
    assert!(failed.status.error.is_none());
    assert_eq!(
        failed.repositories[0].status.error.as_deref(),
        Some("scan failed")
    );
    assert_eq!(failed.repositories[0].branches[0].branch.name, "a");
    assert!(failed.repositories[1].status.error.is_none());
    // When
    query.branches.lock().insert("/a".into(), Ok(vec![]));
    let recovered = usecase.refresh_repository("/a").await;
    // Then
    assert_eq!(query.scans.load(Ordering::SeqCst), 4);
    assert!(recovered.repositories[0].status.error.is_none());
    assert!(recovered.repositories[0].branches.is_empty());
    assert!(recovered.repositories[0].worktrees.is_empty());
    assert_eq!(recovered.repositories[1].branches[0].branch.name, "b");
    assert!(usecase.lists.lock().nodes("/a").is_none());
    // When
    usecase.refresh_repository("/missing").await;
    // Then
    assert_eq!(query.scans.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn test_repositoryと全体の競合_開始順によらず新しい結果と他repositoryの成功を保持する() {
    for local_first in [true, false] {
        // Given
        let query = FakeQuery::new();
        let usecase = Arc::new(WorkspaceListUsecase::new(query.clone()));
        usecase.refresh().await;
        query.delayed_scan.store(2, Ordering::SeqCst);
        let old = {
            let usecase = usecase.clone();
            tokio::spawn(async move {
                if local_first {
                    usecase.refresh_repository("/a").await
                } else {
                    usecase.refresh().await
                }
            })
        };
        query.started.notified().await;
        query.branches.lock().insert("/a".into(), Ok(vec![]));
        query
            .branches
            .lock()
            .insert("/b".into(), Ok(vec![branch("/b", "new-b")]));
        // When
        if local_first {
            usecase.refresh().await;
        } else {
            usecase.refresh_repository("/a").await;
        }
        query.release.notify_one();
        let result = old.await.unwrap();
        // Then
        assert!(result.repositories[0].branches.is_empty());
        assert_eq!(result.repositories[0].status.state, "empty");
        assert!(result.repositories[0].worktrees.is_empty());
        assert_eq!(
            result.repositories[1].branches[0].branch.name,
            if local_first { "new-b" } else { "b" }
        );
        assert!(result.repositories[1].status.error.is_none());
    }
}

#[tokio::test]
async fn test_pr取得_遅いrepositoryを待たず一覧と他repositoryのprを公開する() {
    // Given
    let query = FakeQuery::new();
    query.prs.lock().insert(
        "/b".into(),
        Ok(PrStatus {
            open_prs: HashMap::from([(
                "b".into(),
                PrInfo {
                    number: 7,
                    url: "pr-7".into(),
                },
            )]),
            merged_branches: vec![],
        }),
    );
    let (release, receive) = std::sync::mpsc::channel();
    let receive = Mutex::new(receive);
    *query.on_pr.lock() = Some(Arc::new(move |path| {
        if path == "/a" {
            receive.lock().recv_timeout(Duration::from_secs(5)).unwrap();
        }
    }));
    let usecase = WorkspaceListUsecase::new(query);
    // When
    let snapshot = tokio::time::timeout(Duration::from_secs(2), usecase.refresh())
        .await
        .unwrap();
    // Then
    assert_eq!(snapshot.repositories.len(), 2);
    assert_eq!(snapshot.repositories[0].branches.len(), 1);
    let result = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if usecase.snapshot().repositories[1].branches[0].has_pr {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    release.send(()).unwrap();
    result.unwrap();
}

#[tokio::test]
async fn test_pr情報保持_取得中と取得失敗では既知のpr情報を表示し続ける() {
    // Given
    let query = FakeQuery::new();
    *query.paths.lock() = Ok(vec!["/a".into()]);
    let mut done = branch("/unused", "done");
    done.worktree_path = None;
    query
        .branches
        .lock()
        .insert("/a".into(), Ok(vec![branch("/a", "a"), done]));
    query.prs.lock().insert(
        "/a".into(),
        Ok(PrStatus {
            open_prs: HashMap::from([(
                "a".into(),
                PrInfo {
                    number: 7,
                    url: "pr-7".into(),
                },
            )]),
            merged_branches: vec!["done".into()],
        }),
    );
    let usecase = WorkspaceListUsecase::new(query.clone());
    usecase.refresh().await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while !usecase.snapshot().repositories[0].branches[0].has_pr {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    query
        .prs
        .lock()
        .insert("/a".into(), Err("gh timeout".into()));
    let (release, receive) = std::sync::mpsc::channel();
    let receive = Mutex::new(receive);
    *query.on_pr.lock() = Some(Arc::new(move |_| {
        receive.lock().recv_timeout(Duration::from_secs(5)).unwrap();
    }));
    let assert_known = |snapshot: WorkspaceListSnapshotDto| {
        let branches = &snapshot.repositories[0].branches;
        assert_eq!(branches[0].pr_number, Some(7));
        assert_eq!(branches[0].pr_url.as_deref(), Some("pr-7"));
        assert!(branches[1].branch.is_merged);
    };

    // When
    let fetching = usecase.refresh().await;
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while Arc::strong_count(&usecase.notify) != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    // Then
    assert_known(fetching);
    assert_known(usecase.snapshot());
}

#[tokio::test]
async fn test_pr反映通知_現行世代だけ通知し古い世代と削除済みrepositoryは通知しない() {
    for outcome in ["current", "superseded", "removed"] {
        // Given
        let query = FakeQuery::new();
        *query.paths.lock() = Ok(vec!["/a".into()]);
        query.prs.lock().insert(
            "/a".into(),
            Ok(PrStatus {
                open_prs: HashMap::from([(
                    "a".into(),
                    PrInfo {
                        number: 7,
                        url: "pr-7".into(),
                    },
                )]),
                merged_branches: vec![],
            }),
        );
        let (release, receive) = std::sync::mpsc::channel();
        let receive = Mutex::new(receive);
        let started = Arc::new(tokio::sync::Notify::new());
        let pr_started = started.clone();
        *query.on_pr.lock() = Some(Arc::new(move |_| {
            pr_started.notify_one();
            receive.lock().recv_timeout(Duration::from_secs(5)).unwrap();
        }));
        let notifications = Arc::new(AtomicUsize::new(0));
        let notified = notifications.clone();
        let usecase = WorkspaceListUsecase::new(query).with_notifier(move || {
            notified.fetch_add(1, Ordering::SeqCst);
        });
        usecase.refresh().await;
        tokio::time::timeout(Duration::from_secs(2), started.notified())
            .await
            .unwrap();
        let before_pr = notifications.load(Ordering::SeqCst);
        assert_eq!(before_pr, 3);

        // When
        match outcome {
            "superseded" => {
                usecase.lists.lock().begin_repository("/a");
            }
            "removed" => {
                let mut lists = usecase.lists.lock();
                let generation = lists.begin();
                lists.complete_repositories(generation, Ok(vec![]));
            }
            _ => {}
        }
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while Arc::strong_count(&usecase.notify) != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        // Then
        assert_eq!(
            notifications.load(Ordering::SeqCst),
            before_pr + usize::from(outcome == "current")
        );
        let snapshot = usecase.snapshot();
        if outcome == "removed" {
            assert!(snapshot.repositories.is_empty());
        } else {
            assert_eq!(
                snapshot.repositories[0].branches[0].has_pr,
                outcome == "current"
            );
            assert_eq!(
                snapshot.repositories[0].branches[0].pr_number,
                if outcome == "current" { Some(7) } else { None }
            );
        }
    }
}

#[tokio::test]
async fn test_全体更新の統合_複数の直接要求は進行中の後に一回だけ再取得する() {
    // Given
    let query = FakeQuery::new();
    *query.paths.lock() = Ok(vec!["/a".into()]);
    query.delayed_scan.store(0, Ordering::SeqCst);
    let usecase = WorkspaceListUsecase::new(query.clone());
    let first = tokio::spawn({
        let usecase = usecase.clone();
        async move { usecase.refresh().await }
    });
    query.started.notified().await;
    // When
    let pending = async { futures_util::future::join_all((0..5).map(|_| usecase.refresh())).await };
    let release = async {
        tokio::task::yield_now().await;
        assert_eq!(query.scans.load(Ordering::SeqCst), 1);
        query
            .branches
            .lock()
            .insert("/a".into(), Ok(vec![branch("/a", "new")]));
        query.release.notify_one();
    };
    let (results, ()) = tokio::join!(pending, release);
    first.await.unwrap();
    // Then
    assert_eq!(query.scans.load(Ordering::SeqCst), 2);
    for result in results {
        assert_eq!(result.repositories[0].branches[0].branch.name, "new");
    }
    usecase.refresh().await;
    assert_eq!(query.scans.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn test_全体更新の統合_呼出元の中断後も待機要求を完了し失敗から再取得できる() {
    // Given
    let query = FakeQuery::new();
    *query.paths.lock() = Ok(vec!["/a".into()]);
    query.delayed_scan.store(0, Ordering::SeqCst);
    query
        .branches
        .lock()
        .insert("/a".into(), Err("offline".into()));
    let usecase = WorkspaceListUsecase::new(query.clone());
    let first = tokio::spawn({
        let usecase = usecase.clone();
        async move { usecase.refresh().await }
    });
    query.started.notified().await;
    // When
    first.abort();
    let release = async {
        tokio::task::yield_now().await;
        query.release.notify_one();
    };
    let (failed, ()) = tokio::join!(usecase.refresh(), release);
    // Then
    assert_eq!(query.scans.load(Ordering::SeqCst), 2);
    assert_eq!(
        failed.repositories[0].status.error.as_deref(),
        Some("offline")
    );
    query.branches.lock().insert("/a".into(), Ok(vec![]));
    let recovered = usecase.refresh().await;
    assert!(recovered.repositories[0].status.error.is_none());
    assert!(recovered.repositories[0].branches.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_一覧状態_各階層の初回取得中と取得済みを区別する() {
    use crate::usecase::workflow::{WorkspaceSequenceDto, WorkspaceTreeItemDto};

    // Given
    let query = FakeQuery::new();
    *query.paths.lock() = Ok(vec!["/a".into()]);
    query.delayed_scan.store(0, Ordering::SeqCst);
    let mut tree = nodes("workflow");
    tree.nodes
        .push(WorkspaceTreeItemDto::Sequence(WorkspaceSequenceDto {
            worktree: None,
            id: "workflow".into(),
            title: "Workflow".into(),
            status: "active".into(),
            workflow_capabilities: None,
            children: vec![],
            updated_at: 1.0,
        }));
    query.nodes.lock().insert("/a".into(), Ok(tree));
    let release_nodes = pause_nodes(&query, "/a");
    let usecase = WorkspaceListUsecase::new(query.clone());
    assert_eq!(usecase.snapshot().status.state, "loading");

    // When
    let pending = tokio::spawn({
        let usecase = usecase.clone();
        async move { usecase.refresh().await }
    });
    query.started.notified().await;
    let scanning = usecase.snapshot();
    // Then
    assert_eq!(scanning.status.state, "ready");
    assert_eq!(scanning.repositories[0].status.state, "loading");

    // When
    query.release.notify_one();
    query.started.notified().await;
    let reading = usecase.snapshot();
    // Then
    assert_eq!(reading.repositories[0].status.state, "ready");
    assert_eq!(reading.repositories[0].worktrees[0].status.state, "loading");

    // When
    release_nodes.send(()).unwrap();
    let completed = pending.await.unwrap();
    // Then
    assert_eq!(completed.repositories[0].worktrees[0].status.state, "ready");

    // When
    query.nodes.lock().insert("/a".into(), Ok(nodes("empty")));
    let empty = usecase.refresh_worktree("/a").await;
    // Then
    assert_eq!(empty.repositories[0].worktrees[0].status.state, "empty");
}

#[tokio::test]
async fn test_pr定期取得_遅いrepositoryが他repositoryの取得を止めない() {
    let query = FakeQuery::new();
    let usecase = WorkspaceListUsecase::new(query.clone());
    usecase.refresh().await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while Arc::strong_count(&usecase.notify) != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let (release, receive) = std::sync::mpsc::channel();
    let receive = Mutex::new(receive);
    let other = Arc::new(tokio::sync::Notify::new());
    let reached = other.clone();
    *query.on_pr.lock() = Some(Arc::new(move |path| {
        if path == "/a" {
            receive.lock().recv_timeout(Duration::from_secs(5)).unwrap();
        } else {
            reached.notify_one();
        }
    }));
    let refresh = tokio::spawn({
        let usecase = usecase.clone();
        async move { usecase.refresh_external_information().await }
    });
    let result = tokio::time::timeout(Duration::from_secs(2), other.notified()).await;
    release.send(()).unwrap();
    result.unwrap();
    refresh.await.unwrap();
}
