//! Pure workflow approval transition decisions.

use crate::domain::workflow::value_objects::NodeDefinition;

/// 既定の完了条件（session 二信号 / command exit code / fanout 全子完了）を
/// 満たした node の処遇。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionDisposition {
    Complete,
    RequestApproval,
}

pub fn decide_completion_disposition(node: &NodeDefinition) -> CompletionDisposition {
    if node.requires_approval_completion() {
        CompletionDisposition::RequestApproval
    } else {
        CompletionDisposition::Complete
    }
}

pub(crate) fn decide_session_completion(
    signals: crate::domain::workflow::NodeCompletionSignalState,
    standalone: bool,
    node: Option<&NodeDefinition>,
) -> crate::domain::workflow::entities::workflow_execution::NodeCompletionHandshakeDecision {
    use crate::domain::workflow::entities::workflow_execution::NodeCompletionHandshakeDecision as D;
    if !signals.is_ready() {
        return D::AwaitingSignal;
    }
    if standalone {
        return D::CompleteAuto;
    }
    match node
        .filter(|node| node.is_session())
        .map(decide_completion_disposition)
    {
        Some(CompletionDisposition::Complete) => D::CompleteAuto,
        Some(CompletionDisposition::RequestApproval) => D::RequestApproval,
        None => D::NotApplicable,
    }
}

pub(crate) fn artifact_changes_execution(delegated: bool, changed: bool) -> bool {
    delegated || changed
}

pub(crate) fn fanout_complete(expected: usize, children: impl IntoIterator<Item = bool>) -> bool {
    let mut count = 0;
    for complete in children {
        if !complete {
            return false;
        }
        count += 1;
    }
    count >= expected
}

pub(crate) fn scope_completion_status(
    node: &NodeDefinition,
    approved: bool,
) -> crate::domain::workflow::NodeExecutionStatus {
    if !approved && decide_completion_disposition(node) == CompletionDisposition::RequestApproval {
        crate::domain::workflow::NodeExecutionStatus::WaitingApproval
    } else {
        crate::domain::workflow::NodeExecutionStatus::Succeeded
    }
}

pub(crate) fn empty_isolated_fanout_ready(
    active: bool,
    status: crate::domain::workflow::NodeExecutionStatus,
    failed: bool,
) -> bool {
    active && status == crate::domain::workflow::NodeExecutionStatus::Running && !failed
}

pub(crate) fn composite_artifact_key(
    node: &NodeDefinition,
    name: &str,
    parent: Option<&crate::domain::workflow::ExecutionParentRef>,
) -> Option<String> {
    match &node.kind {
        crate::domain::workflow::NodeKind::Sequence(sequence) => sequence
            .children
            .iter()
            .find(|child| child.name == name)
            .map(|child| child.name.clone()),
        crate::domain::workflow::NodeKind::Fanout(fanout) => {
            fanout.artifact_key(parent?.fanout_slot()?)
        }
        _ => None,
    }
}

pub(crate) fn is_empty_isolated_fanout(item_count: Option<usize>, isolated: bool) -> bool {
    item_count == Some(0) && isolated
}

pub(crate) struct ScopeChild<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub parent: Option<&'a crate::domain::workflow::ExecutionParentRef>,
    pub predecessor: Option<&'a str>,
    pub status: crate::domain::workflow::NodeExecutionStatus,
    pub artifact: Option<&'a serde_json::Value>,
}

fn current_children<'a>(
    children: &'a [ScopeChild<'a>],
) -> impl Iterator<Item = &'a ScopeChild<'a>> {
    let replaced: std::collections::HashSet<_> = children
        .iter()
        .filter_map(|child| child.predecessor)
        .collect();
    children
        .iter()
        .filter(move |child| !replaced.contains(child.id))
}

pub(crate) fn advance_composite(
    workflow: &crate::domain::workflow::WorkflowDefinition,
    node: &NodeDefinition,
    completed_child: &str,
    counts: &std::collections::HashMap<String, u32>,
    item_count: Option<usize>,
    children: &[ScopeChild<'_>],
) -> Result<Option<super::routing::RouteDecision>, crate::domain::workflow::WorkflowError> {
    use crate::domain::workflow::{NodeExecutionStatus, NodeKind};
    match &node.kind {
        NodeKind::Sequence(sequence) => {
            let artifact = current_children(children)
                .filter(|child| child.name == completed_child)
                .last()
                .and_then(|child| child.artifact);
            super::routing::route_in_scope(workflow, sequence, completed_child, artifact, counts)
                .map(Some)
                .map_err(Into::into)
        }
        NodeKind::Fanout(fanout) => Ok(item_count
            .filter(|count| {
                fanout_complete(
                    count * fanout.children.len(),
                    current_children(children)
                        .map(|child| child.status == NodeExecutionStatus::Succeeded),
                )
            })
            .map(|_| super::routing::RouteDecision::Completed)),
        _ => Ok(None),
    }
}

pub(crate) fn finish_composite(
    node: &NodeDefinition,
    status: crate::domain::workflow::NodeExecutionStatus,
    approved: bool,
    item_count: Option<usize>,
    children: &[ScopeChild<'_>],
) -> Result<
    Option<(
        crate::domain::workflow::NodeExecutionStatus,
        Option<serde_json::Value>,
    )>,
    crate::domain::workflow::WorkflowError,
> {
    use crate::domain::workflow::{NodeExecutionStatus as S, NodeKind, WorkflowError};
    if status == S::Aborted || status == S::WaitingApproval && !approved {
        return Ok(None);
    }
    if let NodeKind::Fanout(fanout) = &node.kind {
        if item_count
            .is_none_or(|count| current_children(children).count() < count * fanout.children.len())
        {
            return Ok(None);
        }
    }
    let status = scope_completion_status(node, approved);
    if status == S::WaitingApproval {
        return Ok(Some((status, None)));
    }
    let mut artifacts = serde_json::Map::new();
    for child in current_children(children) {
        let Some(key) = composite_artifact_key(node, child.name, child.parent) else {
            if matches!(node.kind, NodeKind::Sequence(_)) {
                continue;
            }
            return Err(WorkflowError::invalid_state(match node.kind {
                NodeKind::Fanout(_) => {
                    format!("fanout child '{}' has no valid slot coordinates", child.id)
                }
                _ => format!("sequence child '{}' is undefined", child.id),
            }));
        };
        match &node.kind {
            NodeKind::Sequence(_) => {
                artifacts.remove(&key);
                if child.status == S::Succeeded {
                    if let Some(value) = child.artifact {
                        artifacts.insert(key, value.clone());
                    }
                }
            }
            NodeKind::Fanout(_) => {
                artifacts.insert(
                    key,
                    child.artifact.cloned().unwrap_or(serde_json::Value::Null),
                );
            }
            _ => {
                return Err(WorkflowError::invalid_state(format!(
                    "node '{}' is not composite",
                    node.name
                )))
            }
        }
    }
    Ok(Some((status, Some(serde_json::Value::Object(artifacts)))))
}

pub(crate) enum CompletionParent<'a> {
    Root,
    Delegate(&'a str),
    Scope(&'a str),
}

pub(crate) fn completion_parent<'a>(
    parent: Option<&'a str>,
    is_delegate: impl FnOnce(&str) -> bool,
) -> CompletionParent<'a> {
    match parent {
        None => CompletionParent::Root,
        Some(id) if is_delegate(id) => CompletionParent::Delegate(id),
        Some(id) => CompletionParent::Scope(id),
    }
}

#[cfg(test)]
#[path = "transition_test.rs"]
mod transition_tests;
