use super::*;
use crate::domain::workflow::entities::workflow_execution::ExecutionTree;
use crate::domain::workflow::repository::WorkflowStartupRecord;
use crate::domain::workflow::{ExecutionOrigin, ExecutionTreeLaunch, NodeFact, TreeRootFact};
use std::sync::Mutex;

struct Repository {
    terminal: Mutex<Option<NodeFact>>,
    concurrent_facts: Mutex<std::collections::VecDeque<Option<NodeFact>>>,
    unpersisted_appends: Mutex<usize>,
    append_attempts: Mutex<Vec<Option<i64>>>,
    unreadable: bool,
    fail_load: bool,
    fail_append: bool,
    appended: Mutex<Vec<(NodeFact, f64)>>,
}

#[async_trait::async_trait]
impl WorkflowStartupRepository for Repository {
    async fn list_tree_ids(&self) -> Result<Vec<String>, WorkflowError> {
        Ok(vec!["tree".into()])
    }

    async fn load(&self, tree_id: &str) -> Result<Option<WorkflowStartupRecord>, WorkflowError> {
        if self.fail_load {
            return Err(WorkflowError::external("read failed"));
        }
        let root = TreeRootFact {
            repository_root: None,
            workspace_identity: "/repo".into(),
            worktree_path: "/repo".into(),
            created_from: ExecutionOrigin::Cli,
            request: String::new(),
            workflow_name: "old".into(),
            definition: None,
            launched_as: ExecutionTreeLaunch::Workflow,
        };
        let mut execution = ExecutionTree::restore_without_definition(tree_id, &root, 1.0);
        if let Some(fact) = self.terminal.lock().unwrap().as_ref() {
            execution.replay_terminal_fact(fact, 2.0);
        }
        Ok(Some(WorkflowStartupRecord {
            execution,
            definition_error: self
                .unreadable
                .then(|| "Workflow definition is unavailable: completion".into()),
        }))
    }
}

fn repository() -> Repository {
    Repository {
        terminal: Mutex::new(None),
        concurrent_facts: Default::default(),
        unpersisted_appends: Default::default(),
        append_attempts: Default::default(),
        unreadable: true,
        fail_load: false,
        fail_append: false,
        appended: Mutex::new(Vec::new()),
    }
}

#[tokio::test]
async fn test_起動時定義確認_読めない定義は要対応を返し事実を追記しない() {
    let repository = repository();
    for _ in 0..2 {
        assert!(matches!(
            check_startup_definition(&repository, "tree").await,
            Err(WorkflowError::IncompatibleStoredEvent(reason)) if reason.contains("completion")
        ));
    }
    assert!(repository.appended.lock().unwrap().is_empty());
    assert!(repository.append_attempts.lock().unwrap().is_empty());
    assert!(repository.terminal.lock().unwrap().is_none());
}

#[tokio::test]
async fn test_起動時定義確認_読める定義と既存の終端事実には追記しない() {
    for terminal in [
        None,
        Some(NodeFact::ExecutionCompleted),
        Some(NodeFact::AbortRequested(Default::default())),
    ] {
        let mut repository = repository();
        repository.unreadable = terminal.is_some();
        *repository.terminal.lock().unwrap() = terminal;
        check_startup_definition(&repository, "tree").await.unwrap();
        assert!(repository.appended.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn test_起動時定義確認_読取失敗を返し修復後に成功する() {
    let mut repository = repository();
    repository.fail_load = true;
    assert!(check_startup_definition(&repository, "tree")
        .await
        .unwrap_err()
        .to_string()
        .contains("read failed"));
    repository.fail_load = false;
    repository.unreadable = false;
    repository.fail_append = true;
    check_startup_definition(&repository, "tree").await.unwrap();
    assert!(repository.append_attempts.lock().unwrap().is_empty());
}

struct Startup {
    calls: Mutex<Vec<String>>,
    failure: Option<&'static str>,
    conflict: bool,
    temporary: bool,
}

impl Startup {
    fn record(&self, call: String) -> Result<(), WorkflowError> {
        let first = !self.calls.lock().unwrap().contains(&call);
        self.calls.lock().unwrap().push(call.clone());
        if self.failure == Some(call.as_str()) && (!(self.conflict || self.temporary) || first) {
            if self.temporary {
                Err(WorkflowError::Store(
                    crate::domain::failure::StorageFailure::from(
                        crate::domain::local_event::CommitBatchError::QueueBusy,
                    )
                    .with_message(call),
                ))
            } else if self.conflict {
                Err(WorkflowError::Conflict(call))
            } else {
                Err(WorkflowError::external(call))
            }
        } else {
            Ok(())
        }
    }
}

#[async_trait::async_trait]
impl WorkflowStartupRepository for Startup {
    async fn list_tree_ids(&self) -> Result<Vec<String>, WorkflowError> {
        self.record("list".into())?;
        Ok(["first", "second"].map(String::from).into())
    }

    async fn load(&self, tree_id: &str) -> Result<Option<WorkflowStartupRecord>, WorkflowError> {
        self.record(format!("load:{tree_id}"))?;
        let mut repository = repository();
        repository.unreadable = false;
        repository.load(tree_id).await
    }
}

#[async_trait::async_trait]
impl WorkflowStartupGateway for Startup {
    fn current_timestamp(&self) -> f64 {
        3.0
    }

    async fn reconcile_tree(&self, tree_id: &str, timestamp: f64) -> Result<(), WorkflowError> {
        assert_eq!(timestamp, 3.0);
        tokio::task::yield_now().await;
        self.record(format!("reconcile:{tree_id}"))
    }
}

#[tokio::test]
async fn test_起動時復旧_列挙と定義確認とreconciliationの順序を所有する() {
    // Given
    let startup = Arc::new(Startup {
        calls: Mutex::new(Vec::new()),
        failure: None,
        conflict: false,
        temporary: false,
    });
    let usecase = WorkflowStartupUsecase::new(
        crate::usecase::work_queue::WorkQueueUsecase::new(std::sync::Arc::new(
            crate::usecase::work_queue::ImmediateWorkQueueRuntime::default(),
        )),
        startup.clone(),
        startup.clone(),
    );

    // When
    let (first, second) = tokio::join!(usecase.execute(), usecase.execute());
    first.unwrap();
    second.unwrap();

    // Then
    let pass = [
        "list",
        "load:first",
        "reconcile:first",
        "load:second",
        "reconcile:second",
    ];
    let calls = startup.calls.lock().unwrap();
    assert_eq!(calls.first().unwrap(), "list");
    for tree in ["first", "second"] {
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.ends_with(tree))
                .cloned()
                .collect::<Vec<_>>(),
            pass.iter()
                .filter(|call| call.ends_with(tree))
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        );
    }
}

#[tokio::test]
async fn test_起動時復旧_定義確認に失敗したtreeを再生せず後続を処理して最初の失敗を返す() {
    for conflict in [false, true] {
        for failure in ["load:first", "reconcile:first"] {
            // Given
            let startup = Arc::new(Startup {
                calls: Mutex::new(Vec::new()),
                failure: Some(failure),
                conflict,
                temporary: false,
            });
            let usecase = WorkflowStartupUsecase::new(
                crate::usecase::work_queue::WorkQueueUsecase::new(std::sync::Arc::new(
                    crate::usecase::work_queue::ImmediateWorkQueueRuntime::default(),
                )),
                startup.clone(),
                startup.clone(),
            );

            // When
            let result = usecase.execute().await;
            usecase.execute().await.unwrap();

            // Then
            if conflict {
                result.unwrap();
            } else {
                assert!(result.unwrap_err().to_string().contains(failure));
            }
            let calls = startup.calls.lock().unwrap();
            assert!(calls.contains(&"reconcile:second".into()));
            assert_eq!(
                calls
                    .iter()
                    .filter(|call| call.as_str() == "reconcile:first")
                    .count(),
                if conflict && failure == "reconcile:first" {
                    2
                } else if conflict || failure == "reconcile:first" {
                    1
                } else {
                    0
                }
            );
        }
    }
}

#[tokio::test]
async fn test_起動時復旧_列挙失敗は後続操作を呼ばず返す() {
    // Given
    let startup = Arc::new(Startup {
        calls: Mutex::new(Vec::new()),
        failure: Some("list"),
        conflict: false,
        temporary: false,
    });
    let usecase = WorkflowStartupUsecase::new(
        crate::usecase::work_queue::WorkQueueUsecase::new(std::sync::Arc::new(
            crate::usecase::work_queue::ImmediateWorkQueueRuntime::default(),
        )),
        startup.clone(),
        startup.clone(),
    );
    // When / Then
    assert!(usecase
        .execute()
        .await
        .unwrap_err()
        .to_string()
        .contains("list"));
    assert_eq!(*startup.calls.lock().unwrap(), ["list"]);
}

struct ConflictingStartup {
    conflicts: usize,
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl WorkflowStartupGateway for ConflictingStartup {
    fn current_timestamp(&self) -> f64 {
        3.0
    }

    async fn reconcile_tree(&self, _tree_id: &str, _timestamp: f64) -> Result<(), WorkflowError> {
        if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) < self.conflicts {
            Err(WorkflowError::Conflict("head advanced".into()))
        } else {
            Ok(())
        }
    }
}

#[tokio::test]
async fn test_起動時前進_競合後に読み直して再試行しabortしない() {
    for conflicts in [1, 5] {
        // Given
        let mut repository = repository();
        repository.unreadable = false;
        let repository = Arc::new(repository);
        let runtime = Arc::new(ConflictingStartup {
            conflicts,
            calls: Default::default(),
        });
        let usecase = WorkflowStartupUsecase::new(
            crate::usecase::work_queue::WorkQueueUsecase::new(std::sync::Arc::new(
                crate::usecase::work_queue::ImmediateWorkQueueRuntime::default(),
            )),
            repository.clone(),
            runtime.clone(),
        );
        // When
        usecase.execute().await.unwrap();
        usecase.execute().await.unwrap();
        // Then
        assert_eq!(
            runtime.calls.load(std::sync::atomic::Ordering::SeqCst),
            conflicts + 1
        );
        assert!(repository.appended.lock().unwrap().is_empty());
    }
}

#[derive(Default)]
struct FailedStartup(std::sync::atomic::AtomicUsize);

#[async_trait::async_trait]
impl WorkflowStartupGateway for FailedStartup {
    fn current_timestamp(&self) -> f64 {
        3.0
    }

    async fn reconcile_tree(&self, _: &str, _: f64) -> Result<(), WorkflowError> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(WorkflowError::external("advancement failed"))
    }
}

#[tokio::test]
async fn test_起動失敗abort_追記直前に完了またはabortされた実行木へ追記しない() {
    for terminal in [
        NodeFact::ExecutionCompleted,
        NodeFact::AbortRequested(crate::domain::workflow::AbortRequestedFact {
            reason: Some("user abort".into()),
        }),
    ] {
        // Given
        let mut repository = repository();
        repository.unreadable = false;
        repository
            .concurrent_facts
            .lock()
            .unwrap()
            .push_back(Some(terminal.clone()));
        let repository = Arc::new(repository);
        let runtime = Arc::new(FailedStartup::default());
        let usecase = WorkflowStartupUsecase::new(
            crate::usecase::work_queue::WorkQueueUsecase::new(std::sync::Arc::new(
                crate::usecase::work_queue::ImmediateWorkQueueRuntime::default(),
            )),
            repository.clone(),
            runtime.clone(),
        );

        // When
        assert!(usecase.execute().await.is_err());
        usecase.execute().await.unwrap();

        // Then
        assert!(repository.terminal.lock().unwrap().is_none());
        assert!(repository.appended.lock().unwrap().is_empty());
        assert!(repository.append_attempts.lock().unwrap().is_empty());
        assert_eq!(runtime.0.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn test_起動失敗abort_競合時だけ最新記録で有界に再判定し前進は繰り返さない() {
    for conflicts in [0, 1, 5] {
        // Given
        let mut repository = repository();
        repository.unreadable = false;
        *repository.concurrent_facts.lock().unwrap() = vec![None; conflicts].into();
        let repository = Arc::new(repository);
        let runtime = Arc::new(FailedStartup::default());
        let usecase = WorkflowStartupUsecase::new(
            crate::usecase::work_queue::WorkQueueUsecase::new(std::sync::Arc::new(
                crate::usecase::work_queue::ImmediateWorkQueueRuntime::default(),
            )),
            repository.clone(),
            runtime.clone(),
        );

        // When
        assert!(usecase.execute().await.is_err());
        usecase.execute().await.unwrap();

        // Then
        assert!(repository.append_attempts.lock().unwrap().is_empty());
        assert!(repository.appended.lock().unwrap().is_empty());
        assert_eq!(runtime.0.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn test_起動失敗abort_保存エラーは再試行しない() {
    // Given
    let mut repository = repository();
    repository.unreadable = false;
    repository.fail_append = true;
    let repository = Arc::new(repository);
    let runtime = Arc::new(FailedStartup::default());
    let usecase = WorkflowStartupUsecase::new(
        crate::usecase::work_queue::WorkQueueUsecase::new(std::sync::Arc::new(
            crate::usecase::work_queue::ImmediateWorkQueueRuntime::default(),
        )),
        repository.clone(),
        runtime.clone(),
    );

    // When
    assert!(usecase.execute().await.is_err());
    usecase.execute().await.unwrap();

    // Then
    assert!(repository.append_attempts.lock().unwrap().is_empty());
    assert!(repository.appended.lock().unwrap().is_empty());
    assert_eq!(runtime.0.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_起動時復旧_定義不明でも保存を試みず要対応を記録する() {
    for unreadable in [false, true] {
        for unpersisted in [1, 5] {
            // Given
            let mut repository = repository();
            repository.unreadable = unreadable;
            *repository.unpersisted_appends.lock().unwrap() = unpersisted;
            let repository = Arc::new(repository);
            let runtime = Arc::new(FailedStartup::default());
            let usecase = WorkflowStartupUsecase::new(
                crate::usecase::work_queue::WorkQueueUsecase::new(std::sync::Arc::new(
                    crate::usecase::work_queue::ImmediateWorkQueueRuntime::default(),
                )),
                repository.clone(),
                runtime.clone(),
            );

            // When
            assert!(usecase.execute().await.is_err());
            usecase.execute().await.unwrap();

            // Then
            assert!(repository.append_attempts.lock().unwrap().is_empty());
            assert!(repository.appended.lock().unwrap().is_empty());
            assert!(repository.terminal.lock().unwrap().is_none());
            assert_eq!(
                runtime.0.load(std::sync::atomic::Ordering::SeqCst),
                usize::from(!unreadable)
            );
            let observations = usecase.queue.failure_query().records("tree").await;
            assert_eq!(observations.len(), 1);
            assert!(observations[0].requires_attention);
        }
    }
}

#[tokio::test]
async fn test_定義不明_追記経路に入らず実行木を維持する() {
    for terminal in [
        NodeFact::ExecutionCompleted,
        NodeFact::AbortRequested(Default::default()),
    ] {
        // Given
        let repository = Arc::new(repository());
        repository
            .concurrent_facts
            .lock()
            .unwrap()
            .push_back(Some(terminal.clone()));
        let runtime = Arc::new(ConflictingStartup {
            conflicts: 0,
            calls: Default::default(),
        });
        let usecase = WorkflowStartupUsecase::new(
            crate::usecase::work_queue::WorkQueueUsecase::new(std::sync::Arc::new(
                crate::usecase::work_queue::ImmediateWorkQueueRuntime::default(),
            )),
            repository.clone(),
            runtime.clone(),
        );
        // When
        assert!(usecase.execute().await.is_err());
        // Then
        assert!(repository.terminal.lock().unwrap().is_none());
        assert!(repository.appended.lock().unwrap().is_empty());
        assert!(repository.append_attempts.lock().unwrap().is_empty());
        assert_eq!(runtime.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn test_起動時復旧_一覧の一時的失敗を再試行して各実行木を再開する() {
    let startup = Arc::new(Startup {
        calls: Mutex::new(Vec::new()),
        failure: Some("list"),
        conflict: false,
        temporary: true,
    });
    WorkflowStartupUsecase::new(
        crate::usecase::work_queue::WorkQueueUsecase::new(std::sync::Arc::new(
            crate::usecase::work_queue::ImmediateWorkQueueRuntime::default(),
        )),
        startup.clone(),
        startup.clone(),
    )
    .execute()
    .await
    .unwrap();
    let calls = startup.calls.lock().unwrap();
    assert_eq!(&calls[..2], &["list", "list"]);
    for tree in ["first", "second"] {
        assert_eq!(
            calls
                .iter()
                .filter(|call| **call == format!("reconcile:{tree}"))
                .count(),
            1
        );
    }
}

struct FailingStartupList(WorkflowError);

#[async_trait::async_trait]
impl WorkflowStartupRepository for FailingStartupList {
    async fn list_tree_ids(&self) -> Result<Vec<String>, WorkflowError> {
        Err(self.0.clone())
    }

    async fn load(&self, _: &str) -> Result<Option<WorkflowStartupRecord>, WorkflowError> {
        panic!("failed enumeration must not load a tree")
    }
}

#[tokio::test]
async fn test_起動復旧_作業列を通しても元のstore失敗を保持する() {
    use crate::domain::local_event::LocalEventQueryError;
    for source in [
        LocalEventQueryError::Corrupt {
            correlation_id: "corrupt".into(),
        },
        LocalEventQueryError::ResponseTooLarge,
        LocalEventQueryError::CanonicalWriterRequired,
        LocalEventQueryError::InvalidRequest,
    ] {
        // Given
        let queue = crate::usecase::work_queue::WorkQueueUsecase::new(Arc::new(
            crate::usecase::work_queue::ImmediateWorkQueueRuntime::default(),
        ));
        let source = crate::domain::failure::StorageFailure::from(source);
        let startup = WorkflowStartupUsecase::new(
            queue,
            Arc::new(FailingStartupList(WorkflowError::Store(source.clone()))),
            Arc::new(Startup {
                calls: Default::default(),
                failure: None,
                conflict: false,
                temporary: false,
            }),
        );
        // When
        let error = startup.execute().await.unwrap_err();
        // Then
        assert!(matches!(error, WorkflowError::Store(failure) if failure.source == source.source));
    }
}

#[derive(Default)]
struct CountingRuntime {
    inner: crate::usecase::work_queue::ImmediateWorkQueueRuntime,
    attempts: std::sync::atomic::AtomicUsize,
    expire_on: Option<usize>,
}

#[async_trait::async_trait]
impl crate::usecase::work_queue::WorkQueueRuntime for CountingRuntime {
    fn now(&self) -> std::time::Duration {
        self.inner.now()
    }
    fn timestamp_ms(&self) -> u64 {
        self.inner.timestamp_ms()
    }
    fn jitter(&self) -> f64 {
        self.inner.jitter()
    }
    fn spawn(&self, task: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>) {
        self.inner.spawn(task);
    }
    async fn sleep(&self, duration: std::time::Duration) {
        self.inner.sleep(duration).await;
    }
    async fn attempt(
        &self,
        attempt: crate::usecase::work_queue::Attempt<'_>,
    ) -> Result<Option<std::time::Duration>, crate::usecase::work_queue::WorkFailure> {
        let count = self
            .attempts
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        if self.expire_on == Some(count) {
            return Err(crate::usecase::work_queue::WorkFailure {
                kind: crate::domain::failure::Failure::Technical(
                    crate::domain::failure::TechnicalFailureNature::TimedOut,
                ),
                message: "attempt timed out".into(),
            });
        }
        self.inner.attempt(attempt).await
    }
}

#[tokio::test]
async fn test_起動復旧_列挙と各treeの期限を一度だけ適用する() {
    // Given
    let runtime = Arc::new(CountingRuntime::default());
    let startup = Arc::new(Startup {
        calls: Default::default(),
        failure: None,
        conflict: false,
        temporary: false,
    });
    let usecase = WorkflowStartupUsecase::new(
        crate::usecase::work_queue::WorkQueueUsecase::new(runtime.clone()),
        startup.clone(),
        startup,
    );
    // When
    usecase.execute().await.unwrap();
    // Then
    assert_eq!(
        runtime.attempts.load(std::sync::atomic::Ordering::SeqCst),
        3
    );
}

#[tokio::test]
async fn test_起動復旧_再試行の期限切れを直前の元失敗で上書きしない() {
    use crate::domain::failure::{StorageFailureSource, TechnicalFailureNature};
    // Given
    let runtime = Arc::new(CountingRuntime {
        expire_on: Some(2),
        ..Default::default()
    });
    let queue = crate::usecase::work_queue::WorkQueueUsecase::new(runtime);
    // When
    let result: Result<(), _> = execute_recovery(
        &queue,
        crate::usecase::work_queue::WorkKey::new("workflow_recovery", "tree"),
        |_| async {
            Err(WorkflowError::from(
                crate::domain::local_event::CommitBatchError::QueueBusy,
            ))
        },
    )
    .await;
    // Then
    let WorkflowError::Store(failure) = result.unwrap_err() else {
        panic!("expected storage failure")
    };
    assert_eq!(failure.nature, TechnicalFailureNature::TimedOut);
    assert!(
        matches!(failure.source, StorageFailureSource::Technical(error) if error.message == "attempt timed out")
    );
}
