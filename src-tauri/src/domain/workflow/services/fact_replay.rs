//! 純粋事実ログ（node_events）からの実行木の導出（tree fold）。
//!
//! 入力は 1 tree 分の事実行列のみ。遷移イベントは存在せず、完了・進行の
//! 規則（Submit + Stop 揃いで完了・fanout 全子完了・sequence 前進・
//! approval・on_failure）はこの fold と live 経路が共有する aggregate の
//! derive 系メソッドだけが知る。規則の変更は過去ログの解釈に遡及する
//! （「当時完了と判定した」という記録は持たない。許容済みトレードオフ）。

use crate::domain::workflow::entities::workflow_execution::{
    RuntimeNodeExecution, RuntimeNodeExecutionStatus, WorkflowDefaults,
    WorkflowExecution as WorkflowExecutionAggregate, WorkflowExecutionRestore,
};
use crate::domain::workflow::services::event_replay;
use crate::domain::workflow::{
    Artifact, ExecutionStatus, NodeCompletion, NodeCompletionSignal, NodeDefinition, NodeExecution,
    NodeExecutionFailure, NodeExecutionFailureKind, NodeExecutionStatus, NodeFact, NodeFactRecord,
    NodeKind, NodeKindName, RuntimeExecutionState, SessionRootFact, TreeRootFact,
    WorkflowDefinition, WorkflowExecution as WorkflowExecutionReadModel,
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

    let started_at = timestamp_of(first);
    let mut aggregate = restore_aggregate(tree_id, &root, first, started_at);

    for record in records {
        apply_record(&mut aggregate, record)
            .map_err(|reason| format!("tree {tree_id} seq {}: {reason}", record.seq))?;
    }

    Ok(Some(FoldedTree { aggregate, root }))
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
        RuntimeNodeExecutionStatus::Running => NodeExecutionStatus::Running,
        RuntimeNodeExecutionStatus::Paused => NodeExecutionStatus::Paused,
        RuntimeNodeExecutionStatus::WaitingApproval => NodeExecutionStatus::WaitingApproval,
        RuntimeNodeExecutionStatus::Succeeded => NodeExecutionStatus::Succeeded,
        RuntimeNodeExecutionStatus::Failed => NodeExecutionStatus::Failed,
        RuntimeNodeExecutionStatus::Aborted => NodeExecutionStatus::Aborted,
    };
    let result_summary = aggregate
        .node_history
        .iter()
        .rev()
        .find(|entry| entry.node_name == node.node_name && entry.attempt == node.attempt)
        .and_then(|entry| entry.result.clone())
        .or_else(|| node.result_summary.clone());
    let contract = aggregate
        .workflow
        .node_by_name(&node.node_name)
        .and_then(|definition| definition.artifact.clone());
    NodeExecution {
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
    first: &NodeFactRecord,
    started_at: f64,
) -> WorkflowExecutionAggregate {
    let (definition, worktree_path, created_from, request) = match root {
        TreeRootFact::Workflow(workflow) => (
            workflow.definition.clone(),
            workflow.worktree_path.clone(),
            workflow.created_from,
            (!workflow.request.is_empty()).then(|| workflow.request.clone()),
        ),
        TreeRootFact::Session(session) => (
            standalone_session_definition(&first.meta.node_name, session),
            session.worktree_path.clone(),
            session.created_from,
            None,
        ),
    };
    WorkflowExecutionAggregate::restore_runtime(WorkflowExecutionRestore {
        id: tree_id.to_string(),
        workflow: definition,
        workflow_defaults: WorkflowDefaults,
        worktree_path,
        created_from,
        started_at,
        updated_at: started_at,
        request,
        ..WorkflowExecutionRestore::default()
    })
}

/// 単独 Session は「session node 1つの定義」として同じ fold に載る。
fn standalone_session_definition(node_name: &str, session: &SessionRootFact) -> WorkflowDefinition {
    WorkflowDefinition {
        name: node_name.to_string(),
        description: String::new(),
        builtin: false,
        schemas: Default::default(),
        nodes: vec![NodeDefinition {
            name: node_name.to_string(),
            kind: NodeKind::Session(session.session.clone()),
            artifact: None,
            input: Vec::new(),
            completion: NodeCompletion::Auto,
            worktree: None,
        }],
        entry: node_name.to_string(),
    }
}

fn apply_record(
    aggregate: &mut WorkflowExecutionAggregate,
    record: &NodeFactRecord,
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
                Some(code) => aggregate.derive_leaf_failed(
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
                // プロセスが消えた session は中断（Paused の導出）。ただし
                // Stop 受信済みで Submit を待つだけの node は対話プロセスに
                // 依存しないため中断しない。決着済み node への遅延事実も無視。
                let should_pause = aggregate.node_execution(id).is_some_and(|node| {
                    node.status.is_active()
                        && matches!(
                            node.completion_signals,
                            crate::domain::workflow::NodeCompletionSignalState::Pending
                                | crate::domain::workflow::NodeCompletionSignalState::SubmitReceived
                        )
                });
                if should_pause {
                    let _ = aggregate.pause_node_execution(id, timestamp);
                }
                Ok(())
            }
            NodeKindName::Fanout | NodeKindName::Sequence => Ok(()),
        },
        NodeFact::SubmitReceived(_) => {
            let _ = aggregate.record_node_completion_signal(
                id,
                NodeCompletionSignal::Submit,
                timestamp,
            );
            aggregate.derive_session_settlement(id, timestamp)
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
        NodeFact::ArchiveRequested | NodeFact::RestoreRequested => Ok(()),
    }
}

/// 単独 session（および workflow の子 session node）の事実列から導出した
/// session 状態。repository の read と GC の生存保護が同じ規則を読む。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionFactsView {
    pub provider_session_id: Option<String>,
    pub transcript_ref: Option<String>,
    pub initial_instruction_admitted: bool,
    /// 後続の attach / resume が無い process_exited（= Paused の根拠）。
    pub exited: bool,
    /// 後続の restore が無い archive_requested。
    pub archived: bool,
    pub last_exit_abnormal: bool,
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
    for record in records {
        if record.meta.node_execution_id != node_execution_id {
            continue;
        }
        match &record.fact {
            NodeFact::SessionAttached(fact) if fact.session_id == session_id => {
                if fact.provider_session_id.is_some() {
                    view.provider_session_id = fact.provider_session_id.clone();
                    view.transcript_ref = fact.transcript_ref.clone();
                }
                view.initial_instruction_admitted |= fact.initial_instruction_admitted;
                exited = None;
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
    view.last_exit_abnormal = exited.is_some_and(|fact| {
        fact.exit_code != Some(0) || fact.failure_reason.is_some() || fact.failure_kind.is_some()
    });
    view
}
