use std::collections::{HashMap, HashSet};

use super::transition::{decide_completion_disposition, CompletionDisposition};
use crate::domain::workflow::{
    ExecutionTreeLaunch, ItemsSource, NodeCompletionSignal, NodeCompletionSignalState,
    NodeDefinition, NodeExecutionStatus, NodeFact, NodeFactRecord, NodeKind, NodeKindName,
    Predicate, Rule, TreeRootFact,
};

#[derive(Debug, Clone)]
pub(crate) struct NodeReadRules {
    pub status: NodeExecutionStatus,
    pub completion_signals: NodeCompletionSignalState,
    pub delegate_waits_for_child: bool,
    pub has_artifact: bool,
    pub hidden: bool,
}

pub(crate) fn artifact_fields(root: &TreeRootFact) -> Vec<String> {
    let mut fields = HashSet::new();
    if let Some(definition) = &root.definition {
        for node in &definition.nodes {
            if let Some(delegate) = &node.completion.delegate {
                collect_fields(&delegate.when, &mut fields);
            }
            if let NodeKind::Sequence(sequence) = &node.kind {
                for rule in sequence
                    .children
                    .iter()
                    .flat_map(|child| child.rules.iter().flatten())
                {
                    match rule {
                        Rule::When { on, .. } => collect_fields(on, &mut fields),
                        Rule::Switch { on, .. } => {
                            fields.insert(on.clone());
                        }
                        _ => {}
                    }
                }
            }
            if let NodeKind::Fanout(fanout) = &node.kind {
                if let Some(ItemsSource::ArtifactField { field_path, .. }) = &fanout.items {
                    fields.insert(field_path.as_string());
                }
            }
        }
    }
    let mut fields: Vec<_> = fields.into_iter().collect();
    fields.sort();
    fields
}

fn collect_fields(predicate: &Predicate<String>, fields: &mut HashSet<String>) {
    match predicate {
        Predicate::Ref(field) => {
            fields.insert(field.clone());
            if let Some(child_field) = field.strip_prefix("child.") {
                fields.insert(child_field.into());
            }
        }
        Predicate::And(predicates) | Predicate::Or(predicates) => {
            for predicate in predicates {
                collect_fields(predicate, fields);
            }
        }
    }
}

pub(crate) fn subtree_read_rules(
    root: &TreeRootFact,
    records: &[NodeFactRecord],
    terminal: Option<&NodeFactRecord>,
) -> Result<HashMap<String, NodeReadRules>, String> {
    match project_read_rules(root, root.definition.as_ref(), records, terminal) {
        Err(_) if terminal.is_some() && root.definition.is_some() => {
            // Terminal history remains readable when its saved definition can no longer replay it.
            project_read_rules(root, None, records, terminal)
        }
        result => result,
    }
}

fn project_read_rules(
    root: &TreeRootFact,
    saved_definition: Option<&crate::domain::workflow::WorkflowDefinition>,
    records: &[NodeFactRecord],
    terminal: Option<&NodeFactRecord>,
) -> Result<HashMap<String, NodeReadRules>, String> {
    let mut nodes: HashMap<String, NodeReadRules> = HashMap::new();
    let mut artifacts = HashMap::new();
    let mut iterations: HashMap<String, u32> = HashMap::new();
    let mut latest_children = HashMap::new();
    let mut child_counts: HashMap<String, HashMap<String, u32>> = HashMap::new();
    let mut delegate_complete = HashSet::new();
    let mut retry = None;
    let mut predecessors = HashMap::new();
    let starts: HashMap<_, _> = records
        .iter()
        .filter(|record| matches!(record.fact, NodeFact::Started(_)))
        .map(|record| (record.meta.node_execution_id.as_str(), record))
        .collect();
    let definition = |id: &str| -> Option<&NodeDefinition> {
        saved_definition?.node_by_name(&starts.get(id)?.meta.node_name)
    };
    for record in records {
        if terminal.is_some_and(|terminal| record.seq > terminal.seq) {
            break;
        }
        let id = record.meta.node_execution_id.as_str();
        match &record.fact {
            NodeFact::Started(started) => {
                if saved_definition.is_some()
                    && definition(id).is_none_or(|node| node.kind_name() != record.meta.kind)
                {
                    return Err(format!(
                        "Node definition '{}' does not match the recorded kind",
                        record.meta.node_name
                    ));
                }
                nodes.insert(
                    id.into(),
                    NodeReadRules {
                        status: NodeExecutionStatus::Running,
                        completion_signals: NodeCompletionSignalState::Pending,
                        delegate_waits_for_child: false,
                        has_artifact: false,
                        hidden: false,
                    },
                );
                let predecessor = retry.take().filter(|previous: &&str| {
                    starts.get(*previous).is_some_and(|previous| {
                        previous.meta.node_name == record.meta.node_name
                            && previous.meta.parent_id == record.meta.parent_id
                    })
                });
                if let Some(previous) = predecessor {
                    predecessors.insert(id.to_string(), previous.to_string());
                }
                if predecessor.is_none() {
                    if let Some(parent) = &record.meta.parent_id {
                        *child_counts
                            .entry(parent.clone())
                            .or_default()
                            .entry(record.meta.node_name.clone())
                            .or_default() += 1;
                    }
                }
                if let Some(parent) = started
                    .parent
                    .as_ref()
                    .filter(|parent| parent.is_delegate_child())
                {
                    if predecessor.is_none() {
                        *iterations.entry(parent.parent_id.clone()).or_default() += 1;
                    }
                    latest_children.insert(parent.parent_id.clone(), id.to_string());
                    if let Some(parent) = nodes.get_mut(&parent.parent_id) {
                        parent.delegate_waits_for_child = true;
                    }
                }
            }
            NodeFact::ArtifactProduced(artifact) => {
                if let Some(node) = nodes.get_mut(id) {
                    node.has_artifact = true;
                }
                artifacts.insert(id.to_string(), &artifact.value);
                if let Some(delegate) =
                    definition(id).and_then(|node| node.completion.delegate.as_ref())
                {
                    let exhausted =
                        iterations.get(id).copied().unwrap_or_default() >= delegate.max_iterations;
                    let complete = exhausted
                        || delegate.when.evaluate(&mut |field| {
                            if field.starts_with("child.") {
                                return false;
                            }
                            artifact
                                .value
                                .get(field)
                                .and_then(serde_json::Value::as_bool)
                                .unwrap_or(false)
                        });
                    if complete {
                        delegate_complete.insert(id.to_string());
                    }
                    if let Some(node) = nodes.get_mut(id) {
                        node.delegate_waits_for_child = !complete;
                    }
                }
            }
            NodeFact::SubmitReceived(_) | NodeFact::StopReceived(_) => {
                if let Some(node) = nodes.get_mut(id).filter(|node| node.status.is_active()) {
                    node.completion_signals = node.completion_signals.receive(
                        if matches!(record.fact, NodeFact::SubmitReceived(_)) {
                            NodeCompletionSignal::Submit
                        } else {
                            NodeCompletionSignal::Stop
                        },
                    );
                }
            }
            NodeFact::DelegateResultInjected(_) => {
                if let Some(node) = nodes.get_mut(id) {
                    node.completion_signals = NodeCompletionSignalState::Pending;
                    node.delegate_waits_for_child = false;
                }
                delegate_complete.remove(id);
            }
            NodeFact::ApprovalGranted(_) | NodeFact::StandaloneSessionNodeCompleted => {
                if let Some(node) = nodes.get_mut(id).filter(|node| node.status.is_active()) {
                    node.status = NodeExecutionStatus::Succeeded;
                }
            }
            NodeFact::ProcessExited(exit)
                if record.meta.kind == NodeKindName::Command && exit.exit_code == Some(0) =>
            {
                if let Some(node) = nodes.get_mut(id).filter(|node| node.status.is_active()) {
                    node.status = completion_status(definition(id));
                }
            }
            NodeFact::RetryRequested => {
                retry = Some(id);
                if let Some(node) = nodes.get_mut(id).filter(|node| node.status.is_active()) {
                    node.status = NodeExecutionStatus::Aborted;
                }
            }
            _ => {}
        }
        let mut pending = std::collections::VecDeque::from([id]);
        while let Some(id) = pending.pop_front() {
            let Some(node) = nodes.get(id) else {
                continue;
            };
            let Some(start) = starts.get(id) else {
                continue;
            };
            if node.status == NodeExecutionStatus::Running
                && start.meta.kind == NodeKindName::Session
                && (definition(id).is_none_or(|node| node.completion.delegate.is_none())
                    || delegate_complete.contains(id))
            {
                use crate::domain::workflow::entities::workflow_execution::NodeCompletionHandshakeDecision as D;
                let status = match super::transition::decide_session_completion(
                    node.completion_signals,
                    saved_definition.is_none() && root.launched_as == ExecutionTreeLaunch::Session,
                    definition(id),
                ) {
                    D::CompleteAuto => Some(NodeExecutionStatus::Succeeded),
                    D::RequestApproval => Some(NodeExecutionStatus::WaitingApproval),
                    _ => None,
                };
                if let Some(status) = status {
                    nodes.get_mut(id).unwrap().status = status;
                }
            }
            if !nodes
                .get(id)
                .is_some_and(|node| node.status == NodeExecutionStatus::Succeeded)
            {
                continue;
            }
            let Some(parent) = start.meta.parent_id.as_deref() else {
                continue;
            };
            let Some(parent_definition) = definition(parent) else {
                continue;
            };
            let complete = match &parent_definition.kind {
                NodeKind::Sequence(sequence) => {
                    let artifact = artifacts.get(id).map(|value| expand_fields(value));
                    matches!(
                        super::routing::route_in_scope(
                            saved_definition.expect("node definition"),
                            sequence,
                            &start.meta.node_name,
                            artifact.as_ref(),
                            &child_counts[parent],
                        )
                        .map_err(|error| format!("{error:?}"))?,
                        super::routing::RouteDecision::Completed
                    )
                }
                NodeKind::Fanout(fanout) => {
                    let item_count = match &fanout.items {
                        None => Some(1),
                        Some(ItemsSource::Literal(items)) => Some(items.len()),
                        Some(ItemsSource::ArtifactField { node, field_path }) => records
                            .iter()
                            .filter(|record| {
                                record.seq <= start.seq && record.meta.node_name == *node
                            })
                            .rev()
                            .find_map(|record| artifacts.get(&record.meta.node_execution_id))
                            .and_then(|value| value.get(field_path.as_string()))
                            .and_then(serde_json::Value::as_u64)
                            .map(|count| count as usize),
                    };
                    let children: Vec<_> = starts
                        .values()
                        .filter(|child| {
                            child.seq <= record.seq
                                && child.meta.parent_id.as_deref() == Some(parent)
                        })
                        .filter(|child| {
                            !predecessors
                                .values()
                                .any(|id| id == &child.meta.node_execution_id)
                        })
                        .collect();
                    item_count.is_some_and(|count| children.len() == count * fanout.children.len())
                        && children.iter().all(|child| {
                            nodes
                                .get(&child.meta.node_execution_id)
                                .is_some_and(|node| node.status == NodeExecutionStatus::Succeeded)
                        })
                }
                _ => false,
            };
            if complete {
                if let Some(node) = nodes
                    .get_mut(parent)
                    .filter(|node| node.status == NodeExecutionStatus::Running)
                {
                    node.status = completion_status(Some(parent_definition));
                    node.has_artifact = node.status == NodeExecutionStatus::Succeeded;
                    pending.push_back(parent);
                }
                continue;
            }
            if !latest_children.get(parent).is_some_and(|child| child == id) {
                continue;
            }
            let Some(delegate) =
                definition(parent).and_then(|node| node.completion.delegate.as_ref())
            else {
                continue;
            };
            let complete = delegate.when.evaluate(&mut |field| {
                let (id, key) = field
                    .strip_prefix("child.")
                    .map(|field| (id, field))
                    .unwrap_or((parent, field.as_str()));
                artifacts
                    .get(id)
                    .and_then(|artifact| artifact.get(key))
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
            });
            if let Some(node) = nodes.get_mut(parent) {
                node.delegate_waits_for_child = false;
            }
            if complete && delegate_complete.insert(parent.to_string()) {
                pending.push_back(parent);
            }
        }
    }

    if let Some(terminal) = terminal {
        for node in nodes.values_mut().filter(|node| node.status.is_active()) {
            node.status = if matches!(terminal.fact, NodeFact::ExecutionCompleted) {
                NodeExecutionStatus::Succeeded
            } else {
                NodeExecutionStatus::Aborted
            };
        }
    }
    let histories = super::history::retry_history_chains(nodes.iter().map(|(id, node)| {
        (
            id.as_str(),
            predecessors.get(id).map(String::as_str),
            !node.status.is_active(),
        )
    }));
    for (_, chain) in histories {
        for id in chain {
            nodes.get_mut(&id).unwrap().hidden = true;
        }
    }
    Ok(nodes)
}

fn completion_status(definition: Option<&NodeDefinition>) -> NodeExecutionStatus {
    match definition.map(decide_completion_disposition) {
        Some(CompletionDisposition::RequestApproval) => NodeExecutionStatus::WaitingApproval,
        _ => NodeExecutionStatus::Succeeded,
    }
}

#[cfg(test)]
#[path = "node_read_test.rs"]
mod node_read_tests;

fn expand_fields(value: &serde_json::Value) -> serde_json::Value {
    fn insert(target: &mut serde_json::Value, segments: &[&str], value: &serde_json::Value) {
        if let Some((segment, rest)) = segments.split_first() {
            if !target.is_object() {
                *target = serde_json::json!({});
            }
            insert(&mut target[*segment], rest, value);
        } else {
            *target = value.clone();
        }
    }
    let mut expanded = serde_json::json!({});
    if let Some(fields) = value.as_object() {
        for (path, value) in fields.iter().filter(|(_, value)| !value.is_null()) {
            insert(&mut expanded, &path.split('.').collect::<Vec<_>>(), value);
        }
    }
    expanded
}
