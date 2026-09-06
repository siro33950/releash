use std::sync::Arc;

use crate::adaptor::gateway::local_event_store::LocalEventStore;

#[cfg(test)]
use super::event::WorkflowEvent;
#[cfg(test)]
use super::execution_store::WorkflowExecutionMetadata;

pub(crate) struct WorkflowSessionFactSeed<'a> {
    pub workflow_name: &'a str,
    pub request: &'a str,
    pub worktree_path: &'a str,
    pub provider: crate::domain::provider_lifecycle::ProviderKind,
    pub workflow_execution_id: &'a str,
    pub node_execution_id: &'a str,
    pub session_id: &'a str,
    pub initial_instruction_admitted: bool,
}

pub(crate) fn seed_workflow_session_facts(
    store: &Arc<LocalEventStore>,
    seed: WorkflowSessionFactSeed<'_>,
) -> Result<(), String> {
    use crate::domain::workflow::{
        ChildEntry, ExecutionOrigin, ExecutionParentRef, ExecutionTreeLaunch, NodeDefinition,
        NodeFact, NodeFactMeta, NodeKind, NodeKindName, SequenceSpec, SessionAttachedFact,
        SessionSpec, StartedFact, TreeRootFact, WorkflowDefinition,
    };

    if !super::fact_log::read_tree_records(store, seed.workflow_execution_id)?.is_empty() {
        return Ok(());
    }
    let definition = WorkflowDefinition {
        name: seed.workflow_name.to_string(),
        description: String::new(),
        builtin: false,
        schemas: Default::default(),
        nodes: vec![
            NodeDefinition {
                name: "main".to_string(),
                kind: NodeKind::Sequence(SequenceSpec {
                    entry: None,
                    children: vec![ChildEntry::reference("impl")],
                }),
                artifact: None,
                input: Vec::new(),
                completion: crate::domain::workflow::NodeCompletion::Auto,
                worktree: None,
            },
            NodeDefinition {
                name: "impl".to_string(),
                kind: NodeKind::Session(SessionSpec {
                    provider: seed.provider,
                    model: None,
                    permission: None,
                    facets: Default::default(),
                }),
                artifact: None,
                input: Vec::new(),
                completion: crate::domain::workflow::NodeCompletion::Auto,
                worktree: None,
            },
        ],
        entry: "main".to_string(),
    };
    let root_meta = NodeFactMeta {
        tree_id: seed.workflow_execution_id.to_string(),
        node_execution_id: seed.workflow_execution_id.to_string(),
        parent_id: None,
        node_name: "main".to_string(),
        kind: NodeKindName::Sequence,
        attempt: 1,
    };
    let node_meta = NodeFactMeta {
        tree_id: seed.workflow_execution_id.to_string(),
        node_execution_id: seed.node_execution_id.to_string(),
        parent_id: Some(seed.workflow_execution_id.to_string()),
        node_name: "impl".to_string(),
        kind: NodeKindName::Session,
        attempt: 1,
    };
    super::fact_log::append_single_fact(
        store,
        &root_meta,
        &NodeFact::Started(StartedFact {
            parent: None,
            root: Some(TreeRootFact {
                workspace_identity: crate::domain::workspace_tree::WorkspaceIdentity::new(
                    seed.worktree_path,
                )
                .as_str()
                .to_string(),
                worktree_path: seed.worktree_path.to_string(),
                created_from: ExecutionOrigin::DesktopUi,
                request: seed.request.to_string(),
                definition,
                launched_as: ExecutionTreeLaunch::Workflow,
            }),
        }),
        1,
    )?;
    super::fact_log::append_single_fact(
        store,
        &node_meta,
        &NodeFact::Started(StartedFact {
            parent: Some(ExecutionParentRef::sequence_child(
                seed.workflow_execution_id,
            )),
            root: None,
        }),
        2,
    )?;
    super::fact_log::append_single_fact(
        store,
        &node_meta,
        &NodeFact::SessionAttached(SessionAttachedFact {
            session_id: seed.session_id.to_string(),
            provider_session_id: None,
            transcript_ref: None,
            initial_instruction_admitted: seed.initial_instruction_admitted,
        }),
        3,
    )
}

#[cfg(test)]
pub(crate) fn seed_canonical_execution(
    store: &Arc<LocalEventStore>,
    execution: &WorkflowExecutionMetadata,
    events: &[WorkflowEvent],
) {
    if events.is_empty() {
        // 事実列が既に seed 済みなら合成しない（既存の木を正とする）。
        let existing = super::fact_log::read_tree_records(store, &execution.execution_id).unwrap();
        if existing.is_empty() {
            let synthesized = synthesized_metadata_events(execution);
            super::fact_log::append_facts_for_events(store, &synthesized).unwrap();
        }
    } else {
        super::fact_log::append_facts_for_events(store, events).unwrap();
    }
}

/// metadata のみの seed を fold（node_events）でも観測できるよう、
/// 状態に対応する最小の事実列を合成する。
#[cfg(test)]
fn synthesized_metadata_events(execution: &WorkflowExecutionMetadata) -> Vec<WorkflowEvent> {
    use crate::domain::workflow::{
        ExecutionStatus, NodeDefinition, NodeKindName, WorkflowDefinition,
    };

    let definition = WorkflowDefinition {
        name: execution.workflow_name.clone(),
        description: String::new(),
        builtin: false,
        schemas: Default::default(),
        nodes: vec![NodeDefinition {
            name: "main".to_string(),
            kind: crate::domain::workflow::NodeKind::Session(
                crate::domain::workflow::SessionSpec::default(),
            ),
            artifact: None,
            input: Vec::new(),
            completion: crate::domain::workflow::NodeCompletion::Auto,
            worktree: None,
        }],
        entry: "main".to_string(),
    };
    let root_node_execution_id = format!("{}-root", execution.execution_id);
    let mut events = vec![
        WorkflowEvent::ExecutionStarted {
            execution_id: execution.execution_id.clone(),
            workflow_name: execution.workflow_name.clone(),
            worktree_path: execution.worktree_path.clone(),
            created_from: execution.created_from,
            request: String::new(),
            definition,
            timestamp: execution.started_at,
        },
        WorkflowEvent::NodeStarted {
            execution_id: execution.execution_id.clone(),
            node_execution_id: root_node_execution_id.clone(),
            node_name: "main".to_string(),
            kind: NodeKindName::Session,
            attempt: 1,
            parent: None,
            timestamp: execution.started_at,
        },
    ];
    let settled_at = execution.completed_at.unwrap_or(execution.updated_at);
    match execution.status {
        ExecutionStatus::Completed => {
            events.push(WorkflowEvent::NodeSubmitReceived {
                execution_id: execution.execution_id.clone(),
                node_execution_id: root_node_execution_id.clone(),
                timestamp: settled_at,
            });
            events.push(WorkflowEvent::NodeStopReceived {
                execution_id: execution.execution_id.clone(),
                node_execution_id: root_node_execution_id,
                timestamp: settled_at,
            });
        }
        ExecutionStatus::Aborted => {
            events.push(WorkflowEvent::ExecutionAborted {
                execution_id: execution.execution_id.clone(),
                aborted_node: None,
                timestamp: settled_at,
            });
        }
        ExecutionStatus::Running => {}
        unsupported => panic!("unsupported synthesized terminal status: {unsupported:?}"),
    }
    events
}

#[cfg(test)]
pub(crate) fn append_canonical_events(
    store: &Arc<LocalEventStore>,
    events: &[WorkflowEvent],
) -> Result<(), String> {
    super::fact_log::append_facts_for_events(store, events)
}
