//! On-demand projection from the append-only workflow execution event log.

use crate::domain::workflow::{
    ApprovalTarget, Artifact, ExecutionStatus, Fanout, NodeExecution, NodeExecutionStatus,
    NodeKindName, TokenUsage,
};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DerivedWorkflowExecutionFields {
    pub(crate) status: ExecutionStatus,
    pub(crate) current_node: Option<String>,
    pub(crate) approval_target: Option<ApprovalTarget>,
    pub(crate) artifacts: Vec<Artifact>,
    pub(crate) fanouts: Vec<Fanout>,
}

fn upsert_artifact(artifacts: &mut Vec<Artifact>, artifact: Artifact) {
    if let Some(current) = artifacts
        .iter_mut()
        .find(|current| current.node_name == artifact.node_name)
    {
        *current = artifact;
    } else {
        artifacts.push(artifact);
    }
}

pub(crate) fn derive_total_token_usage(nodes: &[NodeExecution]) -> TokenUsage {
    // 合成子インスタンスの token は子の合算値のため、leaf だけを数える。
    let mut usage = TokenUsage::default();
    for node in nodes {
        if node.kind.is_composite_kind() {
            continue;
        }
        if let Some(node_usage) = &node.token_usage {
            usage.input_tokens = usage.input_tokens.saturating_add(node_usage.input_tokens);
            usage.output_tokens = usage.output_tokens.saturating_add(node_usage.output_tokens);
        }
    }
    usage
}

pub(crate) fn derive_workflow_execution_fields(
    request: &str,
    started_at: f64,
    status: ExecutionStatus,
    nodes: &[NodeExecution],
) -> DerivedWorkflowExecutionFields {
    let (status, current_node, approval_target) = derive_active_fields(status, nodes);
    DerivedWorkflowExecutionFields {
        status,
        current_node,
        approval_target,
        artifacts: derive_top_level_artifacts(request, started_at, nodes),
        fanouts: derive_fanouts(nodes),
    }
}

pub(crate) fn derive_top_level_artifacts(
    request: &str,
    started_at: f64,
    nodes: &[NodeExecution],
) -> Vec<Artifact> {
    let mut artifacts = vec![Artifact {
        node_name: "request".to_string(),
        contract: None,
        value: serde_json::Value::String(request.to_string()),
        produced_at: started_at,
    }];
    let successful = nodes
        .iter()
        .filter(|node| {
            // fanout 子の成果は親の集約 Artifact に含まれるため、
            // トップレベル一覧には混ぜない。
            node.status == NodeExecutionStatus::Succeeded && !node.is_fanout_child()
        })
        .filter_map(|node| node.artifact.clone())
        .collect::<Vec<_>>();
    for artifact in successful {
        upsert_artifact(&mut artifacts, artifact);
    }
    artifacts
}

pub(crate) fn derive_fanouts(nodes: &[NodeExecution]) -> Vec<Fanout> {
    nodes
        .iter()
        .filter(|parent| parent.kind == NodeKindName::Fanout)
        .cloned()
        .map(|parent| {
            let mut children = nodes
                .iter()
                .filter(|child| {
                    child
                        .parent
                        .as_ref()
                        .is_some_and(|reference| reference.parent_id == parent.id)
                })
                .cloned()
                .collect::<Vec<_>>();
            children.sort_by(|left, right| {
                let slot = |node: &NodeExecution| {
                    node.parent
                        .as_ref()
                        .and_then(|reference| reference.fanout_slot)
                        .map(|slot| (slot.item_index.unwrap_or(0), slot.child_index))
                        .unwrap_or((0, 0))
                };
                slot(left)
                    .cmp(&slot(right))
                    .then_with(|| left.started_at.total_cmp(&right.started_at))
                    .then_with(|| left.id.cmp(&right.id))
            });
            Fanout {
                artifact: (parent.status == NodeExecutionStatus::Succeeded)
                    .then(|| parent.artifact.clone())
                    .flatten(),
                parent,
                children,
            }
        })
        .collect()
}

fn derive_active_fields(
    status: ExecutionStatus,
    nodes: &[NodeExecution],
) -> (ExecutionStatus, Option<String>, Option<ApprovalTarget>) {
    if status.is_finished() {
        return (status, None, None);
    }

    let waiting = nodes
        .iter()
        .rev()
        .filter(|node| node.status == NodeExecutionStatus::WaitingApproval)
        .collect::<Vec<_>>();
    if let Some(node) = waiting.first() {
        let current_node = Some(node.node_name.clone());
        let approval_target = (waiting.len() == 1).then(|| ApprovalTarget {
            node_execution_id: node.id.clone(),
            node_name: node.node_name.clone(),
            session_id: node.session_id.clone(),
        });
        return (ExecutionStatus::Running, current_node, approval_target);
    }

    let current_node = nodes
        .iter()
        .rev()
        .find(|node| node.status.is_active() && !node.kind.is_composite_kind())
        .or_else(|| nodes.iter().rev().find(|node| node.status.is_active()))
        // アクティブが無い（失敗停止した Running など）場合も「現在の node」
        // は空にしない: 最後に開始された node（失敗した node）を指す。
        .or_else(|| nodes.last())
        .map(|node| node.node_name.clone());
    (ExecutionStatus::Running, current_node, None)
}
