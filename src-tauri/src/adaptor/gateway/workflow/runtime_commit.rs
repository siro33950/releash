use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::adaptor::gateway::workflow::engine_error::WorkflowEngineError;
use crate::adaptor::gateway::workflow::event::WorkflowEvent;
use crate::adaptor::gateway::workflow::run::{RunStatus, RunStore, TerminalRunStatus, WorkflowRun};
use crate::adaptor::gateway::workflow::runtime_state::WorkflowExecution;
use crate::adaptor::gateway::workflow::state::{WorkflowExecutionState, WorkflowState};
use crate::usecase::agent_session::status::current_timestamp;

/// `abort_workflow_by_run_id` 内部 lookup の typed 結果。
#[derive(Debug)]
pub(crate) enum AbortTargetLookup {
    NotFound,
    AlreadyTerminal,
    Active {
        current_step_session_id: Option<String>,
        parallel_session_ids: Option<Vec<String>>,
    },
}

/// `abort_workflow_by_run_id` の typed outcome。
///
/// 中断要求に対し、runtime primitive が「実際に中断を実施したか」「対象 run が
/// 存在しないか」「既に終了済みで中断不能だったか」を typed に表現する。
/// `NotFound` / `AlreadyTerminal` は非受理（`WorkflowEngineError` 経由）として
/// 上位に伝播する（Spec [04] Rule「対象不在 / 既に終了した command は受理されない」）。
///
/// engine 内部用途のみのため可視性は module-private に閉じる。外部入口は
/// `abort_workflow_run*` runtime primitive に統一する（Spec [04] 公開 API 最小化）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AbortOutcome {
    /// 対象 run を Aborted に遷移させ、RunAborted event を append した。
    Aborted,
    /// 対象 run が `executions` に存在しない。
    NotFound,
    /// 対象 run は既に terminal で、中断対象でない。
    AlreadyTerminal,
}

pub(crate) struct CommandMutationRollback<'a> {
    pub(crate) run_id: &'a str,
    pub(crate) snapshot_before: WorkflowExecution,
    pub(crate) run_store_snapshot_before: Option<WorkflowRun>,
    pub(crate) context: &'a str,
}

pub(crate) struct RequiredEventCommit<'a> {
    pub(crate) run_id: &'a str,
    pub(crate) snapshot_for_commit: &'a WorkflowState,
    pub(crate) snapshot_before: WorkflowExecution,
    pub(crate) run_store_snapshot_before: Option<WorkflowRun>,
    pub(crate) required_events: Vec<WorkflowEvent>,
    pub(crate) append_error_context: &'a str,
}

/// ロック内で確定した遷移結果。ロック外で永続化・AgentSession起動を行うための情報を持つ。
pub(crate) enum StepOutcome {
    /// 状態を永続化・ブロードキャストするだけ（終了状態遷移など）
    Persist(WorkflowState),
    /// 次のステップに遷移し、AgentSession を起動する
    TransitionAndStart(WorkflowState),
    /// collect仮想stepに遷移し、reduce処理を実行する
    ReduceAndTransition(WorkflowState),
    /// 並列ブロックに遷移し、子ステップを並列起動する
    StartParallel(WorkflowState),
}

impl StepOutcome {
    pub(crate) fn snapshot(&self) -> &WorkflowState {
        match self {
            Self::Persist(snapshot)
            | Self::TransitionAndStart(snapshot)
            | Self::ReduceAndTransition(snapshot)
            | Self::StartParallel(snapshot) => snapshot,
        }
    }

    pub(crate) fn completed_step_session_ids(&self) -> Vec<String> {
        match self {
            Self::Persist(snapshot)
                if matches!(snapshot.state, WorkflowExecutionState::Aborted) =>
            {
                snapshot.current_session_id.iter().cloned().collect()
            }
            Self::Persist(snapshot)
                if matches!(
                    snapshot.state,
                    WorkflowExecutionState::Completed | WorkflowExecutionState::Failed { .. }
                ) =>
            {
                completed_step_session_ids(snapshot)
            }
            Self::Persist(_) => Vec::new(),
            Self::TransitionAndStart(snapshot)
            | Self::ReduceAndTransition(snapshot)
            | Self::StartParallel(snapshot) => completed_step_session_ids(snapshot),
        }
    }
}

fn completed_step_session_ids(snapshot: &WorkflowState) -> Vec<String> {
    let snapshot = crate::adaptor::gateway::workflow::state::workflow_state_to_domain_snapshot(
        snapshot.clone(),
    );
    crate::domain::workflow::services::session_projection::collect_completed_step_session_ids(
        &snapshot,
    )
}

pub(crate) fn terminal_step_session_ids(snapshot: &WorkflowState) -> Vec<String> {
    let snapshot = crate::adaptor::gateway::workflow::state::workflow_state_to_domain_snapshot(
        snapshot.clone(),
    );
    crate::domain::workflow::services::session_projection::collect_terminal_step_session_ids(
        &snapshot,
    )
}

/// `WorkflowExecutionState` から `RunStatus` への変換と Run Store metadata 同期を 1 箇所に集約する。
pub(crate) async fn sync_run_store_from_snapshot(
    run_store: &Arc<RunStore>,
    run_id: &str,
    snapshot: &WorkflowState,
) -> Result<(), WorkflowEngineError> {
    let now = current_timestamp();
    let result = match &snapshot.state {
        WorkflowExecutionState::Completed => {
            run_store
                .complete_run(run_id, TerminalRunStatus::Completed, now, None)
                .await
        }
        WorkflowExecutionState::Failed { reason } => {
            run_store
                .complete_run(run_id, TerminalRunStatus::Failed, now, Some(reason.clone()))
                .await
        }
        WorkflowExecutionState::Aborted => {
            run_store
                .complete_run(run_id, TerminalRunStatus::Aborted, now, None)
                .await
        }
        WorkflowExecutionState::Running | WorkflowExecutionState::WaitingApproval => {
            let status = if matches!(snapshot.state, WorkflowExecutionState::Running) {
                RunStatus::Running
            } else {
                RunStatus::WaitingApproval
            };
            let current_node = snapshot.current_step_name.clone();
            run_store
                .sync_active_projection(run_id, status, Some(current_node), now)
                .await
        }
    };
    result.map_err(|e| {
        WorkflowEngineError::SessionStore(format!("RunStore sync failed for run {run_id}: {e}"))
    })
}

pub(crate) async fn restore_run_store_active_snapshot(
    run_store: &Arc<RunStore>,
    run_snapshot: Option<WorkflowRun>,
) -> Result<(), WorkflowEngineError> {
    let Some(run_snapshot) = run_snapshot else {
        return Ok(());
    };
    let run_id = run_snapshot.run_id.clone();
    run_store
        .restore_active_snapshot_for_rollback(run_snapshot)
        .await
        .map_err(|e| {
            WorkflowEngineError::SessionStore(format!(
                "RunStore rollback failed for run {run_id}: {e}"
            ))
        })
}

pub(crate) async fn rollback_execution_projection_after_run_store_sync_failure(
    executions: &Mutex<HashMap<String, WorkflowExecution>>,
    run_store: &Arc<RunStore>,
    run_id: &str,
    failed_snapshot: &WorkflowState,
) {
    let active_projection = run_store
        .list_active()
        .await
        .into_iter()
        .find(|run| run.run_id == run_id);
    let Some(active_projection) = active_projection else {
        return;
    };
    let rollback_state = match active_projection.status {
        RunStatus::Running => WorkflowExecutionState::Running,
        RunStatus::WaitingApproval => WorkflowExecutionState::WaitingApproval,
        RunStatus::Completed | RunStatus::Failed | RunStatus::Aborted => return,
    };
    let mut execs = executions.lock().await;
    let Some(exec) = execs.get_mut(run_id) else {
        return;
    };
    if exec.state != failed_snapshot.state {
        return;
    }
    exec.state = rollback_state;
    if let Some(current_node_name) = active_projection.current_node_name {
        if let Some(index) = exec
            .workflow
            .nodes
            .iter()
            .position(|node| node.name == current_node_name)
        {
            exec.current_step_index = index;
        }
    }
}
