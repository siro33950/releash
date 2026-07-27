//! Workflow runtime transaction preparation and rollback.

#[cfg(test)]
use std::collections::HashMap;
use std::sync::Arc;

#[cfg(test)]
use tokio::sync::Mutex;

use crate::adaptor::gateway::workflow::execution_store::{
    ExecutionStore, TerminalExecutionStatus, WorkflowExecutionMetadata,
};
use crate::domain::workflow::RuntimeExecutionState;
use crate::domain::workflow::WorkflowEvent;
use crate::domain::workflow::{ExecutionInterruptionReason, ExecutionStatus};
use crate::infrastructure::runtime::workflow_host::execution_state::DomainWorkflowExecution;
use crate::usecase::workflow::runtime_error::WorkflowRuntimeError;
use crate::usecase::workflow::runtime_snapshot::RuntimeCommitSnapshot;

/// `abort_workflow_by_execution_id` 内部 lookup の typed 結果。
#[derive(Debug)]
pub(crate) enum AbortTargetLookup {
    NotFound,
    AlreadyTerminal,
    Active {
        current_node_session_id: Option<String>,
        fanout_session_ids: Option<Vec<String>>,
    },
}

/// `abort_workflow_by_execution_id` の typed outcome。
///
/// 中断要求に対し、runtime primitive が「実際に中断を実施したか」「対象 execution が
/// 存在しないか」「既に終了済みで中断不能だったか」を typed に表現する。
/// `NotFound` / `AlreadyTerminal` は非受理（`WorkflowRuntimeError` 経由）として
/// 上位に伝播する（Spec [04] Rule「対象不在 / 既に終了した command は受理されない」）。
///
/// driver 内部用途のみのため可視性は module-private に閉じる。外部入口は
/// `abort_workflow_execution*` runtime primitive に統一する（Spec [04] 公開 API 最小化）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AbortOutcome {
    /// 対象 execution を Aborted に遷移させ、ExecutionAborted event を append した。
    Aborted,
    /// 対象 execution が `executions` に存在しない。
    NotFound,
    /// 対象 execution は既に terminal で、中断対象でない。
    AlreadyTerminal,
}

pub(crate) struct CommandMutationRollback<'a> {
    pub(crate) execution_id: &'a str,
    pub(crate) snapshot_before: DomainWorkflowExecution,
    pub(crate) execution_store_snapshot_before: Option<WorkflowExecutionMetadata>,
    pub(crate) context: &'a str,
}

pub(crate) struct RequiredEventCommit<'a> {
    pub(crate) operation_kind: crate::domain::local_event::CommitOperationKind,
    pub(crate) execution_id: &'a str,
    pub(crate) snapshot_for_commit: &'a RuntimeCommitSnapshot,
    pub(crate) snapshot_before: DomainWorkflowExecution,
    pub(crate) execution_store_snapshot_before: Option<WorkflowExecutionMetadata>,
    pub(crate) required_events: Vec<WorkflowEvent>,
    pub(crate) append_error_context: &'a str,
}

/// ロック内で確定した遷移結果。ロック外で永続化・AgentSession起動を行うための情報を持つ。
pub(crate) enum NodeOutcome {
    /// 状態を永続化・ブロードキャストするだけ（終了状態遷移など）
    Persist(RuntimeCommitSnapshot),
    /// 同一ステップを policy に従って再実行する
    RetryCurrentNode {
        snapshot: RuntimeCommitSnapshot,
        completed_session_id: Option<String>,
    },
    /// 次のステップに遷移し、AgentSession を起動する
    TransitionAndStart(RuntimeCommitSnapshot),
    /// 並列ブロックに遷移し、子ステップを並列起動する
    StartFanout(RuntimeCommitSnapshot),
}

impl NodeOutcome {
    pub(crate) fn snapshot(&self) -> &RuntimeCommitSnapshot {
        match self {
            Self::Persist(snapshot)
            | Self::RetryCurrentNode { snapshot, .. }
            | Self::TransitionAndStart(snapshot)
            | Self::StartFanout(snapshot) => snapshot,
        }
    }

    pub(crate) fn snapshot_mut(&mut self) -> &mut RuntimeCommitSnapshot {
        match self {
            Self::Persist(snapshot)
            | Self::RetryCurrentNode { snapshot, .. }
            | Self::TransitionAndStart(snapshot)
            | Self::StartFanout(snapshot) => snapshot,
        }
    }

    pub(crate) fn completed_node_session_ids(&self) -> Vec<String> {
        match self {
            Self::Persist(snapshot) if matches!(snapshot.state, RuntimeExecutionState::Aborted) => {
                snapshot.current_session_id.iter().cloned().collect()
            }
            Self::Persist(snapshot)
                if matches!(
                    snapshot.state,
                    RuntimeExecutionState::Completed | RuntimeExecutionState::Failed { .. }
                ) =>
            {
                completed_node_session_ids(snapshot)
            }
            Self::Persist(_) => Vec::new(),
            Self::RetryCurrentNode {
                completed_session_id,
                ..
            } => completed_session_id.iter().cloned().collect(),
            Self::TransitionAndStart(snapshot) | Self::StartFanout(snapshot) => {
                completed_node_session_ids(snapshot)
            }
        }
    }
}

fn completed_node_session_ids(snapshot: &RuntimeCommitSnapshot) -> Vec<String> {
    let snapshot =
        crate::usecase::workflow::runtime_snapshot::runtime_commit_snapshot_to_domain_snapshot(
            snapshot.clone(),
        );
    crate::domain::workflow::services::node_session_projection::collect_completed_node_session_ids(
        &snapshot,
    )
}

pub(crate) fn terminal_node_session_ids(snapshot: &RuntimeCommitSnapshot) -> Vec<String> {
    let snapshot =
        crate::usecase::workflow::runtime_snapshot::runtime_commit_snapshot_to_domain_snapshot(
            snapshot.clone(),
        );
    crate::domain::workflow::services::node_session_projection::collect_terminal_node_session_ids(
        &snapshot,
    )
}

/// `RuntimeExecutionState` から `ExecutionStatus` への変換と Execution Store metadata 同期を 1 箇所に集約する。
pub(crate) async fn sync_execution_store_from_snapshot(
    execution_store: &Arc<ExecutionStore>,
    execution_id: &str,
    snapshot: &RuntimeCommitSnapshot,
) -> Result<(), WorkflowRuntimeError> {
    let now = snapshot.updated_at;
    let total_token_usage = crate::domain::workflow::TokenUsage {
        input_tokens: snapshot.total_token_usage.input_tokens,
        output_tokens: snapshot.total_token_usage.output_tokens,
    };
    let result = match &snapshot.state {
        RuntimeExecutionState::Completed => {
            execution_store
                .complete_execution_with_usage(
                    execution_id,
                    TerminalExecutionStatus::Completed,
                    now,
                    None,
                    Some(total_token_usage),
                )
                .await
        }
        RuntimeExecutionState::Failed { reason, .. } => {
            execution_store
                .complete_execution_with_usage(
                    execution_id,
                    TerminalExecutionStatus::Failed,
                    now,
                    Some(reason.clone()),
                    Some(total_token_usage),
                )
                .await
        }
        RuntimeExecutionState::Aborted => {
            execution_store
                .complete_execution_with_usage(
                    execution_id,
                    TerminalExecutionStatus::Aborted,
                    now,
                    None,
                    Some(total_token_usage),
                )
                .await
        }
        RuntimeExecutionState::Interrupted => {
            let reason = snapshot
                .error_reason
                .as_deref()
                .and_then(ExecutionInterruptionReason::from_reason)
                .unwrap_or(ExecutionInterruptionReason::Crash);
            execution_store
                .interrupt_execution_with_usage(
                    execution_id,
                    reason,
                    Some(snapshot.current_node_name.clone()),
                    now,
                    Some(total_token_usage),
                )
                .await
                .map(|_| ())
        }
        RuntimeExecutionState::Running | RuntimeExecutionState::WaitingApproval => {
            let status = if matches!(snapshot.state, RuntimeExecutionState::Running) {
                ExecutionStatus::Running
            } else {
                ExecutionStatus::WaitingApproval
            };
            let current_node = snapshot.current_node_name.clone();
            execution_store
                .sync_active_projection_with_usage(
                    execution_id,
                    status,
                    Some(current_node),
                    now,
                    Some(total_token_usage),
                )
                .await
        }
    };
    result.map_err(|e| {
        WorkflowRuntimeError::SessionStore(format!(
            "ExecutionStore sync failed for execution {execution_id}: {e}"
        ))
    })
}

pub(crate) async fn restore_execution_store_active_snapshot(
    execution_store: &Arc<ExecutionStore>,
    metadata_snapshot: Option<WorkflowExecutionMetadata>,
) -> Result<(), WorkflowRuntimeError> {
    let Some(metadata_snapshot) = metadata_snapshot else {
        return Ok(());
    };
    let execution_id = metadata_snapshot.execution_id.clone();
    execution_store
        .restore_active_snapshot_for_rollback(metadata_snapshot)
        .await
        .map_err(|e| {
            WorkflowRuntimeError::SessionStore(format!(
                "ExecutionStore rollback failed for execution {execution_id}: {e}"
            ))
        })
}

#[cfg(test)]
pub(crate) async fn rollback_execution_projection_after_execution_store_sync_failure(
    executions: &Mutex<HashMap<String, DomainWorkflowExecution>>,
    execution_store: &Arc<ExecutionStore>,
    execution_id: &str,
    failed_snapshot: &RuntimeCommitSnapshot,
) {
    let Ok(active) = execution_store.list_active().await else {
        return;
    };
    let active_projection = active
        .into_iter()
        .find(|execution| execution.execution_id == execution_id);
    let Some(active_projection) = active_projection else {
        return;
    };
    let rollback_state = match active_projection.status {
        ExecutionStatus::Running => RuntimeExecutionState::Running,
        ExecutionStatus::WaitingApproval => RuntimeExecutionState::WaitingApproval,
        ExecutionStatus::Completed
        | ExecutionStatus::Failed
        | ExecutionStatus::Aborted
        | ExecutionStatus::Interrupted => return,
    };
    let mut execs = executions.lock().await;
    let Some(exec) = execs.get_mut(execution_id) else {
        return;
    };
    if exec.state() != &failed_snapshot.state {
        return;
    }
    exec.restore_after_failed_commit(rollback_state, active_projection.interruption_reason);
    if let Some(current_node) = active_projection.current_node {
        if let Some(index) = exec
            .workflow
            .nodes
            .iter()
            .position(|node| node.name == current_node)
        {
            let _ = exec.set_current_node(index, active_projection.updated_at);
        }
    }
}
