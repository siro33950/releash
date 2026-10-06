use crate::adaptor_gateway_workflow_workflow_host_test_helpers::archive_fixture;
use crate::adaptor_gateway_workflow_workflow_host_test_helpers::archive_fixture_with_resolver;
use crate::adaptor_gateway_workflow_workflow_host_test_helpers::archive_workflow;
use releash_lib::test_support::integration::subscriptions::StateChangeSource;
use releash_lib::test_support::integration::workflow::ExecutionStatus;
use releash_lib::test_support::integration::workflow::ExecutionTreeArchiveRepository;
use releash_lib::test_support::integration::workflow::ManagedWorktreeResolver;
use releash_lib::test_support::integration::workflow::ManagedWorktreeResolverError;
use releash_lib::test_support::integration::workflow::NodeFact;
use releash_lib::test_support::integration::workflow::SessionExecutionTreeRootFacts;
use releash_lib::test_support::integration::workflow::WorkflowError;
use std::sync::Arc;

#[tokio::test]
pub async fn test_実行木archive_abortと自然完了の競合だけを終了状態の再読取で解消する() {
    use releash_lib::test_support::integration::persistence::LocalEventStore;

    use releash_lib::test_support::integration::workflow::AbortExecutionCommand;
    use releash_lib::test_support::integration::workflow::NodeFact;
    use releash_lib::test_support::integration::workflow::WorkflowAbortExecutionGateway;
    use releash_lib::test_support::integration::workflow::WorkflowAbortExecutionUsecase;
    use releash_lib::test_support::integration::workflow::WorkflowError;

    struct CompletingAbort {
        store: Arc<LocalEventStore>,
        complete: bool,
        error: WorkflowError,
    }
    #[async_trait::async_trait]
    impl WorkflowAbortExecutionGateway for CompletingAbort {
        async fn abort_execution(
            &self,
            command: AbortExecutionCommand,
        ) -> Result<(), WorkflowError> {
            if self.complete {
                let records = releash_lib::test_support::integration::workflow::read_tree_records(
                    &self.store,
                    &command.execution_id,
                )
                .await
                .unwrap();
                let meta = &records[0].meta;
                for kind in ["submit_received", "stop_received"] {
                    releash_lib::test_support::integration::workflow::append_single_fact(
                        &self.store,
                        meta,
                        &releash_lib::test_support::integration::workflow::decode(kind, "{}")
                            .unwrap(),
                        2000,
                    )
                    .await
                    .unwrap();
                }
            }
            Err(self.error.clone())
        }
    }

    for complete in [false, true] {
        for error in [
            WorkflowError::invalid_state("already terminal"),
            WorkflowError::NotFound("runtime already released".into()),
            WorkflowError::external("abort persistence failed"),
        ] {
            // Given
            let mut fixture = archive_fixture();
            let id = archive_workflow(&fixture).await;
            fixture
                .runtime
                .test_replace_abort_execution(WorkflowAbortExecutionUsecase::new(Arc::new(
                    CompletingAbort {
                        store: fixture.store.clone(),
                        complete,
                        error: error.clone(),
                    },
                )));
            // When
            let result = fixture.runtime.archive_execution_tree(&id, "manual").await;
            // Then
            let succeeds = complete && !matches!(error, WorkflowError::External(_));
            if succeeds {
                result.unwrap();
                assert!(fixture.sessions.live_sessions.lock().unwrap().is_empty());
            } else {
                assert_eq!(result, Err(error));
            }
            assert_eq!(
                fixture.repository.target(&id).await.unwrap().status,
                if complete {
                    ExecutionStatus::Completed
                } else {
                    ExecutionStatus::Running
                }
            );
            let archives = fixture
                .repository
                .archive_snapshot_for(std::slice::from_ref(&id))
                .await
                .unwrap();
            assert_eq!(archives.records.len(), usize::from(succeeds));
            assert!(
                !releash_lib::test_support::integration::workflow::read_tree_records(
                    &fixture.store,
                    &id
                )
                .await
                .unwrap()
                .iter()
                .any(|record| matches!(record.fact, NodeFact::AbortRequested(_)))
            );
        }
    }
}

#[tokio::test]
pub async fn test_実行木変更の受理_削除との競合と排他記録の保存不能を区別する() {
    use releash_lib::test_support::integration::repository::FileWorktreeOperationLocks;

    use releash_lib::test_support::integration::platform::WorktreeOperations;
    // Given
    let mut fixture = archive_fixture();
    let directory = tempfile::tempdir().unwrap();
    fixture
        .runtime
        .test_replace_worktree_operations(Arc::new(WorktreeOperations::new(Arc::new(
            FileWorktreeOperationLocks::new(directory.path()),
        ))));
    let deletion = fixture
        .runtime
        .test_worktree_operations()
        .delete("/repo")
        .await
        .unwrap();
    // When / Then
    assert!(matches!(
        fixture.runtime.begin_worktree_mutation("/repo"),
        Err(WorkflowError::Conflict(_))
    ));
    drop(deletion);
    std::fs::remove_dir_all(directory.path().join("worktree-operations")).unwrap();
    std::fs::write(directory.path().join("worktree-operations"), "unavailable").unwrap();
    assert!(matches!(
        fixture.runtime.begin_worktree_mutation("/repo"),
        Err(WorkflowError::External(_))
    ));
}

#[tokio::test]
pub async fn test_実行木archive_同じworktreeのworkflowと複数sessionをすべて片付ける() {
    use releash_lib::test_support::integration::workflow::SessionExecutionTreeRootFacts;
    // Given
    let fixture = archive_fixture();
    let workflow = archive_workflow(&fixture).await;
    let ids = [
        "agent-session-00000000000040008000000000000111",
        "agent-session-00000000000040008000000000000112",
    ];
    for id in ids {
        let facts = SessionExecutionTreeRootFacts::new(
            id,
            "/missing/worktree",
            "/missing/worktree",
            releash_lib::test_support::integration::providers::ProviderKind::Codex,
            None,
        )
        .unwrap();
        releash_lib::test_support::integration::workflow::append_fact_batch_for_seed(
            &fixture.store,
            &facts.into_facts(),
            1,
            id,
        )
        .unwrap();
        fixture
            .sessions
            .live_sessions
            .lock()
            .unwrap()
            .insert(id.into());
    }
    // When
    fixture
        .runtime
        .archive_worktree("/missing/worktree")
        .await
        .unwrap();
    // Then
    for id in [workflow.as_str(), ids[0], ids[1]] {
        assert_eq!(
            fixture.repository.target(id).await.unwrap().status,
            ExecutionStatus::Aborted
        );
        assert_eq!(
            fixture
                .repository
                .archive_snapshot_for(&[id.into()])
                .await
                .unwrap()
                .records[0]
                .archive_reason,
            "worktree_removed"
        );
    }
    assert!(fixture.sessions.live_sessions.lock().unwrap().is_empty());
}

#[tokio::test]
pub async fn test_実行木restore_所属worktreeが利用不可なら事実を追記しない() {
    struct UnavailableWorktree;
    #[async_trait::async_trait]
    impl ManagedWorktreeResolver for UnavailableWorktree {
        async fn resolve(&self, _: String) -> Result<String, ManagedWorktreeResolverError> {
            Err(ManagedWorktreeResolverError::Validation(
                "worktree unavailable".into(),
            ))
        }
    }
    // Given
    let fixture = archive_fixture_with_resolver(Arc::new(UnavailableWorktree));
    let id = archive_workflow(&fixture).await;
    fixture
        .runtime
        .archive_execution_tree(&id, "manual")
        .await
        .unwrap();
    let before =
        releash_lib::test_support::integration::workflow::read_tree_records(&fixture.store, &id)
            .await
            .unwrap();
    // When
    assert!(fixture.runtime.restore_execution_tree(&id).await.is_err());
    // Then
    assert_eq!(
        releash_lib::test_support::integration::workflow::read_tree_records(&fixture.store, &id)
            .await
            .unwrap(),
        before
    );
}

#[tokio::test]
pub async fn test_旧sessionarchive移行_128件を越えて時刻と理由と終了状態を維持する() {
    // Given
    let fixture = archive_fixture();
    let mut expected = Vec::new();
    for index in 0..131 {
        let id = format!("00000000-0000-4000-8000-{index:012}");
        let facts = SessionExecutionTreeRootFacts::new(
            &id,
            "/workspace",
            "/gone",
            releash_lib::test_support::integration::providers::ProviderKind::Codex,
            None,
        )
        .unwrap();
        let meta = facts.meta.clone();
        let mut rows: Vec<_> = facts
            .into_facts()
            .iter()
            .take(2)
            .map(|(meta, fact)| {
                releash_lib::test_support::integration::workflow::pending_single_fact(meta, fact, 1)
                    .unwrap()
            })
            .collect();
        let completed = index % 2 == 0;
        if completed {
            for kind in ["submit_received", "stop_received"] {
                rows.push(
                    releash_lib::test_support::integration::workflow::pending_single_fact(
                        &meta,
                        &releash_lib::test_support::integration::workflow::decode(kind, "{}")
                            .unwrap(),
                        2,
                    )
                    .unwrap(),
                );
            }
        }
        let timestamp = 42_123 + index;
        let reason = if index % 3 == 0 {
            "worktree_removed"
        } else {
            "manual"
        };
        let mut legacy = releash_lib::test_support::integration::workflow::pending_single_fact(
            &meta,
            &NodeFact::AbortRequested(Default::default()),
            timestamp,
        )
        .unwrap();
        legacy.row.event_type = "archive_requested".into();
        legacy.row.detail = serde_json::json!({"reason": reason}).to_string();
        rows.push(legacy);
        releash_lib::test_support::integration::workflow::append_pending_rows(&fixture.store, rows)
            .await
            .unwrap();
        expected.push((id, timestamp as f64 / 1000.0, reason, completed));
    }
    assert_eq!(
        fixture
            .repository
            .legacy_session_archive_page(None)
            .await
            .unwrap()
            .len(),
        128
    );
    // When
    fixture
        .runtime
        .migrate_execution_archives(fixture.repository.as_ref())
        .await
        .unwrap();
    // Then
    assert!(fixture
        .repository
        .legacy_session_archive_page(None)
        .await
        .unwrap()
        .is_empty());
    for (id, timestamp, reason, completed) in expected {
        let record = fixture
            .repository
            .archive_snapshot_for(std::slice::from_ref(&id))
            .await
            .unwrap()
            .records
            .remove(0);
        assert_eq!(record.archived_at, timestamp);
        assert_eq!(record.archive_reason, reason);
        assert_eq!(
            fixture.repository.target(&id).await.unwrap().status,
            if completed {
                ExecutionStatus::Completed
            } else {
                ExecutionStatus::Aborted
            }
        );
        let facts = releash_lib::test_support::integration::workflow::read_tree_records(
            &fixture.store,
            &id,
        )
        .await
        .unwrap();
        assert_eq!(
            facts
                .iter()
                .filter(|record| matches!(record.fact, NodeFact::ArchiveRequested(_)))
                .count(),
            2
        );
        assert_eq!(
            facts
                .iter()
                .filter(|record| matches!(record.fact, NodeFact::AbortRequested(_)))
                .count(),
            usize::from(!completed)
        );
    }
    fixture
        .runtime
        .migrate_execution_archives(fixture.repository.as_ref())
        .await
        .unwrap();
    assert!(fixture
        .repository
        .legacy_session_archive_page(None)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
pub async fn test_終了済み実行木_archiveとrestore成功後だけ所属worktreeの購読を更新する() {
    use releash_lib::test_support::integration::subscriptions::StateChangeSource;
    // Given
    let mut fixture = archive_fixture();
    let id = archive_workflow(&fixture).await;
    fixture
        .runtime
        .abort_execution(
            releash_lib::test_support::integration::workflow::AbortExecutionCommand {
                execution_id: id.clone(),
                expected_node_name: None,
            },
        )
        .await
        .unwrap();
    let publisher = releash_lib::test_support::integration::subscriptions::test_subscriptions();
    let mut changes = releash_lib::test_support::integration::subscriptions::changes(&publisher);
    fixture.runtime = fixture.runtime.with_state_publisher(publisher);
    // When / Then
    fixture
        .runtime
        .archive_execution_tree(&id, "manual")
        .await
        .unwrap();
    assert_eq!(
        changes.try_recv().unwrap(),
        StateChangeSource::Worktree("/missing/worktree".into())
    );
    fixture.runtime.restore_execution_tree(&id).await.unwrap();
    assert_eq!(
        changes.try_recv().unwrap(),
        StateChangeSource::Worktree("/missing/worktree".into())
    );
    assert!(fixture
        .runtime
        .archive_execution_tree("missing", "manual")
        .await
        .is_err());
    assert!(fixture
        .runtime
        .restore_execution_tree("missing")
        .await
        .is_err());
    assert!(changes.try_recv().is_err());
}

#[tokio::test]
pub async fn test_実行木archiveとrestore_workspace識別子と異なるworktreeパスを通知する() {
    // Given
    let mut fixture = archive_fixture();
    let id = "agent-session-00000000000040008000000000000113";
    let facts = SessionExecutionTreeRootFacts::new(
        id,
        "/workspace",
        "/missing/worktree",
        releash_lib::test_support::integration::providers::ProviderKind::Codex,
        None,
    )
    .unwrap();
    let meta = facts.meta.clone();
    releash_lib::test_support::integration::workflow::append_fact_batch_for_seed(
        &fixture.store,
        &facts.into_facts(),
        1,
        id,
    )
    .unwrap();
    releash_lib::test_support::integration::workflow::append_single_fact(
        &fixture.store,
        &meta,
        &NodeFact::AbortRequested(Default::default()),
        2,
    )
    .await
    .unwrap();
    let target = fixture.repository.target(id).await.unwrap();
    assert_ne!(target.workspace_identity, target.worktree_path);
    let publisher = releash_lib::test_support::integration::subscriptions::test_subscriptions();
    let mut changes = releash_lib::test_support::integration::subscriptions::changes(&publisher);
    fixture.runtime = fixture.runtime.with_state_publisher(publisher);
    // When / Then
    fixture
        .runtime
        .archive_execution_tree(id, "manual")
        .await
        .unwrap();
    assert_eq!(
        changes.try_recv().unwrap(),
        StateChangeSource::Worktree(target.worktree_path.clone())
    );
    fixture.runtime.restore_execution_tree(id).await.unwrap();
    assert_eq!(
        changes.try_recv().unwrap(),
        StateChangeSource::Worktree(target.worktree_path)
    );
    assert!(changes.try_recv().is_err());
}
