use super::*;

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
    fn append(&self, _: &WorkflowEventDraft) -> Result<(), WorkflowError> {
        panic!("output get is read-only")
    }

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
async fn test_終端の隔離node出力_旧定義でも状態と同じ保存成果を一度の読取で返す() {
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
                        .append_node_event_blocking(
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
async fn test_隔離合成子の出力取得_保存されない成果を一度の読取で実行木の再構築なしに返す() {
    for kind in ["sequence", "fanout"] {
        // Given
        let directory = tempfile::TempDir::new().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into()))
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

#[test]
fn test_成果の事実変換_所有者と親とattemptと順序を保持する() {
    // Given
    let events = vec![
        WorkflowEventDraft {
            execution_id: "tree".into(),
            event_kind: "started".into(),
            timestamp: 1.0,
            payload: serde_json::json!({"nodeExecutionId": "child", "nodeName": "work", "kind": "session", "attempt": 2, "parent": {"parentId": "parent"}}),
        },
        WorkflowEventDraft {
            execution_id: "tree".into(),
            event_kind: "artifact_produced".into(),
            timestamp: 2.0,
            payload: serde_json::json!({"nodeExecutionId": "child", "nodeName": "work", "kind": "session", "attempt": 2, "contract": "result", "value": {"result": true}, "requestId": "request"}),
        },
    ];
    // When
    let records = records_from_drafts(&events).unwrap();
    // Then
    assert_eq!(records[0].meta, records[1].meta);
    assert_eq!(records[1].meta.tree_id, "tree");
    assert_eq!(records[1].meta.node_execution_id, "child");
    assert_eq!(records[1].meta.parent_id.as_deref(), Some("parent"));
    assert_eq!(records[1].meta.attempt, 2);
    assert_eq!(records[1].seq, 2);
    assert_eq!(records[1].timestamp_ms, 2000);
    let crate::domain::workflow::NodeFact::ArtifactProduced(artifact) = &records[1].fact else {
        panic!("artifact fact expected");
    };
    assert_eq!(artifact.request_id.as_deref(), Some("request"));
    assert_eq!(artifact.value, serde_json::json!({"result": true}));
}

#[test]
fn test_成果の事実変換_破損した識別情報と未知の事実を拒否する() {
    // Given
    let valid = serde_json::json!({"nodeExecutionId": "child", "nodeName": "work", "kind": "session", "attempt": 2});
    for (kind, payload) in [("started", serde_json::json!({})), ("unknown", valid)] {
        let event = WorkflowEventDraft {
            execution_id: "tree".into(),
            event_kind: kind.into(),
            timestamp: 1.0,
            payload,
        };
        // When / Then
        assert!(records_from_drafts(&[event]).is_err());
    }
}

#[test]
fn test_成果の事実変換_旧隔離事実を状態入力に復活させず順序を保持する() {
    // Given
    let events = ["isolated_worktree_created", "isolated_worktree_released", "isolated_worktree_lost", "submit_received"]
        .into_iter().map(|event_kind| WorkflowEventDraft {
            execution_id: "tree".into(), event_kind: event_kind.into(), timestamp: 1.0,
            payload: serde_json::json!({"nodeExecutionId": "child", "nodeName": "work", "kind": "session", "attempt": 1,
                "repositoryRoot": "/repo", "worktreePath": "/old", "branch": "old"}),
        }).collect::<Vec<_>>();
    // When
    let records = records_from_drafts(&events).unwrap();
    // Then
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].seq, 4);
    assert!(matches!(
        records[0].fact,
        crate::domain::workflow::NodeFact::SubmitReceived(_)
    ));
}

#[tokio::test]
async fn test_空の隔離fanout出力_保存事実を一度だけ読みstatusと同じ完了成果を返す() {
    // Given
    let directory = tempfile::TempDir::new().unwrap();
    let store =
        LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into())).unwrap();
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
        fact_log::append_single_fact(&store, &records[0].meta, &fact, 2000).unwrap();
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
async fn test_execution読取_実経路で失敗分類を保持する() {
    use crate::adaptor::gateway::local_event_store::test_helpers::ReadFailure;
    use crate::adaptor::protocol::connect::classified_error;
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store =
        LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into())).unwrap();
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
