pub(crate) mod tests {

    use releashd::test_support::integration::persistence::LocalEventStore;
    use releashd::test_support::integration::workflow::ExecutionTreeId;
    use releashd::test_support::integration::workflow::WorkflowEventLogRepository;
    use releashd::test_support::integration::workflow::WorkflowEventRepository;

    use tempfile::TempDir;

    use releashd::test_support::integration::persistence::LocalEventStoreConfig;
    use releashd::test_support::integration::workflow::ExecutionOrigin;
    use releashd::test_support::integration::workflow::NodeCompletion;
    use releashd::test_support::integration::workflow::NodeDefinition;
    use releashd::test_support::integration::workflow::NodeKind;
    use releashd::test_support::integration::workflow::NodeKindName;
    use releashd::test_support::integration::workflow::SessionSpec;
    use releashd::test_support::integration::workflow::WorkflowDefinition;
    use releashd::test_support::integration::workflow::WorkflowEvent;

    fn definition() -> WorkflowDefinition {
        WorkflowDefinition {
            name: "wf".to_string(),
            description: String::new(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                name: "main".to_string(),
                kind: NodeKind::Session(SessionSpec::default()),
                artifact: None,
                input: Vec::new(),
                completion: NodeCompletion::default(),
                worktree: None,
            }],
            entry: "main".to_string(),
        }
    }

    fn started_events(execution_id: &str) -> Vec<WorkflowEvent> {
        vec![
            WorkflowEvent::ExecutionStarted {
                repository_root: None,
                execution_id: execution_id.to_string(),
                workflow_name: "wf".to_string(),
                worktree_path: "/repo".to_string(),
                created_from: ExecutionOrigin::Cli,
                request: "ship it".to_string(),
                definition: definition(),
                timestamp: 1.0,
            },
            WorkflowEvent::NodeStarted {
                worktree: None,
                execution_id: execution_id.to_string(),
                node_execution_id: format!("{execution_id}-root"),
                node_name: "main".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                parent: None,
                timestamp: 1.0,
            },
        ]
    }

    #[tokio::test]
    pub async fn test_実行履歴読取_readとread_pageで失敗分類を保持する() {
        use releashd::test_support::integration::persistence::ReadFailure;
        use releashd::test_support::integration::transport::classified_error;
        // Given
        let directory = TempDir::new().unwrap();
        let store = LocalEventStore::open(LocalEventStoreConfig::production(
            directory.path().into(),
            std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
        ))
        .unwrap();
        let id = ExecutionTreeId::new("00000000-0000-4000-8000-000000000001").unwrap();
        let repository = WorkflowEventLogRepository::with_store(store.clone());
        for (failure, expected) in ReadFailure::cases() {
            store.fail_next_read(failure.clone());
            // When
            let result = repository.read(&id).await;
            // Then
            assert_eq!(classified_error(result.unwrap_err()).code, expected);
        }
    }

    #[tokio::test]
    pub async fn read_returns_fact_rows_with_unified_vocabulary() {
        let tmp = TempDir::new().unwrap();
        let store = LocalEventStore::open(LocalEventStoreConfig::production(
            tmp.path().to_path_buf(),
            std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
        ))
        .unwrap();
        let execution_id = ExecutionTreeId::new("00000000-0000-4000-8000-000000000001").unwrap();
        releashd::test_support::integration::workflow::append_canonical_events(
            &store,
            &started_events(execution_id.as_str()),
        )
        .await
        .unwrap();
        let repo = WorkflowEventLogRepository::with_store(store);

        let events = repo.read(&execution_id).await.unwrap();

        assert_eq!(events[0].event_kind, "started");
        assert_eq!(events[0].payload["root"]["definition"]["name"], "wf");
        assert_eq!(events[0].payload["root"]["launchedAs"], "workflow");
        assert_eq!(events[0].payload["root"]["request"], "ship it");
    }

    #[tokio::test]
    pub async fn read_after_cached_read_observes_incremental_append() {
        let tmp = TempDir::new().unwrap();
        let store = LocalEventStore::open(LocalEventStoreConfig::production(
            tmp.path().to_path_buf(),
            std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
        ))
        .unwrap();
        let execution_id = ExecutionTreeId::new("00000000-0000-4000-8000-000000000002").unwrap();
        releashd::test_support::integration::workflow::append_canonical_events(
            &store,
            &started_events(execution_id.as_str()),
        )
        .await
        .unwrap();
        let repo = WorkflowEventLogRepository::with_store(store.clone());

        // ExecutionStarted と root の NodeStarted は 1 つの root started 行に融合される。
        let first = repo.read(&execution_id).await.unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].event_kind, "started");

        releashd::test_support::integration::workflow::append_canonical_events(
            &store,
            &[WorkflowEvent::ExecutionAborted {
                execution_id: execution_id.to_string(),
                aborted_node: None,
                timestamp: 2.0,
            }],
        )
        .await
        .unwrap();

        let second = repo.read(&execution_id).await.unwrap();
        assert_eq!(second.len(), 2);
        assert_eq!(second[0], first[0]);
        assert_eq!(second[1].event_kind, "abort_requested");
        assert_eq!(second[1].timestamp, 2.0);
    }

    #[tokio::test]
    pub async fn test_実行履歴_未対応定義を落とさず保存されたpayloadをページでも返す() {
        // Given
        let directory = TempDir::new().unwrap();
        let store = LocalEventStore::open(LocalEventStoreConfig::production(
            directory.path().into(),
            std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
        ))
        .unwrap();
        let id = ExecutionTreeId::new("00000000-0000-4000-8000-000000001744").unwrap();
        releashd::test_support::integration::workflow::seed_unavailable_definition(
            &store,
            id.as_str(),
            "/repo",
            "main",
        )
        .await;
        let repository = WorkflowEventLogRepository::with_store(store);

        // When
        let records = repository.read(&id).await.unwrap();

        // Then
        assert_eq!(
            records[0].payload["root"]["definition"]["nodes"]["main"]["sequence"]["output"],
            "session"
        );
    }
}
