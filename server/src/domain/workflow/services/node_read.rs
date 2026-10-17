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

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AppliedFactTime {
    pub timestamp_ms: i64,
}

impl AppliedFactTime {
    fn record(&mut self, fact: &NodeFactRecord) {
        *self = Self {
            timestamp_ms: fact.timestamp_ms,
        };
    }
}

pub(crate) type NodeReadProjectionResult = (HashMap<String, NodeReadRules>, AppliedFactTime);

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NodeReadRules {
    pub status: NodeExecutionStatus,
    pub completed_at: Option<f64>,
    pub parent_id: Option<String>,
    pub completion_signals: NodeCompletionSignalState,
    pub delegate_waits_for_child: bool,
    pub has_artifact: bool,
    pub hidden: bool,
}

pub fn artifact_fields(root: &TreeRootFact) -> Vec<String> {
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
                for child in &sequence.children {
                    for rule in child.rules.iter().flatten() {
                        let mut references = HashSet::new();
                        match rule {
                            Rule::When { on, .. } => collect_fields(on, &mut references),
                            Rule::Switch { on, .. } => {
                                references.insert(on.clone());
                            }
                            _ => {}
                        }
                        for field in references {
                            collect_artifact_field(definition, &child.name, &field, &mut fields);
                        }
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
    let child = super::reference::artifact_child_name(node, key);
    if let Some(child) = child {
        collect_artifact_field(definition, child, rest, fields);
    }
}

pub(crate) fn subtree_read_projection(
    root: &TreeRootFact,
    records: &[NodeFactRecord],
    terminal: Option<&NodeFactRecord>,
    retries: &HashMap<String, String>,
    mut artifact_value: Option<&mut dyn FnMut(i64) -> Result<serde_json::Value, String>>,
) -> Result<NodeReadProjectionResult, String> {
    let mut state = NodeReadState::default();
    loop {
        let definition = state.definition(root);
        let mut projection = NodeReadProjection::new(
            root,
            definition,
            retries,
            artifact_value.as_mut().map(|value| {
                &mut **value as &mut dyn FnMut(i64) -> Result<serde_json::Value, String>
            }),
        );
        projection.state = state;
        let result = projection.drive(records, terminal, false);
        match result {
            Err(_) if projection.state.request_replay(root, terminal) => {
                state = projection.state;
            }
            Err(error) => return Err(error),
            Ok(_) => return projection.finish(terminal),
        }
    }
}

#[derive(Default)]
pub(crate) struct NodeReadState {
    pub seq: i64,
    without_definition: bool,
    pub applied: AppliedFactTime,
    sessions: HashMap<String, String>,
    current_session: Option<String>,
    commands: HashMap<String, (String, Option<String>, Option<String>)>,
    starts: HashMap<String, NodeFactRecord>,
    nodes: HashMap<String, NodeReadRules>,
    artifacts: HashMap<String, serde_json::Value>,
    artifact_rows: HashMap<String, i64>,
    delegates: HashMap<String, DelegateRuntime>,
    fanout_counts: HashMap<String, usize>,
    sequences: HashMap<
        String,
        crate::domain::workflow::entities::workflow_execution::SequenceScopeRuntime,
    >,
    approved: HashSet<String>,
    propagated: HashSet<String>,
    predecessors: HashMap<String, String>,
    continuations: DelegateContinuations<String>,
    children_by_parent: HashMap<String, Vec<String>>,
    pending: Option<NodeFactRecord>,
}

pub(crate) fn update_read_state(
    state: &mut NodeReadState,
    root: &TreeRootFact,
    records: &[NodeFactRecord],
    retries: &HashMap<String, String>,
    artifact_value: &mut dyn FnMut(i64) -> Result<serde_json::Value, String>,
) -> Result<AppliedFactTime, String> {
    let mut projection =
        NodeReadProjection::new(root, state.definition(root), retries, Some(artifact_value));
    projection.state = std::mem::take(state);
    let result = projection.drive(records, None, true);
    *state = projection.state;
    result
}

impl NodeReadState {
    fn definition<'a>(
        &self,
        root: &'a TreeRootFact,
    ) -> Option<&'a crate::domain::workflow::WorkflowDefinition> {
        if self.without_definition {
            None
        } else {
            root.definition.as_ref()
        }
    }

    pub(crate) fn request_replay(
        &mut self,
        root: &TreeRootFact,
        terminal: Option<&NodeFactRecord>,
    ) -> bool {
        if terminal.is_none() || self.without_definition || root.definition.is_none() {
            return false;
        }
        *self = Self {
            without_definition: true,
            ..Default::default()
        };
        true
    }

    pub(crate) fn artifact_seqs(&self, records: &[NodeFactRecord]) -> HashSet<i64> {
        let mut seqs = HashSet::new();
        for record in records {
            if matches!(record.fact, NodeFact::ArtifactProduced(_)) {
                seqs.insert(record.seq);
                if let Some(previous) = self.artifact_rows.get(&record.meta.node_execution_id) {
                    seqs.insert(*previous);
                }
            }
        }
        seqs
    }
}

struct CompletionCheckpoint {
    id: String,
    node: NodeReadRules,
    artifact: Option<serde_json::Value>,
    sequence: Option<crate::domain::workflow::entities::workflow_execution::SequenceScopeRuntime>,
    delegate: Option<DelegateRuntime>,
    propagated: bool,
}

impl CompletionCheckpoint {
    fn restore(self, state: &mut NodeReadState) {
        fn restore_entry<T>(map: &mut HashMap<String, T>, id: &str, value: Option<T>) {
            match value {
                Some(value) => {
                    map.insert(id.to_string(), value);
                }
                None => {
                    map.remove(id);
                }
            }
        }
        state.nodes.insert(self.id.clone(), self.node);
        restore_entry(&mut state.artifacts, &self.id, self.artifact);
        restore_entry(&mut state.sequences, &self.id, self.sequence);
        restore_entry(&mut state.delegates, &self.id, self.delegate);
        if !self.propagated {
            state.propagated.remove(&self.id);
        }
    }
}

struct CompletionJournal {
    entries: Vec<CompletionCheckpoint>,
    seen: HashSet<String>,
    applied: AppliedFactTime,
}

impl CompletionJournal {
    fn new(applied: AppliedFactTime) -> Self {
        Self {
            entries: Vec::new(),
            seen: HashSet::new(),
            applied,
        }
    }

    fn capture(&mut self, state: &NodeReadState, id: &str) {
        let Some(node) = state.nodes.get(id) else {
            return;
        };
        if !self.seen.insert(id.to_string()) {
            return;
        }
        self.entries.push(CompletionCheckpoint {
            id: id.to_string(),
            node: node.clone(),
            artifact: state.artifacts.get(id).cloned(),
            sequence: state.sequences.get(id).cloned(),
            delegate: state.delegates.get(id).cloned(),
            propagated: state.propagated.contains(id),
        });
    }

    fn restore(self, state: &mut NodeReadState) {
        for entry in self.entries {
            entry.restore(state);
        }
        state.applied = self.applied;
    }
}

struct NodeReadProjection<'a, 'v> {
    state: NodeReadState,
    artifact_value: Option<&'v mut dyn FnMut(i64) -> Result<serde_json::Value, String>>,
    root: &'a TreeRootFact,
    saved_definition: Option<&'a crate::domain::workflow::WorkflowDefinition>,
    retry_predecessors: &'a HashMap<String, String>,
}

impl<'a, 'v> NodeReadProjection<'a, 'v> {
    fn new(
        root: &'a TreeRootFact,
        saved_definition: Option<&'a crate::domain::workflow::WorkflowDefinition>,
        retries: &'a HashMap<String, String>,
        artifact_value: Option<&'v mut dyn FnMut(i64) -> Result<serde_json::Value, String>>,
    ) -> Self {
        Self {
            state: NodeReadState::default(),
            root,
            saved_definition,
            retry_predecessors: retries,
            artifact_value,
        }
    }

    fn drive(
        &mut self,
        records: &[NodeFactRecord],
        terminal: Option<&NodeFactRecord>,
        preview: bool,
    ) -> Result<AppliedFactTime, String> {
        for record in records {
            if terminal.is_some_and(|terminal| record.seq > terminal.seq) {
                break;
            }
            if let Some(previous) = self.state.pending.take() {
                if !super::fact_replay::is_submitted_artifact_pair(&previous, record) {
                    self.propagate_completion(&previous, Some(record), None)?;
                }
            }
            self.apply_record(record)?;
            self.state.seq = record.seq;
            let mut pending = record.clone();
            if let NodeFact::Started(started) = &mut pending.fact {
                started.root = None;
            }
            self.state.pending = Some(pending);
        }
        match self.state.pending.clone() {
            Some(last) if preview && matches!(last.fact, NodeFact::SubmitReceived(_)) => {
                Ok(self.state.applied)
            }
            Some(last) if preview => self.preview_completion(&last),
            Some(last) => {
                self.propagate_completion(&last, None, None)?;
                self.state.pending = None;
                Ok(self.state.applied)
            }
            None => Ok(self.state.applied),
        }
    }

    fn definition(&self, id: &str) -> Option<&'a NodeDefinition> {
        self.saved_definition?
            .node_by_name(&self.state.starts.get(id)?.meta.node_name)
    }

    fn start_node(
        &mut self,
        record: &NodeFactRecord,
        started: &crate::domain::workflow::StartedFact,
    ) -> Result<(), String> {
        let id = record.meta.node_execution_id.as_str();
        let mut start = record.clone();
        if let NodeFact::Started(started) = &mut start.fact {
            started.root = None;
        }
        self.state.starts.insert(id.into(), start.clone());

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
        self.state.applied.record(record);
        self.state.nodes.insert(
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
                    .and_then(|parent| self.state.sequences.get(parent))
                    .and_then(|sequence| sequence.artifact_field(node, field_path))
                    .and_then(serde_json::Value::as_u64)
                    .map(|count| count as usize),
            };
            if let Some(count) = count {
                self.state.fanout_counts.insert(id.to_string(), count);
            }
        }
        let predecessor = self.retry_predecessors.get(id).map(String::as_str);
        if let Some(parent) = record.meta.parent_id.as_deref() {
            self.state
                .children_by_parent
                .entry(parent.to_string())
                .or_default()
                .push(id.to_string());
        }
        if let Some(previous) = predecessor {
            if let Some((state, artifact, signals)) =
                self.state.delegates.get(previous).and_then(|state| {
                    state.inherit_pending_result(
                        self.state.artifacts.get(previous),
                        self.state.nodes[previous].completion_signals,
                    )
                })
            {
                if let Some(artifact) = artifact {
                    self.state.artifacts.insert(id.to_string(), artifact);
                }
                let node = self.state.nodes.get_mut(id).unwrap();
                node.completion_signals = signals;
                node.has_artifact = self.state.artifacts.contains_key(id);
                node.delegate_waits_for_child = state.waits_for_child();
                self.state.delegates.insert(id.to_string(), state);
            }
            self.state
                .predecessors
                .insert(id.to_string(), previous.to_string());
        }
        self.state
            .continuations
            .record(id.to_string(), predecessor.map(str::to_string));
        if let Some(parent) = &record.meta.parent_id {
            if self
                .definition(parent)
                .is_some_and(|node| matches!(node.kind, NodeKind::Sequence(_)))
            {
                self.state
                    .sequences
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
            let parent_id = self.state.continuations.resolve(parent.parent_id.clone());
            let state = self
                .state
                .delegates
                .entry(parent_id.to_string())
                .or_default();
            if predecessor.is_none() {
                state.child_started(id);
            } else {
                state.child_restarted(id);
            }
            if let Some(parent) = self.state.nodes.get_mut(&parent_id) {
                parent.delegate_waits_for_child = state.waits_for_child();
            }
        }
        Ok(())
    }

    fn record_artifact(
        &mut self,
        record: &NodeFactRecord,
        artifact: &crate::domain::workflow::ArtifactProducedFact,
    ) -> Result<(), String> {
        let id = record.meta.node_execution_id.as_str();

        if !self.state.nodes.contains_key(id) {
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
            let state = self.state.delegates.entry(id.to_string()).or_default();
            let last = state
                .last_child
                .as_ref()
                .and_then(|id| self.state.artifacts.get(id));
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
            if let Some(node) = self.state.nodes.get_mut(id) {
                node.delegate_waits_for_child = state.waits_for_child();
            }
        }
        if let Some(artifact_value) = &mut self.artifact_value {
            let previous = self
                .state
                .artifact_rows
                .get(id)
                .copied()
                .map(&mut **artifact_value)
                .transpose()?;
            let previous = previous.map(|value| {
                match self
                    .state
                    .starts
                    .get(id)
                    .and_then(|start| match &start.fact {
                        NodeFact::Started(started) if !start.meta.kind.is_composite_kind() => {
                            started.worktree.as_ref()
                        }
                        _ => None,
                    }) {
                    Some(worktree) => worktree.with_artifact(Some(value)),
                    None => value,
                }
            });
            let current = artifact_value(record.seq)?;
            if transition::artifact_changes_execution(delegated, previous.as_ref(), &current) {
                self.state.applied.record(record);
                self.state.artifact_rows.insert(id.to_string(), record.seq);
            }
        }
        self.state.nodes.get_mut(id).unwrap().has_artifact = true;
        self.state.artifacts.insert(id.to_string(), value);
        if self.state.nodes[id].status == NodeExecutionStatus::Succeeded {
            if let Some(parent) = record.meta.parent_id.as_deref().filter(|parent| {
                self.definition(parent)
                    .is_some_and(|node| node.sequence().is_some())
            }) {
                self.state
                    .sequences
                    .entry(parent.to_string())
                    .or_default()
                    .record_child_result(read_child_result(
                        &record.meta,
                        self.state.artifacts.get(id).cloned(),
                        record.timestamp_ms,
                    ));
            }
        }
        Ok(())
    }

    fn apply_record(&mut self, record: &NodeFactRecord) -> Result<(), String> {
        let id = record.meta.node_execution_id.as_str();
        match &record.fact {
            NodeFact::Started(started) => self.start_node(record, started)?,
            NodeFact::ArtifactProduced(artifact) => self.record_artifact(record, artifact)?,
            NodeFact::SessionAttached(attached) => {
                let fanout_child = self.state.starts.get(id).is_some_and(|start| matches!(&start.fact,
                    NodeFact::Started(started) if started.parent.as_ref().is_some_and(|parent| parent.is_fanout_child())));
                use crate::domain::workflow::entities::workflow_execution::{
                    RuntimeNodeExecution, TransitionOutcome,
                };
                if self.state.nodes.get(id).is_some_and(|node| {
                    RuntimeNodeExecution::session_attachment_transition(
                        node.status,
                        self.state.sessions.get(id).map(String::as_str),
                        self.state.current_session.as_deref(),
                        &attached.session_id,
                        fanout_child,
                    ) == TransitionOutcome::Applied
                }) {
                    self.state
                        .sessions
                        .insert(id.to_string(), attached.session_id.clone());
                    if !fanout_child {
                        self.state.current_session = Some(attached.session_id.clone());
                    }
                    self.state.applied.record(record);
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
                if self.state.nodes.get(id).is_some_and(|node| {
                    RuntimeNodeExecution::command_preparation_transition(
                        node.status,
                        self.state.commands.get(id).map(|value| {
                            (value.0.as_str(), value.1.as_deref(), value.2.as_deref())
                        }),
                        value,
                    ) == TransitionOutcome::Applied
                }) {
                    self.state.commands.insert(
                        id.to_string(),
                        (
                            value.0.into(),
                            value.1.map(str::to_string),
                            value.2.map(str::to_string),
                        ),
                    );
                    self.state.applied.record(record);
                }
            }
            NodeFact::SubmitReceived(_) | NodeFact::StopReceived(_) => {
                if matches!(&record.fact, NodeFact::StopReceived(stop) if stop.result_summary.is_some() || stop.token_usage.is_some())
                    && self.state.nodes.contains_key(id)
                {
                    self.state.applied.record(record);
                }
                if let Some(node) = self
                    .state
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
                        self.state.applied.record(record);
                    }
                }
            }
            NodeFact::DelegateResultInjected(child) => {
                if let (Some(node), Some(state)) = (
                    self.state.nodes.get_mut(id),
                    self.state.delegates.get_mut(id),
                ) {
                    if state.inject_result(child, node.status, &mut node.completion_signals) {
                        node.delegate_waits_for_child = state.waits_for_child();
                        self.state.applied.record(record);
                    }
                }
            }
            NodeFact::StandaloneSessionNodeCompleted => {
                if let Some(node) = self.state.nodes.get_mut(id).filter(|node| {
                    node.status.can_complete_standalone(
                        self.root.launched_as,
                        record.meta.kind,
                        record.meta.parent_id.is_none(),
                    )
                }) {
                    self.state.applied.record(record);
                    node.status = NodeExecutionStatus::Succeeded;
                    node.completed_at = Some(record.timestamp_ms as f64 / 1000.0);
                }
            }
            NodeFact::ApprovalGranted(_) => {
                if !self.state.nodes.get(id).is_some_and(|node| {
                    node.status
                        .accepts_approval(self.saved_definition.is_some())
                }) {
                    return Ok(());
                }
                if self
                    .definition(id)
                    .is_some_and(|node| node.kind_name().is_composite_kind())
                {
                    self.state.applied.record(record);
                    self.state.approved.insert(id.to_string());
                } else if let Some(node) = self.state.nodes.get_mut(id) {
                    self.state.applied.record(record);
                    node.status.apply_approval(
                        &mut node.completed_at,
                        self.saved_definition.is_some(),
                        record.timestamp_ms as f64 / 1000.0,
                    );
                }
            }
            NodeFact::ProcessExited(exit)
                if record.meta.kind == NodeKindName::Command && exit.exit_code == Some(0) =>
            {
                if exit.result_summary.is_some() && self.state.nodes.contains_key(id) {
                    self.state.applied.record(record);
                }
                let status = self
                    .definition(id)
                    .map(|node| scope_completion_status(node, false))
                    .unwrap_or(NodeExecutionStatus::Succeeded);
                if let Some(node) = self
                    .state
                    .nodes
                    .get_mut(id)
                    .filter(|node| node.status.is_active())
                {
                    if node.status != status {
                        self.state.applied.record(record);
                    }
                    node.status = status;
                }
            }
            NodeFact::RetryRequested => {
                if let Some(node) = self.state.nodes.get_mut(id).filter(|node| {
                    self.saved_definition.is_none() || node.status.can_restart(record.meta.kind)
                }) {
                    self.state.applied.record(record);
                    node.status = NodeExecutionStatus::Aborted;
                    node.completed_at = Some(record.timestamp_ms as f64 / 1000.0);
                }
            }
            NodeFact::ExecutionCompleted | NodeFact::AbortRequested(_) => {
                self.state.applied.record(record)
            }
            _ => {}
        }
        Ok(())
    }

    fn preview_completion(&mut self, record: &NodeFactRecord) -> Result<AppliedFactTime, String> {
        let mut journal = CompletionJournal::new(self.state.applied);
        let result = self
            .propagate_completion(record, None, Some(&mut journal))
            .map(|()| self.state.applied);
        journal.restore(&mut self.state);
        result
    }

    fn complete_empty_fanout(
        &mut self,
        record: &NodeFactRecord,
        next: Option<&NodeFactRecord>,
        journal: Option<&mut CompletionJournal>,
    ) -> Result<(), String> {
        if !matches!(record.fact, NodeFact::Started(_)) {
            return Ok(());
        }
        let id = record.meta.node_execution_id.as_str();
        let Some(node_definition) = self.definition(id) else {
            return Ok(());
        };
        let NodeKind::Fanout(_) = &node_definition.kind else {
            return Ok(());
        };
        let item_count = self.state.fanout_counts.get(id).copied();
        let isolated =
            matches!(&record.fact, NodeFact::Started(started) if started.worktree.is_some());
        let failed = next.is_some_and(|next| {
            matches!(next.fact, NodeFact::RuntimeFailureObserved(_))
                && next.meta.node_execution_id == id
        });
        let active = !next.is_some_and(|next| matches!(next.fact, NodeFact::AbortRequested(_)));
        if super::transition::is_empty_isolated_fanout(item_count, isolated)
            && self.state.nodes.get(id).is_some_and(|node| {
                super::transition::empty_isolated_fanout_ready(active, node.status, failed)
            })
        {
            if let Some(journal) = journal {
                journal.capture(&self.state, id);
            }
            let children = read_scope_children(
                &self.state.children_by_parent,
                &self.state.starts,
                id,
                &self.state.nodes,
                &self.state.artifacts,
                &self.state.predecessors,
            );
            let completion = transition::finish_composite(
                node_definition,
                self.state.nodes[id].status,
                false,
                item_count,
                &children,
            )
            .map_err(|error| error.to_string())?;
            self.apply_composite_completion(id, completion, record);
        }
        Ok(())
    }

    fn apply_composite_completion(
        &mut self,
        id: &str,
        completion: Option<(NodeExecutionStatus, Option<serde_json::Value>)>,
        record: &NodeFactRecord,
    ) -> bool {
        let Some((status, artifact)) = completion else {
            return false;
        };
        let node = self.state.nodes.get_mut(id).unwrap();
        if node.status != status {
            self.state.applied.record(record);
        }
        node.status = status;
        node.has_artifact = artifact.is_some();
        if let Some(artifact) = artifact {
            self.state.artifacts.insert(id.to_string(), artifact);
        }
        true
    }

    fn propagate_completion(
        &mut self,
        record: &NodeFactRecord,
        next: Option<&NodeFactRecord>,
        mut journal: Option<&mut CompletionJournal>,
    ) -> Result<(), String> {
        let id = record.meta.node_execution_id.as_str();
        let mut pending = std::collections::VecDeque::from([id.to_string()]);
        self.complete_empty_fanout(record, next, journal.as_deref_mut())?;
        while let Some(id) = pending.pop_front() {
            let id = id.as_str();
            let Some(node) = self.state.nodes.get(id) else {
                continue;
            };
            let Some(start) = self.state.starts.get(id).cloned() else {
                continue;
            };
            if let Some(journal) = journal.as_deref_mut() {
                journal.capture(&self.state, id);
            }
            if node.status == NodeExecutionStatus::Running
                && start.meta.kind == NodeKindName::Session
                && (self
                    .definition(id)
                    .is_none_or(|node| node.completion.delegate.is_none())
                    || self
                        .state
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
                    self.state.nodes.get_mut(id).unwrap().status = status;
                }
            }
            if let Some(node_definition) = self
                .definition(id)
                .filter(|node| node.kind_name().is_composite_kind())
            {
                if self.state.nodes[id].status == NodeExecutionStatus::Succeeded
                    || self.state.approved.contains(id)
                {
                    let children = read_scope_children(
                        &self.state.children_by_parent,
                        &self.state.starts,
                        id,
                        &self.state.nodes,
                        &self.state.artifacts,
                        &self.state.predecessors,
                    );
                    let completion = transition::finish_composite(
                        node_definition,
                        self.state.nodes[id].status,
                        true,
                        self.state.fanout_counts.get(id).copied(),
                        &children,
                    )
                    .map_err(|error| error.to_string())?;
                    self.apply_composite_completion(id, completion, record);
                }
            }
            if self.state.nodes[id].status != NodeExecutionStatus::Succeeded
                || !self.state.propagated.insert(id.to_string())
            {
                continue;
            }
            if !start.meta.kind.is_composite_kind() {
                if let NodeFact::Started(started) = &start.fact {
                    if let Some(worktree) = &started.worktree {
                        let artifact = worktree.with_artifact(self.state.artifacts.remove(id));
                        self.state.artifacts.insert(id.to_string(), artifact);
                        self.state.nodes.get_mut(id).unwrap().has_artifact = true;
                    }
                }
            }
            self.state
                .nodes
                .get_mut(id)
                .unwrap()
                .completed_at
                .get_or_insert(record.timestamp_ms as f64 / 1000.0);
            let Some(parent) = start.meta.parent_id.as_deref() else {
                continue;
            };
            let parent = self.state.continuations.resolve(parent.to_string());
            let parent = parent.as_str();
            let Some(parent_definition) = self.definition(parent) else {
                continue;
            };
            if let Some(journal) = journal.as_deref_mut() {
                journal.capture(&self.state, parent);
            }
            match transition::completion_parent(Some(parent), |id| {
                self.definition(id)
                    .is_some_and(|node| node.completion.delegate.is_some())
            }) {
                transition::CompletionParent::Root => {}
                transition::CompletionParent::Scope(parent) => {
                    let sequence = self.state.sequences.entry(parent.to_string()).or_default();
                    if parent_definition.sequence().is_some() {
                        sequence.record_child_result(read_child_result(
                            &start.meta,
                            self.state.artifacts.get(id).cloned(),
                            record.timestamp_ms,
                        ));
                    }
                    let children = read_scope_children(
                        &self.state.children_by_parent,
                        &self.state.starts,
                        parent,
                        &self.state.nodes,
                        &self.state.artifacts,
                        &self.state.predecessors,
                    );
                    let route = transition::advance_composite(
                        self.saved_definition.expect("node definition"),
                        parent_definition,
                        &start.meta.node_name,
                        &sequence.child_counts,
                        self.state.fanout_counts.get(parent).copied(),
                        &children,
                    )
                    .map_err(|error| error.to_string())?;
                    if matches!(route, Some(super::routing::RouteDecision::Completed)) {
                        let completion = transition::finish_composite(
                            parent_definition,
                            self.state.nodes[parent].status,
                            false,
                            self.state.fanout_counts.get(parent).copied(),
                            &children,
                        )
                        .map_err(|error| error.to_string())?;
                        if self.apply_composite_completion(parent, completion, record) {
                            pending.push_back(parent.to_string());
                        }
                    }
                }
                transition::CompletionParent::Delegate(parent) => {
                    let Some(state) = self.state.delegates.get_mut(parent) else {
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
                            self.state.artifacts.get(parent),
                            self.state.artifacts.get(id),
                        )
                        .map_err(|error| error.to_string())?
                    else {
                        continue;
                    };
                    self.state.artifacts.insert(parent.to_string(), artifact);
                    let node = self.state.nodes.get_mut(parent).unwrap();
                    node.delegate_waits_for_child = state.waits_for_child();
                    if state.handshake(node.completion_signals).is_none() {
                        pending.push_back(parent.to_string());
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
            self.state.applied.record(terminal);
            for node in self
                .state
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
        for (id, node) in &mut self.state.nodes {
            let parent = self
                .state
                .starts
                .get(id.as_str())
                .and_then(|start| start.meta.parent_id.as_deref());
            if let Some(parent) = parent.filter(|parent| {
                self.saved_definition
                    .and_then(|definition| {
                        definition.node_by_name(&self.state.starts.get(*parent)?.meta.node_name)
                    })
                    .is_some_and(|node| node.completion.delegate.is_some())
            }) {
                node.parent_id = Some(self.state.continuations.resolve(parent.to_string()));
            }
        }
        let histories =
            super::history::retry_history_chains(self.state.nodes.iter().map(|(id, node)| {
                (
                    id.as_str(),
                    self.state.predecessors.get(id).map(String::as_str),
                    !node.status.is_active(),
                )
            }));
        for (_, chain) in histories {
            for id in chain {
                self.state.nodes.get_mut(&id).unwrap().hidden = true;
            }
        }
        Ok((self.state.nodes, self.state.applied))
    }
}

fn read_scope_children<'a>(
    children: &'a HashMap<String, Vec<String>>,
    starts: &'a HashMap<String, NodeFactRecord>,
    parent: &str,
    nodes: &'a HashMap<String, NodeReadRules>,
    artifacts: &'a HashMap<String, serde_json::Value>,
    predecessors: &'a HashMap<String, String>,
) -> Vec<transition::ScopeChild<'a>> {
    children
        .get(parent)
        .into_iter()
        .flatten()
        .filter_map(|id| {
            let record = starts.get(id)?;
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

fn read_child_result(
    meta: &crate::domain::workflow::NodeFactMeta,
    artifact: Option<serde_json::Value>,
    timestamp_ms: i64,
) -> crate::domain::workflow::value_objects::RuntimeArtifact {
    crate::domain::workflow::value_objects::RuntimeArtifact {
        node_name: meta.node_name.clone(),
        attempt: meta.attempt,
        session_id: None,
        result: None,
        artifact,
        contract: None,
        token_usage: None,
        completed_at: timestamp_ms as f64 / 1000.0,
    }
}
