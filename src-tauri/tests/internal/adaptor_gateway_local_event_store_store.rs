use releash_lib::test_support::integration::persistence::PreparedNodeEvent;
use releash_lib::test_support::integration::platform::AgentSessionRemovalMutation;
use releash_lib::test_support::integration::platform::ExpectedStreamHead;
use releash_lib::test_support::integration::platform::UncommittedDomainEvent;
use releash_lib::test_support::integration::repository::LocalEventTransactionRepository;

use releash_lib::test_support::integration::persistence::LocalEventStore;
use releash_lib::test_support::integration::persistence::LocalEventStoreConfig;
use releash_lib::test_support::integration::persistence::LocalEventStoreOpenError;
use releash_lib::test_support::integration::persistence::MAX_BATCH_DECODED_BYTES;
use releash_lib::test_support::integration::persistence::MAX_BATCH_EVENTS;
use releash_lib::test_support::integration::persistence::MAX_BATCH_STATE_MUTATIONS;
use releash_lib::test_support::integration::platform::CommitBatchError;
use releash_lib::test_support::integration::platform::CommitOperationKind;
use releash_lib::test_support::integration::platform::Deadline;
use releash_lib::test_support::integration::platform::LocalDomainEvent;
use releash_lib::test_support::integration::platform::LocalStateMutation;
use releash_lib::test_support::integration::platform::OperationContext;
use releash_lib::test_support::integration::platform::StreamId;
use releash_lib::test_support::integration::platform::StreamVersion;
use releash_lib::test_support::integration::transport::classified_error;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

#[test]
pub fn test_local_event_store_複製descriptorが残っても終了後にwriter_lockを解放する() {
    // Given: 子プロセスへの継承と同様に writer lock の descriptor が複製されている
    let directory = tempfile::tempdir().unwrap();
    let config = || {
        LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
            std::sync::Arc::new(
                releash_lib::test_support::integration::platform::RetryLimiter::new(),
            ),
        )
    };
    let store = LocalEventStore::open(config()).unwrap();
    let inherited_lock = store.test_writer_lock().try_clone().unwrap();
    assert!(matches!(
        LocalEventStore::open(config()),
        Err(LocalEventStoreOpenError::WriterLockHeld)
    ));

    // When: 全 worker を終了して store を閉じる
    drop(store);

    // Then: 複製 descriptor の close を待たずに同じ store を開き直せる
    let reopened = LocalEventStore::open(config()).unwrap();
    drop(inherited_lock);
    assert!(matches!(
        LocalEventStore::open(config()),
        Err(LocalEventStoreOpenError::WriterLockHeld)
    ));
    drop(reopened);
}

#[tokio::test]
pub async fn test_node事実追記_読取後の外部追記と競合したbatchは一行も保存しない() {
    use releash_lib::test_support::integration::persistence::read_tree;
    use releash_lib::test_support::integration::persistence::NewNodeEventRow;
    use releash_lib::test_support::integration::platform::CommitBatchError;
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().into(),
        std::sync::Arc::new(releash_lib::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let row = NewNodeEventRow {
        tree_id: "tree".into(),
        node_execution_id: "node".into(),
        parent_id: None,
        node_name: "main".into(),
        kind: "session".into(),
        attempt: 1,
        event_type: "started".into(),
        session_id: None,
        detail: "{}".into(),
    };
    let head = store.append_node_event(row.clone(), Some(1)).await.unwrap();
    store.append_node_event(row.clone(), Some(2)).await.unwrap();
    // When
    let result = store
        .append_node_events_at_head(
            vec![(row.clone(), Some(3)), (row.clone(), Some(3))],
            Some(("tree".into(), head)),
        )
        .await;
    // Then
    assert!(matches!(result, Err(CommitBatchError::TreeHeadConflict)));
    let rows = store
        .submit_query(|connection| {
            read_tree(connection, "tree")
                .map_err(|_| releash_lib::test_support::integration::platform::LocalEventQueryError::InvalidRequest)
        })
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        store
            .append_node_events_at_head(
                vec![(row, Some(4))],
                Some(("tree".into(), rows.last().unwrap().seq)),
            )
            .await
            .unwrap(),
        vec![3]
    );
}

fn empty_batch(
    store: &LocalEventStore,
) -> releash_lib::test_support::integration::platform::LocalAtomicBatch {
    use releash_lib::test_support::integration::platform::CommitIdentity;
    use releash_lib::test_support::integration::platform::CommitOperationKind;
    use releash_lib::test_support::integration::platform::IdempotencyBinding;
    use releash_lib::test_support::integration::platform::LocalAtomicBatch;
    LocalAtomicBatch {
        commit_id: CommitIdentity::parse("write-test").unwrap(),
        idempotency: IdempotencyBinding {
            installation_id: store.installation_id().into(),
            operation_kind: CommitOperationKind::UserMutation,
            idempotency_key: "write-test".into(),
            payload_hash: [0; 32],
        },
        expected_heads: vec![],
        events: vec![],
        state_mutations: vec![],
    }
}

fn fact_row() -> releash_lib::test_support::integration::persistence::NewNodeEventRow {
    releash_lib::test_support::integration::persistence::NewNodeEventRow {
        tree_id: "tree".into(),
        node_execution_id: "node".into(),
        parent_id: None,
        node_name: "main".into(),
        kind: "session".into(),
        attempt: 1,
        event_type: "started".into(),
        session_id: None,
        detail: "{}".into(),
    }
}

#[tokio::test(flavor = "current_thread")]
pub async fn test_書込待ち_writer停滞中も同じruntimeの読取が完了する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().into(),
        std::sync::Arc::new(releash_lib::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let stall = store.fault_injector().arm_node_event_append_stall();
    let append = store.append_node_event(fact_row(), None);
    tokio::pin!(append);
    assert!(futures_util::poll!(append.as_mut()).is_pending());
    stall.wait_until_arrived();
    // When
    let read = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        store.submit_query(|_| Ok(42)),
    )
    .await;
    // Then
    assert_eq!(read.unwrap().unwrap(), 42);
    assert!(futures_util::poll!(append.as_mut()).is_pending());
    stall.release();
    assert_eq!(append.await.unwrap(), 1);
}

#[tokio::test]
pub async fn test_書込混雑_全入口と両車線でunavailableを返す() {
    use releash_lib::test_support::integration::persistence::PreparedNodeEvent;
    use releash_lib::test_support::integration::persistence::WriteJob;
    use releash_lib::test_support::integration::persistence::CRITICAL_LANE_MAX_BYTES;
    use releash_lib::test_support::integration::persistence::CRITICAL_LANE_MAX_REQUESTS;
    use releash_lib::test_support::integration::persistence::NORMAL_LANE_MAX_BYTES;
    use releash_lib::test_support::integration::persistence::NORMAL_LANE_MAX_REQUESTS;

    use releash_lib::test_support::integration::transport::classified_error;

    // Given: writer を停止し、車線を件数または byte 上限まで満たす
    for by_bytes in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let store = LocalEventStore::open(LocalEventStoreConfig::production(
            directory.path().into(),
            std::sync::Arc::new(
                releash_lib::test_support::integration::platform::RetryLimiter::new(),
            ),
        ))
        .unwrap();
        let stall = store.fault_injector().arm_node_event_append_stall();
        let append = store.append_node_event(fact_row(), None);
        tokio::pin!(append);
        assert!(futures_util::poll!(append.as_mut()).is_pending());
        stall.wait_until_arrived();
        for (critical, count, bytes) in [
            (false, NORMAL_LANE_MAX_REQUESTS, NORMAL_LANE_MAX_BYTES),
            (true, CRITICAL_LANE_MAX_REQUESTS, CRITICAL_LANE_MAX_BYTES),
        ] {
            for _ in 0..if by_bytes { 1 } else { count } {
                assert!(store
                    .test_queue()
                    .admit(WriteJob {
                        critical,
                        bytes: if by_bytes { bytes } else { 0 },
                        run: Box::new(|_| {})
                    })
                    .is_ok());
            }
        }
        // When / Then
        for operation in [
            CommitOperationKind::UserMutation,
            CommitOperationKind::Workflow,
        ] {
            let mut batch = empty_batch(&store);
            batch.idempotency.operation_kind = operation;
            // 空 batch も byte 上限で拒否されるよう、小さい node event を含める
            let node = PreparedNodeEvent {
                row: fact_row(),
                timestamp_ms: 1,
                expect_tree_absent: false,
            };
            let error = store
                .commit_batch_with_node_events(batch.clone(), vec![node])
                .await
                .unwrap_err();
            assert_eq!(error, CommitBatchError::QueueBusy);
            assert_eq!(
                classified_error(error).code,
                connectrpc::ErrorCode::Unavailable
            );
            if !by_bytes {
                let error = store.commit_batch(batch).await.unwrap_err();
                assert_eq!(
                    classified_error(error).code,
                    connectrpc::ErrorCode::Unavailable
                );
            }
        }
        for error in [
            store.append_node_event(fact_row(), None).await.unwrap_err(),
            store
                .append_node_events(vec![(fact_row(), None)])
                .await
                .unwrap_err(),
            store
                .append_node_events_at_head(vec![(fact_row(), None)], Some(("tree".into(), 1)))
                .await
                .unwrap_err(),
        ] {
            let workflow =
                releash_lib::test_support::integration::workflow::WorkflowError::from(error);
            assert_eq!(
                classified_error(workflow).code,
                connectrpc::ErrorCode::Unavailable
            );
        }
        store.test_queue().close();
        stall.release();
        append.await.unwrap();
    }
}

#[tokio::test]
pub async fn test_batch上限_件数と合計byte超過は保存せずresource_exhaustedを返す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().into(),
        std::sync::Arc::new(releash_lib::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let stream_id = StreamId::provider_lifecycle("test").unwrap();
    let mut batch = empty_batch(&store);
    batch.expected_heads.push(ExpectedStreamHead {
        stream_id: stream_id.clone(),
        expected: StreamVersion::zero(),
    });
    let event = UncommittedDomainEvent {
        stream_id,
        occurred_at_ms: 1,
        event: LocalDomainEvent::ProviderLifecycle(
            releash_lib::test_support::integration::providers::ProviderLifecycleEvent::StopObserved {
                binding_id: "test".into(),
            },
        ),
    };
    batch.events = vec![event; MAX_BATCH_EVENTS + 1];
    // When / Then
    let error = store.commit_batch(batch).await.unwrap_err();
    assert_eq!(
        releash_lib::test_support::integration::transport::classified_error(error).code,
        connectrpc::ErrorCode::ResourceExhausted
    );
    for (count, detail_bytes) in [(MAX_BATCH_EVENTS + 1, 2), (1, MAX_BATCH_DECODED_BYTES)] {
        let mut row = fact_row();
        row.detail = "x".repeat(detail_bytes);
        let nodes = vec![
            PreparedNodeEvent {
                row,
                timestamp_ms: 1,
                expect_tree_absent: false
            };
            count
        ];
        let error = store
            .commit_batch_with_node_events(empty_batch(&store), nodes)
            .await
            .unwrap_err();
        assert_eq!(
            releash_lib::test_support::integration::transport::classified_error(error).code,
            connectrpc::ErrorCode::ResourceExhausted
        );
    }
    let mutation = LocalStateMutation::AgentSessionRemoval(AgentSessionRemovalMutation {
        node_event_tree_id: "absent-tree".into(),
        ownership_projection_id: None,
        ownership_stream: None,
        ownership_expected: None,
    });
    let mut oversized = empty_batch(&store);
    oversized.state_mutations = vec![mutation.clone(); MAX_BATCH_STATE_MUTATIONS + 1];
    let error = store.commit_batch(oversized).await.unwrap_err();
    assert_eq!(
        releash_lib::test_support::integration::transport::classified_error(error).code,
        connectrpc::ErrorCode::ResourceExhausted
    );
    let mut oversized = empty_batch(&store);
    oversized.state_mutations = vec![LocalStateMutation::AgentSessionRemoval(
        AgentSessionRemovalMutation {
            node_event_tree_id: "x".repeat(MAX_BATCH_DECODED_BYTES),
            ownership_projection_id: None,
            ownership_stream: None,
            ownership_expected: None,
        },
    )];
    let error = store.commit_batch(oversized).await.unwrap_err();
    assert_eq!(
        releash_lib::test_support::integration::transport::classified_error(error).code,
        connectrpc::ErrorCode::ResourceExhausted
    );
    let mut combined = empty_batch(&store);
    combined.state_mutations.push(mutation.clone());
    let mut row = fact_row();
    row.detail = "x".repeat(MAX_BATCH_DECODED_BYTES - 256);
    let error = store
        .commit_batch_with_node_events(
            combined,
            vec![PreparedNodeEvent {
                row,
                timestamp_ms: 1,
                expect_tree_absent: false,
            }],
        )
        .await
        .unwrap_err();
    assert_eq!(
        releash_lib::test_support::integration::transport::classified_error(error).code,
        connectrpc::ErrorCode::ResourceExhausted
    );
    let mut boundary = empty_batch(&store);
    boundary.state_mutations = vec![mutation; MAX_BATCH_STATE_MUTATIONS];
    assert!(LocalEventStore::validate_batch_size(0, 0, boundary.state_mutations.len(), 0).is_ok());
    let mut prepared = store.prepare(empty_batch(&store), 0).unwrap();
    prepared.decoded_bytes = MAX_BATCH_DECODED_BYTES;
    assert!(LocalEventStore::validate_batch_size(0, 0, 0, prepared.decoded_bytes).is_ok());
    prepared.decoded_bytes += 1;
    assert_eq!(
        LocalEventStore::validate_batch_size(0, 0, 0, prepared.decoded_bytes),
        Err(CommitBatchError::CapacityExceeded)
    );
    assert_eq!(store.pending_write_request_count(), 0);
    assert_eq!(store.append_node_event(fact_row(), None).await.unwrap(), 1);
}

#[tokio::test]
pub async fn test_batch件数超過_shape検査とcodec実行より前に拒否する() {
    use releash_lib::test_support::integration::persistence::CborValue;
    use releash_lib::test_support::integration::persistence::EventCodecError;
    use releash_lib::test_support::integration::persistence::EventCodecRegistry;
    use releash_lib::test_support::integration::persistence::LocalEventPayloadCodec;

    use std::sync::Arc;

    struct RejectEncoding;
    impl LocalEventPayloadCodec for RejectEncoding {
        fn event_type(&self) -> &'static str {
            "test.reject_encoding"
        }
        fn payload_version(&self) -> i64 {
            1
        }
        fn handles(&self, _: &LocalDomainEvent) -> bool {
            true
        }
        fn encode(&self, _: &LocalDomainEvent) -> Result<CborValue, EventCodecError> {
            panic!("件数超過の batch を encode してはならない")
        }
        fn decode(
            &self,
            _: i64,
            _: &CborValue,
        ) -> Result<Option<LocalDomainEvent>, EventCodecError> {
            unreachable!()
        }
    }
    // Given
    let directory = tempfile::tempdir().unwrap();
    let mut config = LocalEventStoreConfig::production(
        directory.path().into(),
        std::sync::Arc::new(releash_lib::test_support::integration::platform::RetryLimiter::new()),
    );
    let mut registry = EventCodecRegistry::new();
    registry.register(Arc::new(RejectEncoding));
    config.registry = Arc::new(registry);
    let store = LocalEventStore::open(config).unwrap();
    for kind in ["events", "mutations", "nodes"] {
        for invalid_shape in [true, false] {
            let mut batch = empty_batch(&store);
            let stream_id = StreamId::provider_lifecycle("test").unwrap();
            batch.expected_heads = vec![
                ExpectedStreamHead {
                    stream_id: stream_id.clone(),
                    expected: StreamVersion::zero(),
                };
                if invalid_shape { 2 } else { 1 }
            ];
            batch.events =
                vec![UncommittedDomainEvent {
                stream_id, occurred_at_ms: 1,
                event: LocalDomainEvent::ProviderLifecycle(
                    releash_lib::test_support::integration::providers::ProviderLifecycleEvent::StopObserved {
                        binding_id: "test".into(),
                    }),
            }; if kind == "events" { MAX_BATCH_EVENTS + 1 } else { 1 }];
            if kind == "mutations" {
                batch.state_mutations =
                    vec![
                        LocalStateMutation::AgentSessionRemoval(AgentSessionRemovalMutation {
                            node_event_tree_id: "tree".into(),
                            ownership_projection_id: None,
                            ownership_stream: None,
                            ownership_expected: None,
                        });
                        MAX_BATCH_STATE_MUTATIONS + 1
                    ];
            }
            let nodes = vec![
                PreparedNodeEvent {
                    row: fact_row(),
                    timestamp_ms: 1,
                    expect_tree_absent: false
                };
                if kind == "nodes" {
                    MAX_BATCH_EVENTS + 1
                } else {
                    0
                }
            ];
            // When / Then
            if kind != "nodes" {
                assert_eq!(
                    store.commit_batch(batch.clone()).await.unwrap_err(),
                    CommitBatchError::CapacityExceeded
                );
            }
            assert_eq!(
                store
                    .commit_batch_with_node_events(batch, nodes)
                    .await
                    .unwrap_err(),
                CommitBatchError::CapacityExceeded
            );
        }
    }
    assert_eq!(store.pending_write_request_count(), 0);
}

#[tokio::test]
pub async fn test_node事実追記_件数とbyteの上限まで保存し超過は保存しない() {
    for (count, detail_bytes) in [(MAX_BATCH_EVENTS, 2), (1, MAX_BATCH_DECODED_BYTES - 256)] {
        let directory = tempfile::tempdir().unwrap();
        let store = LocalEventStore::open(LocalEventStoreConfig::production(
            directory.path().into(),
            std::sync::Arc::new(
                releash_lib::test_support::integration::platform::RetryLimiter::new(),
            ),
        ))
        .unwrap();
        let mut row = fact_row();
        row.detail = "x".repeat(detail_bytes);
        let mut oversized = vec![(row.clone(), None); count];
        if count == MAX_BATCH_EVENTS {
            oversized.push((row.clone(), None));
        } else {
            oversized[0].0.detail.push('x');
        }

        let error = store.append_node_events(oversized).await.unwrap_err();
        assert_eq!(error, CommitBatchError::CapacityExceeded);
        assert_eq!(
            classified_error(
                releash_lib::test_support::integration::workflow::WorkflowError::from(error)
            )
            .code,
            connectrpc::ErrorCode::ResourceExhausted
        );
        let sequences = store
            .append_node_events_at_head(vec![(row, None); count], Some(("tree".into(), 0)))
            .await
            .unwrap();
        assert_eq!(sequences.len(), count);
        assert_eq!(sequences[0], 1);
        assert_eq!(sequences[count - 1], count as i64);
    }
}

#[tokio::test]
pub async fn test_書込待ち_期限と取り消しで待ちを終えても受理済みの事実は保存する() {
    use connectrpc::ErrorCode;
    use releash_lib::test_support::integration::platform::Deadline;
    use releash_lib::test_support::integration::platform::OperationContext;
    use releash_lib::test_support::integration::transport::ConnectFailure;
    use std::time::Duration;
    use std::time::Instant;
    for expire in [false, true] {
        // Given
        let directory = tempfile::tempdir().unwrap();
        let store = LocalEventStore::open(LocalEventStoreConfig::production(
            directory.path().into(),
            std::sync::Arc::new(
                releash_lib::test_support::integration::platform::RetryLimiter::new(),
            ),
        ))
        .unwrap();
        let stall = store.fault_injector().arm_node_event_append_stall();
        let token = tokio_util::sync::CancellationToken::new();
        let context = OperationContext::new(
            expire.then(|| Deadline::new(Instant::now() + Duration::from_millis(30))),
            std::sync::Arc::new(token.clone()),
        );
        let append = releash_lib::test_support::integration::platform::scope(
            context,
            store.append_node_event(fact_row(), None),
        );
        tokio::pin!(append);
        assert!(futures_util::poll!(&mut append).is_pending());
        stall.wait_until_arrived();
        // When
        if !expire {
            token.cancel();
        }
        let error = tokio::time::timeout(Duration::from_secs(2), append)
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
        stall.release();
        assert_eq!(store.append_node_event(fact_row(), None).await.unwrap(), 2);
        assert_eq!(
            store
                .submit_query(|connection| connection
                    .query_row("SELECT count(*) FROM node_events", [], |row| row
                        .get::<_, i64>(0))
                    .map_err(|error| {
                        releash_lib::test_support::integration::persistence::storage_unavailable(
                            &error,
                        )
                    }))
                .await
                .unwrap(),
            2
        );
    }
}

#[test]
pub fn test_起動時分類_db競合を呼出期限で打ち切り既存の資源失敗へ写す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let layout =
        releash_lib::test_support::integration::persistence::StoreLayout::new(directory.path());
    let path = layout.database_path();
    let blocker = rusqlite::Connection::open(&path).unwrap();
    blocker
        .execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA locking_mode=EXCLUSIVE; CREATE TABLE locked(value);",
        )
        .unwrap();
    let started = Instant::now();
    let context = OperationContext::default()
        .with_deadline(Deadline::new(started + Duration::from_millis(100)));
    // When
    let result = releash_lib::test_support::integration::platform::sync_scope(context, || {
        releash_lib::test_support::integration::persistence::classify_existing_database(
            &layout,
            &path,
            Arc::new(releash_lib::test_support::integration::platform::RetryLimiter::new()),
        )
    });
    // Then
    assert!(
        matches!(
            result,
            Err(LocalEventStoreOpenError::StorageUnavailable(
                releash_lib::test_support::integration::platform::TechnicalFailure {
                    nature: releash_lib::test_support::integration::platform::TechnicalFailureNature::Transient,
                    ..
                }
            ))
        ),
        "{result:?}"
    );
    assert!(started.elapsed() >= Duration::from_millis(100));
    assert!(started.elapsed() < Duration::from_secs(1));
}
pub(crate) mod startup_error_classification_tests {}
