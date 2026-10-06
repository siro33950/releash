//! 純粋事実ログ（node_events）からの実行木の導出（tree fold）。
//!
//! 終端状態は事実から復元し、Node の公開情報には解釈可能な保存定義を使う。
//! 進行規則は live 経路と aggregate の derive 系メソッドを共有する。

use std::collections::HashMap;

use crate::domain::workflow::entities::workflow_execution::{
    ExecutionTree as ExecutionTreeAggregate, ExecutionTreeRestore, RuntimeNodeExecution,
    RuntimeNodeExecutionStatus, WorkflowDefaults,
};
use crate::domain::workflow::services::{event_replay, transition};
use crate::domain::workflow::{
    AgentSessionActivity, Artifact, ExecutionStatus, ExecutionTree as ExecutionTreeReadModel,
    NodeCompletionSignal, NodeExecution, NodeExecutionStatus, NodeFact, NodeFactRecord,
    NodeKindName, RuntimeExecutionState, TreeRootFact,
};

#[cfg(test)]
#[path = "fact_replay_test.rs"]
mod fact_replay_test;

/// fold の結果: 導出された実行木の状態。
#[derive(Debug, Clone, PartialEq)]
pub struct FoldedTree {
    pub aggregate: ExecutionTreeAggregate,
    /// root started に記録された木の実行構成。
    pub root: TreeRootFact,
    pub artifact_contracts: HashMap<String, String>,
    /// Session Node ごとに、同じ事実走査から導出した最新の provider 活動状態。
    pub session_activities: HashMap<String, AgentSessionActivity>,
    /// Session Node ごとに、同じ事実走査から導出した表示名の入力。
    pub session_display_names: HashMap<String, SessionDisplayNameInputs>,
    /// session が attach された Session Node ごとに、同じ事実走査から導出した session 状態。
    pub sessions: HashMap<String, FoldedSession>,
}

/// Session Node に最初に attach された session と、その状態。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldedSession {
    pub session_id: String,
    pub facts: SessionFactsView,
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
            NodeFact::ArchiveRequested(_) => self.archived = true,
            NodeFact::RestoreRequested => {
                self.exited = true;
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
    Ok(fold_all(tree_id, records)?.map(|(_, folded)| folded))
}

/// 1 tree の事実を 1 件ずつ受け取って進める fold。
///
/// 全件からの導出（[`fold_execution_tree`]）と、追記された事実だけを足す読み取りが
/// 同じ規則を通る。
#[derive(Debug, Clone)]
pub struct TreeFold {
    tree_id: String,
    /// 確定した事実までを適用した状態。読み出しは [`TreeFold::view`] を通す。
    folded: FoldedTree,
    session_title_observation_states: HashMap<String, SessionTitleObservationState>,
    session_facts: HashMap<String, SessionFactsFold>,
    /// 後から現れた Session Node にも当てる、root の archive / restore の履歴。
    root_archive_facts: Vec<NodeFact>,
    terminal_seen: bool,
    /// 次の事実が対の Artifact かどうかで決着の時点が変わるため、適用を保留している Submit。
    pending_submit: Option<NodeFactRecord>,
}

impl TreeFold {
    /// 全件を読んで、fold の状態とその時点の実行木を作る。木が存在しなければ None。
    pub fn from_records(
        tree_id: &str,
        records: &[NodeFactRecord],
    ) -> Result<Option<(Self, FoldedTree)>, String> {
        fold_all(tree_id, records)
    }

    fn start(tree_id: &str, root: TreeRootFact, started_at: f64) -> Self {
        Self {
            tree_id: tree_id.to_string(),
            folded: FoldedTree {
                aggregate: restore_aggregate(tree_id, &root, started_at),
                root,
                artifact_contracts: HashMap::new(),
                session_activities: HashMap::new(),
                session_display_names: HashMap::new(),
                sessions: HashMap::new(),
            },
            session_title_observation_states: HashMap::new(),
            session_facts: HashMap::new(),
            root_archive_facts: Vec::new(),
            terminal_seen: false,
            pending_submit: None,
        }
    }

    /// この事実を足しても全件からの導出と同じ結果にならない場合に true。
    /// 呼び出し側はその tree を最初から読み直す。
    pub fn requires_restart(&self, record: &NodeFactRecord) -> bool {
        record.meta.parent_id.is_none()
            && matches!(
                &record.fact,
                NodeFact::RepositoryRootObserved(repository_root)
                    if self.folded.root.repository_root.as_ref() != Some(repository_root)
            )
    }

    pub fn push(&mut self, record: &NodeFactRecord) -> Result<(), String> {
        let tree_id = self.tree_id.as_str();
        if record.meta.tree_id != tree_id {
            return Err(format!(
                "node fact belongs to tree {} instead of {tree_id}",
                record.meta.tree_id
            ));
        }
        let before_terminal = !self.terminal_seen;
        let follows_submit = match self.pending_submit.take() {
            Some(submit) => {
                let pair = is_submitted_artifact_pair(&submit, record);
                apply_record(&mut self.folded.aggregate, &submit, pair)
                    .map_err(|reason| format!("tree {tree_id} seq {}: {reason}", submit.seq))?;
                pair
            }
            None => false,
        };
        // archive / restore は木の終端後にだけ起きる事実なので終端の打ち切りから外す。
        let tree_archive = matches!(
            record.fact,
            NodeFact::ArchiveRequested(_) | NodeFact::RestoreRequested
        );
        if before_terminal || tree_archive {
            if let NodeFact::ArtifactProduced(fact) = &record.fact {
                if let Some(contract) = &fact.contract {
                    self.folded
                        .artifact_contracts
                        .insert(record.meta.node_execution_id.clone(), contract.clone());
                }
            }
            if matches!(record.fact, NodeFact::SubmitReceived(_)) {
                self.pending_submit = Some(record.clone());
            } else {
                apply_record(&mut self.folded.aggregate, record, false)
                    .map_err(|reason| format!("tree {tree_id} seq {}: {reason}", record.seq))?;
            }
        }
        if follows_submit {
            self.folded
                .aggregate
                .derive_session_settlement(&record.meta.node_execution_id, timestamp_of(record))
                .map_err(|reason| format!("tree {tree_id} seq {}: {reason}", record.seq))?;
        }
        self.observe_session(record);
        if record.fact.terminal_state().is_some() {
            self.terminal_seen = true;
        }
        Ok(())
    }

    fn observe_session(&mut self, record: &NodeFactRecord) {
        let node_execution_id = &record.meta.node_execution_id;
        let root_archive = record.meta.parent_id.is_none()
            && matches!(
                record.fact,
                NodeFact::ArchiveRequested(_) | NodeFact::RestoreRequested
            );
        if root_archive {
            for (id, session) in &mut self.session_facts {
                if id != node_execution_id {
                    session.push(&record.fact);
                }
            }
        }
        if record.meta.kind == NodeKindName::Session {
            let title_observation_state = self
                .session_title_observation_states
                .entry(node_execution_id.clone())
                .or_default();
            title_observation_state.apply(&record.fact);
            let activity = self
                .folded
                .session_activities
                .entry(node_execution_id.clone())
                .or_default();
            *activity = activity.after_fact(&record.fact);
            let display_name = self
                .folded
                .session_display_names
                .entry(node_execution_id.clone())
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
            let root_archive_facts = &self.root_archive_facts;
            self.session_facts
                .entry(node_execution_id.clone())
                .or_insert_with(|| {
                    let mut session = SessionFactsFold::new(None);
                    for fact in root_archive_facts {
                        session.push(fact);
                    }
                    session
                })
                .push(&record.fact);
        }
        if root_archive {
            self.root_archive_facts.push(record.fact.clone());
        }
    }

    /// 現時点の実行木の状態。未確定の末尾は読み出し用の複製にだけ適用する。
    pub fn view(&self) -> Result<FoldedTree, String> {
        let mut folded = self.folded.clone();
        if let Some(submit) = &self.pending_submit {
            apply_record(&mut folded.aggregate, submit, false)
                .map_err(|reason| format!("tree {} seq {}: {reason}", self.tree_id, submit.seq))?;
        }
        if !self.terminal_seen {
            folded.aggregate.derive_empty_isolated_fanouts(None)?;
        }
        folded.sessions = self
            .session_facts
            .iter()
            .filter_map(|(node_execution_id, session)| {
                Some((
                    node_execution_id.clone(),
                    FoldedSession {
                        session_id: session.session_id.clone()?,
                        facts: session.view(),
                    },
                ))
            })
            .collect();
        Ok(folded)
    }
}

fn fold_all(
    tree_id: &str,
    records: &[NodeFactRecord],
) -> Result<Option<(TreeFold, FoldedTree)>, String> {
    #[cfg(any(test, feature = "test-support"))]
    TREE_FOLDS.with(|count| count.set(count.get() + 1));
    let result = fold_records_from_start(tree_id, records)?;
    #[cfg(any(test, feature = "test-support"))]
    if let Some((_, folded)) = &result {
        assert_appending_matches_full_fold(tree_id, records, folded);
    }
    Ok(result)
}

fn fold_records_from_start(
    tree_id: &str,
    records: &[NodeFactRecord],
) -> Result<Option<(TreeFold, FoldedTree)>, String> {
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
    let Some(root) = started.root.as_deref() else {
        return Err(format!(
            "tree {tree_id} root started carries no tree root fact"
        ));
    };
    let mut root = root.clone();

    for record in records {
        if record.meta.parent_id.is_none() {
            if let NodeFact::RepositoryRootObserved(repository_root) = &record.fact {
                match &root.repository_root {
                    Some(existing) if existing != repository_root => {
                        return Err(format!("tree {tree_id} has conflicting repository roots"));
                    }
                    _ => root.repository_root = Some(repository_root.clone()),
                }
            }
        }
    }

    let terminal = records
        .iter()
        .any(|record| record.fact.terminal_state().is_some());
    match fold_records(tree_id, records, root.clone(), terminal) {
        Err(_) if terminal && root.definition.is_some() => {
            fold_records(tree_id, records, root.without_definition(), terminal)
        }
        result => result,
    }
    .map(Some)
}

fn fold_records(
    tree_id: &str,
    records: &[NodeFactRecord],
    root: TreeRootFact,
    terminal: bool,
) -> Result<(TreeFold, FoldedTree), String> {
    if !terminal && root.definition.is_none() {
        return Err("non-terminal execution requires a workflow definition".into());
    }
    let mut fold = TreeFold::start(tree_id, root, timestamp_of(&records[0]));
    for record in records {
        fold.push(record)?;
    }
    let folded = fold.view()?;
    Ok((fold, folded))
}

/// どの位置で区切っても、途中まで fold してから残りを足した結果が全件 fold と一致する。
#[cfg(any(test, feature = "test-support"))]
fn assert_appending_matches_full_fold(
    tree_id: &str,
    records: &[NodeFactRecord],
    full: &FoldedTree,
) {
    for split in 1..records.len() {
        let Ok(Some((mut fold, _))) = fold_records_from_start(tree_id, &records[..split]) else {
            continue;
        };
        let appended = records[split..].iter().try_for_each(|record| {
            if fold.requires_restart(record) {
                return Err(());
            }
            fold.push(record).map_err(|_| ())
        });
        if appended.is_err() {
            continue;
        }
        let Ok(view) = fold.view() else { continue };
        assert_eq!(
            &view,
            full,
            "appending facts after seq {} must match the full fold",
            records[split - 1].seq
        );
    }
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static TREE_FOLDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// この thread で、事実を最初から fold した回数。
#[cfg(any(test, feature = "test-support"))]
pub fn tree_fold_count() -> usize {
    TREE_FOLDS.with(std::cell::Cell::get)
}

#[cfg(any(test, feature = "test-support"))]
pub fn without_tree_fold<T>(read: impl FnOnce() -> T) -> T {
    let before = TREE_FOLDS.with(std::cell::Cell::get);
    let result = read();
    assert_eq!(
        TREE_FOLDS.with(std::cell::Cell::get),
        before,
        "output must not reconstruct the execution tree"
    );
    result
}

pub(super) fn is_submitted_artifact_pair(
    submit: &NodeFactRecord,
    artifact: &NodeFactRecord,
) -> bool {
    matches!(submit.fact, NodeFact::SubmitReceived(_))
        && matches!(artifact.fact, NodeFact::ArtifactProduced(_))
        && submit.meta.node_execution_id == artifact.meta.node_execution_id
}

fn timestamp_of(record: &NodeFactRecord) -> f64 {
    record.timestamp_ms as f64 / 1000.0
}

/// fold 済みの実行木から公開 read model を導出する。
pub fn derive_read_model(tree: &FoldedTree) -> ExecutionTreeReadModel {
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
        .map(|node| {
            read_model_node(
                aggregate,
                node,
                tree.artifact_contracts.get(&node.id).cloned(),
            )
        })
        .collect();
    let fields = event_replay::derive_workflow_execution_fields(
        &request,
        aggregate.started_at,
        status,
        &nodes,
    );
    ExecutionTreeReadModel {
        id: aggregate.id.clone(),
        workflow_name: aggregate.workflow_name.clone(),
        status: fields.status,
        current_node: fields.current_node,
        created_from: aggregate.created_from,
        worktree_path: aggregate.worktree_path.clone(),
        started_at: aggregate.started_at,
        updated_at: aggregate.updated_at,
        completed_at: status.is_finished().then_some(aggregate.updated_at),
        error_reason: aggregate.error_reason.clone(),
        total_token_usage: event_replay::derive_total_token_usage(&nodes),
        node_executions: nodes,
        artifacts: fields.artifacts,
        fanouts: fields.fanouts,
        approval_target: fields.approval_target,
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn derive_node_artifact(
    tree: &FoldedTree,
    records: &[NodeFactRecord],
    node_name: &str,
) -> Option<Artifact> {
    let submitted = records
        .iter()
        .rev()
        .filter(|record| {
            record.meta.node_name == node_name
                && matches!(
                    record.fact,
                    NodeFact::SubmitReceived(_) | NodeFact::ArtifactProduced(_)
                )
        })
        .find_map(|record| {
            tree.aggregate
                .node_execution(&record.meta.node_execution_id)
                .filter(|node| node.artifact.is_some())
        });
    let node = submitted.or_else(|| {
        tree.aggregate
            .node_executions
            .iter()
            .filter(|node| node.node_name == node_name && node.artifact.is_some())
            .max_by(|left, right| {
                left.completed_at
                    .unwrap_or(left.started_at)
                    .total_cmp(&right.completed_at.unwrap_or(right.started_at))
            })
    })?;
    Some(Artifact {
        node_name: node_name.to_string(),
        contract: tree
            .aggregate
            .node_definition(node_name)
            .and_then(|definition| definition.artifact.clone()),
        value: node.artifact.clone()?,
        produced_at: node.completed_at.unwrap_or(node.started_at),
    })
}

fn read_model_node(
    aggregate: &ExecutionTreeAggregate,
    node: &RuntimeNodeExecution,
    recorded_contract: Option<String>,
) -> NodeExecution {
    let status = match node.status {
        RuntimeNodeExecutionStatus::Running => NodeExecutionStatus::Running,
        RuntimeNodeExecutionStatus::WaitingApproval => NodeExecutionStatus::WaitingApproval,
        RuntimeNodeExecutionStatus::Succeeded => NodeExecutionStatus::Succeeded,
        RuntimeNodeExecutionStatus::Aborted => NodeExecutionStatus::Aborted,
    };
    let result_summary = node.result_summary.clone();
    let contract = recorded_contract.or_else(|| {
        aggregate
            .node_definition(&node.node_name)
            .and_then(|definition| definition.artifact.clone())
    });
    NodeExecution {
        worktree: node.worktree.clone(),
        id: node.id.clone(),
        execution_id: node.execution_id.clone(),
        node_name: node.node_name.clone(),
        kind: node.kind,
        attempt: node.attempt,
        status,
        process_presence: Default::default(),
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
) -> ExecutionTreeAggregate {
    let Some(workflow) = &root.definition else {
        return ExecutionTreeAggregate::restore_without_definition(tree_id, root, started_at);
    };
    ExecutionTreeAggregate::restore_runtime(ExecutionTreeRestore {
        id: tree_id.to_string(),
        workflow: workflow.clone(),
        workflow_defaults: WorkflowDefaults,
        worktree_path: root.worktree_path.clone(),
        workspace_identity: root.workspace_identity.clone(),
        repository_root: root.repository_root.clone(),
        launched_as: root.launched_as,
        created_from: root.created_from,
        started_at,
        updated_at: started_at,
        request: (!root.request.is_empty()).then(|| root.request.clone()),
        ..ExecutionTreeRestore::default()
    })
}

pub(super) fn apply_record(
    aggregate: &mut ExecutionTreeAggregate,
    record: &NodeFactRecord,
    defer_submit_settlement: bool,
) -> Result<(), String> {
    if !matches!(record.fact, NodeFact::AbortRequested(_)) {
        aggregate.derive_empty_isolated_fanouts(
            matches!(record.fact, NodeFact::RuntimeFailureObserved(_))
                .then_some(record.meta.node_execution_id.as_str()),
        )?;
    }
    let id = record.meta.node_execution_id.as_str();
    let timestamp = timestamp_of(record);
    match &record.fact {
        NodeFact::Started(started) => {
            aggregate.replay_started_fact(&record.meta, started, timestamp)
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
                    if aggregate
                        .node_definition(&record.meta.node_name)
                        .map(transition::decide_completion_disposition)
                        == Some(transition::CompletionDisposition::RequestApproval)
                    {
                        let _ = aggregate.mark_node_waiting_approval(id, timestamp);
                        Ok(())
                    } else {
                        aggregate.derive_leaf_completed(id, timestamp)
                    }
                }
                Some(_) | None => Ok(()),
            },
            NodeKindName::Session | NodeKindName::Fanout | NodeKindName::Sequence => Ok(()),
        },
        NodeFact::RuntimeFailureObserved(_) | NodeFact::RepositoryRootObserved(_) => Ok(()),
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
        NodeFact::DelegateResultInjected(child_execution_id) => {
            let injection =
                crate::domain::workflow::entities::workflow_execution::DelegateInjection {
                    node_execution_id: id.to_string(),
                    child_execution_id: child_execution_id.clone(),
                };
            aggregate.record_delegate_injected(&injection, timestamp);
            Ok(())
        }
        NodeFact::ApprovalGranted(_) => aggregate.derive_approval_completion(id, timestamp),
        NodeFact::RetryRequested => {
            let _ = aggregate.request_node_retry(id, timestamp);
            Ok(())
        }
        NodeFact::ResumeRequested => Ok(()),
        NodeFact::StandaloneSessionNodeCompleted => {
            match aggregate.complete_standalone_session_node(id, timestamp) {
                crate::domain::workflow::entities::workflow_execution::TransitionOutcome::Applied
                | crate::domain::workflow::entities::workflow_execution::TransitionOutcome::AlreadyApplied => Ok(()),
                outcome => Err(format!("standalone session node completion failed: {outcome:?}")),
            }
        }
        NodeFact::AbortRequested(_) | NodeFact::ExecutionCompleted => {
            aggregate.replay_terminal_fact(&record.fact, timestamp);
            Ok(())
        }
        NodeFact::AgentActivityObserved(_)
        | NodeFact::SessionContinuationAdmitted(_)
        | NodeFact::SessionNodeRenamed(_)
        | NodeFact::ProviderSessionTitleObserved(_) => Ok(()),
        NodeFact::ArchiveRequested(fact) => {
            if record.meta.parent_id.is_none() {
                aggregate.replay_archive(Some(fact.clone()));
            }
            Ok(())
        }
        NodeFact::RestoreRequested => {
            if record.meta.parent_id.is_none() {
                aggregate.replay_archive(None);
            }
            Ok(())
        }
    }
}

pub(super) fn restore_artifact_scope(
    root: &TreeRootFact,
    first: &NodeFactRecord,
    definition: &crate::domain::workflow::NodeDefinition,
    workflow: &crate::domain::workflow::WorkflowDefinition,
) -> ExecutionTreeAggregate {
    let mut aggregate = ExecutionTreeAggregate::restore_runtime(ExecutionTreeRestore {
        id: first.meta.tree_id.clone(),
        workflow: crate::domain::workflow::WorkflowDefinition {
            name: root.workflow_name.clone(),
            description: String::new(),
            builtin: false,
            schemas: workflow.schemas.clone(),
            nodes: std::iter::once(definition)
                .chain(workflow.nodes.iter().filter(|candidate| {
                    match &definition.kind {
                        crate::domain::workflow::NodeKind::Sequence(spec) => spec
                            .children
                            .iter()
                            .any(|child| child.name == candidate.name),
                        crate::domain::workflow::NodeKind::Fanout(spec) => spec
                            .children
                            .iter()
                            .any(|child| child.name == candidate.name),
                        _ => definition
                            .completion
                            .delegate
                            .as_ref()
                            .is_some_and(|delegate| delegate.child == candidate.name),
                    }
                }))
                .cloned()
                .collect(),
            entry: definition.name.clone(),
        },
        worktree_path: root.worktree_path.clone(),
        workspace_identity: root.workspace_identity.clone(),
        repository_root: root.repository_root.clone(),
        launched_as: root.launched_as,
        created_from: root.created_from,
        started_at: timestamp_of(first),
        ..ExecutionTreeRestore::default()
    });
    let _ = aggregate.replay_started();
    aggregate
}

pub(super) fn fold_leaf_artifact(
    root: &TreeRootFact,
    records: &[&NodeFactRecord],
    submitted_artifact_sequences: &std::collections::HashSet<i64>,
) -> Result<Option<(RuntimeNodeExecution, i64)>, String> {
    let Some(first) = records.first() else {
        return Ok(None);
    };
    let mut aggregate = if let Some(workflow) = root.definition.as_ref() {
        let Some(definition) = workflow.node_by_name(&first.meta.node_name) else {
            return Ok(None);
        };
        restore_artifact_scope(root, first, definition, workflow)
    } else {
        restore_aggregate(&first.meta.tree_id, root, timestamp_of(first))
    };
    let mut settled_seq = first.seq;
    for (index, record) in records.iter().enumerate() {
        let previous = aggregate
            .node_executions
            .first()
            .map(|node| (node.status, node.completed_at));
        if root.definition.is_some() && matches!(record.fact, NodeFact::Started(_)) {
            aggregate.replay_node_started(
                &record.meta.node_execution_id,
                &record.meta.node_name,
                record.meta.kind,
                record.meta.attempt,
                None,
                timestamp_of(record),
            )?;
        } else {
            let defer = records.get(index + 1).is_some_and(|next| {
                is_submitted_artifact_pair(record, next)
                    && submitted_artifact_sequences.contains(&next.seq)
            });
            apply_record(&mut aggregate, record, defer)?;
            if submitted_artifact_sequences.contains(&record.seq) {
                aggregate.derive_session_settlement(
                    &record.meta.node_execution_id,
                    timestamp_of(record),
                )?;
            }
        }
        if aggregate
            .node_executions
            .first()
            .map(|node| (node.status, node.completed_at))
            != previous
        {
            settled_seq = record.seq;
        }
    }
    Ok(aggregate
        .node_executions
        .first()
        .cloned()
        .map(|node| (node, settled_seq)))
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
    /// session が受理済みの delegate 続行指示の識別子。
    pub admitted_continuations: Vec<String>,
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
    let mut session = SessionFactsFold::new(Some(session_id));
    for record in records {
        let root_archive = record.meta.parent_id.is_none()
            && matches!(
                record.fact,
                NodeFact::ArchiveRequested(_) | NodeFact::RestoreRequested
            );
        if record.meta.node_execution_id != node_execution_id && !root_archive {
            continue;
        }
        session.push(&record.fact);
    }
    session.view()
}

/// 1 つの Session Node の事実を 1 件ずつ受け取って session 状態を進める。
#[derive(Debug, Clone)]
struct SessionFactsFold {
    /// None の間は、最初に attach された session を対象にする。
    session_id: Option<String>,
    view: SessionFactsView,
    exited: Option<crate::domain::workflow::ProcessExitedFact>,
    title_observation_state: SessionTitleObservationState,
    /// 対象の session が決まる前に届いた続行指示（session, 識別子）。
    unassigned_continuations: Vec<(String, String)>,
}

impl SessionFactsFold {
    fn new(session_id: Option<&str>) -> Self {
        Self {
            session_id: session_id.map(str::to_string),
            view: SessionFactsView::default(),
            exited: None,
            title_observation_state: session_id
                .map(SessionTitleObservationState::for_session)
                .unwrap_or_default(),
            unassigned_continuations: Vec::new(),
        }
    }

    fn push(&mut self, fact: &NodeFact) {
        if let (None, NodeFact::SessionAttached(attached)) = (&self.session_id, fact) {
            self.session_id = Some(attached.session_id.clone());
            let admitted = std::mem::take(&mut self.unassigned_continuations);
            self.view.admitted_continuations.extend(
                admitted
                    .into_iter()
                    .filter(|(session_id, _)| session_id == &attached.session_id)
                    .map(|(_, request_id)| request_id),
            );
        }
        self.title_observation_state.apply(fact);
        self.view.activity = self.view.activity.after_fact(fact);
        let session_id = self.session_id.as_deref();
        match fact {
            NodeFact::SessionAttached(fact) if Some(fact.session_id.as_str()) == session_id => {
                if fact.provider_session_id.is_some() {
                    self.view.provider_session_id = fact.provider_session_id.clone();
                    self.view.transcript_ref = fact.transcript_ref.clone();
                }
                self.view.initial_instruction_admitted |= fact.initial_instruction_admitted;
                self.exited = None;
                self.view.exited = false;
            }
            NodeFact::SessionContinuationAdmitted(fact) => match session_id {
                Some(session_id) if fact.session_id == session_id => {
                    self.view
                        .admitted_continuations
                        .push(fact.request_id.clone());
                }
                Some(_) => {}
                None => self
                    .unassigned_continuations
                    .push((fact.session_id.clone(), fact.request_id.clone())),
            },
            NodeFact::SessionNodeRenamed(fact) => self.view.manual_name = Some(fact.name.clone()),
            NodeFact::ProviderSessionTitleObserved(fact)
                if self.title_observation_state.accepts_title() =>
            {
                self.view.provider_session_title = Some(fact.title.clone());
            }
            NodeFact::ProcessExited(fact) => self.exited = Some(fact.clone()),
            NodeFact::ResumeRequested => {
                self.exited = None;
                self.view.exited = false;
            }
            NodeFact::RestoreRequested => {
                self.view.exited = true;
                self.view.archived = false;
            }
            NodeFact::AbortRequested(_) => self.view.exited = true,
            NodeFact::ArchiveRequested(_) => self.view.archived = true,
            _ => {}
        }
    }

    fn view(&self) -> SessionFactsView {
        let mut view = self.view.clone();
        view.exited |= self.exited.is_some();
        view.last_exit_abnormal = self.exited.as_ref().is_some_and(|fact| fact.is_abnormal());
        view
    }
}

pub fn derive_tree_archive(
    records: &[NodeFactRecord],
) -> Option<crate::domain::workflow::ExecutionTreeArchiveRecord> {
    records
        .iter()
        .rev()
        .filter(|record| record.meta.parent_id.is_none())
        .find_map(|record| match &record.fact {
            NodeFact::ArchiveRequested(fact) => {
                Some(Some(crate::domain::workflow::ExecutionTreeArchiveRecord {
                    execution_id: record.meta.tree_id.clone(),
                    archived_at: fact.archived_at,
                    archive_reason: fact.reason.clone(),
                }))
            }
            NodeFact::RestoreRequested => Some(None),
            _ => None,
        })
        .flatten()
}
