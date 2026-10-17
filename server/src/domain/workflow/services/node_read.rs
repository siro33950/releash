use std::collections::{HashMap, HashSet};

use super::transition::{self, scope_completion_status};
use crate::domain::workflow::entities::workflow_execution::delegate::{
    DelegateContinuations, DelegateRuntime,
};
use crate::domain::workflow::{
    ExecutionTreeLaunch, ItemsSource, NodeCompletionSignal, NodeCompletionSignalState,
    NodeDefinition, NodeExecutionStatus, NodeFact, NodeFactRecord, NodeKind, NodeKindName,
    Predicate, Rule, TreeRootFact,
};

pub(crate) const CLOCK_FACT_TYPES: &[&str] = &[
    "started",
    "session_attached",
    "command_spawned",
    "artifact_produced",
    "submit_received",
    "stop_received",
    "approval_granted",
    "retry_requested",
    "delegate_result_injected",
    "standalone_session_node_completed",
    "process_exited",
    "execution_completed",
    "abort_requested",
];

pub(crate) fn clock_scope<'a>(
    target: &'a str,
    starts: &HashMap<&str, &'a NodeFactRecord>,
) -> &'a str {
    let mut current = target;
    while let Some(start) = starts
        .get(current)
        .filter(|start| start.meta.kind.is_composite_kind())
    {
        let Some(parent) = start.meta.parent_id.as_deref().filter(|parent| {
            starts
                .get(*parent)
                .is_some_and(|parent| parent.meta.kind.is_composite_kind())
        }) else {
            break;
        };
        current = parent;
    }
    current
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AppliedFactTime {
    pub seq: i64,
    pub timestamp_ms: i64,
}

impl AppliedFactTime {
    fn record(&mut self, fact: &NodeFactRecord) {
        *self = Self {
            seq: fact.seq,
            timestamp_ms: fact.timestamp_ms,
        };
    }
}

pub(crate) struct NodeReadProjectionResult {
    pub nodes: HashMap<String, NodeReadRules>,
    pub applied: AppliedFactTime,
}

#[derive(Debug, Clone)]
pub(crate) struct NodeReadRules {
    pub status: NodeExecutionStatus,
    pub completed_at: Option<f64>,
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
                if let Some(ItemsSource::ArtifactField { node, field_path }) = &fanout.items {
                    collect_artifact_field(definition, node, &field_path.as_string(), &mut fields);
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
    let child = super::reference::composite_artifact_child(node, key);
    if let Some(child) = child {
        collect_artifact_field(definition, &child.name, rest, fields);
    }
}

pub(crate) fn subtree_read_projection(
    root: &TreeRootFact,
    records: &[NodeFactRecord],
    terminal: Option<&NodeFactRecord>,
    retries: &HashMap<String, String>,
    artifact_changes: &HashMap<i64, bool>,
) -> Result<NodeReadProjectionResult, String> {
    match project_read_rules(
        root,
        root.definition.as_ref(),
        records,
        terminal,
        retries,
        artifact_changes,
    ) {
        Err(_) if terminal.is_some() && root.definition.is_some() => {
            // Terminal history remains readable when its saved definition can no longer replay it.
            project_read_rules(root, None, records, terminal, retries, artifact_changes)
        }
        result => result,
    }
}

fn project_read_rules(
    root: &TreeRootFact,
    saved_definition: Option<&crate::domain::workflow::WorkflowDefinition>,
    records: &[NodeFactRecord],
    terminal: Option<&NodeFactRecord>,
    retries: &HashMap<String, String>,
    artifact_changes: &HashMap<i64, bool>,
) -> Result<NodeReadProjectionResult, String> {
    let mut projection =
        NodeReadProjection::new(root, saved_definition, records, retries, artifact_changes);
    for (position, record) in records.iter().enumerate() {
        if terminal.is_some_and(|terminal| record.seq > terminal.seq) {
            break;
        }
        projection.apply_record(record)?;
        if records
            .get(position + 1)
            .is_some_and(|next| super::fact_replay::is_submitted_artifact_pair(record, next))
        {
            continue;
        }
        projection.propagate_completion(record, records.get(position + 1))?;
    }
    projection.finish(terminal)
}

struct NodeReadProjection<'a> {
    applied: AppliedFactTime,
    artifact_changes: &'a HashMap<i64, bool>,
    sessions: HashMap<&'a str, &'a str>,
    current_session: Option<&'a str>,
    commands: HashMap<&'a str, (&'a str, Option<&'a str>, Option<&'a str>)>,
    root: &'a TreeRootFact,
    saved_definition: Option<&'a crate::domain::workflow::WorkflowDefinition>,
    starts: HashMap<&'a str, &'a NodeFactRecord>,
    retry_predecessors: &'a HashMap<String, String>,
    nodes: HashMap<String, NodeReadRules>,
    artifacts: HashMap<String, serde_json::Value>,
    delegates: HashMap<String, DelegateRuntime>,
    fanout_counts: HashMap<String, usize>,
    sequences: HashMap<
        String,
        crate::domain::workflow::entities::workflow_execution::SequenceScopeRuntime,
    >,
    approved: HashSet<&'a str>,
    propagated: HashSet<String>,
    predecessors: HashMap<String, String>,
    continuations: DelegateContinuations<'a>,
    children_by_parent: HashMap<&'a str, Vec<&'a NodeFactRecord>>,
}

impl<'a> NodeReadProjection<'a> {
    fn new(
        root: &'a TreeRootFact,
        saved_definition: Option<&'a crate::domain::workflow::WorkflowDefinition>,
        records: &'a [NodeFactRecord],
        retries: &'a HashMap<String, String>,
        artifact_changes: &'a HashMap<i64, bool>,
    ) -> Self {
        Self {
            applied: AppliedFactTime::default(),
            artifact_changes,
            sessions: HashMap::new(),
            current_session: None,
            commands: HashMap::new(),
            root,
            saved_definition,
            starts: records
                .iter()
                .filter(|record| matches!(record.fact, NodeFact::Started(_)))
                .map(|record| (record.meta.node_execution_id.as_str(), record))
                .collect(),
            retry_predecessors: retries,
            nodes: HashMap::new(),
            artifacts: HashMap::new(),
            delegates: HashMap::new(),
            fanout_counts: HashMap::new(),
            sequences: HashMap::new(),
            approved: HashSet::new(),
            propagated: HashSet::new(),
            predecessors: HashMap::new(),
            continuations: DelegateContinuations::default(),
            children_by_parent: HashMap::new(),
        }
    }

    fn definition(&self, id: &str) -> Option<&'a NodeDefinition> {
        self.saved_definition?
            .node_by_name(&self.starts.get(id)?.meta.node_name)
    }

    fn start_node(
        &mut self,
        record: &'a NodeFactRecord,
        started: &crate::domain::workflow::StartedFact,
    ) -> Result<(), String> {
        let id = record.meta.node_execution_id.as_str();

        if self.saved_definition.is_some()
            && self
                .definition(id)
                .is_none_or(|node| node.kind_name() != record.meta.kind)
        {
            return Err(format!(
                "Node definition '{}' does not match the recorded kind",
                record.meta.node_name
            ));
        }
        self.applied.record(record);
        self.nodes.insert(
            id.into(),
            NodeReadRules {
                status: NodeExecutionStatus::Running,
                completed_at: None,
                parent_id: record.meta.parent_id.clone(),
                completion_signals: NodeCompletionSignalState::Pending,
                delegate_waits_for_child: false,
                has_artifact: false,
                hidden: false,
            },
        );
        if let Some(fanout) = self.definition(id).and_then(NodeDefinition::fanout) {
            let count = match &fanout.items {
                None => Some(1),
                Some(ItemsSource::Literal(items)) => Some(items.len()),
                Some(ItemsSource::ArtifactField { node, field_path }) => record
                    .meta
                    .parent_id
                    .as_ref()
                    .and_then(|parent| self.sequences.get(parent))
                    .and_then(|sequence| sequence.artifact_field(node, field_path))
                    .and_then(serde_json::Value::as_u64)
                    .map(|count| count as usize),
            };
            if let Some(count) = count {
                self.fanout_counts.insert(id.to_string(), count);
            }
        }
        let predecessor = self.retry_predecessors.get(id).map(String::as_str);
        if let Some(parent) = record.meta.parent_id.as_deref() {
            self.children_by_parent
                .entry(parent)
                .or_default()
                .push(record);
        }
        if let Some(previous) = predecessor {
            if let Some((state, artifact, signals)) =
                self.delegates.get(previous).and_then(|state| {
                    state.inherit_pending_result(
                        self.artifacts.get(previous),
                        self.nodes[previous].completion_signals,
                    )
                })
            {
                if let Some(artifact) = artifact {
                    self.artifacts.insert(id.to_string(), artifact);
                }
                let node = self.nodes.get_mut(id).unwrap();
                node.completion_signals = signals;
                node.has_artifact = self.artifacts.contains_key(id);
                node.delegate_waits_for_child = state.waits_for_child();
                self.delegates.insert(id.to_string(), state);
            }
            self.predecessors
                .insert(id.to_string(), previous.to_string());
        }
        self.continuations.record(id, predecessor);
        if let Some(parent) = &record.meta.parent_id {
            if self
                .definition(parent)
                .is_some_and(|node| matches!(node.kind, NodeKind::Sequence(_)))
            {
                self.sequences
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
            let parent_id = self.continuations.resolve(&parent.parent_id);
            let state = self.delegates.entry(parent_id.to_string()).or_default();
            if predecessor.is_none() {
                state.child_started(id);
            } else {
                state.child_restarted(id);
            }
            if let Some(parent) = self.nodes.get_mut(parent_id) {
                parent.delegate_waits_for_child = state.waits_for_child();
            }
        }
        Ok(())
    }

    fn record_artifact(
        &mut self,
        record: &'a NodeFactRecord,
        artifact: &crate::domain::workflow::ArtifactProducedFact,
    ) -> Result<(), String> {
        let id = record.meta.node_execution_id.as_str();

        if !self.nodes.contains_key(id) {
            return Ok(());
        }
        let mut value = expand_fields(&artifact.value);
        let delegated = self
            .definition(id)
            .is_some_and(|node| node.completion.delegate.is_some());
        if let Some(delegate) = self
            .definition(id)
            .and_then(|node| node.completion.delegate.as_ref())
        {
            let state = self.delegates.entry(id.to_string()).or_default();
            let last = state
                .last_child
                .as_ref()
                .and_then(|id| self.artifacts.get(id));
            let Some(prepared) = state.prepare_artifact(delegate, value, last) else {
                return Ok(());
            };
            value = prepared;
            state.submitted(
                crate::domain::workflow::entities::workflow_execution::delegate::delegate_predicate(
                    delegate,
                    Some(&value),
                ),
            );
            if let Some(node) = self.nodes.get_mut(id) {
                node.delegate_waits_for_child = state.waits_for_child();
            }
        }
        if transition::artifact_changes_execution(
            delegated,
            self.artifact_changes
                .get(&record.seq)
                .copied()
                .ok_or_else(|| format!("artifact change is missing for fact {}", record.seq))?,
        ) {
            self.applied.record(record);
        }
        self.nodes.get_mut(id).unwrap().has_artifact = true;
        self.artifacts.insert(id.to_string(), value);
        if self.nodes[id].status == NodeExecutionStatus::Succeeded {
            if let Some(parent) = record.meta.parent_id.as_deref().filter(|parent| {
                self.definition(parent)
                    .is_some_and(|node| node.sequence().is_some())
            }) {
                self.sequences
                    .entry(parent.to_string())
                    .or_default()
                    .record_child_result(crate::domain::workflow::value_objects::RuntimeArtifact {
                        node_name: record.meta.node_name.clone(),
                        attempt: record.meta.attempt,
                        session_id: None,
                        result: None,
                        artifact: self.artifacts.get(id).cloned(),
                        contract: None,
                        token_usage: None,
                        completed_at: record.timestamp_ms as f64 / 1000.0,
                    });
            }
        }
        Ok(())
    }

    fn apply_record(&mut self, record: &'a NodeFactRecord) -> Result<(), String> {
        let id = record.meta.node_execution_id.as_str();
        match &record.fact {
            NodeFact::Started(started) => self.start_node(record, started)?,
            NodeFact::ArtifactProduced(artifact) => self.record_artifact(record, artifact)?,
            NodeFact::SessionAttached(attached) => {
                let fanout_child = self.starts.get(id).is_some_and(|start| matches!(&start.fact,
                    NodeFact::Started(started) if started.parent.as_ref().is_some_and(|parent| parent.is_fanout_child())));
                use crate::domain::workflow::entities::workflow_execution::{
                    RuntimeNodeExecution, TransitionOutcome,
                };
                if self.nodes.get(id).is_some_and(|node| {
                    RuntimeNodeExecution::session_attachment_transition(
                        node.status,
                        self.sessions.get(id).copied(),
                        self.current_session,
                        &attached.session_id,
                        fanout_child,
                    ) == TransitionOutcome::Applied
                }) {
                    self.sessions.insert(id, &attached.session_id);
                    if !fanout_child {
                        self.current_session = Some(&attached.session_id);
                    }
                    self.applied.record(record);
                }
            }
            NodeFact::CommandSpawned(command) => {
                let value = (
                    command.display_command.as_str(),
                    command.shell.as_deref(),
                    command.cwd.as_deref(),
                );
                use crate::domain::workflow::entities::workflow_execution::{
                    RuntimeNodeExecution, TransitionOutcome,
                };
                if self.nodes.get(id).is_some_and(|node| {
                    RuntimeNodeExecution::command_preparation_transition(
                        node.status,
                        self.commands.get(id).copied(),
                        value,
                    ) == TransitionOutcome::Applied
                }) {
                    self.commands.insert(id, value);
                    self.applied.record(record);
                }
            }
            NodeFact::SubmitReceived(_) | NodeFact::StopReceived(_) => {
                if matches!(&record.fact, NodeFact::StopReceived(stop) if stop.result_summary.is_some() || stop.token_usage.is_some())
                    && self.nodes.contains_key(id)
                {
                    self.applied.record(record);
                }
                if let Some(node) = self
                    .nodes
                    .get_mut(id)
                    .filter(|node| node.status.is_active())
                {
                    let previous = node.completion_signals;
                    node.completion_signals = node.completion_signals.receive(
                        if matches!(record.fact, NodeFact::SubmitReceived(_)) {
                            NodeCompletionSignal::Submit
                        } else {
                            NodeCompletionSignal::Stop
                        },
                    );
                    if node.completion_signals != previous {
                        self.applied.record(record);
                    }
                }
            }
            NodeFact::DelegateResultInjected(child) => {
                if let (Some(node), Some(state)) =
                    (self.nodes.get_mut(id), self.delegates.get_mut(id))
                {
                    if state.inject_result(child, node.status, &mut node.completion_signals) {
                        node.delegate_waits_for_child = state.waits_for_child();
                        self.applied.record(record);
                    }
                }
            }
            NodeFact::StandaloneSessionNodeCompleted => {
                if let Some(node) = self.nodes.get_mut(id).filter(|node| {
                    node.status.can_complete_standalone(
                        self.root.launched_as,
                        record.meta.kind,
                        record.meta.parent_id.is_none(),
                    )
                }) {
                    self.applied.record(record);
                    node.status = NodeExecutionStatus::Succeeded;
                    node.completed_at = Some(record.timestamp_ms as f64 / 1000.0);
                }
            }
            NodeFact::ApprovalGranted(_) => {
                if !self.nodes.get(id).is_some_and(|node| {
                    node.status.accepts_approval(
                        self.root.launched_as == ExecutionTreeLaunch::Workflow
                            && self.saved_definition.is_some(),
                    )
                }) {
                    return Ok(());
                }
                if self
                    .definition(id)
                    .is_some_and(|node| node.kind_name().is_composite_kind())
                {
                    self.applied.record(record);
                    self.approved.insert(id);
                } else if let Some(node) = self
                    .nodes
                    .get_mut(id)
                    .filter(|node| node.status.is_active())
                {
                    self.applied.record(record);
                    node.status = NodeExecutionStatus::Succeeded;
                    node.completed_at = Some(record.timestamp_ms as f64 / 1000.0);
                }
            }
            NodeFact::ProcessExited(exit)
                if record.meta.kind == NodeKindName::Command && exit.exit_code == Some(0) =>
            {
                if exit.result_summary.is_some() && self.nodes.contains_key(id) {
                    self.applied.record(record);
                }
                let status = self
                    .definition(id)
                    .map(|node| scope_completion_status(node, false))
                    .unwrap_or(NodeExecutionStatus::Succeeded);
                if let Some(node) = self
                    .nodes
                    .get_mut(id)
                    .filter(|node| node.status.is_active())
                {
                    if node.status != status {
                        self.applied.record(record);
                    }
                    node.status = status;
                }
            }
            NodeFact::RetryRequested => {
                if let Some(node) = self.nodes.get_mut(id).filter(|node| {
                    self.saved_definition.is_none() || node.status.can_restart(record.meta.kind)
                }) {
                    self.applied.record(record);
                    node.status = NodeExecutionStatus::Aborted;
                    node.completed_at = Some(record.timestamp_ms as f64 / 1000.0);
                }
            }
            NodeFact::ExecutionCompleted | NodeFact::AbortRequested(_) => {
                self.applied.record(record)
            }
            _ => {}
        }
        Ok(())
    }

    fn propagate_completion(
        &mut self,
        record: &'a NodeFactRecord,
        next: Option<&NodeFactRecord>,
    ) -> Result<(), String> {
        let id = record.meta.node_execution_id.as_str();
        let mut pending = std::collections::VecDeque::from([id]);
        for start in
            std::iter::once(record).filter(|record| matches!(record.fact, NodeFact::Started(_)))
        {
            let empty_id = &id;
            let Some(node_definition) = self.definition(empty_id) else {
                continue;
            };
            let NodeKind::Fanout(_) = &node_definition.kind else {
                continue;
            };
            let item_count = self.fanout_counts.get(*empty_id).copied();
            let isolated =
                matches!(&start.fact, NodeFact::Started(started) if started.worktree.is_some());
            let failed = next.is_some_and(|next| {
                matches!(next.fact, NodeFact::RuntimeFailureObserved(_))
                    && next.meta.node_execution_id == *empty_id
            });
            let active = !next.is_some_and(|next| matches!(next.fact, NodeFact::AbortRequested(_)));
            if super::transition::is_empty_isolated_fanout(item_count, isolated)
                && self.nodes.get(*empty_id).is_some_and(|node| {
                    super::transition::empty_isolated_fanout_ready(active, node.status, failed)
                })
            {
                let children = read_scope_children(
                    &self.children_by_parent,
                    empty_id,
                    &self.nodes,
                    &self.artifacts,
                    &self.predecessors,
                );
                if let Some((status, artifact)) = transition::finish_composite(
                    node_definition,
                    self.nodes[*empty_id].status,
                    false,
                    item_count,
                    &children,
                )
                .map_err(|error| error.to_string())?
                {
                    let node = self.nodes.get_mut(*empty_id).unwrap();
                    if node.status != status {
                        self.applied.record(record);
                    }
                    node.status = status;
                    node.has_artifact = artifact.is_some();
                    if let Some(artifact) = artifact {
                        self.artifacts.insert(empty_id.to_string(), artifact);
                    }
                }
                pending.push_back(empty_id);
            }
        }
        while let Some(id) = pending.pop_front() {
            let Some(node) = self.nodes.get(id) else {
                continue;
            };
            let Some(start) = self.starts.get(id) else {
                continue;
            };
            if node.status == NodeExecutionStatus::Running
                && start.meta.kind == NodeKindName::Session
                && (self
                    .definition(id)
                    .is_none_or(|node| node.completion.delegate.is_none())
                    || self
                        .delegates
                        .get(id)
                        .is_some_and(|state| state.handshake(node.completion_signals).is_none()))
            {
                use crate::domain::workflow::entities::workflow_execution::NodeCompletionHandshakeDecision as D;
                let status = match super::transition::decide_session_completion(
                    node.completion_signals,
                    self.saved_definition.is_none()
                        && self.root.launched_as == ExecutionTreeLaunch::Session,
                    self.definition(id),
                ) {
                    D::CompleteAuto => Some(NodeExecutionStatus::Succeeded),
                    D::RequestApproval => Some(NodeExecutionStatus::WaitingApproval),
                    _ => None,
                };
                if let Some(status) = status {
                    self.nodes.get_mut(id).unwrap().status = status;
                }
            }
            if let Some(node_definition) = self
                .definition(id)
                .filter(|node| node.kind_name().is_composite_kind())
            {
                if self.nodes[id].status == NodeExecutionStatus::Succeeded
                    || self.approved.contains(id)
                {
                    let children = read_scope_children(
                        &self.children_by_parent,
                        id,
                        &self.nodes,
                        &self.artifacts,
                        &self.predecessors,
                    );
                    if let Some((status, artifact)) = transition::finish_composite(
                        node_definition,
                        self.nodes[id].status,
                        true,
                        self.fanout_counts.get(id).copied(),
                        &children,
                    )
                    .map_err(|error| error.to_string())?
                    {
                        let node = self.nodes.get_mut(id).unwrap();
                        if node.status != status {
                            self.applied.record(record);
                        }
                        node.status = status;
                        node.has_artifact = artifact.is_some();
                        if let Some(artifact) = artifact {
                            self.artifacts.insert(id.to_string(), artifact);
                        }
                    }
                }
            }
            if self.nodes[id].status != NodeExecutionStatus::Succeeded
                || !self.propagated.insert(id.to_string())
            {
                continue;
            }
            self.nodes
                .get_mut(id)
                .unwrap()
                .completed_at
                .get_or_insert(record.timestamp_ms as f64 / 1000.0);
            let Some(parent) = start.meta.parent_id.as_deref() else {
                continue;
            };
            let parent = self.continuations.resolve(parent);
            let Some(parent_definition) = self.definition(parent) else {
                continue;
            };
            match transition::completion_parent(Some(parent), |id| {
                self.definition(id)
                    .is_some_and(|node| node.completion.delegate.is_some())
            }) {
                transition::CompletionParent::Root => {}
                transition::CompletionParent::Scope(parent) => {
                    let sequence = self.sequences.entry(parent.to_string()).or_default();
                    if parent_definition.sequence().is_some() {
                        sequence.record_child_result(
                            crate::domain::workflow::value_objects::RuntimeArtifact {
                                node_name: start.meta.node_name.clone(),
                                attempt: start.meta.attempt,
                                session_id: None,
                                result: None,
                                artifact: self.artifacts.get(id).cloned(),
                                contract: None,
                                token_usage: None,
                                completed_at: record.timestamp_ms as f64 / 1000.0,
                            },
                        );
                    }
                    let children = read_scope_children(
                        &self.children_by_parent,
                        parent,
                        &self.nodes,
                        &self.artifacts,
                        &self.predecessors,
                    );
                    let route = transition::advance_composite(
                        self.saved_definition.expect("node definition"),
                        parent_definition,
                        &start.meta.node_name,
                        &sequence.child_counts,
                        self.fanout_counts.get(parent).copied(),
                        &children,
                    )
                    .map_err(|error| error.to_string())?;
                    if matches!(route, Some(super::routing::RouteDecision::Completed)) {
                        if let Some((status, artifact)) = transition::finish_composite(
                            parent_definition,
                            self.nodes[parent].status,
                            false,
                            self.fanout_counts.get(parent).copied(),
                            &children,
                        )
                        .map_err(|error| error.to_string())?
                        {
                            let node = self.nodes.get_mut(parent).unwrap();
                            if node.status != status {
                                self.applied.record(record);
                            }
                            node.status = status;
                            node.has_artifact = artifact.is_some();
                            if let Some(artifact) = artifact {
                                self.artifacts.insert(parent.to_string(), artifact);
                            }
                            pending.push_back(parent);
                        }
                    }
                }
                transition::CompletionParent::Delegate(parent) => {
                    let Some(state) = self.delegates.get_mut(parent) else {
                        continue;
                    };
                    let delegate = parent_definition
                        .completion
                        .delegate
                        .as_ref()
                        .expect("delegate parent");
                    let Some(artifact) = state
                        .settle_child(
                            delegate,
                            id,
                            self.artifacts.get(parent),
                            self.artifacts.get(id),
                        )
                        .map_err(|error| error.to_string())?
                    else {
                        continue;
                    };
                    self.artifacts.insert(parent.to_string(), artifact);
                    let node = self.nodes.get_mut(parent).unwrap();
                    node.delegate_waits_for_child = state.waits_for_child();
                    if state.handshake(node.completion_signals).is_none() {
                        pending.push_back(parent);
                    }
                }
            }
        }
        Ok(())
    }

    fn finish(
        mut self,
        terminal: Option<&NodeFactRecord>,
    ) -> Result<NodeReadProjectionResult, String> {
        if let Some(terminal) = terminal {
            self.applied.record(terminal);
            for node in self
                .nodes
                .values_mut()
                .filter(|node| node.status.is_active())
            {
                node.completed_at = Some(terminal.timestamp_ms as f64 / 1000.0);
                node.status = if matches!(terminal.fact, NodeFact::ExecutionCompleted) {
                    NodeExecutionStatus::Succeeded
                } else {
                    NodeExecutionStatus::Aborted
                };
            }
        }
        for (id, node) in &mut self.nodes {
            let parent = self
                .starts
                .get(id.as_str())
                .and_then(|start| start.meta.parent_id.as_deref());
            if let Some(parent) = parent.filter(|parent| {
                self.saved_definition
                    .and_then(|definition| {
                        definition.node_by_name(&self.starts.get(*parent)?.meta.node_name)
                    })
                    .is_some_and(|node| node.completion.delegate.is_some())
            }) {
                node.parent_id = Some(self.continuations.resolve(parent).to_string());
            }
        }
        let histories =
            super::history::retry_history_chains(self.nodes.iter().map(|(id, node)| {
                (
                    id.as_str(),
                    self.predecessors.get(id).map(String::as_str),
                    !node.status.is_active(),
                )
            }));
        for (_, chain) in histories {
            for id in chain {
                self.nodes.get_mut(&id).unwrap().hidden = true;
            }
        }
        Ok(NodeReadProjectionResult {
            nodes: self.nodes,
            applied: self.applied,
        })
    }
}

fn read_scope_children<'a>(
    children: &'a HashMap<&str, Vec<&'a NodeFactRecord>>,
    parent: &str,
    nodes: &'a HashMap<String, NodeReadRules>,
    artifacts: &'a HashMap<String, serde_json::Value>,
    predecessors: &'a HashMap<String, String>,
) -> Vec<transition::ScopeChild<'a>> {
    children
        .get(parent)
        .into_iter()
        .flatten()
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
