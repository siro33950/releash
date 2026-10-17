use std::collections::{HashMap, HashSet};

use super::transition::{self, scope_completion_status};
use crate::domain::workflow::entities::workflow_execution::delegate::{
    delegate_continuation, DelegateRuntime,
};
use crate::domain::workflow::{
    ExecutionTreeLaunch, ItemsSource, NodeCompletionSignal, NodeCompletionSignalState,
    NodeDefinition, NodeExecutionStatus, NodeFact, NodeFactRecord, NodeKind, NodeKindName,
    Predicate, Rule, TreeRootFact,
};

#[derive(Debug, Clone)]
pub(crate) struct NodeReadRules {
    pub status: NodeExecutionStatus,
    pub parent_id: Option<String>,
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
                let mut references = HashSet::new();
                collect_fields(&delegate.when, &mut references);
                for field in references {
                    if let Some(path) = field.strip_prefix("child.") {
                        collect_artifact_field(definition, &delegate.child, path, &mut fields);
                    }
                    fields.insert(field);
                }
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
        }
        Predicate::And(predicates) | Predicate::Or(predicates) => {
            for predicate in predicates {
                collect_fields(predicate, fields);
            }
        }
    }
}

fn collect_artifact_field(
    definition: &crate::domain::workflow::WorkflowDefinition,
    node: &str,
    path: &str,
    fields: &mut HashSet<String>,
) {
    fields.insert(path.into());
    let Some((key, rest)) = path.split_once('.') else {
        return;
    };
    let Some(node) = definition.node_by_name(node) else {
        return;
    };
    let child = match &node.kind {
        NodeKind::Sequence(sequence) => sequence.children.iter().find(|child| child.name == key),
        NodeKind::Fanout(fanout) => fanout.child_entry_for_artifact_key(key),
        _ => None,
    };
    if let Some(child) = child {
        collect_artifact_field(definition, &child.name, rest, fields);
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
    let mut delegates: HashMap<String, DelegateRuntime> = HashMap::new();
    let mut fanout_counts: HashMap<String, usize> = HashMap::new();
    let mut sequences: HashMap<
        String,
        crate::domain::workflow::entities::workflow_execution::SequenceScopeRuntime,
    > = HashMap::new();
    let mut approved = HashSet::new();
    let mut propagated = HashSet::new();
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
                        parent_id: record.meta.parent_id.clone(),
                        completion_signals: NodeCompletionSignalState::Pending,
                        delegate_waits_for_child: false,
                        has_artifact: false,
                        hidden: false,
                    },
                );
                if let Some(fanout) = definition(id).and_then(NodeDefinition::fanout) {
                    let count = match &fanout.items {
                        None => Some(1),
                        Some(ItemsSource::Literal(items)) => Some(items.len()),
                        Some(ItemsSource::ArtifactField { node, field_path }) => record
                            .meta
                            .parent_id
                            .as_ref()
                            .and_then(|parent| sequences.get(parent))
                            .and_then(|sequence| sequence.artifact_field(node, field_path))
                            .and_then(serde_json::Value::as_u64)
                            .map(|count| count as usize),
                    };
                    if let Some(count) = count {
                        fanout_counts.insert(id.to_string(), count);
                    }
                }
                let predecessor = retry.take().filter(|previous: &&str| {
                    starts.get(*previous).is_some_and(|previous| {
                        super::history::same_retry_target(
                            &previous.meta.node_name,
                            previous.meta.parent_id.as_deref(),
                            &record.meta.node_name,
                            record.meta.parent_id.as_deref(),
                        )
                    })
                });
                if let Some(previous) = predecessor {
                    if let Some((state, artifact, signals)) =
                        delegates.get(previous).and_then(|state| {
                            state.inherit_pending_result(
                                artifacts.get(previous),
                                nodes[previous].completion_signals,
                            )
                        })
                    {
                        if let Some(artifact) = artifact {
                            artifacts.insert(id.to_string(), artifact);
                        }
                        let node = nodes.get_mut(id).unwrap();
                        node.completion_signals = signals;
                        node.has_artifact = artifacts.contains_key(id);
                        node.delegate_waits_for_child = state.waits_for_child();
                        delegates.insert(id.to_string(), state);
                    }
                    predecessors.insert(id.to_string(), previous.to_string());
                }
                if let Some(parent) = &record.meta.parent_id {
                    if definition(parent)
                        .is_some_and(|node| matches!(node.kind, NodeKind::Sequence(_)))
                    {
                        sequences
                            .entry(parent.clone())
                            .or_default()
                            .replay_child_start(&record.meta.node_name, record.meta.attempt);
                    }
                }
                if let Some(parent) = started
                    .parent
                    .as_ref()
                    .filter(|parent| parent.is_delegate_child())
                {
                    let parent_id = delegate_continuation(
                        &parent.parent_id,
                        records
                            .iter()
                            .filter(|other| other.seq <= record.seq)
                            .map(|other| {
                                (
                                    other.meta.node_execution_id.as_str(),
                                    predecessors
                                        .get(&other.meta.node_execution_id)
                                        .map(String::as_str),
                                )
                            }),
                    );
                    let state = delegates.entry(parent_id.to_string()).or_default();
                    if predecessor.is_none() {
                        state.child_started(id);
                    } else {
                        state.child_restarted(id);
                    }
                    if let Some(parent) = nodes.get_mut(parent_id) {
                        parent.delegate_waits_for_child = state.waits_for_child();
                    }
                }
            }
            NodeFact::ArtifactProduced(artifact) => {
                if !nodes.contains_key(id) {
                    continue;
                }
                let mut value = expand_fields(&artifact.value);
                if let Some(delegate) =
                    definition(id).and_then(|node| node.completion.delegate.as_ref())
                {
                    let state = delegates.entry(id.to_string()).or_default();
                    let last = state.last_child.as_ref().and_then(|id| artifacts.get(id));
                    let Some(prepared) = state.prepare_artifact(delegate, value, last) else {
                        continue;
                    };
                    value = prepared;
                    state.submitted(crate::domain::workflow::entities::workflow_execution::delegate::delegate_predicate(delegate, Some(&value)));
                    if let Some(node) = nodes.get_mut(id) {
                        node.delegate_waits_for_child = state.waits_for_child();
                    }
                }
                nodes.get_mut(id).unwrap().has_artifact = true;
                artifacts.insert(id.to_string(), value);
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
            NodeFact::DelegateResultInjected(child) => {
                if let (Some(node), Some(state)) = (nodes.get_mut(id), delegates.get_mut(id)) {
                    if state.inject_result(child, node.status, &mut node.completion_signals) {
                        node.delegate_waits_for_child = state.waits_for_child();
                    }
                }
            }
            NodeFact::ApprovalGranted(_) | NodeFact::StandaloneSessionNodeCompleted => {
                if definition(id).is_some_and(|node| node.kind_name().is_composite_kind()) {
                    approved.insert(id);
                } else if let Some(node) = nodes.get_mut(id).filter(|node| node.status.is_active())
                {
                    node.status = NodeExecutionStatus::Succeeded;
                }
            }
            NodeFact::ProcessExited(exit)
                if record.meta.kind == NodeKindName::Command && exit.exit_code == Some(0) =>
            {
                if let Some(node) = nodes.get_mut(id).filter(|node| node.status.is_active()) {
                    node.status = definition(id)
                        .map(|node| scope_completion_status(node, false))
                        .unwrap_or(NodeExecutionStatus::Succeeded);
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
        for (empty_id, start) in &starts {
            if start.seq > record.seq {
                continue;
            }
            let Some(node_definition) = definition(empty_id) else {
                continue;
            };
            let NodeKind::Fanout(_) = &node_definition.kind else {
                continue;
            };
            let item_count = fanout_counts.get(*empty_id).copied();
            let isolated =
                matches!(&start.fact, NodeFact::Started(started) if started.worktree.is_some());
            let next = records.iter().find(|next| next.seq > start.seq);
            let failed = next.is_some_and(|next| {
                matches!(next.fact, NodeFact::RuntimeFailureObserved(_))
                    && next.meta.node_execution_id == *empty_id
            });
            let active = !next.is_some_and(|next| matches!(next.fact, NodeFact::AbortRequested(_)));
            if super::transition::is_empty_isolated_fanout(item_count, isolated)
                && nodes.get(*empty_id).is_some_and(|node| {
                    super::transition::empty_isolated_fanout_ready(active, node.status, failed)
                })
            {
                let children = read_scope_children(
                    records,
                    record.seq,
                    empty_id,
                    &nodes,
                    &artifacts,
                    &predecessors,
                );
                if let Some((status, artifact)) = transition::finish_composite(
                    node_definition,
                    nodes[*empty_id].status,
                    false,
                    item_count,
                    &children,
                )
                .map_err(|error| error.to_string())?
                {
                    let node = nodes.get_mut(*empty_id).unwrap();
                    node.status = status;
                    node.has_artifact = artifact.is_some();
                    if let Some(artifact) = artifact {
                        artifacts.insert(empty_id.to_string(), artifact);
                    }
                }
                pending.push_back(empty_id);
            }
        }
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
                    || delegates
                        .get(id)
                        .is_some_and(|state| state.handshake(node.completion_signals).is_none()))
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
            if let Some(node_definition) =
                definition(id).filter(|node| node.kind_name().is_composite_kind())
            {
                if nodes[id].status == NodeExecutionStatus::Succeeded || approved.contains(id) {
                    let children = read_scope_children(
                        records,
                        record.seq,
                        id,
                        &nodes,
                        &artifacts,
                        &predecessors,
                    );
                    if let Some((status, artifact)) = transition::finish_composite(
                        node_definition,
                        nodes[id].status,
                        true,
                        fanout_counts.get(id).copied(),
                        &children,
                    )
                    .map_err(|error| error.to_string())?
                    {
                        let node = nodes.get_mut(id).unwrap();
                        node.status = status;
                        node.has_artifact = artifact.is_some();
                        if let Some(artifact) = artifact {
                            artifacts.insert(id.to_string(), artifact);
                        }
                    }
                }
            }
            if nodes[id].status != NodeExecutionStatus::Succeeded
                || !propagated.insert(id.to_string())
            {
                continue;
            }
            let Some(parent) = start.meta.parent_id.as_deref() else {
                continue;
            };
            let parent = delegate_continuation(
                parent,
                records
                    .iter()
                    .filter(|other| other.seq <= record.seq)
                    .map(|other| {
                        (
                            other.meta.node_execution_id.as_str(),
                            predecessors
                                .get(&other.meta.node_execution_id)
                                .map(String::as_str),
                        )
                    }),
            );
            let Some(parent_definition) = definition(parent) else {
                continue;
            };
            match transition::completion_parent(Some(parent), |id| {
                definition(id).is_some_and(|node| node.completion.delegate.is_some())
            }) {
                transition::CompletionParent::Root => {}
                transition::CompletionParent::Scope(parent) => {
                    let sequence = sequences.entry(parent.to_string()).or_default();
                    if parent_definition.sequence().is_some() {
                        sequence.record_child_result(
                            crate::domain::workflow::value_objects::RuntimeArtifact {
                                node_name: start.meta.node_name.clone(),
                                attempt: start.meta.attempt,
                                session_id: None,
                                result: None,
                                artifact: artifacts.get(id).cloned(),
                                contract: None,
                                token_usage: None,
                                completed_at: record.timestamp_ms as f64 / 1000.0,
                            },
                        );
                    }
                    let children = read_scope_children(
                        records,
                        record.seq,
                        parent,
                        &nodes,
                        &artifacts,
                        &predecessors,
                    );
                    let route = transition::advance_composite(
                        saved_definition.expect("node definition"),
                        parent_definition,
                        &start.meta.node_name,
                        &sequence.child_counts,
                        fanout_counts.get(parent).copied(),
                        &children,
                    )
                    .map_err(|error| error.to_string())?;
                    if matches!(route, Some(super::routing::RouteDecision::Completed)) {
                        if let Some((status, artifact)) = transition::finish_composite(
                            parent_definition,
                            nodes[parent].status,
                            false,
                            fanout_counts.get(parent).copied(),
                            &children,
                        )
                        .map_err(|error| error.to_string())?
                        {
                            let node = nodes.get_mut(parent).unwrap();
                            node.status = status;
                            node.has_artifact = artifact.is_some();
                            if let Some(artifact) = artifact {
                                artifacts.insert(parent.to_string(), artifact);
                            }
                            pending.push_back(parent);
                        }
                    }
                }
                transition::CompletionParent::Delegate(parent) => {
                    let Some(state) = delegates.get_mut(parent) else {
                        continue;
                    };
                    let delegate = parent_definition
                        .completion
                        .delegate
                        .as_ref()
                        .expect("delegate parent");
                    let Some(artifact) = state
                        .settle_child(delegate, id, artifacts.get(parent), artifacts.get(id))
                        .map_err(|error| error.to_string())?
                    else {
                        continue;
                    };
                    artifacts.insert(parent.to_string(), artifact);
                    let node = nodes.get_mut(parent).unwrap();
                    node.delegate_waits_for_child = state.waits_for_child();
                    if state.handshake(node.completion_signals).is_none() {
                        pending.push_back(parent);
                    }
                }
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
    for (id, node) in &mut nodes {
        let parent = starts
            .get(id.as_str())
            .and_then(|start| start.meta.parent_id.as_deref());
        if let Some(parent) = parent.filter(|parent| {
            definition(parent).is_some_and(|node| node.completion.delegate.is_some())
        }) {
            node.parent_id = Some(
                delegate_continuation(
                    parent,
                    records.iter().map(|record| {
                        (
                            record.meta.node_execution_id.as_str(),
                            predecessors
                                .get(&record.meta.node_execution_id)
                                .map(String::as_str),
                        )
                    }),
                )
                .to_string(),
            );
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

fn read_scope_children<'a>(
    records: &'a [NodeFactRecord],
    seq: i64,
    parent: &str,
    nodes: &'a HashMap<String, NodeReadRules>,
    artifacts: &'a HashMap<String, serde_json::Value>,
    predecessors: &'a HashMap<String, String>,
) -> Vec<transition::ScopeChild<'a>> {
    records
        .iter()
        .filter(|record| record.seq <= seq && record.meta.parent_id.as_deref() == Some(parent))
        .filter_map(|record| {
            let NodeFact::Started(started) = &record.fact else {
                return None;
            };
            let id = &record.meta.node_execution_id;
            Some(transition::ScopeChild {
                id,
                name: &record.meta.node_name,
                parent: started.parent.as_ref(),
                predecessor: predecessors.get(id).map(String::as_str),
                status: nodes.get(id)?.status,
                artifact: artifacts.get(id),
            })
        })
        .collect()
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
