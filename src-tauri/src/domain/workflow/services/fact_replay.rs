//! 純粋事実ログ（node_events）からの実行木の導出（tree fold）。
//!
//! 入力は 1 tree 分の事実行列のみ。遷移イベントは存在せず、完了・進行の
//! 規則（Submit + Stop 揃いで完了・fanout 全子完了・sequence 前進・
//! approval・on_failure）はこの fold と live 経路が共有する aggregate の
//! derive 系メソッドだけが知る。規則の変更は過去ログの解釈に遡及する
//! （「当時完了と判定した」という記録は持たない。許容済みトレードオフ）。

use std::collections::HashMap;

use crate::domain::workflow::entities::workflow_execution::{
    RuntimeNodeExecution, RuntimeNodeExecutionStatus, WorkflowDefaults,
    WorkflowExecution as WorkflowExecutionAggregate, WorkflowExecutionRestore,
};
use crate::domain::workflow::services::event_replay;
use crate::domain::workflow::{
    AgentSessionActivity, Artifact, ExecutionStatus, IsolatedWorktreeLedgerSnapshot,
    NodeCompletionSignal, NodeExecution, NodeExecutionFailure, NodeExecutionFailureKind,
    NodeExecutionStatus, NodeFact, NodeFactRecord, NodeKindName, RuntimeExecutionState,
    TreeRootFact, WorkflowExecution as WorkflowExecutionReadModel,
};

#[cfg(test)]
#[path = "fact_replay_test.rs"]
mod fact_replay_test;

/// fold の結果: 導出された実行木の状態。
#[derive(Debug)]
pub struct FoldedTree {
    pub aggregate: WorkflowExecutionAggregate,
    /// root started に記録された木の実行構成。
    pub root: TreeRootFact,
    /// 同じ tree の純粋事実から復元した隔離 worktree 台帳。
    pub isolated_worktrees: IsolatedWorktreeLedgerSnapshot,
    /// Session Node ごとに、同じ事実走査から導出した最新の provider 活動状態。
    pub session_activities: HashMap<String, AgentSessionActivity>,
    /// Session Node ごとに、同じ事実走査から導出した表示名の入力。
    pub session_display_names: HashMap<String, SessionDisplayNameInputs>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionDisplayNameInputs {
    pub manual_name: Option<String>,
    pub provider_session_title: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct SessionTitleObservationState {
    session_id: Option<String>,
    exited: bool,
    archived: bool,
}

impl SessionTitleObservationState {
    fn for_session(session_id: &str) -> Self {
        Self {
            session_id: Some(session_id.to_string()),
            ..Self::default()
        }
    }

    fn apply(&mut self, fact: &NodeFact) {
        match fact {
            NodeFact::SessionAttached(fact) => match &self.session_id {
                Some(session_id) if session_id == &fact.session_id => self.exited = false,
                Some(_) => {}
                None => {
                    self.session_id = Some(fact.session_id.clone());
                    self.exited = false;
                }
            },
            NodeFact::ProcessExited(_) => self.exited = true,
            NodeFact::ResumeRequested => self.exited = false,
            NodeFact::ArchiveRequested => self.archived = true,
            NodeFact::RestoreRequested => {
                self.exited = false;
                self.archived = false;
            }
            _ => {}
        }
    }

    fn accepts_title(&self) -> bool {
        !self.exited && !self.archived
    }
}

/// 1 tree 分の事実行列から実行木の状態を導出する。
///
/// 空の行列、または root の started を持たない行列は「木は存在しない」。
pub fn fold_execution_tree(
    tree_id: &str,
    records: &[NodeFactRecord],
) -> Result<Option<FoldedTree>, String> {
    for record in records {
        if record.meta.tree_id != tree_id {
            return Err(format!(
                "node fact belongs to tree {} instead of {tree_id}",
                record.meta.tree_id
            ));
        }
    }
    let Some(first) = records.first() else {
        return Ok(None);
    };
    let NodeFact::Started(started) = &first.fact else {
        return Err(format!("tree {tree_id} does not begin with a started fact"));
    };
    let Some(root) = started.root.clone() else {
        return Err(format!(
            "tree {tree_id} root started carries no tree root fact"
        ));
    };

    let isolated_worktrees = IsolatedWorktreeLedgerSnapshot::from_records(records)?;
    let started_at = timestamp_of(first);
    let mut aggregate = restore_aggregate(tree_id, &root, started_at);
    let mut session_activities: HashMap<String, AgentSessionActivity> = HashMap::new();
    let mut session_display_names: HashMap<String, SessionDisplayNameInputs> = HashMap::new();
    let mut session_title_observation_states: HashMap<String, SessionTitleObservationState> =
        HashMap::new();

    for (index, record) in records.iter().enumerate() {
        let defer_submit_settlement = records
            .get(index + 1)
            .is_some_and(|next| is_submitted_artifact_pair(record, next));
        apply_record(&mut aggregate, record, defer_submit_settlement)
            .map_err(|reason| format!("tree {tree_id} seq {}: {reason}", record.seq))?;
        if index
            .checked_sub(1)
            .and_then(|previous| records.get(previous))
            .is_some_and(|previous| is_submitted_artifact_pair(previous, record))
        {
            aggregate
                .derive_session_settlement(&record.meta.node_execution_id, timestamp_of(record))
                .map_err(|reason| format!("tree {tree_id} seq {}: {reason}", record.seq))?;
        }
        if record.meta.kind == NodeKindName::Session {
            let title_observation_state = session_title_observation_states
                .entry(record.meta.node_execution_id.clone())
                .or_default();
            title_observation_state.apply(&record.fact);
            let activity = session_activities
                .entry(record.meta.node_execution_id.clone())
                .or_default();
            *activity = activity.after_fact(&record.fact);
            let display_name = session_display_names
                .entry(record.meta.node_execution_id.clone())
                .or_default();
            match &record.fact {
                NodeFact::SessionNodeRenamed(fact) => {
                    display_name.manual_name = Some(fact.name.clone());
                }
                NodeFact::ProviderSessionTitleObserved(fact)
                    if title_observation_state.accepts_title() =>
                {
                    display_name.provider_session_title = Some(fact.title.clone());
                }
                _ => {}
            }
        }
    }

    aggregate.resolve_recovery_dependencies();
    Ok(Some(FoldedTree {
        aggregate,
        root,
        isolated_worktrees,
        session_activities,
        session_display_names,
    }))
}

fn is_submitted_artifact_pair(submit: &NodeFactRecord, artifact: &NodeFactRecord) -> bool {
    matches!(submit.fact, NodeFact::SubmitReceived(_))
        && matches!(artifact.fact, NodeFact::ArtifactProduced(_))
        && submit.meta.node_execution_id == artifact.meta.node_execution_id
}

fn timestamp_of(record: &NodeFactRecord) -> f64 {
    record.timestamp_ms as f64 / 1000.0
}

/// fold 済みの実行木から公開 read model を導出する。
pub fn derive_read_model(tree: &FoldedTree) -> WorkflowExecutionReadModel {
    let aggregate = &tree.aggregate;
    let status = match aggregate.state() {
        RuntimeExecutionState::Completed => ExecutionStatus::Completed,
        RuntimeExecutionState::Aborted => ExecutionStatus::Aborted,
        _ => ExecutionStatus::Running,
    };
    let request = aggregate.request.clone().unwrap_or_default();
    let nodes: Vec<NodeExecution> = aggregate
        .node_executions
        .iter()
        .map(|node| read_model_node(aggregate, node))
        .collect();
    let fields = event_replay::derive_workflow_execution_fields(
        &request,
        aggregate.started_at,
        status,
        &nodes,
    );
    WorkflowExecutionReadModel {
        id: aggregate.id.clone(),
        workflow_name: aggregate.workflow.name.clone(),
        status: fields.status,
        current_node: fields.current_node,
        created_from: aggregate.created_from,
        worktree_path: aggregate.worktree_path.clone(),
        started_at: aggregate.started_at,
        updated_at: aggregate.updated_at,
        completed_at: status.is_finished().then_some(aggregate.updated_at),
        error_reason: aggregate.error_reason.clone(),
        interruption_reason: None,
        resume_from_node: None,
        total_token_usage: event_replay::derive_total_token_usage(&nodes),
        node_executions: nodes,
        artifacts: fields.artifacts,
        fanouts: fields.fanouts,
        approval_target: fields.approval_target,
    }
}

fn read_model_node(
    aggregate: &WorkflowExecutionAggregate,
    node: &RuntimeNodeExecution,
) -> NodeExecution {
    let status = match node.status {
        RuntimeNodeExecutionStatus::Unresolved => NodeExecutionStatus::Unresolved,
        RuntimeNodeExecutionStatus::Running => NodeExecutionStatus::Running,
        RuntimeNodeExecutionStatus::Paused => NodeExecutionStatus::Paused,
        RuntimeNodeExecutionStatus::WaitingApproval => NodeExecutionStatus::WaitingApproval,
        RuntimeNodeExecutionStatus::Succeeded => NodeExecutionStatus::Succeeded,
        RuntimeNodeExecutionStatus::Failed => NodeExecutionStatus::Failed,
        RuntimeNodeExecutionStatus::Aborted => NodeExecutionStatus::Aborted,
    };
    let result_summary = node.result_summary.clone();
    let contract = aggregate
        .workflow
        .node_by_name(&node.node_name)
        .and_then(|definition| definition.artifact.clone());
    NodeExecution {
        recovery_reason: node.recovery_reason.clone(),
        id: node.id.clone(),
        execution_id: node.execution_id.clone(),
        node_name: node.node_name.clone(),
        kind: node.kind,
        attempt: node.attempt,
        status,
        session_id: node.session_id.clone(),
        display_command: node.display_command.clone(),
        result_summary,
        artifact: node.artifact.clone().map(|value| Artifact {
            node_name: node.node_name.clone(),
            contract,
            value,
            produced_at: node.completed_at.unwrap_or(node.started_at),
        }),
        token_usage: node.token_usage.clone(),
        failure: node.failure.as_ref().map(|failure| NodeExecutionFailure {
            reason: failure.reason.clone(),
            kind: failure.kind,
        }),
        parent: node.parent.clone(),
        completion_signals: node.completion_signals,
        started_at: node.started_at,
        completed_at: node.completed_at,
    }
}

fn restore_aggregate(
    tree_id: &str,
    root: &TreeRootFact,
    started_at: f64,
) -> WorkflowExecutionAggregate {
    let mut aggregate = WorkflowExecutionAggregate::restore_runtime(WorkflowExecutionRestore {
        id: tree_id.to_string(),
        workflow: root.definition.clone(),
        workflow_defaults: WorkflowDefaults,
        worktree_path: root.worktree_path.clone(),
        launched_as: root.launched_as,
        created_from: root.created_from,
        started_at,
        updated_at: started_at,
        request: (!root.request.is_empty()).then(|| root.request.clone()),
        ..WorkflowExecutionRestore::default()
    });
    aggregate.restore_definition_resolution((*root.definition_resolution).clone());
    aggregate
}

fn apply_record(
    aggregate: &mut WorkflowExecutionAggregate,
    record: &NodeFactRecord,
    defer_submit_settlement: bool,
) -> Result<(), String> {
    let id = record.meta.node_execution_id.as_str();
    let timestamp = timestamp_of(record);
    match &record.fact {
        NodeFact::Started(started) => {
            if started.root.is_some() {
                let _ = aggregate.replay_started();
            }
            aggregate.replay_node_started(
                id,
                &record.meta.node_name,
                record.meta.kind,
                record.meta.attempt,
                started.parent.clone(),
                timestamp,
            )
        }
        NodeFact::SessionAttached(fact) => {
            let _ = aggregate.attach_node_session(id, fact.session_id.clone(), timestamp);
            Ok(())
        }
        NodeFact::CommandSpawned(fact) => {
            let _ =
                aggregate.record_node_display_command(id, fact.display_command.clone(), timestamp);
            Ok(())
        }
        NodeFact::ProcessExited(fact) => match record.meta.kind {
            NodeKindName::Command => match fact.exit_code {
                Some(0) => {
                    if fact.result_summary.is_some() {
                        let _ = aggregate.record_pending_result(
                            id,
                            fact.result_summary.clone(),
                            None,
                            None,
                            None,
                            timestamp,
                        );
                    }
                    aggregate.derive_leaf_completed(id, timestamp)
                }
                Some(code) => aggregate.derive_leaf_process_exit_failed(
                    id,
                    fact.failure_reason
                        .clone()
                        .unwrap_or_else(|| format!("command exited with status {code}")),
                    fact.failure_kind
                        .unwrap_or(NodeExecutionFailureKind::InfrastructureCrash),
                    timestamp,
                ),
                None => {
                    // プロセス喪失: 完了せず、再開可能な中断として導出する。
                    let _ = aggregate.pause_node_execution(id, timestamp);
                    Ok(())
                }
            },
            NodeKindName::Session => {
                // プロセスが消えた session は正常終了なら中断、異常終了なら失敗。
                // 決着済み node への遅延事実だけを無視する。
                let should_apply = aggregate
                    .node_execution(id)
                    .is_some_and(|node| node.status.is_active());
                if should_apply {
                    if fact.is_abnormal() {
                        let reason = fact.failure_reason.clone().unwrap_or_else(|| {
                            fact.exit_code.map_or_else(
                                || "provider process was lost".to_string(),
                                |code| format!("provider process exited with status {code}"),
                            )
                        });
                        aggregate.derive_leaf_process_exit_failed(
                            id,
                            reason,
                            fact.failure_kind
                                .unwrap_or(NodeExecutionFailureKind::InfrastructureCrash),
                            timestamp,
                        )?;
                    } else {
                        let _ = aggregate.derive_session_process_exit(id, timestamp);
                    }
                }
                Ok(())
            }
            NodeKindName::Fanout | NodeKindName::Sequence => Ok(()),
        },
        NodeFact::RuntimeFailureObserved(fact) => {
            aggregate.derive_leaf_failed(id, fact.reason.clone(), fact.failure_kind, timestamp)
        }
        NodeFact::SubmitReceived(_) => {
            let _ = aggregate.record_node_completion_signal(
                id,
                NodeCompletionSignal::Submit,
                timestamp,
            );
            if defer_submit_settlement {
                Ok(())
            } else {
                aggregate.derive_session_settlement(id, timestamp)
            }
        }
        NodeFact::SubmitRejected(_) => Ok(()),
        NodeFact::StopReceived(fact) => {
            if fact.result_summary.is_some() || fact.token_usage.is_some() {
                let _ = aggregate.record_pending_result(
                    id,
                    fact.result_summary.clone(),
                    None,
                    None,
                    fact.token_usage.clone(),
                    timestamp,
                );
            }
            let _ =
                aggregate.record_node_completion_signal(id, NodeCompletionSignal::Stop, timestamp);
            aggregate.derive_session_settlement(id, timestamp)
        }
        NodeFact::ArtifactProduced(fact) => {
            let _ = aggregate.replay_artifact_produced(
                id,
                &record.meta.node_name,
                fact.contract.clone(),
                fact.value.clone(),
                timestamp,
            );
            Ok(())
        }
        NodeFact::ApprovalGranted(_) => aggregate.derive_approval_completion(id, timestamp),
        NodeFact::RetryRequested => {
            let _ = aggregate.request_node_retry(id, timestamp);
            Ok(())
        }
        NodeFact::ResumeRequested => {
            let _ = aggregate.resume_node_execution(id, timestamp);
            Ok(())
        }
        NodeFact::AbortRequested => {
            let _ = aggregate.replay_aborted_at(timestamp);
            Ok(())
        }
        NodeFact::AgentActivityObserved(_)
        | NodeFact::SessionNodeRenamed(_)
        | NodeFact::ProviderSessionTitleObserved(_)
        | NodeFact::ArchiveRequested
        | NodeFact::RestoreRequested
        | NodeFact::IsolatedWorktreeCreated(_)
        | NodeFact::IsolatedWorktreeReleased
        | NodeFact::IsolatedWorktreeLost => Ok(()),
    }
}

/// 単独 session（および workflow の子 session node）の事実列から導出した
/// session 状態。repository の read と GC の生存保護が同じ規則を読む。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionFactsView {
    pub provider_session_id: Option<String>,
    pub transcript_ref: Option<String>,
    pub manual_name: Option<String>,
    pub provider_session_title: Option<String>,
    pub initial_instruction_admitted: bool,
    /// 後続の attach / resume が無い process_exited（= Paused の根拠）。
    pub exited: bool,
    /// 後続の restore が無い archive_requested。
    pub archived: bool,
    pub last_exit_abnormal: bool,
    pub activity: AgentSessionActivity,
}

impl SessionFactsView {
    pub fn is_open(&self) -> bool {
        !self.archived && !self.exited
    }
}

/// session node に対する事実の走査で session 状態を導出する。
pub fn derive_session_facts(
    records: &[NodeFactRecord],
    node_execution_id: &str,
    session_id: &str,
) -> SessionFactsView {
    let mut view = SessionFactsView::default();
    let mut exited: Option<&crate::domain::workflow::ProcessExitedFact> = None;
    let mut title_observation_state = SessionTitleObservationState::for_session(session_id);
    for record in records {
        if record.meta.node_execution_id != node_execution_id {
            continue;
        }
        title_observation_state.apply(&record.fact);
        view.activity = view.activity.after_fact(&record.fact);
        match &record.fact {
            NodeFact::SessionAttached(fact) if fact.session_id == session_id => {
                if fact.provider_session_id.is_some() {
                    view.provider_session_id = fact.provider_session_id.clone();
                    view.transcript_ref = fact.transcript_ref.clone();
                }
                view.initial_instruction_admitted |= fact.initial_instruction_admitted;
                exited = None;
            }
            NodeFact::SessionNodeRenamed(fact) => view.manual_name = Some(fact.name.clone()),
            NodeFact::ProviderSessionTitleObserved(fact)
                if title_observation_state.accepts_title() =>
            {
                view.provider_session_title = Some(fact.title.clone());
            }
            NodeFact::ProcessExited(fact) => exited = Some(fact),
            NodeFact::ResumeRequested | NodeFact::RestoreRequested => {
                exited = None;
                view.archived = false;
            }
            NodeFact::ArchiveRequested => view.archived = true,
            _ => {}
        }
    }
    view.exited = exited.is_some();
    view.last_exit_abnormal = exited.is_some_and(|fact| fact.is_abnormal());
    view
}
