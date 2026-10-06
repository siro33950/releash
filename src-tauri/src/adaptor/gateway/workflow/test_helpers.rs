use crate::domain::workflow::{
    ChildEntry, ExecutionOrigin, ExecutionParentRef, NodeDefinition, NodeKind, NodeKindName,
    SequenceSpec, WorkflowDefinition, WorkflowEvent,
};
pub const TREE: &str = "00000000-0000-4000-8000-00000000e001";
pub fn definition() -> WorkflowDefinition {
    WorkflowDefinition {
        name: "wf".to_string(),
        description: String::new(),
        builtin: false,
        schemas: Default::default(),
        nodes: vec![
            NodeDefinition {
                name: "a".to_string(),
                ..NodeDefinition::default()
            },
            NodeDefinition {
                name: "run".to_string(),
                kind: NodeKind::Command(crate::domain::workflow::CommandSpec {
                    command: "true".to_string(),
                    env: [(
                        crate::domain::workflow::EnvironmentVariableName::new("DOC").unwrap(),
                        crate::domain::workflow::InputParameterRef::new("document").unwrap(),
                    )]
                    .into_iter()
                    .collect(),
                }),
                input: vec![crate::domain::workflow::InputParam {
                    name: "document".to_string(),
                    contract: None,
                }],
                ..NodeDefinition::default()
            },
            NodeDefinition {
                name: "main".to_string(),
                kind: NodeKind::Sequence(SequenceSpec {
                    entry: None,
                    children: vec![ChildEntry::reference("a"), ChildEntry::reference("run")],
                }),
                ..NodeDefinition::default()
            },
        ],
        entry: "main".to_string(),
    }
}
pub fn started_event() -> WorkflowEvent {
    WorkflowEvent::ExecutionStarted {
        repository_root: None,
        execution_id: TREE.to_string(),
        workflow_name: "wf".to_string(),
        worktree_path: "/repo".to_string(),
        created_from: ExecutionOrigin::Cli,
        request: "please".to_string(),
        definition: definition(),
        timestamp: 1.0,
    }
}
pub fn node_started(
    node_execution_id: &str,
    node_name: &str,
    kind: NodeKindName,
    parent: Option<ExecutionParentRef>,
    timestamp: f64,
) -> WorkflowEvent {
    WorkflowEvent::NodeStarted {
        worktree: None,
        execution_id: TREE.to_string(),
        node_execution_id: node_execution_id.to_string(),
        node_name: node_name.to_string(),
        kind,
        attempt: 1,
        parent,
        timestamp,
    }
}
