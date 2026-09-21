use super::*;
use crate::adaptor::gateway::workflow::workflow_host::runtime_commit::sync_execution_store_from_snapshot;
use crate::domain::workflow::entities::workflow_execution::{
    WorkflowExecution as Aggregate, WorkflowExecutionRestore,
};
use crate::domain::workflow::RuntimeExecutionState;
use crate::usecase::workflow::runtime_snapshot::RuntimeCommitSnapshot;
use crate::usecase::workspace_tree::TestWorkspaceQueryService;

#[tokio::test]
async fn test_workflow実行ストア_共通summaryの全項目を保ちactiveを優先する() {
    // Given
    let summary = crate::domain::workflow::WorkflowExecutionSummary {
        execution_id: uuid::Uuid::new_v4().to_string(),
        workflow_name: "review".into(),
        status: ExecutionStatus::Aborted,
        worktree_path: "/repo".into(),
        current_node: Some("review".into()),
        created_from: ExecutionOrigin::Cli,
        started_at: 1.0,
        updated_at: 3.0,
        completed_at: Some(3.0),
        error_reason: Some("aborted".into()),
        total_token_usage: TokenUsage {
            input_tokens: 13,
            output_tokens: 8,
        },
    };
    let store =
        ExecutionStore::new_canonical(TestWorkspaceQueryService::new(vec![summary.clone()]));
    let active = crate::domain::workflow::WorkflowExecutionSummary {
        status: ExecutionStatus::Running,
        current_node: Some("implement".into()),
        updated_at: 2.0,
        completed_at: None,
        error_reason: None,
        ..summary.clone()
    };

    // When / Then
    assert_eq!(
        store
            .get_execution_record(&summary.execution_id)
            .await
            .unwrap(),
        Some(summary.clone())
    );
    store
        .register_active_execution(active.clone())
        .await
        .unwrap();
    assert_eq!(
        store
            .get_execution_record(&summary.execution_id)
            .await
            .unwrap(),
        Some(active)
    );
    store
        .cancel_reservation(&summary.execution_id)
        .await
        .unwrap();
    assert_eq!(
        store
            .get_execution_record(&summary.execution_id)
            .await
            .unwrap(),
        Some(summary)
    );
}

#[test]
fn test_workflow実行ストア_旧ファイル永続化経路を持たない() {
    // Given
    let source = include_str!("execution_store.rs");

    // When / Then: 本番では無効なテスト専用の永続化処理の復活も検出する。
    assert!(!source.contains("workflow_executions"));
}

#[tokio::test]
async fn test_workflow実行ストア_rollbackも登録時と同じ制約で拒否し既存予約を保つ() {
    // Given
    let store = ExecutionStore::new_in_memory_for_tests();
    let execution = WorkflowExecutionMetadata {
        execution_id: uuid::Uuid::new_v4().to_string(),
        workflow_name: "review".into(),
        status: ExecutionStatus::Running,
        worktree_path: "/repo".into(),
        current_node: None,
        created_from: ExecutionOrigin::Cli,
        started_at: 1.0,
        updated_at: 1.0,
        completed_at: None,
        error_reason: None,
        total_token_usage: TokenUsage::default(),
    };
    store
        .register_active_execution(execution.clone())
        .await
        .unwrap();
    let invalid = WorkflowExecutionMetadata {
        execution_id: "not-a-uuid".into(),
        ..execution.clone()
    };

    // When / Then
    assert!(matches!(
        store.restore_active_snapshot_for_rollback(invalid).await,
        Err(ExecutionStoreError::InvalidExecutionId { .. })
    ));
    for status in [ExecutionStatus::Completed, ExecutionStatus::Aborted] {
        assert!(matches!(
            store
                .restore_active_snapshot_for_rollback(WorkflowExecutionMetadata {
                    status,
                    ..execution.clone()
                })
                .await,
            Err(ExecutionStoreError::NonActiveStatusInActiveSet { .. })
        ));
    }
    assert!(matches!(
        store
            .restore_active_snapshot_for_rollback(WorkflowExecutionMetadata {
                worktree_path: "/other".into(),
                ..execution.clone()
            })
            .await,
        Err(ExecutionStoreError::ExecutionIdWorktreeMismatch { .. })
    ));
    let competing = WorkflowExecutionMetadata {
        execution_id: uuid::Uuid::new_v4().to_string(),
        ..execution.clone()
    };
    assert!(matches!(
        store
            .restore_active_snapshot_for_rollback(competing.clone())
            .await,
        Err(ExecutionStoreError::WorktreeAlreadyActive { .. })
    ));
    let reservation = store
        .reserve_active_interruption(&execution.execution_id)
        .await
        .unwrap();
    assert!(matches!(
        store
            .restore_active_snapshot_for_rollback(execution.clone())
            .await,
        Err(ExecutionStoreError::TransitionInProgress { .. })
    ));
    assert_eq!(
        store
            .active_execution_snapshot(&execution.execution_id)
            .await,
        Some(execution.clone())
    );
    store
        .cancel_reservation(&execution.execution_id)
        .await
        .unwrap();
    assert!(matches!(
        store.restore_active_snapshot_for_rollback(competing).await,
        Err(ExecutionStoreError::WorktreeAlreadyActive { .. })
    ));
    store.finish_active_interruption(reservation).await.unwrap();
    store
        .restore_active_snapshot_for_rollback(execution.clone())
        .await
        .unwrap();
    assert_eq!(
        store
            .active_execution_snapshot(&execution.execution_id)
            .await,
        Some(execution.clone())
    );
    assert_eq!(
        store.inner.lock().await.by_worktree.get("/repo"),
        Some(&execution.execution_id)
    );
}

#[tokio::test]
async fn test_workflow実行ストア_完了とabortで予約を解放し遅延更新でも復活しない() {
    // Given / When / Then
    for state in [
        RuntimeExecutionState::Completed,
        RuntimeExecutionState::Aborted,
    ] {
        let store = Arc::new(ExecutionStore::new_in_memory_for_tests());
        let execution_id = uuid::Uuid::new_v4().to_string();
        let aggregate = Aggregate::restore_runtime(WorkflowExecutionRestore {
            id: execution_id.clone(),
            worktree_path: "/repo".into(),
            ..Default::default()
        });
        let snapshot = RuntimeCommitSnapshot::from_execution(&aggregate).unwrap();
        let metadata = WorkflowExecutionMetadata {
            execution_id: execution_id.clone(),
            workflow_name: "review".into(),
            status: ExecutionStatus::Running,
            worktree_path: "/repo".into(),
            current_node: None,
            created_from: ExecutionOrigin::Cli,
            started_at: 1.0,
            updated_at: 1.0,
            completed_at: None,
            error_reason: None,
            total_token_usage: TokenUsage::default(),
        };
        store
            .register_active_execution(metadata.clone())
            .await
            .unwrap();
        store
            .sync_active_projection_with_usage(
                &execution_id,
                ExecutionStatus::Running,
                Some("review".into()),
                2.0,
                Some(TokenUsage {
                    input_tokens: 13,
                    output_tokens: 8,
                }),
            )
            .await
            .unwrap();
        let active = store
            .active_execution_snapshot(&execution_id)
            .await
            .unwrap();
        assert_eq!(active.current_node.as_deref(), Some("review"));
        assert_eq!(active.updated_at, 2.0);
        assert_eq!(
            active.total_token_usage,
            TokenUsage {
                input_tokens: 13,
                output_tokens: 8
            }
        );
        let mut terminal = snapshot.clone();
        terminal.state = state;
        sync_execution_store_from_snapshot(&store, &execution_id, &terminal)
            .await
            .unwrap();
        sync_execution_store_from_snapshot(&store, &execution_id, &snapshot)
            .await
            .unwrap();
        assert!(store
            .active_execution_snapshot(&execution_id)
            .await
            .is_none());
        assert!(store
            .get_execution_record(&execution_id)
            .await
            .unwrap()
            .is_none());
        assert!(store.inner.lock().await.by_worktree.is_empty());
        store
            .restore_active_snapshot_for_rollback(metadata.clone())
            .await
            .unwrap();
        assert_eq!(
            store.active_execution_snapshot(&execution_id).await,
            Some(metadata)
        );
        store.cancel_reservation(&execution_id).await.unwrap();
        assert!(store.inner.lock().await.by_worktree.is_empty());
    }
}
