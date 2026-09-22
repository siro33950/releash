use super::*;
use crate::domain::workflow::entities::workflow_execution::ExecutionTree;
use crate::domain::workflow::repository::WorkflowStartupRecord;
use crate::domain::workflow::{
    ExecutionOrigin, ExecutionTreeLaunch, NodeFact, NodeFactMeta, NodeKindName, TreeRootFact,
};
use std::sync::Mutex;

struct Repository {
    terminal: Mutex<Option<NodeFact>>,
    head: Mutex<i64>,
    concurrent_facts: Mutex<std::collections::VecDeque<Option<NodeFact>>>,
    unpersisted_appends: Mutex<usize>,
    append_attempts: Mutex<Vec<Option<i64>>>,
    unreadable: bool,
    fail_load: bool,
    fail_append: bool,
    appended: Mutex<Vec<(NodeFact, f64)>>,
}

impl WorkflowStartupRepository for Repository {
    fn list_tree_ids(&self) -> Result<Vec<String>, WorkflowError> {
        Ok(vec!["tree".into()])
    }

    fn load(&self, tree_id: &str) -> Result<Option<WorkflowStartupRecord>, WorkflowError> {
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
            root: NodeFactMeta {
                tree_id: tree_id.into(),
                node_execution_id: "root".into(),
                parent_id: None,
                node_name: "main".into(),
                kind: NodeKindName::Command,
                attempt: 1,
            },
            definition_error: self
                .unreadable
                .then(|| "Workflow definition is unavailable: completion".into()),
            head: *self.head.lock().unwrap(),
        }))
    }
    fn append(
        &self,
        root: &NodeFactMeta,
        fact: &NodeFact,
        timestamp: f64,
        expected_head: Option<i64>,
    ) -> Result<(), WorkflowError> {
        assert_eq!(root.tree_id, "tree");
        assert_eq!(root.node_execution_id, "root");
        self.append_attempts.lock().unwrap().push(expected_head);
        let mut head = self.head.lock().unwrap();
        if let Some(concurrent) = self.concurrent_facts.lock().unwrap().pop_front() {
            *head += 1;
            if let Some(fact) = concurrent {
                *self.terminal.lock().unwrap() = Some(fact);
            }
        }
        if expected_head.is_some_and(|expected| expected != *head) {
            return Err(WorkflowError::Conflict("head advanced".into()));
        }
        if self.fail_append {
            return Err(WorkflowError::external("append failed"));
        }
        let mut unpersisted = self.unpersisted_appends.lock().unwrap();
        if *unpersisted > 0 {
            *unpersisted -= 1;
            return Err(WorkflowError::Conflict("abort was not persisted".into()));
        }
        self.appended
            .lock()
            .unwrap()
            .push((fact.clone(), timestamp));
        *self.terminal.lock().unwrap() = Some(fact.clone());
        *head += 1;
        Ok(())
    }
}

fn repository() -> Repository {
    Repository {
        terminal: Mutex::new(None),
        head: Mutex::new(1),
        concurrent_facts: Default::default(),
        unpersisted_appends: Default::default(),
        append_attempts: Default::default(),
        unreadable: true,
        fail_load: false,
        fail_append: false,
        appended: Mutex::new(Vec::new()),
    }
}

#[test]
fn test_起動時abort_理由付き事実を保存し再実行では追記しない() {
    // Given
    let repository = repository();
    // When
    abort_unavailable_definition(&repository, "tree", 3.0).unwrap();
    abort_unavailable_definition(&repository, "tree", 4.0).unwrap();
    // Then
    assert_eq!(
        *repository.appended.lock().unwrap(),
        vec![(
            NodeFact::AbortRequested(crate::domain::workflow::AbortRequestedFact {
                reason: Some("Workflow definition is unavailable: completion".into())
            }),
            3.0
        )]
    );
}

#[test]
fn test_起動時abort_読める定義と既存の終端事実には追記しない() {
    // Given
    for terminal in [
        None,
        Some(NodeFact::ExecutionCompleted),
        Some(NodeFact::AbortRequested(Default::default())),
    ] {
        let mut repository = repository();
        repository.unreadable = terminal.is_some();
        *repository.terminal.lock().unwrap() = terminal;
        // When
        abort_unavailable_definition(&repository, "tree", 3.0).unwrap();
        // Then
        assert!(repository.appended.lock().unwrap().is_empty());
    }
}

#[test]
fn test_起動時abort_読取と追記の失敗を返し再試行で保存できる() {
    // Given
    for fail_load in [true, false] {
        let mut repository = repository();
        repository.fail_load = fail_load;
        repository.fail_append = !fail_load;
        // When / Then
        assert!(abort_unavailable_definition(&repository, "tree", 3.0).is_err());
        assert!(repository.appended.lock().unwrap().is_empty());
        repository.fail_load = false;
        repository.fail_append = false;
        abort_unavailable_definition(&repository, "tree", 4.0).unwrap();
        assert_eq!(repository.appended.lock().unwrap().len(), 1);
    }
}

struct Startup {
    calls: Mutex<Vec<String>>,
    failure: Option<&'static str>,
    conflict: bool,
}

impl Startup {
    fn record(&self, call: String) -> Result<(), WorkflowError> {
        self.calls.lock().unwrap().push(call.clone());
        if self.failure == Some(call.as_str()) {
            if self.conflict {
                Err(WorkflowError::Conflict(call))
            } else {
                Err(WorkflowError::external(call))
            }
        } else {
            Ok(())
        }
    }
}

impl WorkflowStartupRepository for Startup {
    fn list_tree_ids(&self) -> Result<Vec<String>, WorkflowError> {
        self.record("list".into())?;
        Ok(["first", "second"].map(String::from).into())
    }

    fn load(&self, tree_id: &str) -> Result<Option<WorkflowStartupRecord>, WorkflowError> {
        self.record(format!("load:{tree_id}"))?;
        repository().load(tree_id)
    }

    fn append(
        &self,
        root: &NodeFactMeta,
        fact: &NodeFact,
        timestamp: f64,
        _expected_head: Option<i64>,
    ) -> Result<(), WorkflowError> {
        assert!(matches!(fact, NodeFact::AbortRequested(_)));
        assert_eq!(timestamp, 3.0);
        self.record(format!("append:{}", root.tree_id))
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
async fn test_起動時復旧_列挙とabort保存とreconciliationの順序を所有する() {
    // Given
    let startup = Arc::new(Startup {
        calls: Mutex::new(Vec::new()),
        failure: None,
        conflict: false,
    });
    let usecase = WorkflowStartupUsecase::new(startup.clone(), startup.clone());

    // When
    let (first, second) = tokio::join!(usecase.execute(), usecase.execute());
    first.unwrap();
    second.unwrap();

    // Then
    let pass = [
        "list",
        "load:first",
        "append:first",
        "reconcile:first",
        "load:second",
        "append:second",
        "reconcile:second",
    ];
    assert_eq!(*startup.calls.lock().unwrap(), pass);
}

#[tokio::test]
async fn test_起動時復旧_abort失敗したtreeを再生せず後続を処理して最初の失敗を返す() {
    for conflict in [false, true] {
        for failure in ["load:first", "append:first", "reconcile:first"] {
            // Given
            let startup = Arc::new(Startup {
                calls: Mutex::new(Vec::new()),
                failure: Some(failure),
                conflict,
            });
            let usecase = WorkflowStartupUsecase::new(startup.clone(), startup.clone());

            // When
            let error = usecase.execute().await.unwrap_err();
            usecase.execute().await.unwrap();

            // Then
            assert!(error.to_string().contains(failure));
            let calls = startup.calls.lock().unwrap();
            assert!(
                calls
                    .iter()
                    .filter(|call| call.as_str() == "reconcile:first")
                    .count()
                    <= 1
            );
            assert_eq!(
                calls.contains(&"reconcile:first".into()),
                failure == "reconcile:first"
            );
            assert_eq!(
                &calls[calls.len() - 3..],
                ["load:second", "append:second", "reconcile:second"]
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
    });
    let usecase = WorkflowStartupUsecase::new(startup.clone(), startup.clone());
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
async fn test_起動時前進_競合でも前進を再試行せず理由付きでabortする() {
    for conflicts in [1, super::super::command::CONTROL_PLANE_MAX_ATTEMPTS] {
        // Given
        let mut repository = repository();
        repository.unreadable = false;
        let repository = Arc::new(repository);
        let runtime = Arc::new(ConflictingStartup {
            conflicts,
            calls: Default::default(),
        });
        let usecase = WorkflowStartupUsecase::new(repository.clone(), runtime.clone());
        // When
        let result = usecase.execute().await;
        usecase.execute().await.unwrap();
        // Then
        assert!(result.unwrap_err().to_string().contains("head advanced"));
        assert_eq!(runtime.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        let appended = repository.appended.lock().unwrap();
        assert_eq!(appended.len(), 1);
        assert!(matches!(&appended[0].0, NodeFact::AbortRequested(fact)
            if fact.reason.as_ref().is_some_and(|reason| reason.contains("head advanced"))));
        assert_eq!(
            *repository.terminal.lock().unwrap(),
            Some(appended[0].0.clone())
        );
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
        let usecase = WorkflowStartupUsecase::new(repository.clone(), runtime.clone());

        // When
        assert!(usecase.execute().await.is_err());
        usecase.execute().await.unwrap();

        // Then
        assert_eq!(*repository.terminal.lock().unwrap(), Some(terminal));
        assert!(repository.appended.lock().unwrap().is_empty());
        assert_eq!(*repository.append_attempts.lock().unwrap(), [Some(1)]);
        assert_eq!(runtime.0.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn test_起動失敗abort_競合時だけ最新記録で有界に再判定し前進は繰り返さない() {
    for conflicts in [0, 1, super::super::command::CONTROL_PLANE_MAX_ATTEMPTS] {
        // Given
        let mut repository = repository();
        repository.unreadable = false;
        *repository.concurrent_facts.lock().unwrap() = vec![None; conflicts].into();
        let repository = Arc::new(repository);
        let runtime = Arc::new(FailedStartup::default());
        let usecase = WorkflowStartupUsecase::new(repository.clone(), runtime.clone());

        // When
        assert!(usecase.execute().await.is_err());
        usecase.execute().await.unwrap();

        // Then
        let limit = super::super::command::CONTROL_PLANE_MAX_ATTEMPTS;
        assert_eq!(
            *repository.append_attempts.lock().unwrap(),
            (1..=(conflicts + 1).min(limit))
                .map(|head| Some(head as i64))
                .collect::<Vec<_>>()
        );
        let appended = repository.appended.lock().unwrap();
        assert_eq!(appended.len(), usize::from(conflicts < limit));
        if conflicts < limit {
            assert!(matches!(&appended[0].0, NodeFact::AbortRequested(fact)
                if fact.reason.as_ref().is_some_and(|reason| reason.contains("advancement failed"))));
        }
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
    let usecase = WorkflowStartupUsecase::new(repository.clone(), runtime.clone());

    // When
    assert!(usecase.execute().await.is_err());
    usecase.execute().await.unwrap();

    // Then
    assert_eq!(*repository.append_attempts.lock().unwrap(), [Some(1)]);
    assert!(repository.appended.lock().unwrap().is_empty());
    assert_eq!(runtime.0.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_起動時abort_未保存なら有界に再評価して保存し前進は繰り返さない() {
    for unreadable in [false, true] {
        for unpersisted in [1, super::super::command::CONTROL_PLANE_MAX_ATTEMPTS] {
            // Given
            let mut repository = repository();
            repository.unreadable = unreadable;
            *repository.unpersisted_appends.lock().unwrap() = unpersisted;
            let repository = Arc::new(repository);
            let runtime = Arc::new(FailedStartup::default());
            let usecase = WorkflowStartupUsecase::new(repository.clone(), runtime.clone());

            // When
            assert!(usecase.execute().await.is_err());
            usecase.execute().await.unwrap();

            // Then
            let limit = super::super::command::CONTROL_PLANE_MAX_ATTEMPTS;
            assert_eq!(
                *repository.append_attempts.lock().unwrap(),
                vec![if unreadable { None } else { Some(1) }; (unpersisted + 1).min(limit)]
            );
            let appended = repository.appended.lock().unwrap();
            assert_eq!(appended.len(), usize::from(unpersisted < limit));
            if unpersisted < limit {
                let reason = if unreadable {
                    "definition is unavailable"
                } else {
                    "advancement failed"
                };
                assert!(matches!(&appended[0].0, NodeFact::AbortRequested(fact)
                    if fact.reason.as_ref().is_some_and(|value| value.contains(reason))));
            }
            assert_eq!(
                runtime.0.load(std::sync::atomic::Ordering::SeqCst),
                usize::from(!unreadable || unpersisted < limit)
            );
        }
    }
}
