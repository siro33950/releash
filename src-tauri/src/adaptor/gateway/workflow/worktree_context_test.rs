use super::*;
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::adaptor::gateway::workflow::fact_log;
use crate::domain::workflow::*;
use std::sync::Arc;

const TREE: &str = "00000000-0000-4000-8000-000000001733";
const ROOT: &str = "/repo-worktrees/development";

struct Fixture {
    directory: tempfile::TempDir,
    store: Arc<LocalEventStore>,
    backend: FactLogReadBackend,
    root: NodeFactMeta,
    child: NodeFactMeta,
}

impl Fixture {
    fn new(isolated_child: bool) -> Self {
        Self::with_contract(isolated_child, false)
    }

    fn with_contract(isolated_child: bool, contract: bool) -> Self {
        Self::with_slots(isolated_child, contract, false)
    }

    fn with_slots(isolated_child: bool, contract: bool, fanout: bool) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into()))
                .unwrap();
        let mut definition: WorkflowDefinition = serde_saphyr::from_str(&format!("name: context\ndescription: test\nnodes:\n  main: {{worktree: isolated, sequence: {{children: [child]}}}}\n  child: {{worktree: {}, session: {{provider: codex, facets: {{instruction: policy-confirmation}}}}}}", if isolated_child {"isolated"} else {"shared"})).unwrap();
        if fanout {
            definition.nodes.iter_mut().find(|node| node.name == "main").unwrap().kind =
                serde_saphyr::from_str::<WorkflowDefinition>("name: test\ndescription: test\nnodes:\n  main: {fanout: {items: [x, y], children: [child]}}\n  child: {session: {provider: codex}}")
                    .unwrap().node_by_name("main").unwrap().kind.clone();
        }
        if contract {
            definition.schemas.insert(
                "result".into(),
                SchemaDef::Object {
                    properties: [("summary".into(), SchemaDef::String { r#enum: None })].into(),
                    required: ["summary".into()].into(),
                },
            );
            definition
                .nodes
                .iter_mut()
                .find(|node| node.name == "child")
                .unwrap()
                .artifact = Some("result".into());
        }
        let root = NodeFactMeta {
            tree_id: TREE.into(),
            node_execution_id: TREE.into(),
            parent_id: None,
            node_name: "main".into(),
            kind: if fanout {
                NodeKindName::Fanout
            } else {
                NodeKindName::Sequence
            },
            attempt: 1,
        };
        fact_log::append_single_fact(
            &store,
            &root,
            &NodeFact::Started(StartedFact {
                worktree: Some(IsolatedWorktree::for_attempt("/repo", TREE, 1)),
                parent: None,
                root: Some(Box::new(TreeRootFact {
                    repository_root: Some("/repo".into()),
                    workspace_identity: ROOT.into(),
                    worktree_path: ROOT.into(),
                    created_from: ExecutionOrigin::Cli,
                    request: String::new(),
                    workflow_name: definition.name.clone(),
                    definition: Some(definition),
                    launched_as: ExecutionTreeLaunch::Workflow,
                })),
            }),
            1,
        )
        .unwrap();
        let child = NodeFactMeta {
            tree_id: TREE.into(),
            node_execution_id: "child-attempt".into(),
            parent_id: Some(TREE.into()),
            node_name: "child".into(),
            kind: NodeKindName::Session,
            attempt: 2,
        };
        fact_log::append_single_fact(
            &store,
            &child,
            &NodeFact::Started(StartedFact {
                worktree: isolated_child
                    .then(|| IsolatedWorktree::for_attempt("/repo", &child.node_execution_id, 2)),
                parent: Some(if fanout {
                    ExecutionParentRef::fanout_child(TREE, Some(0), 0)
                } else {
                    ExecutionParentRef::sequence_child(TREE)
                }),
                root: None,
            }),
            2,
        )
        .unwrap();
        let backend = FactLogReadBackend::Live(store.clone());
        Self {
            directory,
            store,
            backend,
            root,
            child,
        }
    }

    fn read(&self) -> crate::usecase::workflow::WorkflowReadUsecase {
        crate::adaptor::controller::wiring::build_canonical_workflow_read_usecase(
            self.directory.path(),
            Some(self.directory.path().join("workflows")),
        )
        .unwrap()
    }

    fn append_child(&self, fact: NodeFact) {
        fact_log::append_single_fact(&self.store, &self.child, &fact, 3).unwrap();
    }
}

#[tokio::test]
async fn test_隔離読み取り_sessionの起動先を直近の隔離祖先から導出しworkspaceを維持する() {
    // Given
    for isolated_child in [false, true] {
        let fixture = Fixture::new(isolated_child);
        let owner = if isolated_child {
            &fixture.child
        } else {
            &fixture.root
        };
        let expected =
            IsolatedWorktree::for_attempt("/repo", &owner.node_execution_id, owner.attempt);

        // When
        use crate::domain::agent_session::repository::AgentSessionRepository;
        fixture.append_child(NodeFact::SessionAttached(SessionAttachedFact {
            session_id: "agent".into(),
            provider_session_id: None,
            transcript_ref: None,
            initial_instruction_admitted: false,
        }));
        let repository = crate::adaptor::gateway::agent_session::LocalAgentSessionRepository::new(
            fixture.store.clone(),
        );
        let context = repository.find("agent").await.unwrap().unwrap();
        let workspace = workspace_worktree_path(&fixture.backend, &expected.path)
            .await
            .unwrap();

        // Then
        assert_eq!(context.session().worktree_path(), expected.path);
        assert_eq!(context.session().workspace().as_str(), ROOT);
        assert_eq!(workspace, ROOT);
        assert_eq!(
            crate::adaptor::controller::wiring::build_workspace_worktree_path_usecase(
                fixture.directory.path()
            )
            .workspace_worktree_path(&expected.path)
            .await
            .unwrap(),
            ROOT
        );
    }
}

#[tokio::test]
async fn test_隔離読み取り_実体なしでも実行中と失敗後とabort後のbranchとpathを再構築する() {
    // Given
    for terminal in [
        None,
        Some(NodeFact::RuntimeFailureObserved(
            RuntimeFailureObservedFact {
                reason: "creation failed".into(),
                failure_kind: NodeExecutionFailureKind::InfrastructureCrash,
            },
        )),
        Some(NodeFact::AbortRequested(Default::default())),
    ] {
        let fixture = Fixture::new(true);
        let expected = IsolatedWorktree::for_attempt("/repo", &fixture.child.node_execution_id, 2);
        if let Some(fact) = terminal {
            fixture.append_child(fact);
        }

        // When
        let state = fixture
            .read()
            .get_execution_state(TREE)
            .await
            .unwrap()
            .unwrap();
        let node = state
            .node_executions
            .iter()
            .find(|node| node.id == fixture.child.node_execution_id)
            .unwrap();
        let dto = crate::adaptor::presenter::workflow::workflow_execution_to_view(state.clone());

        // Then
        assert_eq!(node.worktree.as_ref(), Some(&expected));
        assert!(node.artifact.is_none());
        let value = serde_json::to_value(dto).unwrap();
        let nodes = value["nodeExecutions"].as_array().unwrap();
        let node = nodes
            .iter()
            .find(|node| node["id"] == fixture.child.node_execution_id)
            .unwrap();
        assert_eq!(node["attempt"], fixture.child.attempt);
        assert_eq!(node["worktree"]["branch"], expected.branch);
        assert_eq!(node["worktree"]["path"], expected.path);
    }
}

#[tokio::test]
async fn test_隔離出力_再構築後もcontractなしsessionと合成子の成果を取得する() {
    // Given
    let fixture = Fixture::new(true);
    fixture.append_child(NodeFact::SubmitReceived(SubmitReceivedFact {
        request_id: Some("submitted".into()),
    }));
    fixture.append_child(NodeFact::StopReceived(StopReceivedFact {
        result_summary: None,
        token_usage: None,
    }));
    let expected = IsolatedWorktree::for_attempt("/repo", &fixture.child.node_execution_id, 2);

    // When
    let read = fixture.read();
    let child = read.get_output(TREE, "child").await.unwrap();
    let composite = read.get_output(TREE, "main").await.unwrap();

    // Then
    let crate::usecase::workflow::WorkflowGetOutputResult::Submitted {
        structured_output: child,
        ..
    } = child
    else {
        panic!("child output is missing");
    };
    assert_eq!(
        child,
        serde_json::json!({"worktree": {"branch": expected.branch, "path": expected.path}})
    );
    let crate::usecase::workflow::WorkflowGetOutputResult::Submitted {
        structured_output: composite,
        ..
    } = composite
    else {
        panic!("composite output is missing");
    };
    assert_eq!(composite["child"], child);
    assert_eq!(
        composite["worktree"]["path"],
        IsolatedWorktree::for_attempt("/repo", TREE, 1).path
    );
}

#[tokio::test]
async fn test_隔離読み取り_所有者やattemptや配置が一致しないpathを拒否する() {
    // Given
    let fixture = Fixture::new(true);
    let owned = IsolatedWorktree::for_attempt("/repo", &fixture.child.node_execution_id, 2);

    // When / Then
    for path in [
        IsolatedWorktree::for_attempt("/repo", "unknown", 2).path,
        IsolatedWorktree::for_attempt("/repo", &fixture.child.node_execution_id, 3).path,
        owned.path.replace("/repo-worktrees", "/another-worktrees"),
    ] {
        assert!(
            workspace_worktree_path(&fixture.backend, &path)
                .await
                .is_err(),
            "{path}"
        );
    }
    assert_eq!(
        workspace_worktree_path(&fixture.backend, ROOT)
            .await
            .unwrap(),
        ROOT
    );
}

#[tokio::test]
async fn test_隔離出力_contractの提出情報を保ちworktreeを合成する() {
    // Given
    let fixture = Fixture::with_contract(true, true);
    fixture.append_child(NodeFact::ArtifactProduced(ArtifactProducedFact {
        contract: Some("result".into()),
        value: serde_json::json!({"summary": "done"}),
        request_id: Some("request-1".into()),
    }));
    fixture.append_child(NodeFact::SubmitReceived(SubmitReceivedFact {
        request_id: Some("request-1".into()),
    }));
    fixture.append_child(NodeFact::StopReceived(StopReceivedFact {
        result_summary: None,
        token_usage: None,
    }));
    // When
    let output = fixture.read().get_output(TREE, "child").await.unwrap();
    // Then
    let crate::usecase::workflow::WorkflowGetOutputResult::Submitted {
        structured_output,
        contract,
        request_id,
        ..
    } = output
    else {
        panic!("output is missing");
    };
    assert_eq!(contract.as_deref(), Some("result"));
    assert_eq!(request_id.as_deref(), Some("request-1"));
    assert_eq!(structured_output["summary"], "done");
    assert_eq!(
        structured_output["worktree"]["path"],
        IsolatedWorktree::for_attempt("/repo", "child-attempt", 2).path
    );
}

#[tokio::test]
async fn test_隔離出力_同名slotの開始順と提出順が異なっても提出した所有者の値を返す() {
    // Given
    let fixture = Fixture::with_slots(true, true, true);
    let second = NodeFactMeta {
        node_execution_id: "second-slot".into(),
        attempt: 1,
        ..fixture.child.clone()
    };
    fact_log::append_single_fact(
        &fixture.store,
        &second,
        &NodeFact::Started(StartedFact {
            worktree: None,
            parent: Some(ExecutionParentRef::fanout_child(TREE, Some(1), 0)),
            root: None,
        }),
        3,
    )
    .unwrap();
    for (meta, timestamp, request) in [
        (&fixture.child, 4, "first"),
        (&second, 5, "second"),
        (&fixture.child, 6, "last"),
    ] {
        fact_log::append_single_fact(
            &fixture.store,
            meta,
            &NodeFact::ArtifactProduced(ArtifactProducedFact {
                contract: Some("result".into()),
                value: serde_json::json!({"summary": request}),
                request_id: Some(request.into()),
            }),
            timestamp,
        )
        .unwrap();
        // When
        let output = fixture.read().get_output(TREE, "child").await.unwrap();
        // Then
        let expected =
            IsolatedWorktree::for_attempt("/repo", &meta.node_execution_id, meta.attempt);
        assert_eq!(
            output,
            crate::usecase::workflow::WorkflowGetOutputResult::Submitted {
                contract: Some("result".into()),
                structured_output: serde_json::json!({"summary": request, "worktree": {"branch": expected.branch, "path": expected.path}}),
                request_id: Some(request.into()),
                submitted_at: Some(timestamp as f64 / 1000.0),
                timestamp: timestamp as f64 / 1000.0,
            }
        );
    }
}

#[tokio::test]
async fn test_隔離出力_contractなしslotも最後に提出した所有者の成果を返す() {
    // Given
    let fixture = Fixture::with_slots(true, false, true);
    let second = NodeFactMeta {
        node_execution_id: "second-slot".into(),
        attempt: 1,
        ..fixture.child.clone()
    };
    fact_log::append_single_fact(
        &fixture.store,
        &second,
        &NodeFact::Started(StartedFact {
            worktree: None,
            parent: Some(ExecutionParentRef::fanout_child(TREE, Some(1), 0)),
            root: None,
        }),
        3,
    )
    .unwrap();
    for (meta, timestamp) in [(&fixture.child, 4), (&second, 6)] {
        fact_log::append_single_fact(
            &fixture.store,
            meta,
            &NodeFact::SubmitReceived(SubmitReceivedFact {
                request_id: Some(meta.node_execution_id.clone()),
            }),
            timestamp,
        )
        .unwrap();
        fact_log::append_single_fact(
            &fixture.store,
            meta,
            &NodeFact::StopReceived(StopReceivedFact {
                result_summary: None,
                token_usage: None,
            }),
            timestamp + 1,
        )
        .unwrap();
        // When
        let output = fixture.read().get_output(TREE, "child").await.unwrap();
        // Then
        let crate::usecase::workflow::WorkflowGetOutputResult::Submitted {
            structured_output, ..
        } = output
        else {
            panic!("submitted slot must have output");
        };
        let expected =
            IsolatedWorktree::for_attempt("/repo", &meta.node_execution_id, meta.attempt);
        assert_eq!(
            structured_output,
            serde_json::json!({"worktree": {"branch": expected.branch, "path": expected.path}})
        );
        let folded = fact_log::fold_tree_from(&fixture.backend, TREE)
            .await
            .unwrap()
            .unwrap();
        let records = fact_log::read_tree_records_from(&fixture.backend, TREE)
            .await
            .unwrap();
        assert_eq!(
            crate::domain::workflow::services::fact_replay::derive_node_artifact(
                &folded, &records, "child"
            )
            .unwrap()
            .value,
            structured_output
        );
    }
}

#[tokio::test]
async fn test_隔離context_取得済みrootだけで隔離cwdを導出しroot行を再取得しない() {
    // Given
    let fixture = Fixture::new(false);
    let row = fixture
        .backend
        .run_indexed(|connection| {
            Ok(node_events::first_row_of_tree(connection, TREE)
                .unwrap()
                .unwrap())
        })
        .await
        .unwrap();
    let root = stored_definition::read_tree_context(&row.detail)
        .unwrap()
        .unwrap();
    let root_meta = fact_log::node_meta_from_row(&row).unwrap();
    rusqlite::Connection::open(fixture.directory.path().join("local-event-store.sqlite3"))
        .unwrap()
        .execute(
            "DELETE FROM node_events WHERE node_execution_id = ?1",
            [TREE],
        )
        .unwrap();
    // When
    let path = execution_worktree_path(&fixture.backend, fixture.child, root_meta, root)
        .await
        .unwrap();
    // Then
    assert_eq!(path, IsolatedWorktree::for_attempt("/repo", TREE, 1).path);
}

#[tokio::test]
async fn test_実効cwd_自身か直近の隔離祖先で確定したら上位行と定義を読まない() {
    // Given
    for isolated_child in [true, false] {
        let fixture = Fixture::new(isolated_child);
        let row = fixture
            .backend
            .run_indexed(|connection| {
                Ok(node_events::first_row_of_tree(connection, TREE)
                    .unwrap()
                    .unwrap())
            })
            .await
            .unwrap();
        let mut root = stored_definition::read_tree_context(&row.detail)
            .unwrap()
            .unwrap();
        let mut root_meta = fact_log::node_meta_from_row(&row).unwrap();
        root_meta.node_name = "unread-root".into();
        root.definition["nodes"]["unread-root"] = serde_json::json!({"worktree": 123});
        let ancestor = NodeFactMeta {
            node_execution_id: "nearest".into(),
            parent_id: Some("missing-parent".into()),
            ..fixture.root.clone()
        };
        fact_log::append_single_fact(
            &fixture.store,
            &ancestor,
            &NodeFact::Started(StartedFact {
                worktree: None,
                parent: Some(ExecutionParentRef::sequence_child("missing-parent")),
                root: None,
            }),
            4,
        )
        .unwrap();
        let mut child = fixture.child;
        child.parent_id = Some(
            if isolated_child {
                "missing-parent"
            } else {
                "nearest"
            }
            .into(),
        );
        let owner = if isolated_child { &child } else { &ancestor };
        let expected =
            IsolatedWorktree::for_attempt("/repo", &owner.node_execution_id, owner.attempt);
        // When
        let path = execution_worktree_path(&fixture.backend, child, root_meta, root)
            .await
            .unwrap();
        // Then
        assert_eq!(path, expected.path);
    }
}

#[tokio::test]
async fn test_実効cwd_祖先の欠落と循環と別木と不正定義はcorruptになる() {
    for invalid in [
        "missing",
        "cycle",
        "other-tree",
        "definition",
        "missing-definition",
    ] {
        // Given
        let fixture = Fixture::new(false);
        let row = fixture
            .backend
            .run_indexed(|connection| {
                Ok(node_events::first_row_of_tree(connection, TREE)
                    .unwrap()
                    .unwrap())
            })
            .await
            .unwrap();
        let mut root = stored_definition::read_tree_context(&row.detail)
            .unwrap()
            .unwrap();
        let mut root_meta = fact_log::node_meta_from_row(&row).unwrap();
        let mut child = fixture.child;
        match invalid {
            "missing" => child.parent_id = Some("missing".into()),
            "cycle" => child.parent_id = Some(child.node_execution_id.clone()),
            "other-tree" => root_meta.tree_id = "another".into(),
            "missing-definition" => root_meta.node_name = "missing".into(),
            "definition" => root.definition["nodes"]["main"]["worktree"] = serde_json::json!(123),
            _ => unreachable!(),
        }
        // When / Then
        assert!(matches!(
            execution_worktree_path(&fixture.backend, child, root_meta, root).await,
            Err(WorktreeContextReadError::Corrupt(_))
        ));
    }
}

#[tokio::test]
async fn test_実効cwd_祖先sql読み取り障害はinternalへ伝わる() {
    // Given
    let fixture = Fixture::new(false);
    let row = fixture
        .backend
        .run_indexed(|connection| {
            Ok(node_events::first_row_of_tree(connection, TREE)
                .unwrap()
                .unwrap())
        })
        .await
        .unwrap();
    let root = stored_definition::read_tree_context(&row.detail)
        .unwrap()
        .unwrap();
    let root_meta = fact_log::node_meta_from_row(&row).unwrap();
    let mut child = fixture.child;
    child.parent_id = Some("ancestor".into());
    rusqlite::Connection::open(fixture.directory.path().join("local-event-store.sqlite3"))
        .unwrap()
        .execute("DROP TABLE node_events", [])
        .unwrap();
    // When
    let error = execution_worktree_path(&fixture.backend, child, root_meta, root)
        .await
        .unwrap_err();
    // Then
    assert!(matches!(
        error,
        WorktreeContextReadError::Read(
            crate::domain::local_event::LocalEventQueryError::Internal { .. }
        )
    ));
}

#[tokio::test]
async fn test_workspace解決_通常pathではstoreを構築せず隔離pathだけ保存事実を読む() {
    // Given
    use crate::usecase::workspace_tree::WorkspaceWorktreePathQuery;
    let fixture = Fixture::new(true);
    let query = StoredWorkspaceWorktreePathQuery::new(fixture.directory.path().into());
    let isolated = IsolatedWorktree::for_attempt(
        "/repo",
        &fixture.child.node_execution_id,
        fixture.child.attempt,
    );
    // When / Then
    assert_eq!(
        query.workspace_worktree_path(&isolated.path).await.unwrap(),
        ROOT
    );
    let missing_store = fixture.directory.path().join("missing");
    let query = StoredWorkspaceWorktreePathQuery::new(missing_store.clone());
    assert_eq!(query.workspace_worktree_path(ROOT).await.unwrap(), ROOT);
    assert!(!missing_store.exists());
    assert!(query.workspace_worktree_path(&isolated.path).await.is_err());
    assert_eq!(
        workspace_worktree_path_with(ROOT, || panic!("ordinary paths must not construct a store"))
            .await
            .unwrap(),
        ROOT
    );
}

async fn workspace_worktree_path(
    backend: &FactLogReadBackend,
    path: &str,
) -> Result<String, crate::domain::workflow::WorkflowError> {
    workspace_worktree_path_with(path, || Ok(backend.clone())).await
}

#[tokio::test]
async fn test_実効cwd_rootのretryで初回rootとidが変わってもworkspaceを継承する() {
    // Given
    let fixture = Fixture::new(false);
    let row = fixture
        .backend
        .run_indexed(|connection| {
            Ok(node_events::first_row_of_tree(connection, TREE)
                .unwrap()
                .unwrap())
        })
        .await
        .unwrap();
    let mut root = stored_definition::read_tree_context(&row.detail)
        .unwrap()
        .unwrap();
    root.definition["nodes"]["main"] = serde_json::json!({"session": {"provider": "codex"}});
    let mut root_meta = fact_log::node_meta_from_row(&row).unwrap();
    root_meta.kind = NodeKindName::Session;
    let current = NodeFactMeta {
        node_execution_id: "retried-root".into(),
        attempt: 2,
        ..root_meta.clone()
    };
    // When
    let path = execution_worktree_path(&fixture.backend, current, root_meta, root)
        .await
        .unwrap();
    // Then
    assert_eq!(path, ROOT);
}

#[tokio::test]
async fn test_workspace所在地読取_実経路で失敗分類を保持する() {
    use crate::adaptor::gateway::local_event_store::test_helpers::ReadFailure;
    use crate::adaptor::protocol::connect::classified_error;
    // Given
    let fixture = Fixture::new(false);
    let isolated = IsolatedWorktree::for_attempt("/repo", TREE, 1);
    for (failure, expected) in ReadFailure::cases() {
        fixture.store.fail_next_read(failure);
        // When
        let error = workspace_worktree_path(&fixture.backend, &isolated.path)
            .await
            .unwrap_err();
        // Then
        assert_eq!(classified_error(error).code, expected);
    }
}

#[tokio::test]
async fn test_workspace所在地読取_保存されたrootの破損をdata_lossとして返す() {
    // Given
    let fixture = Fixture::new(false);
    let isolated = IsolatedWorktree::for_attempt("/repo", TREE, 1);
    rusqlite::Connection::open(fixture.directory.path().join("local-event-store.sqlite3"))
        .unwrap()
        .execute(
            "UPDATE node_events SET detail = '{' WHERE parent_id IS NULL",
            [],
        )
        .unwrap();
    // When
    let error = workspace_worktree_path(&fixture.backend, &isolated.path)
        .await
        .unwrap_err();
    // Then
    assert_eq!(
        crate::adaptor::protocol::connect::classified_error(error).code,
        connectrpc::ErrorCode::DataLoss
    );
}
