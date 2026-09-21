//! Gateway-owned workflow execution metadata registry.
//!
//! 役割:
//! - active な execution を `execution_id` キーの in-memory map で管理し、worktree_path → execution_id の
//!   secondary index を提供する。
//! - production は SQLite projection/obligation を authority とする。
//! - 状態遷移ロジックは持たず、driver からの「開始通知」「終了通知」を受けて反映するのみ。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tokio::sync::Mutex;

pub use crate::domain::workflow::WorkflowExecutionSummary as WorkflowExecutionMetadata;
pub(crate) use crate::domain::workflow::{ExecutionOrigin, ExecutionStatus};
use crate::domain::workflow::{TokenUsage, WorkflowExecution as DomainWorkflowExecution};

/// Workflow 実行の Abort に伴う、同一 execution / worktree の直列化のための予約。
/// Abort の event commit 前に取得し、runtime cleanup 完了または Abort 不成立時に解放する。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActiveInterruptionReservation {
    pub(crate) execution_id: String,
    pub(crate) worktree_path: String,
}

/// Execution Store の入力境界で `execution_id` を UUID として検証する。
fn is_valid_execution_id(execution_id: &str) -> bool {
    uuid::Uuid::parse_str(execution_id).is_ok()
}

/// Execution Store の in-memory state。`active` と `by_worktree` を単一 Mutex で保護することで、
/// 重複チェックと挿入を原子的に行う（Spec Rule: 同一 worktree への並行登録不整合を防ぐ）。
struct ExecutionStoreInner {
    active: HashMap<String, WorkflowExecutionMetadata>,
    by_worktree: HashMap<String, String>,
    pending_interrupted_transitions: HashSet<String>,
    pending_resume_worktrees: HashMap<String, String>,
}

impl ExecutionStoreInner {
    fn new() -> Self {
        Self {
            active: HashMap::new(),
            by_worktree: HashMap::new(),
            pending_interrupted_transitions: HashSet::new(),
            pending_resume_worktrees: HashMap::new(),
        }
    }

    /// `execution_id` をキーに `active` / `by_worktree` の両方から削除する補助関数。
    /// `by_worktree` の entry は `worktree_path` から逆引きするため、active から
    /// 取り出した `worktree_path` のみを対象に削除する。
    fn remove_execution(&mut self, execution_id: &str) -> Option<WorkflowExecutionMetadata> {
        let removed = self.active.remove(execution_id)?;
        if self
            .by_worktree
            .get(&removed.worktree_path)
            .is_some_and(|id| id == execution_id)
        {
            self.by_worktree.remove(&removed.worktree_path);
        }
        Some(removed)
    }
}

/// active な execution の登録・予約と、単一 execution の取得を提供する。
///
/// active summary と worktree の secondary index を in-memory に保持する。
/// active に存在しない execution は SQLite の query service から取得する。
pub struct ExecutionStore {
    inner: Mutex<ExecutionStoreInner>,
    canonical_query: Option<Arc<dyn crate::usecase::workspace_tree::WorkspaceQueryService>>,
}

impl ExecutionStore {
    pub(crate) fn new_canonical(
        canonical_query: Arc<dyn crate::usecase::workspace_tree::WorkspaceQueryService>,
    ) -> Self {
        Self {
            inner: Mutex::new(ExecutionStoreInner::new()),
            canonical_query: Some(canonical_query),
        }
    }

    #[cfg(test)]
    pub fn new_in_memory_for_tests() -> Self {
        Self {
            inner: Mutex::new(ExecutionStoreInner::new()),
            canonical_query: None,
        }
    }

    /// 新規 execution を active として登録する。同じ worktree の競合と execution ID の付け替えを拒否する。
    pub async fn register_active_execution(
        &self,
        execution: WorkflowExecutionMetadata,
    ) -> Result<(), ExecutionStoreError> {
        if !is_valid_execution_id(&execution.execution_id) {
            return Err(ExecutionStoreError::InvalidExecutionId {
                execution_id: execution.execution_id.clone(),
            });
        }
        // Spec issues-1011 finding 11: terminal status の active 登録を型レベル相当の
        // runtime guard で禁止する。active 集合の不変条件（is_active な execution のみが
        // active に存在する）を API 境界で強制し、`update_active` の typed invariant と
        // 整合させる。
        if !execution.status.is_active() {
            return Err(ExecutionStoreError::NonActiveStatusInActiveSet {
                execution_id: execution.execution_id.clone(),
                status: execution.status,
            });
        }
        {
            let mut inner = self.inner.lock().await;
            if inner
                .pending_interrupted_transitions
                .contains(&execution.execution_id)
            {
                return Err(ExecutionStoreError::TransitionInProgress {
                    execution_id: execution.execution_id.clone(),
                });
            }
            // 同一 execution_id が別 worktree で既に登録されている場合は不整合（古い by_worktree が
            // 孤立する原因）なので拒否する。
            if let Some(existing) = inner.active.get(&execution.execution_id) {
                if existing.worktree_path != execution.worktree_path {
                    return Err(ExecutionStoreError::ExecutionIdWorktreeMismatch {
                        execution_id: execution.execution_id.clone(),
                        existing_worktree_path: existing.worktree_path.clone(),
                        new_worktree_path: execution.worktree_path.clone(),
                    });
                }
            }
            // 同一 worktree に別 execution_id の active execution があれば拒否する。
            if let Some(existing_execution_id) = inner.by_worktree.get(&execution.worktree_path) {
                if existing_execution_id != &execution.execution_id {
                    return Err(ExecutionStoreError::WorktreeAlreadyActive {
                        worktree_path: execution.worktree_path.clone(),
                        existing_execution_id: existing_execution_id.clone(),
                    });
                }
            }
            if let Some(existing_execution_id) =
                inner.pending_resume_worktrees.get(&execution.worktree_path)
            {
                if existing_execution_id != &execution.execution_id {
                    return Err(ExecutionStoreError::WorktreeAlreadyActive {
                        worktree_path: execution.worktree_path.clone(),
                        existing_execution_id: existing_execution_id.clone(),
                    });
                }
            }
            inner.by_worktree.insert(
                execution.worktree_path.clone(),
                execution.execution_id.clone(),
            );
            inner
                .active
                .insert(execution.execution_id.clone(), execution.clone());
        }
        Ok(())
    }

    /// command rollback 専用: mutation 前の active snapshot を戻す。
    pub(crate) async fn restore_active_snapshot_for_rollback(
        &self,
        execution: WorkflowExecutionMetadata,
    ) -> Result<(), ExecutionStoreError> {
        self.register_active_execution(execution).await
    }

    /// active execution の属性を更新し、identity と active status を維持する。
    async fn update_active<F>(
        &self,
        execution_id: &str,
        mutator: F,
    ) -> Result<(), ExecutionStoreError>
    where
        F: FnOnce(&mut WorkflowExecutionMetadata),
    {
        if !is_valid_execution_id(execution_id) {
            return Err(ExecutionStoreError::InvalidExecutionId {
                execution_id: execution_id.to_string(),
            });
        }
        {
            let mut inner = self.inner.lock().await;
            let Some(execution) = inner.active.get_mut(execution_id) else {
                // 対象が存在しない場合は no-op（呼出元の状態遷移後 race を許容する）。
                return Ok(());
            };
            let previous = execution.clone();
            mutator(execution);
            // Spec issues-1011 finding 10: typed invariant guard。
            // 呼出側が execution_id / worktree_path / terminal status を変更しないことを mutation 後に
            // 必ず再検証する。違反時は in-memory state を rollback する。
            if execution.execution_id != previous.execution_id {
                *execution = previous.clone();
                return Err(ExecutionStoreError::ImmutableFieldChanged {
                    execution_id: previous.execution_id,
                    field: "execution_id".to_string(),
                });
            }
            if execution.worktree_path != previous.worktree_path {
                *execution = previous.clone();
                return Err(ExecutionStoreError::ImmutableFieldChanged {
                    execution_id: previous.execution_id,
                    field: "worktree_path".to_string(),
                });
            }
            if !execution.status.is_active() {
                *execution = previous.clone();
                return Err(ExecutionStoreError::NonActiveNotAllowedInUpdate {
                    execution_id: previous.execution_id,
                });
            }
        };
        Ok(())
    }

    /// driver の active snapshot から token usage を含む read projection を同期する。
    pub async fn sync_active_projection_with_usage(
        &self,
        execution_id: &str,
        status: ExecutionStatus,
        current_node: Option<String>,
        updated_at: f64,
        total_token_usage: Option<TokenUsage>,
    ) -> Result<(), ExecutionStoreError> {
        self.update_active(execution_id, |execution| {
            execution.status = status;
            execution.current_node = current_node;
            execution.updated_at = updated_at;
            if let Some(total_token_usage) = total_token_usage {
                execution.total_token_usage = total_token_usage;
            }
        })
        .await
    }

    /// active execution の現在値を rollback 用 snapshot として取得する。
    pub async fn active_execution_snapshot(
        &self,
        execution_id: &str,
    ) -> Option<WorkflowExecutionMetadata> {
        let inner = self.inner.lock().await;
        inner.active.get(execution_id).cloned()
    }

    /// Active interruption の event append から process/session cleanup 完了まで、同じ
    /// execution command と worktree reservation を直列化する。
    pub async fn reserve_active_interruption(
        &self,
        execution_id: &str,
    ) -> Result<ActiveInterruptionReservation, ExecutionStoreError> {
        if !is_valid_execution_id(execution_id) {
            return Err(ExecutionStoreError::InvalidExecutionId {
                execution_id: execution_id.to_string(),
            });
        }
        let mut inner = self.inner.lock().await;
        let active = inner.active.get(execution_id).cloned().ok_or_else(|| {
            ExecutionStoreError::ExecutionNotFound {
                execution_id: execution_id.to_string(),
            }
        })?;
        if !active.status.is_active() {
            return Err(ExecutionStoreError::InvalidStatusTransition {
                execution_id: execution_id.to_string(),
                actual: active.status,
                expected: "running",
            });
        }
        if !inner
            .pending_interrupted_transitions
            .insert(execution_id.to_string())
        {
            return Err(ExecutionStoreError::TransitionInProgress {
                execution_id: execution_id.to_string(),
            });
        }
        if let Some(owner) = inner
            .pending_resume_worktrees
            .get(&active.worktree_path)
            .cloned()
        {
            if owner != execution_id {
                inner.pending_interrupted_transitions.remove(execution_id);
                return Err(ExecutionStoreError::WorktreeAlreadyActive {
                    worktree_path: active.worktree_path,
                    existing_execution_id: owner,
                });
            }
        }
        inner
            .pending_resume_worktrees
            .insert(active.worktree_path.clone(), execution_id.to_string());
        Ok(ActiveInterruptionReservation {
            execution_id: execution_id.to_string(),
            worktree_path: active.worktree_path,
        })
    }

    pub async fn finish_active_interruption(
        &self,
        reservation: ActiveInterruptionReservation,
    ) -> Result<(), ExecutionStoreError> {
        let mut inner = self.inner.lock().await;
        let execution_reserved = inner
            .pending_interrupted_transitions
            .remove(&reservation.execution_id);
        let worktree_reserved = inner
            .pending_resume_worktrees
            .get(&reservation.worktree_path)
            .is_some_and(|owner| owner == &reservation.execution_id);
        if worktree_reserved {
            inner
                .pending_resume_worktrees
                .remove(&reservation.worktree_path);
        }
        if !execution_reserved || !worktree_reserved {
            return Err(ExecutionStoreError::InterruptionReservationChanged {
                execution_id: reservation.execution_id,
            });
        }
        Ok(())
    }

    pub async fn interrupted_transition_pending(&self, execution_id: &str) -> bool {
        self.inner
            .lock()
            .await
            .pending_interrupted_transitions
            .contains(execution_id)
    }

    /// 起動時に実行中の metadata を event-log projection に揃える。
    pub async fn reconcile_orphan_from_projection(
        &self,
        mut metadata: WorkflowExecutionMetadata,
        projection: &DomainWorkflowExecution,
    ) -> Result<WorkflowExecutionMetadata, ExecutionStoreError> {
        if !is_valid_execution_id(&metadata.execution_id) {
            return Err(ExecutionStoreError::InvalidExecutionId {
                execution_id: metadata.execution_id,
            });
        }
        if metadata.status.is_finished() {
            return Err(ExecutionStoreError::InvalidStatusTransition {
                execution_id: metadata.execution_id.clone(),
                actual: metadata.status,
                expected: "running",
            });
        }
        if projection.id != metadata.execution_id {
            return Err(ExecutionStoreError::ImmutableFieldChanged {
                execution_id: metadata.execution_id,
                field: "execution_id".to_string(),
            });
        }
        let projected_worktree_path =
            crate::domain::workspace_tree::WorkspaceIdentity::new(&projection.worktree_path)
                .as_str()
                .to_string();
        if projected_worktree_path != metadata.worktree_path {
            return Err(ExecutionStoreError::ExecutionIdWorktreeMismatch {
                execution_id: metadata.execution_id,
                existing_worktree_path: metadata.worktree_path,
                new_worktree_path: projected_worktree_path,
            });
        }
        metadata.workflow_name = projection.workflow_name.clone();
        metadata.status = projection.status;
        metadata.current_node = projection.current_node.clone();
        metadata.created_from = projection.created_from;
        metadata.started_at = projection.started_at;
        metadata.updated_at = projection.updated_at;
        metadata.completed_at = projection.completed_at;
        metadata.error_reason = projection.error_reason.clone();
        metadata.total_token_usage = projection.total_token_usage.clone();
        Ok(metadata)
    }

    /// active set から該当 execution を取り除き、reservation 状態を完全に撤回する。
    pub async fn cancel_reservation(&self, execution_id: &str) -> Result<(), ExecutionStoreError> {
        if !is_valid_execution_id(execution_id) {
            return Err(ExecutionStoreError::InvalidExecutionId {
                execution_id: execution_id.to_string(),
            });
        }
        let mut inner = self.inner.lock().await;
        inner.remove_execution(execution_id);
        Ok(())
    }

    pub(crate) async fn get_execution_record(
        &self,
        execution_id: &str,
    ) -> Result<Option<WorkflowExecutionMetadata>, ExecutionStoreError> {
        if !is_valid_execution_id(execution_id) {
            return Err(ExecutionStoreError::InvalidExecutionId {
                execution_id: execution_id.to_string(),
            });
        }
        {
            let inner = self.inner.lock().await;
            if let Some(execution) = inner.active.get(execution_id) {
                return Ok(Some(execution.clone()));
            }
        }
        if let Some(query) = &self.canonical_query {
            let query = Arc::clone(query);
            let execution_id = execution_id.to_string();
            return tokio::task::spawn_blocking(move || query.execution_summary(&execution_id))
                .await
                .map_err(|error| ExecutionStoreError::AuthorityReadFailed {
                    reason: error.to_string(),
                })?
                .map_err(|error| ExecutionStoreError::AuthorityReadFailed {
                    reason: error.to_string(),
                });
        }
        Ok(None)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExecutionStoreError {
    #[error("worktree {worktree_path} already has active execution {existing_execution_id}")]
    WorktreeAlreadyActive {
        worktree_path: String,
        existing_execution_id: String,
    },
    #[error(
        "execution_id {execution_id} is already active on worktree {existing_worktree_path} \
         and cannot be re-registered to {new_worktree_path}"
    )]
    ExecutionIdWorktreeMismatch {
        execution_id: String,
        existing_worktree_path: String,
        new_worktree_path: String,
    },
    #[error("invalid execution_id format (must be UUID): {execution_id}")]
    InvalidExecutionId { execution_id: String },
    #[error(
        "cannot register execution {execution_id} into active set with non-active status: {status:?}"
    )]
    NonActiveStatusInActiveSet {
        execution_id: String,
        status: ExecutionStatus,
    },
    #[error("update_active for {execution_id} attempted to change immutable field {field}")]
    ImmutableFieldChanged { execution_id: String, field: String },
    #[error("update_active for {execution_id} cannot transition to a non-active status")]
    NonActiveNotAllowedInUpdate { execution_id: String },
    #[error("canonical workflow authority read failed: {reason}")]
    AuthorityReadFailed { reason: String },
    #[error("workflow execution was not found: {execution_id}")]
    ExecutionNotFound { execution_id: String },
    #[error(
        "execution {execution_id} cannot transition from {actual:?}; expected one of {expected}"
    )]
    InvalidStatusTransition {
        execution_id: String,
        actual: ExecutionStatus,
        expected: &'static str,
    },
    #[error("execution {execution_id} already has an interrupted transition in progress")]
    TransitionInProgress { execution_id: String },
    #[error("active interruption reservation for execution {execution_id} changed")]
    InterruptionReservationChanged { execution_id: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// テスト内で使う安定 UUID。register_active_execution/update_active/cancel_reservation の API 境界で
    /// UUID 形式検証が走るため、テスト識別子は UUID にする（Spec issues-1011 finding 4）。
    fn test_uuid(seed: u8) -> String {
        let bytes = [seed; 16];
        uuid::Uuid::from_bytes(bytes).to_string()
    }

    fn make_execution(
        execution_id: &str,
        worktree: &str,
        status: ExecutionStatus,
        started_at: f64,
    ) -> WorkflowExecutionMetadata {
        WorkflowExecutionMetadata {
            execution_id: execution_id.to_string(),
            workflow_name: "wf".to_string(),
            status,
            worktree_path: worktree.to_string(),
            current_node: Some("node-1".to_string()),
            created_from: ExecutionOrigin::DesktopUi,
            started_at,
            updated_at: started_at,
            completed_at: None,
            error_reason: None,
            total_token_usage: TokenUsage::default(),
        }
    }

    /// Rule: 同一 worktree に進行中の実行が存在する間は、新たな workflow 起動は拒否される
    #[tokio::test]
    async fn second_active_execution_on_same_worktree_is_rejected() {
        let store = ExecutionStore::new_in_memory_for_tests();
        let execution_id_1 = test_uuid(1);
        let execution_id_2 = test_uuid(2);
        store
            .register_active_execution(make_execution(
                &execution_id_1,
                "/wt/a",
                ExecutionStatus::Running,
                100.0,
            ))
            .await
            .unwrap();
        let err = store
            .register_active_execution(make_execution(
                &execution_id_2,
                "/wt/a",
                ExecutionStatus::Running,
                101.0,
            ))
            .await
            .unwrap_err();
        match err {
            ExecutionStoreError::WorktreeAlreadyActive {
                existing_execution_id,
                ..
            } => {
                assert_eq!(existing_execution_id, execution_id_1);
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
        // 既存 active が継続している
        assert_eq!(store.inner.lock().await.active.len(), 1);
        assert_eq!(
            store.inner.lock().await.by_worktree.get("/wt/a").cloned(),
            Some(execution_id_1)
        );
    }

    /// Spec issues-1011 finding 6: 同一 execution_id を別 worktree_path で再登録しようとすると
    /// `ExecutionIdWorktreeMismatch` で拒否される。古い by_worktree index が孤立しない。
    #[tokio::test]
    async fn register_active_rejects_same_execution_id_with_different_worktree() {
        let store = ExecutionStore::new_in_memory_for_tests();
        let execution_id = test_uuid(1);
        store
            .register_active_execution(make_execution(
                &execution_id,
                "/wt/a",
                ExecutionStatus::Running,
                100.0,
            ))
            .await
            .unwrap();
        let err = store
            .register_active_execution(make_execution(
                &execution_id,
                "/wt/b",
                ExecutionStatus::Running,
                101.0,
            ))
            .await
            .unwrap_err();
        match err {
            ExecutionStoreError::ExecutionIdWorktreeMismatch {
                existing_worktree_path,
                new_worktree_path,
                ..
            } => {
                assert_eq!(existing_worktree_path, "/wt/a");
                assert_eq!(new_worktree_path, "/wt/b");
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
        // 古い by_worktree index は維持されている（/wt/b への孤立 entry は出ない）
        assert_eq!(
            store.inner.lock().await.by_worktree.get("/wt/a").cloned(),
            Some(execution_id.clone())
        );
        assert_eq!(
            store.inner.lock().await.by_worktree.get("/wt/b").cloned(),
            None
        );
        assert_eq!(store.inner.lock().await.active.len(), 1);
    }

    /// Rule: 既に進行している worktree 上の実行は、新たな識別子を採番せずそのまま実行インスタンスとして扱われる
    ///
    /// Execution Store は採番しないことを確認する。同じ execution_id で再登録すれば（driver 側で
    /// `execution_id` を昇格させる経路に相当）通る。
    #[tokio::test]
    async fn register_with_same_execution_id_for_same_worktree_is_idempotent() {
        let store = ExecutionStore::new_in_memory_for_tests();
        let execution = make_execution(&test_uuid(1), "/wt/a", ExecutionStatus::Running, 100.0);
        store
            .register_active_execution(execution.clone())
            .await
            .unwrap();
        // 同一 execution_id の再登録は許容（idempotent）
        store.register_active_execution(execution).await.unwrap();
        assert_eq!(store.inner.lock().await.active.len(), 1);
    }

    /// Spec issues-1011: 同一 worktree への並行 `register_active_execution` で active / by_worktree が
    /// 整合する。Mutex で重複チェックと挿入を 1 critical section に閉じているので、
    /// レース後の状態は「ちょうど 1 つ active」かつ「by_worktree の entry が 1 つ」になる。
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_register_active_on_same_worktree_keeps_active_and_by_worktree_consistent() {
        let store = std::sync::Arc::new(ExecutionStore::new_in_memory_for_tests());
        let mut handles = Vec::new();
        // 8 並列で同一 worktree に異なる execution_id (UUID) で register_active_execution を試みる。
        for i in 0..8 {
            let store_cloned = std::sync::Arc::clone(&store);
            handles.push(tokio::spawn(async move {
                let execution = make_execution(
                    &test_uuid(i),
                    "/wt/race",
                    ExecutionStatus::Running,
                    100.0 + i as f64,
                );
                store_cloned.register_active_execution(execution).await
            }));
        }
        let mut ok_count = 0usize;
        let mut conflict_count = 0usize;
        for h in handles {
            match h.await.unwrap() {
                Ok(()) => ok_count += 1,
                Err(ExecutionStoreError::WorktreeAlreadyActive { .. }) => conflict_count += 1,
                Err(other) => panic!("unexpected error: {other:?}"),
            }
        }
        assert_eq!(
            ok_count, 1,
            "exactly one register_active_execution must succeed under concurrent contention"
        );
        assert_eq!(conflict_count, 7);
        // 結果状態: active は 1 つだけ、by_worktree も 1 entry のみ。
        assert_eq!(store.inner.lock().await.active.len(), 1);
        let resolved = store
            .inner
            .lock()
            .await
            .by_worktree
            .get("/wt/race")
            .cloned();
        assert!(resolved.is_some());
    }

    /// Spec issues-1011 finding 4: ExecutionStore API 境界で execution_id UUID 検証が走る。
    /// 非 UUID 形式の execution_id は register_active_execution / update_active / cancel_reservation で
    /// `InvalidExecutionId` として拒否される（command 層への漏れを防ぐ二重防御）。
    #[tokio::test]
    async fn execution_store_api_boundary_rejects_non_uuid_execution_id() {
        let store = ExecutionStore::new_in_memory_for_tests();

        // register_active_execution は非 UUID を拒否する
        let bad = make_execution("not-a-uuid", "/wt/x", ExecutionStatus::Running, 100.0);
        assert!(matches!(
            store.register_active_execution(bad).await,
            Err(ExecutionStoreError::InvalidExecutionId { .. })
        ));
        assert_eq!(store.inner.lock().await.active.len(), 0);

        // update_active も非 UUID を拒否する
        assert!(matches!(
            store.update_active("../etc/passwd", |_| {}).await,
            Err(ExecutionStoreError::InvalidExecutionId { .. })
        ));

        // cancel_reservation も非 UUID を拒否する
        assert!(matches!(
            store.cancel_reservation("not-a-uuid").await,
            Err(ExecutionStoreError::InvalidExecutionId { .. })
        ));
    }

    /// active reservation は Running だけを受け付ける。
    #[tokio::test]
    async fn register_active_rejects_non_active_status() {
        let store = ExecutionStore::new_in_memory_for_tests();
        for terminal in [ExecutionStatus::Completed, ExecutionStatus::Aborted] {
            let execution = make_execution(&test_uuid(1), "/wt/a", terminal, 100.0);
            let err = store
                .register_active_execution(execution)
                .await
                .unwrap_err();
            assert!(
                matches!(err, ExecutionStoreError::NonActiveStatusInActiveSet { status, .. } if status == terminal),
                "non-active status must be rejected, got: {err:?}"
            );
        }
        assert_eq!(store.inner.lock().await.active.len(), 0);
    }

    /// Spec issues-1011 finding 10: `update_active` は execution_id を変更しようとした場合に拒否する。
    /// 違反時は in-memory state を rollback する。
    #[tokio::test]
    async fn update_active_rejects_execution_id_mutation_and_rolls_back() {
        let store = ExecutionStore::new_in_memory_for_tests();
        let execution_id = test_uuid(1);
        store
            .register_active_execution(make_execution(
                &execution_id,
                "/wt/a",
                ExecutionStatus::Running,
                100.0,
            ))
            .await
            .unwrap();
        let result = store
            .update_active(&execution_id, |r| {
                r.execution_id = test_uuid(2);
            })
            .await;
        assert!(matches!(
            result,
            Err(ExecutionStoreError::ImmutableFieldChanged { ref field, .. }) if field == "execution_id"
        ));
        // rollback: 元の execution_id のままで active 維持
        let active = store
            .inner
            .lock()
            .await
            .active
            .values()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].execution_id, execution_id);
    }

    /// Spec issues-1011 finding 10: `update_active` は worktree_path 変更も拒否する。
    #[tokio::test]
    async fn update_active_rejects_worktree_path_mutation_and_rolls_back() {
        let store = ExecutionStore::new_in_memory_for_tests();
        let execution_id = test_uuid(1);
        store
            .register_active_execution(make_execution(
                &execution_id,
                "/wt/a",
                ExecutionStatus::Running,
                100.0,
            ))
            .await
            .unwrap();
        let result = store
            .update_active(&execution_id, |r| {
                r.worktree_path = "/wt/b".to_string();
            })
            .await;
        assert!(matches!(
            result,
            Err(ExecutionStoreError::ImmutableFieldChanged { ref field, .. }) if field == "worktree_path"
        ));
        // by_worktree index は元の path を保持
        assert_eq!(
            store.inner.lock().await.by_worktree.get("/wt/a").cloned(),
            Some(execution_id.clone())
        );
        assert_eq!(
            store.inner.lock().await.by_worktree.get("/wt/b").cloned(),
            None
        );
    }

    /// `update_active` は非 active 状態への変更を拒否し、変更前の active 状態に戻す。
    #[tokio::test]
    async fn update_active_rejects_terminal_transition_and_rolls_back() {
        let store = ExecutionStore::new_in_memory_for_tests();
        let execution_id = test_uuid(1);
        store
            .register_active_execution(make_execution(
                &execution_id,
                "/wt/a",
                ExecutionStatus::Running,
                100.0,
            ))
            .await
            .unwrap();
        for terminal in [ExecutionStatus::Completed, ExecutionStatus::Aborted] {
            let result = store
                .update_active(&execution_id, |r| {
                    r.status = terminal;
                })
                .await;
            assert!(
                matches!(result, Err(ExecutionStoreError::NonActiveNotAllowedInUpdate { .. })),
                "non-active transition via update_active must be rejected for {terminal:?}, got: {result:?}"
            );
            // active のまま
            let active = store
                .inner
                .lock()
                .await
                .active
                .values()
                .cloned()
                .collect::<Vec<_>>();
            assert_eq!(active.len(), 1);
            assert_eq!(active[0].status, ExecutionStatus::Running);
        }
    }
}

#[cfg(test)]
#[path = "execution_store_test.rs"]
mod execution_store_tests;
