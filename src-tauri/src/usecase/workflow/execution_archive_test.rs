use crate::adaptor::gateway::workflow::workflow_host::test_helpers::{
    archive_fixture, archive_fixture_with_resolver, archive_workflow,
};
use crate::domain::workflow::{ExecutionStatus, ExecutionTreeArchiveRepository};
use crate::usecase::workflow::runtime_resolver::{
    ManagedWorktreeResolver, ManagedWorktreeResolverError,
};
use std::sync::Arc;

#[tokio::test]
async fn test_実行木archive_abortと自然完了の競合だけを終了状態の再読取で解消する() {
    use crate::adaptor::gateway::local_event_store::LocalEventStore;
    use crate::adaptor::gateway::workflow::fact_log;
    use crate::domain::workflow::{NodeFact, WorkflowError};
    use crate::usecase::workflow::command::{AbortExecutionCommand, WorkflowAbortExecutionUsecase};
    use crate::usecase::workflow::ports::WorkflowAbortExecutionGateway;

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
                let records =
                    fact_log::read_tree_records(&self.store, &command.execution_id).unwrap();
                let meta = &records[0].meta;
                for kind in ["submit_received", "stop_received"] {
                    fact_log::append_single_fact(
                        &self.store,
                        meta,
                        &NodeFact::decode(kind, "{}").unwrap(),
                        2000,
                    )
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
            fixture.runtime.abort_execution =
                WorkflowAbortExecutionUsecase::new(Arc::new(CompletingAbort {
                    store: fixture.store.clone(),
                    complete,
                    error: error.clone(),
                }));
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
                fixture.repository.target(&id).unwrap().status,
                if complete {
                    ExecutionStatus::Completed
                } else {
                    ExecutionStatus::Running
                }
            );
            let archives = fixture
                .repository
                .archive_snapshot_for(&[id.clone()])
                .unwrap();
            assert_eq!(archives.records.len(), usize::from(succeeds));
            assert!(!fact_log::read_tree_records(&fixture.store, &id)
                .unwrap()
                .iter()
                .any(|record| matches!(record.fact, NodeFact::AbortRequested)));
        }
    }
}

#[tokio::test]
async fn test_実行木変更の受理_削除との競合と排他記録の保存不能を区別する() {
    use crate::adaptor::gateway::repository::worktree_operation::FileWorktreeOperationLocks;
    use crate::domain::workflow::WorkflowError;
    use crate::usecase::worktree_operation::WorktreeOperations;
    // Given
    let mut fixture = archive_fixture();
    let directory = tempfile::tempdir().unwrap();
    fixture.runtime.worktree_operations = Arc::new(WorktreeOperations::new(Arc::new(
        FileWorktreeOperationLocks::new(directory.path()),
    )));
    let deletion = fixture
        .runtime
        .worktree_operations
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
async fn test_実行木archive_同じworktreeのworkflowと複数sessionをすべて片付ける() {
    use crate::domain::workflow::{ExecutionTreeArchiveRepository, SessionExecutionTreeRootFacts};
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
            crate::domain::provider_lifecycle::ProviderKind::Codex,
            None,
        )
        .unwrap();
        crate::adaptor::gateway::workflow::fact_log::append_fact_batch_for_seed(
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
            fixture.repository.target(id).unwrap().status,
            ExecutionStatus::Aborted
        );
        assert_eq!(
            fixture
                .repository
                .archive_snapshot_for(&[id.into()])
                .unwrap()
                .records[0]
                .archive_reason,
            "worktree_removed"
        );
    }
    assert!(fixture.sessions.live_sessions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn test_実行木restore_所属worktreeが利用不可なら事実を追記しない() {
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
        crate::adaptor::gateway::workflow::fact_log::read_tree_records(&fixture.store, &id)
            .unwrap();
    // When
    assert!(fixture.runtime.restore_execution_tree(&id).await.is_err());
    // Then
    assert_eq!(
        crate::adaptor::gateway::workflow::fact_log::read_tree_records(&fixture.store, &id)
            .unwrap(),
        before
    );
}

#[tokio::test]
async fn test_旧sessionarchive移行_128件を越えて時刻と理由と終了状態を維持する() {
    use crate::adaptor::gateway::workflow::fact_log;
    use crate::domain::workflow::{NodeFact, SessionExecutionTreeRootFacts};
    // Given
    let fixture = archive_fixture();
    let mut expected = Vec::new();
    for index in 0..131 {
        let id = format!("00000000-0000-4000-8000-{index:012}");
        let facts = SessionExecutionTreeRootFacts::new(
            &id,
            "/workspace",
            "/gone",
            crate::domain::provider_lifecycle::ProviderKind::Codex,
            None,
        )
        .unwrap();
        let meta = facts.meta.clone();
        let mut rows: Vec<_> = facts
            .into_facts()
            .iter()
            .map(|(meta, fact)| fact_log::pending_single_fact(meta, fact, 1).unwrap())
            .collect();
        let completed = index % 2 == 0;
        if completed {
            for kind in ["submit_received", "stop_received"] {
                rows.push(
                    fact_log::pending_single_fact(&meta, &NodeFact::decode(kind, "{}").unwrap(), 2)
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
        let mut legacy =
            fact_log::pending_single_fact(&meta, &NodeFact::AbortRequested, timestamp).unwrap();
        legacy.row.event_type = "archive_requested".into();
        legacy.row.detail = serde_json::json!({"reason": reason}).to_string();
        rows.push(legacy);
        fact_log::append_pending_rows_blocking(&fixture.store, rows).unwrap();
        expected.push((id, timestamp as f64 / 1000.0, reason, completed));
    }
    assert_eq!(
        fixture
            .repository
            .legacy_session_archive_page(None)
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
        .unwrap()
        .is_empty());
    for (id, timestamp, reason, completed) in expected {
        let record = fixture
            .repository
            .archive_snapshot_for(std::slice::from_ref(&id))
            .unwrap()
            .records
            .remove(0);
        assert_eq!(record.archived_at, timestamp);
        assert_eq!(record.archive_reason, reason);
        assert_eq!(
            fixture.repository.target(&id).unwrap().status,
            if completed {
                ExecutionStatus::Completed
            } else {
                ExecutionStatus::Aborted
            }
        );
        let facts = fact_log::read_tree_records(&fixture.store, &id).unwrap();
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
                .filter(|record| matches!(record.fact, NodeFact::AbortRequested))
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
        .unwrap()
        .is_empty());
}
