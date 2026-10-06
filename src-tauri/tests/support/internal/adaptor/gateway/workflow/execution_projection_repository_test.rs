use releash_lib::test_support::integration::usecase::workflow::ports::WorkflowExecutionProjectionRepository;

use super::*;
use crate::adaptor::gateway::workflow::fact_log;

use crate::adaptor::gateway::local_event_store::LocalEventStoreConfig;
use crate::adaptor::gateway::workflow::{
    definition_repository::{
        WorkflowDefinitionFileRepository, WorkflowDefinitionFileSourceGateway,
    },
    event_repository::WorkflowEventLogRepository,
    facet_repository::WorkflowFacetFileRepository,
};
use crate::domain::workflow::{
    ExecutionOrigin, ExecutionParentRef, NodeKindName, SecretSourceGateway, WorkflowDefinition,
    WorkflowEvent,
};
use crate::usecase::workflow::output::WorkflowOutputUsecase;
use crate::usecase::workflow::ports::WorkflowEventRepository;
use crate::usecase::workflow::query_service::{WorkflowGetOutputResult, WorkflowQueryService};
use std::sync::atomic::{AtomicUsize, Ordering};

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

struct NoSecrets;

impl SecretSourceGateway for NoSecrets {
    fn configured_secret_values(&self) -> Result<Vec<String>, WorkflowError> {
        panic!("output get does not read secrets")
    }
}

#[tokio::test]
pub async fn test_終端の隔離node出力_旧定義でも状態と同じ保存成果を一度の読取で返す() {
    use crate::adaptor::gateway::local_event_store::node_events::NewNodeEventRow;
    use crate::domain::workflow::ExecutionStatus;

    for kind in ["sequence", "fanout"] {
        for (terminal, status) in [
            ("execution_completed", ExecutionStatus::Completed),
            ("abort_requested", ExecutionStatus::Aborted),
        ] {
            for legacy_worktree in [false, true] {
                // Given
                let directory = tempfile::TempDir::new().unwrap();
                let store = LocalEventStore::open(LocalEventStoreConfig::production(
                    directory.path().into(),
                    std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
                ))
                .unwrap();
                let id = "00000000-0000-4000-8000-000000001836";
                let mut root = serde_json::json!({
                    "worktree": {"branch": "saved-main", "path": "/saved/main"},
                    "root": {
                        "workspaceIdentity": "/repo", "worktreePath": "/repo",
                        "createdFrom": "cli", "request": "", "launchedAs": "workflow",
                        "definition": {"name": "old", "description": "", "entry": "main", "nodes": {
                            "main": {"worktree": "isolated", kind: {"children": ["work"]}},
                            "work": {"command": "true", "completion": "approval"}
                        }}
                    }
                });
                if kind == "sequence" {
                    root["root"]["definition"]["nodes"]["main"]["sequence"]["output"] =
                        serde_json::json!("work");
                }
                if legacy_worktree {
                    root.as_object_mut().unwrap().remove("worktree");
                }
                assert!(
                    super::super::stored_definition::decode_started(&root.to_string()).is_err()
                );
                let parent = if kind == "sequence" {
                    ExecutionParentRef::sequence_child("main-id")
                } else {
                    ExecutionParentRef::fanout_child("main-id", None, 0)
                };
                let mut facts = vec![("main-id", "main", kind, "started", root)];
                if legacy_worktree {
                    facts.push(("main-id", "main", kind, "isolated_worktree_created", serde_json::json!({
                        "repositoryRoot": "/repo", "worktreePath": "/saved/main", "branch": "saved-main"
                    })));
                }
                facts.extend([
                    (
                        "work-id",
                        "work",
                        "command",
                        "started",
                        serde_json::json!({"parent": parent}),
                    ),
                    (
                        "work-id",
                        "work",
                        "command",
                        "artifact_produced",
                        serde_json::json!({"value": {"answer": "kept"}}),
                    ),
                    (
                        "work-id",
                        "work",
                        "command",
                        "process_exited",
                        serde_json::json!({"exitCode": 0}),
                    ),
                    ("main-id", "main", kind, terminal, serde_json::json!({})),
                ]);
                for (index, (node, name, kind, event_type, detail)) in facts.into_iter().enumerate()
                {
                    store
                        .append_node_event(
                            NewNodeEventRow {
                                tree_id: id.into(),
                                node_execution_id: node.into(),
                                parent_id: (node == "work-id").then(|| "main-id".into()),
                                node_name: name.into(),
                                kind: kind.into(),
                                attempt: 1,
                                event_type: event_type.into(),
                                session_id: None,
                                detail: detail.to_string(),
                            },
                            Some((index as i64 + 1) * 1000),
                        )
                        .await
                        .unwrap();
                }
                let projection =
                    Arc::new(WorkflowExecutionProjectionLogRepository::new(store.clone()));
                let events = Arc::new(CountingEvents {
                    repository: WorkflowEventLogRepository::with_store(store.clone()),
                    reads: AtomicUsize::new(0),
                });
                let usecase = WorkflowOutputUsecase::new(
                    WorkflowQueryService::new(
                        Arc::new(WorkflowDefinitionFileRepository::new(
                            directory.path(),
                            directory.path(),
                        )),
                        Arc::new(WorkflowDefinitionFileSourceGateway::new(
                            directory.path(),
                            directory.path(),
                        )),
                        Arc::new(WorkflowFacetFileRepository::new(directory.path())),
                        events.clone(),
                        projection.clone(),
                    ),
                    Arc::new(NoSecrets),
                );
                let state = projection
                    .get_execution(&ExecutionTreeId::new(id).unwrap())
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(state.status, status);
                let artifact = state
                    .node_executions
                    .iter()
                    .find(|node| node.node_name == "main")
                    .unwrap()
                    .artifact
                    .as_ref();
                let expected = if status == ExecutionStatus::Completed {
                    let artifact = artifact.unwrap();
                    assert_eq!(
                        artifact.value,
                        serde_json::json!({
                            "worktree": {"branch": "saved-main", "path": "/saved/main"},
                            "work": {"answer": "kept"}
                        })
                    );
                    WorkflowGetOutputResult::Submitted {
                        contract: None,
                        submitted_at: None,
                        request_id: None,
                        timestamp: artifact.produced_at,
                        structured_output: artifact.value.clone(),
                    }
                } else {
                    assert!(artifact.is_none());
                    WorkflowGetOutputResult::NotSubmitted
                };
                // When
                let output = without_tree_fold(usecase.get_output(id, "main"))
                    .await
                    .unwrap();
                // Then
                assert_eq!(output, expected);
                assert_eq!(events.reads.load(Ordering::SeqCst), 1);
            }
        }
    }
}

#[tokio::test]
pub async fn test_隔離合成子の出力取得_保存されない成果を一度の読取で実行木の再構築なしに返す() {
    for kind in ["sequence", "fanout"] {
        // Given
        let directory = tempfile::TempDir::new().unwrap();
        let store = LocalEventStore::open(LocalEventStoreConfig::production(
            directory.path().into(),
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        ))
        .unwrap();
        let id = "00000000-0000-4000-8000-000000001733";
        let definition: WorkflowDefinition = serde_saphyr::from_str(&format!(
            "name: test\ndescription: test\nnodes:\n  main: {{worktree: isolated, {kind}: {{children: [work]}}}}\n  work: {{worktree: isolated, session: {{provider: codex}}}}"
        )).unwrap();
        let node_kind = definition.node_by_name("main").unwrap().kind_name();
        let parent = if kind == "sequence" {
            ExecutionParentRef::sequence_child("main-id")
        } else {
            ExecutionParentRef::fanout_child("main-id", None, 0)
        };
        fact_log::append_facts_for_events(
            &store,
            &[
                WorkflowEvent::ExecutionStarted {
                    execution_id: id.into(),
                    workflow_name: "test".into(),
                    repository_root: Some("/repo".into()),
                    worktree_path: "/repo".into(),
                    created_from: ExecutionOrigin::Cli,
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
        let usecase = WorkflowOutputUsecase::new(
            WorkflowQueryService::new(
                Arc::new(WorkflowDefinitionFileRepository::new(
                    directory.path(),
                    directory.path(),
                )),
                Arc::new(WorkflowDefinitionFileSourceGateway::new(
                    directory.path(),
                    directory.path(),
                )),
                Arc::new(WorkflowFacetFileRepository::new(directory.path())),
                events.clone(),
                Arc::new(WorkflowExecutionProjectionLogRepository::new(store.clone())),
            ),
            Arc::new(NoSecrets),
        );
        // When / Then
        assert_eq!(
            without_tree_fold(usecase.get_output(id, "main"))
                .await
                .unwrap(),
            WorkflowGetOutputResult::NotSubmitted
        );
        assert_eq!(events.reads.swap(0, Ordering::SeqCst), 1);
        let records = fact_log::read_tree_records(&store, id).await.unwrap();
        let leaf = records[1].meta.clone();
        fact_log::append_single_fact(
            &store,
            &leaf,
            &crate::domain::workflow::NodeFact::SubmitReceived(
                crate::domain::workflow::SubmitReceivedFact { request_id: None },
            ),
            3000,
        )
        .await
        .unwrap();
        fact_log::append_single_fact(
            &store,
            &leaf,
            &crate::domain::workflow::NodeFact::StopReceived(
                crate::domain::workflow::StopReceivedFact {
                    result_summary: None,
                    token_usage: None,
                },
            ),
            4000,
        )
        .await
        .unwrap();
        let records = fact_log::read_tree_records(&store, id).await.unwrap();
        assert!(!records.iter().any(|record| matches!(
            record.fact,
            crate::domain::workflow::NodeFact::ArtifactProduced(_)
        )));
        let main_worktree =
            crate::domain::workflow::IsolatedWorktree::for_attempt("/repo", "main-id", 1);
        let leaf_worktree =
            crate::domain::workflow::IsolatedWorktree::for_attempt("/repo", "work-id", 2);
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
        std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
    ))
    .unwrap();
    let id = "00000000-0000-4000-8000-000000001733";
    let definition: WorkflowDefinition = serde_saphyr::from_str("name: test\ndescription: test\nnodes:\n  main: {worktree: isolated, fanout: {items: [], children: [work]}}\n  work: {session: {provider: codex}}").unwrap();
    fact_log::append_facts_for_events(
        &store,
        &[
            WorkflowEvent::ExecutionStarted {
                execution_id: id.into(),
                workflow_name: "test".into(),
                repository_root: Some("/repo".into()),
                worktree_path: "/repo".into(),
                created_from: ExecutionOrigin::Cli,
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
    let usecase = WorkflowOutputUsecase::new(
        WorkflowQueryService::new(
            Arc::new(WorkflowDefinitionFileRepository::new(
                directory.path(),
                directory.path(),
            )),
            Arc::new(WorkflowDefinitionFileSourceGateway::new(
                directory.path(),
                directory.path(),
            )),
            Arc::new(WorkflowFacetFileRepository::new(directory.path())),
            events.clone(),
            Arc::new(WorkflowExecutionProjectionLogRepository::new(store.clone())),
        ),
        Arc::new(NoSecrets),
    );
    // When
    let output = without_tree_fold(usecase.get_output(id, "main"))
        .await
        .unwrap();
    // Then
    assert_eq!(events.reads.load(Ordering::SeqCst), 1);
    let records = fact_log::read_tree_records(&store, id).await.unwrap();
    let folded = fact_replay::fold_execution_tree(id, &records)
        .unwrap()
        .unwrap();
    let read_model = fact_replay::derive_read_model(&folded);
    assert_eq!(
        read_model.status,
        crate::domain::workflow::ExecutionStatus::Completed
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
        crate::domain::workflow::NodeFact::AbortRequested(Default::default()),
        crate::domain::workflow::NodeFact::RuntimeFailureObserved(
            crate::domain::workflow::RuntimeFailureObservedFact {
                reason: "creation failed".into(),
                failure_kind:
                    crate::domain::workflow::NodeExecutionFailureKind::InfrastructureCrash,
            },
        ),
    ] {
        fact_log::append_single_fact(&store, &records[0].meta, &fact, 2000)
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
        let records = fact_log::read_tree_records(&store, id).await.unwrap();
        let folded = fact_replay::fold_execution_tree(id, &records)
            .unwrap()
            .unwrap();
        let read_model = fact_replay::derive_read_model(&folded);
        assert_eq!(
            read_model.status,
            crate::domain::workflow::ExecutionStatus::Aborted
        );
        assert!(read_model.node_executions[0].artifact.is_none());
    }
}

async fn without_tree_fold<T>(read: impl std::future::Future<Output = T>) -> T {
    let mut read = std::pin::pin!(read);
    std::future::poll_fn(|context| fact_replay::without_tree_fold(|| read.as_mut().poll(context)))
        .await
}

#[tokio::test]
pub async fn test_execution読取_実経路で失敗分類を保持する() {
    use crate::adaptor::gateway::local_event_store::test_helpers::ReadFailure;
    use crate::adaptor::presenter::connect::classified_error;
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().into(),
        std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
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
