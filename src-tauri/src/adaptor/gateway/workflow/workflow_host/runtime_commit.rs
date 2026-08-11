//! Workflow runtime transaction preparation and rollback.

use std::sync::Arc;

use crate::adaptor::gateway::workflow::execution_store::{
    ExecutionStore, TerminalExecutionStatus, WorkflowExecutionMetadata,
};
use crate::adaptor::gateway::workflow::workflow_host::execution_state::DomainWorkflowExecution;
use crate::domain::workflow::ExecutionStatus;
use crate::domain::workflow::RuntimeExecutionState;
use crate::domain::workflow::WorkflowEvent;
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

pub(crate) struct RequiredEventCommit<'a> {
    pub(crate) operation_kind: crate::domain::local_event::CommitOperationKind,
    pub(crate) execution_id: &'a str,
    pub(crate) snapshot_for_commit: &'a RuntimeCommitSnapshot,
    pub(crate) snapshot_before: DomainWorkflowExecution,
    pub(crate) execution_store_snapshot_before: Option<WorkflowExecutionMetadata>,
    pub(crate) required_events: Vec<WorkflowEvent>,
    pub(crate) append_error_context: &'a str,
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
        RuntimeExecutionState::Running => {
            let current_node = snapshot.current_node_name.clone();
            execution_store
                .sync_active_projection_with_usage(
                    execution_id,
                    ExecutionStatus::Running,
                    Some(current_node),
                    now,
                    Some(total_token_usage),
                )
                .await
        }
        #[cfg(test)]
        RuntimeExecutionState::WaitingApproval => {
            execution_store
                .sync_active_projection_with_usage(
                    execution_id,
                    ExecutionStatus::WaitingApproval,
                    Some(snapshot.current_node_name.clone()),
                    now,
                    Some(total_token_usage),
                )
                .await
        }
        #[cfg(test)]
        RuntimeExecutionState::Interrupted => Err(
            crate::adaptor::gateway::workflow::execution_store::ExecutionStoreError::InvalidStatusTransition {
                execution_id: execution_id.to_string(),
                actual: ExecutionStatus::Interrupted,
                expected: "running|completed|aborted",
            },
        ),
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
