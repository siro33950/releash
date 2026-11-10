use releashd::test_support::integration::workflow::WorkflowExecutionProjectionRepository;

use releashd::test_support::integration::persistence::LocalEventStore;
use releashd::test_support::integration::workflow::ExecutionTreeId;
use releashd::test_support::integration::workflow::WorkflowError;
use releashd::test_support::integration::workflow::WorkflowEventDraft;
use releashd::test_support::integration::workflow::WorkflowExecutionProjectionLogRepository;
use std::sync::Arc;

use releashd::test_support::integration::persistence::LocalEventStoreConfig;
use releashd::test_support::integration::workflow::ExecutionParentRef;
use releashd::test_support::integration::workflow::NodeKindName;

use releashd::test_support::integration::workflow::WorkflowDefinitionContent;
use releashd::test_support::integration::workflow::WorkflowDefinitionFileRepository;

use releashd::test_support::integration::workflow::WorkflowEvent;
use releashd::test_support::integration::workflow::WorkflowEventLogRepository;
use releashd::test_support::integration::workflow::WorkflowEventRepository;
use releashd::test_support::integration::workflow::WorkflowGetOutputResult;
use releashd::test_support::integration::workflow::WorkflowOutputUsecase;
use releashd::test_support::integration::workflow::WorkflowQueryService;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

struct CountingEvents {
    repository: WorkflowEventLogRepository,
    reads: AtomicUsize,
}

#[async_trait::async_trait]
impl WorkflowEventRepository for CountingEvents {
    async fn read(&self, id: &ExecutionTreeId) -> Result<Vec<WorkflowEventDraft>, WorkflowError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.repository.read(id).await
    }
}

#[tokio::test]
pub async fn test_隔離合成子の出力取得_保存されない成果を一度の読取で実行木の再構築なしに返す() {
    for kind in ["sequence", "fanout"] {
        // Given
        let directory = tempfile::TempDir::new().unwrap();
        let store = LocalEventStore::open(LocalEventStoreConfig::production(
            directory.path().into(),
            std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
        ))
        .unwrap();
        let id = "00000000-0000-4000-8000-000000001733";
        let definition: WorkflowDefinitionContent = serde_saphyr::from_str(&format!(
            "name: test\ndescription: test\nnodes:\n  main: {{worktree: isolated, {kind}: {{children: [work]}}}}\n  work: {{worktree: isolated, session: {{provider: codex}}}}"
        )).unwrap();
        let node_kind = definition.node_by_name("main").unwrap().kind_name();
        let parent = if kind == "sequence" {
            ExecutionParentRef::sequence_child("main-id")
        } else {
            ExecutionParentRef::fanout_child("main-id", None, 0)
        };
        releashd::test_support::integration::workflow::append_facts_for_events(
            &store,
            &[
                WorkflowEvent::ExecutionStarted {
                    execution_id: id.into(),
                    workflow_name: "test".into(),
                    repository_root: Some("/repo".into()),
                    worktree_path: "/repo".into(),
                    request: String::new(),
                    definition,
                    timestamp: 1.0,
                },
                WorkflowEvent::NodeStarted {
                    worktree: None,
                    execution_id: id.into(),
                    node_execution_id: "main-id".into(),
                    node_name: "main".into(),
                    kind: node_kind,
                    attempt: 1,
                    parent: None,
                    timestamp: 1.0,
                },
                WorkflowEvent::NodeStarted {
                    worktree: None,
                    execution_id: id.into(),
                    node_execution_id: "work-id".into(),
                    node_name: "work".into(),
                    kind: NodeKindName::Session,
                    attempt: 2,
                    parent: Some(parent),
                    timestamp: 2.0,
                },
            ],
        )
        .await
        .unwrap();
        let events = Arc::new(CountingEvents {
            repository: WorkflowEventLogRepository::with_store(store.clone()),
            reads: AtomicUsize::new(0),
        });
        let usecase = WorkflowOutputUsecase::new(WorkflowQueryService::new(
            Arc::new(WorkflowDefinitionFileRepository::new(
                directory.path(),
                directory.path(),
            )),
            events.clone(),
            Arc::new(WorkflowExecutionProjectionLogRepository::new(store.clone())),
        ));
        // When / Then
        assert_eq!(
            without_tree_fold(usecase.get_output(id, "main"))
                .await
                .unwrap(),
            WorkflowGetOutputResult::NotSubmitted
        );
        assert_eq!(events.reads.swap(0, Ordering::SeqCst), 1);
        let records = releashd::test_support::integration::workflow::read_tree_records(&store, id)
            .await
            .unwrap();
        let leaf = records[1].meta.clone();
        releashd::test_support::integration::workflow::append_single_fact(
            &store,
            &leaf,
            &releashd::test_support::integration::workflow::NodeFact::SubmitReceived(
                releashd::test_support::integration::workflow::SubmitReceivedFact {
                    request_id: None,
                },
            ),
            3000,
        )
        .await
        .unwrap();
        releashd::test_support::integration::workflow::append_single_fact(
            &store,
            &leaf,
            &releashd::test_support::integration::workflow::NodeFact::StopReceived(
                releashd::test_support::integration::workflow::StopReceivedFact {
                    result_summary: None,
                    token_usage: None,
                },
            ),
            4000,
        )
        .await
        .unwrap();
        let records = releashd::test_support::integration::workflow::read_tree_records(&store, id)
            .await
            .unwrap();
        assert!(!records.iter().any(|record| matches!(
            record.fact,
            releashd::test_support::integration::workflow::NodeFact::ArtifactProduced(_)
        )));
        let main_worktree =
            releashd::test_support::integration::workflow::IsolatedWorktree::for_attempt(
                "/repo", "main-id", 1,
            );
        let leaf_worktree =
            releashd::test_support::integration::workflow::IsolatedWorktree::for_attempt(
                "/repo", "work-id", 2,
            );
        assert_eq!(
            without_tree_fold(usecase.get_output(id, "main"))
                .await
                .unwrap(),
            WorkflowGetOutputResult::Submitted {
                contract: None,
                submitted_at: None,
                request_id: None,
                timestamp: 4.0,
                structured_output: serde_json::json!({ "worktree": {"branch": main_worktree.branch, "path": main_worktree.path}, "work": {"worktree": {"branch": leaf_worktree.branch, "path": leaf_worktree.path}} }),
            }
        );
        assert_eq!(events.reads.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
pub async fn test_空の隔離fanout出力_保存事実を一度だけ読みstatusと同じ完了成果を返す() {
    // Given
    let directory = tempfile::TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().into(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let id = "00000000-0000-4000-8000-000000001733";
    let definition: WorkflowDefinitionContent = serde_saphyr::from_str("name: test\ndescription: test\nnodes:\n  main: {worktree: isolated, fanout: {items: [], children: [work]}}\n  work: {session: {provider: codex}}").unwrap();
    releashd::test_support::integration::workflow::append_facts_for_events(
        &store,
        &[
            WorkflowEvent::ExecutionStarted {
                execution_id: id.into(),
                workflow_name: "test".into(),
                repository_root: Some("/repo".into()),
                worktree_path: "/repo".into(),
                request: String::new(),
                definition,
                timestamp: 1.0,
            },
            WorkflowEvent::NodeStarted {
                worktree: None,
                execution_id: id.into(),
                node_execution_id: "main-id".into(),
                node_name: "main".into(),
                kind: NodeKindName::Fanout,
                attempt: 1,
                parent: None,
                timestamp: 1.0,
            },
        ],
    )
    .await
    .unwrap();
    let events = Arc::new(CountingEvents {
        repository: WorkflowEventLogRepository::with_store(store.clone()),
        reads: AtomicUsize::new(0),
    });
    let usecase = WorkflowOutputUsecase::new(WorkflowQueryService::new(
        Arc::new(WorkflowDefinitionFileRepository::new(
            directory.path(),
            directory.path(),
        )),
        events.clone(),
        Arc::new(WorkflowExecutionProjectionLogRepository::new(store.clone())),
    ));
    // When
    let output = without_tree_fold(usecase.get_output(id, "main"))
        .await
        .unwrap();
    // Then
    assert_eq!(events.reads.load(Ordering::SeqCst), 1);
    let records = releashd::test_support::integration::workflow::read_tree_records(&store, id)
        .await
        .unwrap();
    let folded = releashd::test_support::integration::workflow::fold_execution_tree(id, &records)
        .unwrap()
        .unwrap();
    let read_model = releashd::test_support::integration::workflow::derive_read_model(&folded);
    assert_eq!(
        read_model.status,
        releashd::test_support::integration::workflow::ExecutionStatus::Completed
    );
    let artifact = read_model.node_executions[0].artifact.as_ref().unwrap();
    assert_eq!(
        output,
        WorkflowGetOutputResult::Submitted {
            contract: None,
            submitted_at: None,
            request_id: None,
            timestamp: 1.0,
            structured_output: artifact.value.clone(),
        }
    );
    assert_eq!(artifact.value.as_object().unwrap().len(), 1);
    assert_eq!(
        artifact.value["worktree"]["branch"],
        "releash/isolated/main-id-a1"
    );
    // Given: approval を宣言しない同じ空 Fanout に保存済み abort がある
    for fact in [
        releashd::test_support::integration::workflow::NodeFact::AbortRequested(Default::default()),
        releashd::test_support::integration::workflow::NodeFact::RuntimeFailureObserved(
            releashd::test_support::integration::workflow::RuntimeFailureObservedFact {
                reason: "creation failed".into(),
                failure_kind:
                    releashd::test_support::integration::workflow::NodeExecutionFailureKind::InfrastructureCrash,
            },
        ),
    ] {
        releashd::test_support::integration::workflow::append_single_fact(&store, &records[0].meta, &fact, 2000)
            .await
            .unwrap();
        events.reads.store(0, Ordering::SeqCst);
        // When / Then
        assert_eq!(
            without_tree_fold(usecase.get_output(id, "main"))
                .await
                .unwrap(),
            WorkflowGetOutputResult::NotSubmitted
        );
        assert_eq!(events.reads.load(Ordering::SeqCst), 1);
        let records = releashd::test_support::integration::workflow::read_tree_records(&store, id).await.unwrap();
        let folded = releashd::test_support::integration::workflow::fold_execution_tree(id, &records)
            .unwrap()
            .unwrap();
        let read_model = releashd::test_support::integration::workflow::derive_read_model(&folded);
        assert_eq!(
            read_model.status,
            releashd::test_support::integration::workflow::ExecutionStatus::Aborted
        );
        assert!(read_model.node_executions[0].artifact.is_none());
    }
}

async fn without_tree_fold<T>(read: impl std::future::Future<Output = T>) -> T {
    let mut read = std::pin::pin!(read);
    std::future::poll_fn(|context| {
        releashd::test_support::integration::workflow::without_tree_fold(|| {
            read.as_mut().poll(context)
        })
    })
    .await
}

#[tokio::test]
pub async fn test_execution読取_実経路で失敗分類を保持する() {
    use releashd::test_support::integration::persistence::ReadFailure;
    use releashd::test_support::integration::transport::classified_error;
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().into(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let repository = WorkflowExecutionProjectionLogRepository::new(store.clone());
    let id = ExecutionTreeId::new("00000000-0000-4000-8000-000000000001").unwrap();
    for (failure, expected) in ReadFailure::cases() {
        store.fail_next_read(failure);
        // When
        let error = repository.get_execution(&id).await.unwrap_err();
        // Then
        assert_eq!(classified_error(error).code, expected);
    }
}
