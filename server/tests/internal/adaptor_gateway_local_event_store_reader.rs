use connectrpc::ErrorCode;
use releashd::test_support::integration::persistence::storage_unavailable;
use releashd::test_support::integration::persistence::ReaderPool;
use releashd::test_support::integration::persistence::READ_QUEUE_MAX_DEPTH;
use releashd::test_support::integration::platform::Deadline;
use releashd::test_support::integration::platform::LocalEventQueryError;
use releashd::test_support::integration::platform::OperationContext;
use releashd::test_support::integration::transport::ConnectFailure;
use rusqlite::Connection;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

#[tokio::test]
pub async fn test_読み込みキュー_混雑と期限切れとreply喪失を分類する() {
    // Given
    let pool = ReaderPool::new();
    let mut pending = Vec::new();
    for _ in 0..READ_QUEUE_MAX_DEPTH {
        let mut read = Box::pin(pool.submit(|_| Ok(())));
        assert!(futures_util::poll!(&mut read).is_pending());
        pending.push(read);
    }

    // When / Then
    assert_eq!(
        pool.submit(|_| Ok(())).await,
        Err(LocalEventQueryError::QueryBusy)
    );
    let (context, task) = pool.pop_blocking().unwrap().test_into_parts();
    releashd::test_support::integration::platform::sync_scope(
        context.with_deadline(
            releashd::test_support::integration::platform::Deadline::new(std::time::Instant::now()),
        ),
        || {
            (task)(
                &releashd::test_support::integration::transport::configure_busy_handler(
                    Connection::open_in_memory().unwrap(),
                    std::sync::Arc::new(
                        releashd::test_support::integration::platform::RetryLimiter::new(),
                    ),
                )
                .unwrap(),
            )
        },
    );
    assert_eq!(
        pending.remove(0).await,
        Err(LocalEventQueryError::Technical(
            releashd::test_support::integration::platform::TechnicalFailure {
                nature:
                    releashd::test_support::integration::platform::TechnicalFailureNature::TimedOut,
                message: "deadline exceeded".into()
            }
        ))
    );
    pool.close();
    assert_eq!(
        pending.remove(0).await.unwrap_err().connect_code(),
        ErrorCode::Unavailable
    );
    assert_eq!(
        pool.submit(|_| Ok(())).await.unwrap_err().connect_code(),
        ErrorCode::FailedPrecondition
    );
}

#[tokio::test]
pub async fn test_読み込み実行中_期限と取り消しでsqliteを止め接続を再利用できる() {
    use releashd::test_support::integration::platform::Deadline;

    use std::time::Duration;
    use std::time::Instant;
    for expire in [false, true] {
        // Given
        let pool = ReaderPool::new();
        let worker_pool = pool.clone();
        let worker = std::thread::spawn(move || {
            worker_pool.run_worker(
                releashd::test_support::integration::transport::configure_busy_handler(
                    Connection::open_in_memory().unwrap(),
                    std::sync::Arc::new(
                        releashd::test_support::integration::platform::RetryLimiter::new(),
                    ),
                )
                .unwrap(),
            )
        });
        let token = tokio_util::sync::CancellationToken::new();
        let context = OperationContext::new(
            expire.then(|| Deadline::new(Instant::now() + Duration::from_millis(30))),
            Arc::new(token.clone()),
        );
        let (started, ready) = tokio::sync::oneshot::channel();
        let query = releashd::test_support::integration::platform::scope(context, pool.submit(move |connection| {
            let _ = started.send(());
            connection.query_row("WITH RECURSIVE numbers(n) AS (VALUES(0) UNION ALL SELECT n+1 FROM numbers WHERE n<1000000000) SELECT sum(n) FROM numbers", [], |row| row.get::<_, i64>(0)).map_err(|error| storage_unavailable(&error))
        }));
        tokio::pin!(query);
        assert!(futures_util::poll!(&mut query).is_pending());
        ready.await.unwrap();
        // When
        if !expire {
            token.cancel();
        }
        let error = tokio::time::timeout(Duration::from_secs(2), query)
            .await
            .unwrap()
            .unwrap_err();
        // Then
        assert_eq!(
            error.connect_code(),
            if expire {
                ErrorCode::DeadlineExceeded
            } else {
                ErrorCode::Canceled
            }
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), pool.submit(|_| Ok(42)))
                .await
                .unwrap()
                .unwrap(),
            42
        );
        pool.close();
        worker.join().unwrap();
    }
}

#[tokio::test]
pub async fn test_読み込み取消_短い文の間で取り消しても次の文を実行しない() {
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::Ordering;
    // Given
    let pool = ReaderPool::new();
    let worker_pool = pool.clone();
    let worker = std::thread::spawn(move || {
        worker_pool.run_worker(
            releashd::test_support::integration::transport::configure_busy_handler(
                Connection::open_in_memory().unwrap(),
                std::sync::Arc::new(
                    releashd::test_support::integration::platform::RetryLimiter::new(),
                ),
            )
            .unwrap(),
        )
    });
    let token = tokio_util::sync::CancellationToken::new();
    let context = OperationContext::new(None, Arc::new(token.clone()));
    let second_ran = Arc::new(AtomicBool::new(false));
    let observed = second_ran.clone();
    // When
    let result = releashd::test_support::integration::platform::scope(
        context,
        pool.submit(move |connection| {
            connection
                .query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
                .unwrap();
            token.cancel();
            let second = connection.query_row("SELECT 2", [], |row| row.get::<_, i64>(0));
            observed.store(second.is_ok(), Ordering::SeqCst);
            second.map_err(|error| storage_unavailable(&error))
        }),
    )
    .await;
    pool.close();
    worker.join().unwrap();
    // Then
    assert_eq!(result.unwrap_err().connect_code(), ErrorCode::Canceled);
    assert!(!second_ran.load(Ordering::SeqCst));
}

#[tokio::test]
pub async fn test_reader_busy待ち_実際のdb競合で期限と取消を引き継ぐ() {
    for expire in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("busy.db");
        let blocker = Connection::open(&path).unwrap();
        blocker
            .execute_batch("CREATE TABLE value(n); INSERT INTO value VALUES(1);")
            .unwrap();
        let connection = releashd::test_support::integration::transport::open_reader(
            &path,
            std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
        )
        .unwrap();
        blocker.execute_batch("BEGIN EXCLUSIVE").unwrap();
        let pool = ReaderPool::new();
        let worker_pool = pool.clone();
        let worker = std::thread::spawn(move || worker_pool.run_worker(connection));
        let token = tokio_util::sync::CancellationToken::new();
        let context = OperationContext::new(
            expire.then(|| Deadline::new(Instant::now() + Duration::from_millis(100))),
            Arc::new(token.clone()),
        );
        let (started, ready) = tokio::sync::oneshot::channel();
        let (finished, completion) = tokio::sync::oneshot::channel();
        let mut query = Box::pin(releashd::test_support::integration::platform::scope(
            context,
            pool.submit(move |connection| {
                started.send(()).unwrap();
                let result =
                    connection.query_row("SELECT n FROM value", [], |row| row.get::<_, i64>(0));
                finished
                    .send(
                        result
                            .as_ref()
                            .err()
                            .and_then(|error| error.sqlite_error_code()),
                    )
                    .unwrap();
                result.map_err(|error| storage_unavailable(&error))
            }),
        ));
        tokio::select! { _ = ready => {}, result = &mut query => panic!("query ended early: {result:?}") }
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(futures_util::poll!(&mut query).is_pending());
        if !expire {
            token.cancel();
        }
        let error = tokio::time::timeout(Duration::from_secs(1), query)
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(
            error.connect_code(),
            if expire {
                ErrorCode::DeadlineExceeded
            } else {
                ErrorCode::Canceled
            }
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), completion)
                .await
                .unwrap()
                .unwrap(),
            Some(rusqlite::ErrorCode::DatabaseBusy)
        );
        blocker.execute_batch("ROLLBACK").unwrap();
        pool.close();
        worker.join().unwrap();
    }
}

#[tokio::test]
pub async fn test_読み込み資源期限_親が無期限でも長い期限でも二秒で待ちを終える() {
    // Given
    let pool = ReaderPool::new();
    let start = Instant::now();
    let contexts = [
        OperationContext::default(),
        OperationContext::default().with_deadline(Deadline::new(start + Duration::from_secs(10))),
    ];
    let mut queries = Vec::new();
    for context in contexts {
        let mut query = Box::pin(releashd::test_support::integration::platform::scope(
            context,
            pool.submit(|_| -> Result<(), LocalEventQueryError> {
                panic!("expired queued query must not run")
            }),
        ));
        assert!(futures_util::poll!(&mut query).is_pending());
        queries.push(query);
    }
    // When
    for query in queries {
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(4), query)
                .await
                .unwrap(),
            Err(LocalEventQueryError::Technical(
                releashd::test_support::integration::platform::TechnicalFailure {
                    nature: releashd::test_support::integration::platform::TechnicalFailureNature::TimedOut,
                    message: "deadline exceeded".into()
                }
            ))
        );
    }
    // Then
    assert!(start.elapsed() >= Duration::from_secs(2));
    assert!(start.elapsed() < Duration::from_secs(4));
    let connection = releashd::test_support::integration::transport::configure_busy_handler(
        Connection::open_in_memory().unwrap(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    )
    .unwrap();
    for _ in 0..2 {
        let (context, task) = pool.pop_blocking().unwrap().test_into_parts();
        releashd::test_support::integration::platform::sync_scope(context, || (task)(&connection));
    }
    pool.close();
}
pub(crate) mod canonical_runtime_owner_snapshot_tests {

    use releashd::test_support::integration::transport::classified_error;

    use releashd::test_support::integration::fixtures::session_attached;
    use releashd::test_support::integration::persistence::canonical_runtime_owner_snapshot;
    use releashd::test_support::integration::persistence::initialize_schema;
    use releashd::test_support::integration::persistence::FaultInjector;
    use releashd::test_support::integration::persistence::InitialStoreMetadata;
    use releashd::test_support::integration::persistence::MAX_CANONICAL_RUNTIME_OWNER_SNAPSHOT;
    use releashd::test_support::integration::platform::CanonicalRuntimeOwnerView;
    use releashd::test_support::integration::platform::LocalEventQueryError;
    use releashd::test_support::integration::providers::ProviderKind;
    use releashd::test_support::integration::workflow::ExecutionOrigin;
    use releashd::test_support::integration::workflow::ExecutionTreeLaunch;
    use releashd::test_support::integration::workflow::NodeCompletion;
    use releashd::test_support::integration::workflow::NodeDefinition;
    use releashd::test_support::integration::workflow::NodeFact;
    use releashd::test_support::integration::workflow::NodeKind;
    use releashd::test_support::integration::workflow::SessionExecutionTreeRootFacts;
    use releashd::test_support::integration::workflow::SessionSpec;
    use releashd::test_support::integration::workflow::StartedFact;
    use releashd::test_support::integration::workflow::TreeRootFact;
    use releashd::test_support::integration::workflow::WorkflowDefinition;
    use rusqlite::params;
    use rusqlite::Connection;

    fn connection_with_node_events() -> Connection {
        let connection = Connection::open_in_memory().expect("in-memory SQLite");
        initialize_schema(
            &connection,
            &InitialStoreMetadata {
                installation_id: "00000000-0000-4000-8000-000000000001",
                created_at_ms: 1,
            },
            &FaultInjector::new(),
        )
        .expect("initialize schema");
        connection
    }

    fn insert_root(connection: &Connection, tree_id: &str, fact: &NodeFact) {
        let node_name = match fact {
            NodeFact::Started(StartedFact {
                worktree: None,
                root: Some(root),
                ..
            }) => root.definition.as_ref().unwrap().entry.as_str(),
            _ => panic!("root fact must contain a definition"),
        };
        connection
            .execute(
                "INSERT INTO node_events (
                    tree_id, seq, node_execution_id, parent_id, node_name, kind,
                    attempt, event_type, detail, timestamp
                 ) VALUES (?1, 1, ?1, NULL, ?2, 'session', 1, ?3, ?4, 1)",
                params![
                    tree_id,
                    node_name,
                    releashd::test_support::integration::workflow::event_type(fact),
                    releashd::test_support::integration::workflow::encode_detail(fact).unwrap()
                ],
            )
            .expect("insert root fact");
    }

    fn insert_second_fact(connection: &Connection, tree_id: &str, fact: &NodeFact) {
        connection
            .execute(
                "INSERT INTO node_events (
                    tree_id, seq, node_execution_id, parent_id, node_name, kind,
                    attempt, event_type, detail, timestamp
                 ) VALUES (?1, 2, ?1, NULL, 'session', 'session', 1, ?2, ?3, 2)",
                params![
                    tree_id,
                    releashd::test_support::integration::workflow::event_type(fact),
                    releashd::test_support::integration::workflow::encode_detail(fact).unwrap()
                ],
            )
            .expect("insert second fact");
    }

    fn insert_third_fact(connection: &Connection, tree_id: &str, fact: &NodeFact) {
        connection
            .execute(
                "INSERT INTO node_events (
                    tree_id, seq, node_execution_id, parent_id, node_name, kind,
                    attempt, event_type, detail, timestamp
                 ) VALUES (?1, 3, ?1, NULL, 'session', 'session', 1, ?2, ?3, 3)",
                params![
                    tree_id,
                    releashd::test_support::integration::workflow::event_type(fact),
                    releashd::test_support::integration::workflow::encode_detail(fact).unwrap()
                ],
            )
            .expect("insert third fact");
    }

    fn session_root(session_id: &str, workspace_identity: &str, worktree_path: &str) -> NodeFact {
        SessionExecutionTreeRootFacts::new(
            session_id,
            workspace_identity,
            worktree_path,
            ProviderKind::Codex,
            None,
        )
        .unwrap()
        .started
    }

    fn workflow_root(worktree_path: &str) -> NodeFact {
        NodeFact::Started(StartedFact {
            worktree: None,
            parent: None,
            root: Some(Box::new(TreeRootFact {
                repository_root: None,
                workspace_identity: worktree_path.to_string(),
                worktree_path: worktree_path.to_string(),
                created_from: ExecutionOrigin::Cli,
                request: String::new(),
                workflow_name: "wf".to_string(),
                definition: Some(WorkflowDefinition {
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
                }),
                launched_as: ExecutionTreeLaunch::Workflow,
            })),
        })
    }

    fn connection_with_active_workflow_owners(count: usize) -> Connection {
        let connection = connection_with_node_events();
        for index in 0..count {
            insert_root(
                &connection,
                &format!("execution-{index}"),
                &workflow_root(&format!("/snapshot/worktree-{index}")),
            );
        }
        connection
    }

    #[test]
    pub fn test_owner一覧_decodeとfoldの破損をdata_lossで返す() {
        use releashd::test_support::integration::transport::classified_error;

        for decode_failure in [true, false] {
            // Given
            let connection = connection_with_active_workflow_owners(1);
            if decode_failure {
                connection
                    .execute("UPDATE node_events SET detail = '{'", [])
                    .unwrap();
            } else {
                insert_second_fact(
                    &connection,
                    "execution-0",
                    &NodeFact::RepositoryRootObserved("/first".into()),
                );
                insert_third_fact(
                    &connection,
                    "execution-0",
                    &NodeFact::RepositoryRootObserved("/conflicting".into()),
                );
            }

            // When
            let error = canonical_runtime_owner_snapshot(&connection, 1).unwrap_err();

            // Then
            assert_eq!(
                classified_error(error).code,
                connectrpc::ErrorCode::DataLoss
            );
        }
    }

    #[test]
    pub fn test_owner一覧_limit範囲外はinvalid_argumentを維持する() {
        // Given
        let connection = connection_with_node_events();
        for limit in [0, MAX_CANONICAL_RUNTIME_OWNER_SNAPSHOT + 1] {
            // When
            let error = canonical_runtime_owner_snapshot(&connection, limit).unwrap_err();
            // Then
            assert_eq!(
                classified_error(error).code,
                connectrpc::ErrorCode::InvalidArgument
            );
        }
    }

    #[test]
    pub fn app_data_gc_owner_snapshot_returns_one_bounded_lightweight_inventory() {
        let connection = connection_with_active_workflow_owners(2);

        let owners =
            canonical_runtime_owner_snapshot(&connection, 2).expect("complete owner snapshot");

        assert_eq!(owners.len(), 2);
        assert!(owners
            .iter()
            .all(|owner| matches!(owner, CanonicalRuntimeOwnerView::ActiveWorkflow { .. })));
    }

    #[test]
    pub fn test_owner一覧_旧定義の完了とabortを除外してactiveだけを返す() {
        // Given
        let connection = connection_with_active_workflow_owners(1);
        for (tree_id, terminal) in [
            ("completed", NodeFact::ExecutionCompleted),
            ("aborted", NodeFact::AbortRequested(Default::default())),
        ] {
            let fact = workflow_root("/snapshot/legacy");
            insert_root(&connection, tree_id, &fact);
            let mut detail: serde_json::Value = serde_json::from_str(
                &releashd::test_support::integration::workflow::encode_detail(&fact).unwrap(),
            )
            .unwrap();
            detail["root"]["definition"]["nodes"]["main"]["completion"] =
                serde_json::json!("approval");
            assert!(releashd::test_support::integration::workflow::decode(
                "started",
                &detail.to_string()
            )
            .is_err());
            connection
                .execute(
                    "UPDATE node_events SET detail = ?1 WHERE tree_id = ?2 AND seq = 1",
                    params![detail.to_string(), tree_id],
                )
                .unwrap();
            insert_second_fact(&connection, tree_id, &terminal);
        }

        // When
        let owners = canonical_runtime_owner_snapshot(&connection, 1).unwrap();

        // Then
        assert_eq!(
            owners,
            vec![CanonicalRuntimeOwnerView::ActiveWorkflow {
                worktree_path: "/snapshot/worktree-0".into(),
            }]
        );
    }

    #[test]
    pub fn app_data_gc_owner_snapshot_lists_open_session_trees() {
        let connection = connection_with_node_events();
        insert_root(
            &connection,
            "agent-session-1",
            &session_root(
                "agent-session-1",
                "/snapshot/worktree-a",
                "/snapshot/worktree-a",
            ),
        );
        insert_second_fact(
            &connection,
            "agent-session-1",
            &session_attached("agent-session-1"),
        );

        let owners =
            canonical_runtime_owner_snapshot(&connection, 8).expect("complete owner snapshot");

        assert_eq!(
            owners,
            vec![CanonicalRuntimeOwnerView::AgentSession {
                worktree_path: "/snapshot/worktree-a".to_string(),
                active: true,
            }]
        );
    }

    #[test]
    pub fn app_data_gc_owner_snapshot_limit_plus_one_fails_closed() {
        let connection = connection_with_active_workflow_owners(2);

        assert_eq!(
            canonical_runtime_owner_snapshot(&connection, 1),
            Err(LocalEventQueryError::ResponseTooLarge)
        );
    }

    #[test]
    pub fn app_data_gc_owner_snapshot_applies_limit_after_closed_sessions_are_removed() {
        let connection = connection_with_active_workflow_owners(1);
        for session_id in ["closed-session-1", "closed-session-2"] {
            insert_root(
                &connection,
                session_id,
                &session_root(session_id, "/snapshot", &format!("/snapshot/{session_id}")),
            );
            insert_second_fact(&connection, session_id, &session_attached(session_id));
            insert_third_fact(
                &connection,
                session_id,
                &NodeFact::ArchiveRequested(
                    releashd::test_support::integration::workflow::ArchiveRequestedFact {
                        reason: "manual".into(),
                        archived_at: 0.0,
                    },
                ),
            );
        }

        let owners = canonical_runtime_owner_snapshot(&connection, 1).unwrap();

        assert_eq!(owners.len(), 1);
        assert!(matches!(
            owners[0],
            CanonicalRuntimeOwnerView::ActiveWorkflow { .. }
        ));
    }
}

#[tokio::test(flavor = "current_thread")]
pub async fn test_読み込み待ち_同じruntimeの別処理が先に完了する() {
    // Given
    let pool = ReaderPool::new();
    let mut read = Box::pin(pool.submit(|_| Ok(42)));
    assert!(futures_util::poll!(&mut read).is_pending());

    // When
    let other = tokio::spawn(async { 7 }).await.unwrap();

    // Then
    assert_eq!(other, 7);
    assert!(futures_util::poll!(&mut read).is_pending());
    let worker_pool = pool.clone();
    let worker = std::thread::spawn(move || {
        worker_pool.run_worker(
            releashd::test_support::integration::transport::configure_busy_handler(
                Connection::open_in_memory().unwrap(),
                std::sync::Arc::new(
                    releashd::test_support::integration::platform::RetryLimiter::new(),
                ),
            )
            .unwrap(),
        );
    });
    assert_eq!(read.await.unwrap(), 42);
    pool.close();
    worker.join().unwrap();
}
