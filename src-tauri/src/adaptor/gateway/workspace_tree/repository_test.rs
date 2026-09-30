use super::*;
use crate::adaptor::gateway::agent_session::LocalAgentSessionRepository;
use crate::adaptor::gateway::local_event_store::store::LocalEventStoreConfig;
use crate::domain::agent_session::aggregates::{AgentSession, AgentSessionTreeLocation};
use crate::domain::agent_session::repository::AgentSessionRepository;
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workflow::value_objects::EffectiveRules;
use crate::domain::workflow::{
    ChildEntry, ExecutionOrigin, ExecutionParentRef, FacetRefs, FanoutSpec, ItemsSource,
    NodeCompletion, NodeDefinition, NodeKind, NodeKindName, Rule, SchemaDef, SequenceSpec,
    SessionSpec, WorkflowDefinition, WorkflowEvent,
};
use crate::domain::workspace_tree::{WorkspaceNodeStatus, WorkspaceTreeRepository};

const REPORTED_MAIN_ID: &str = "00000000-0000-4000-8000-000000000710";
const REPORTED_REVIEW_ID: &str = "00000000-0000-4000-8000-000000000711";
const REPORTED_REVIEW_SCAN_1_ID: &str = "00000000-0000-4000-8000-000000000712";
const REPORTED_FANOUT_1_ID: &str = "00000000-0000-4000-8000-000000000713";
const REPORTED_FANOUT_CHILD_1_ID: &str = "00000000-0000-4000-8000-000000000714";
const REPORTED_FIX_ROUND_ID: &str = "00000000-0000-4000-8000-000000000715";
const REPORTED_FIX_STEP_ID: &str = "00000000-0000-4000-8000-000000000716";
const REPORTED_REVIEW_SCAN_2_ID: &str = "00000000-0000-4000-8000-000000000717";
const REPORTED_FANOUT_2_ID: &str = "00000000-0000-4000-8000-000000000718";
const REPORTED_FANOUT_CHILD_2_ID: &str = "00000000-0000-4000-8000-000000000719";
const REPORTED_SESSION_1_ID: &str = "00000000-0000-4000-8000-000000000720";
const REPORTED_SESSION_2_ID: &str = "00000000-0000-4000-8000-000000000721";
const REPORTED_FIX_SESSION_ID: &str = "00000000-0000-4000-8000-000000000722";

fn reported_fanout_definition() -> WorkflowDefinition {
    WorkflowDefinition {
        name: "dev-cycle".to_string(),
        description: String::new(),
        builtin: false,
        schemas: std::collections::BTreeMap::from([
            (
                "review-result".to_string(),
                SchemaDef::String { r#enum: None },
            ),
            (
                "thread-scan".to_string(),
                SchemaDef::Array {
                    items: "review-result".to_string(),
                },
            ),
            ("fix-result".to_string(), SchemaDef::String { r#enum: None }),
        ]),
        nodes: vec![
            NodeDefinition {
                name: "main".to_string(),
                kind: NodeKind::Sequence(SequenceSpec {
                    children: vec![ChildEntry::reference("review")],
                    ..SequenceSpec::default()
                }),
                ..NodeDefinition::default()
            },
            NodeDefinition {
                name: "review".to_string(),
                kind: NodeKind::Sequence(SequenceSpec {
                    children: vec![
                        ChildEntry::reference("review_scan"),
                        ChildEntry {
                            name: "fix_round".to_string(),
                            inputs: Vec::new(),
                            rules: Some(vec![Rule::Next("review_scan".to_string())]),
                        },
                        ChildEntry::reference("implementation_confirmation"),
                    ],
                    ..SequenceSpec::default()
                }),
                ..NodeDefinition::default()
            },
            NodeDefinition {
                name: "review_scan".to_string(),
                kind: NodeKind::Sequence(SequenceSpec {
                    children: vec![ChildEntry::reference("review_fanout")],
                    ..SequenceSpec::default()
                }),
                artifact: Some("thread-scan".to_string()),
                ..NodeDefinition::default()
            },
            NodeDefinition {
                name: "review_fanout".to_string(),
                kind: NodeKind::Fanout(FanoutSpec {
                    children: vec![ChildEntry::reference("review_acceptance_opus")],
                    items: None,
                }),
                ..NodeDefinition::default()
            },
            NodeDefinition {
                name: "review_acceptance_opus".to_string(),
                kind: NodeKind::Session(SessionSpec::default()),
                artifact: Some("review-result".to_string()),
                completion: NodeCompletion::require_approval(),
                ..NodeDefinition::default()
            },
            NodeDefinition {
                name: "fix_round".to_string(),
                kind: NodeKind::Sequence(SequenceSpec {
                    children: vec![ChildEntry::reference("fix_step")],
                    ..SequenceSpec::default()
                }),
                artifact: Some("fix-result".to_string()),
                ..NodeDefinition::default()
            },
            NodeDefinition {
                name: "fix_step".to_string(),
                kind: NodeKind::Session(SessionSpec::default()),
                artifact: Some("fix-result".to_string()),
                ..NodeDefinition::default()
            },
            NodeDefinition {
                name: "implementation_confirmation".to_string(),
                kind: NodeKind::Session(SessionSpec::default()),
                ..NodeDefinition::default()
            },
        ],
        entry: "main".to_string(),
    }
}

fn reported_fanout_events(execution_id: &str, workspace: &str) -> Vec<WorkflowEvent> {
    let review_result = serde_json::json!("open");
    let review_fanout_result = serde_json::json!(["open"]);
    let fix_result = serde_json::json!("fixed");
    vec![
        WorkflowEvent::ExecutionStarted {
            repository_root: None,
            execution_id: execution_id.to_string(),
            workflow_name: "dev-cycle".to_string(),
            worktree_path: workspace.to_string(),
            created_from: ExecutionOrigin::DesktopUi,
            request: "https://github.com/siro33950/releash/issues/1696".to_string(),
            definition: reported_fanout_definition(),
            timestamp: 1.0,
        },
        WorkflowEvent::NodeStarted {
            worktree: None,
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_MAIN_ID.to_string(),
            node_name: "main".to_string(),
            kind: NodeKindName::Sequence,
            attempt: 1,
            parent: None,
            timestamp: 2.0,
        },
        WorkflowEvent::NodeStarted {
            worktree: None,
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_REVIEW_ID.to_string(),
            node_name: "review".to_string(),
            kind: NodeKindName::Sequence,
            attempt: 1,
            parent: Some(ExecutionParentRef::sequence_child(REPORTED_MAIN_ID)),
            timestamp: 2.0,
        },
        WorkflowEvent::NodeStarted {
            worktree: None,
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_REVIEW_SCAN_1_ID.to_string(),
            node_name: "review_scan".to_string(),
            kind: NodeKindName::Sequence,
            attempt: 1,
            parent: Some(ExecutionParentRef::sequence_child(REPORTED_REVIEW_ID)),
            timestamp: 2.0,
        },
        WorkflowEvent::NodeStarted {
            worktree: None,
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_FANOUT_1_ID.to_string(),
            node_name: "review_fanout".to_string(),
            kind: NodeKindName::Fanout,
            attempt: 1,
            parent: Some(ExecutionParentRef::sequence_child(
                REPORTED_REVIEW_SCAN_1_ID,
            )),
            timestamp: 2.0,
        },
        WorkflowEvent::NodeStarted {
            worktree: None,
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_FANOUT_CHILD_1_ID.to_string(),
            node_name: "review_acceptance_opus".to_string(),
            kind: NodeKindName::Session,
            attempt: 1,
            parent: Some(ExecutionParentRef::fanout_child(
                REPORTED_FANOUT_1_ID,
                None,
                0,
            )),
            timestamp: 2.0,
        },
        WorkflowEvent::SessionAttached {
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_FANOUT_CHILD_1_ID.to_string(),
            session_id: REPORTED_SESSION_1_ID.to_string(),
            timestamp: 3.0,
        },
        WorkflowEvent::NodeSubmitReceived {
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_FANOUT_CHILD_1_ID.to_string(),
            timestamp: 4.0,
        },
        WorkflowEvent::ArtifactProduced {
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_FANOUT_CHILD_1_ID.to_string(),
            node_name: "review_acceptance_opus".to_string(),
            contract: Some("review-result".to_string()),
            value: review_result.clone(),
            request_id: None,
            submitted_at: None,
            timestamp: 4.0,
        },
        WorkflowEvent::NodeStopReceived {
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_FANOUT_CHILD_1_ID.to_string(),
            timestamp: 5.0,
        },
        WorkflowEvent::ApprovalRequested {
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_FANOUT_CHILD_1_ID.to_string(),
            node_name: "review_acceptance_opus".to_string(),
            result_summary: None,
            timestamp: 5.0,
        },
        WorkflowEvent::ApprovalResolved {
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_FANOUT_CHILD_1_ID.to_string(),
            node_name: "review_acceptance_opus".to_string(),
            comment: None,
            timestamp: 6.0,
        },
        WorkflowEvent::NodeCompleted {
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_FANOUT_CHILD_1_ID.to_string(),
            node_name: "review_acceptance_opus".to_string(),
            attempt: 1,
            result_summary: Some("open".to_string()),
            token_usage: None,
            timestamp: 6.0,
        },
        WorkflowEvent::ArtifactProduced {
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_FANOUT_1_ID.to_string(),
            node_name: "review_fanout".to_string(),
            contract: None,
            value: review_fanout_result.clone(),
            request_id: None,
            submitted_at: None,
            timestamp: 6.0,
        },
        WorkflowEvent::NodeCompleted {
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_FANOUT_1_ID.to_string(),
            node_name: "review_fanout".to_string(),
            attempt: 1,
            result_summary: Some("complete".to_string()),
            token_usage: Some(Default::default()),
            timestamp: 6.0,
        },
        WorkflowEvent::ArtifactProduced {
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_REVIEW_SCAN_1_ID.to_string(),
            node_name: "review_scan".to_string(),
            contract: None,
            value: review_fanout_result,
            request_id: None,
            submitted_at: None,
            timestamp: 6.0,
        },
        WorkflowEvent::NodeCompleted {
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_REVIEW_SCAN_1_ID.to_string(),
            node_name: "review_scan".to_string(),
            attempt: 1,
            result_summary: Some("complete".to_string()),
            token_usage: None,
            timestamp: 6.0,
        },
        WorkflowEvent::NodeStarted {
            worktree: None,
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_FIX_ROUND_ID.to_string(),
            node_name: "fix_round".to_string(),
            kind: NodeKindName::Sequence,
            attempt: 1,
            parent: Some(ExecutionParentRef::sequence_child(REPORTED_REVIEW_ID)),
            timestamp: 6.0,
        },
        WorkflowEvent::NodeStarted {
            worktree: None,
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_FIX_STEP_ID.to_string(),
            node_name: "fix_step".to_string(),
            kind: NodeKindName::Session,
            attempt: 1,
            parent: Some(ExecutionParentRef::sequence_child(REPORTED_FIX_ROUND_ID)),
            timestamp: 6.0,
        },
        WorkflowEvent::SessionAttached {
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_FIX_STEP_ID.to_string(),
            session_id: REPORTED_FIX_SESSION_ID.to_string(),
            timestamp: 7.0,
        },
        WorkflowEvent::NodeSubmitReceived {
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_FIX_STEP_ID.to_string(),
            timestamp: 8.0,
        },
        WorkflowEvent::ArtifactProduced {
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_FIX_STEP_ID.to_string(),
            node_name: "fix_step".to_string(),
            contract: Some("fix-result".to_string()),
            value: fix_result.clone(),
            request_id: None,
            submitted_at: None,
            timestamp: 8.0,
        },
        WorkflowEvent::NodeStopReceived {
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_FIX_STEP_ID.to_string(),
            timestamp: 9.0,
        },
        WorkflowEvent::NodeCompleted {
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_FIX_STEP_ID.to_string(),
            node_name: "fix_step".to_string(),
            attempt: 1,
            result_summary: Some("fixed".to_string()),
            token_usage: None,
            timestamp: 9.0,
        },
        WorkflowEvent::ArtifactProduced {
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_FIX_ROUND_ID.to_string(),
            node_name: "fix_round".to_string(),
            contract: Some("fix-result".to_string()),
            value: fix_result,
            request_id: None,
            submitted_at: None,
            timestamp: 9.0,
        },
        WorkflowEvent::NodeCompleted {
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_FIX_ROUND_ID.to_string(),
            node_name: "fix_round".to_string(),
            attempt: 1,
            result_summary: Some("fixed".to_string()),
            token_usage: None,
            timestamp: 9.0,
        },
        WorkflowEvent::NodeStarted {
            worktree: None,
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_REVIEW_SCAN_2_ID.to_string(),
            node_name: "review_scan".to_string(),
            kind: NodeKindName::Sequence,
            attempt: 2,
            parent: Some(ExecutionParentRef::sequence_child(REPORTED_REVIEW_ID)),
            timestamp: 9.0,
        },
        WorkflowEvent::NodeStarted {
            worktree: None,
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_FANOUT_2_ID.to_string(),
            node_name: "review_fanout".to_string(),
            kind: NodeKindName::Fanout,
            attempt: 1,
            parent: Some(ExecutionParentRef::sequence_child(
                REPORTED_REVIEW_SCAN_2_ID,
            )),
            timestamp: 9.0,
        },
        WorkflowEvent::NodeStarted {
            worktree: None,
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_FANOUT_CHILD_2_ID.to_string(),
            node_name: "review_acceptance_opus".to_string(),
            kind: NodeKindName::Session,
            attempt: 1,
            parent: Some(ExecutionParentRef::fanout_child(
                REPORTED_FANOUT_2_ID,
                None,
                0,
            )),
            timestamp: 9.0,
        },
        WorkflowEvent::SessionAttached {
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_FANOUT_CHILD_2_ID.to_string(),
            session_id: REPORTED_SESSION_2_ID.to_string(),
            timestamp: 10.0,
        },
        WorkflowEvent::NodeSubmitReceived {
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_FANOUT_CHILD_2_ID.to_string(),
            timestamp: 11.0,
        },
        WorkflowEvent::ArtifactProduced {
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_FANOUT_CHILD_2_ID.to_string(),
            node_name: "review_acceptance_opus".to_string(),
            contract: Some("review-result".to_string()),
            value: review_result,
            request_id: None,
            submitted_at: None,
            timestamp: 11.0,
        },
        WorkflowEvent::NodeStopReceived {
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_FANOUT_CHILD_2_ID.to_string(),
            timestamp: 12.0,
        },
        WorkflowEvent::ApprovalRequested {
            execution_id: execution_id.to_string(),
            node_execution_id: REPORTED_FANOUT_CHILD_2_ID.to_string(),
            node_name: "review_acceptance_opus".to_string(),
            result_summary: None,
            timestamp: 12.0,
        },
    ]
}
fn fanout_with_sequence_child_definition() -> WorkflowDefinition {
    WorkflowDefinition {
        name: "multi-execution".to_string(),
        description: String::new(),
        builtin: false,
        schemas: Default::default(),
        nodes: vec![
            NodeDefinition {
                name: "reviews".to_string(),
                kind: NodeKind::Fanout(FanoutSpec {
                    children: vec![ChildEntry::reference("review-sequence")],
                    items: Some(ItemsSource::Literal(vec![serde_json::json!("only")])),
                }),
                ..NodeDefinition::default()
            },
            NodeDefinition {
                name: "review-sequence".to_string(),
                kind: NodeKind::Sequence(SequenceSpec::default()),
                ..NodeDefinition::default()
            },
        ],
        entry: "reviews".to_string(),
    }
}

fn duplicate_session_binding_events(execution_id: &str, workspace: &str) -> Vec<WorkflowEvent> {
    let definition = WorkflowDefinition {
        name: "duplicate-session-binding".to_string(),
        description: String::new(),
        builtin: false,
        schemas: Default::default(),
        nodes: vec![
            NodeDefinition {
                name: "steps".to_string(),
                kind: NodeKind::Sequence(SequenceSpec {
                    children: vec![
                        ChildEntry::reference("plan"),
                        ChildEntry::reference("implement"),
                    ],
                    ..SequenceSpec::default()
                }),
                ..NodeDefinition::default()
            },
            NodeDefinition {
                name: "plan".to_string(),
                kind: NodeKind::Session(SessionSpec::default()),
                ..NodeDefinition::default()
            },
            NodeDefinition {
                name: "implement".to_string(),
                kind: NodeKind::Session(SessionSpec::default()),
                ..NodeDefinition::default()
            },
        ],
        entry: "steps".to_string(),
    };
    let duplicate_session_id = format!("{execution_id}-session");
    vec![
        WorkflowEvent::ExecutionStarted {
            repository_root: None,
            execution_id: execution_id.to_string(),
            workflow_name: "duplicate-session-binding".to_string(),
            worktree_path: workspace.to_string(),
            created_from: ExecutionOrigin::DesktopUi,
            request: "duplicate Session binding".to_string(),
            definition,
            timestamp: 1.0,
        },
        WorkflowEvent::NodeStarted {
            worktree: None,
            execution_id: execution_id.to_string(),
            node_execution_id: format!("{execution_id}-steps"),
            node_name: "steps".to_string(),
            kind: NodeKindName::Sequence,
            attempt: 1,
            parent: None,
            timestamp: 2.0,
        },
        WorkflowEvent::NodeStarted {
            worktree: None,
            execution_id: execution_id.to_string(),
            node_execution_id: format!("{execution_id}-plan"),
            node_name: "plan".to_string(),
            kind: NodeKindName::Session,
            attempt: 1,
            parent: Some(ExecutionParentRef::sequence_child(format!(
                "{execution_id}-steps"
            ))),
            timestamp: 3.0,
        },
        WorkflowEvent::SessionAttached {
            execution_id: execution_id.to_string(),
            node_execution_id: format!("{execution_id}-plan"),
            session_id: duplicate_session_id.clone(),
            timestamp: 4.0,
        },
        WorkflowEvent::NodeStarted {
            worktree: None,
            execution_id: execution_id.to_string(),
            node_execution_id: format!("{execution_id}-implement"),
            node_name: "implement".to_string(),
            kind: NodeKindName::Session,
            attempt: 1,
            parent: Some(ExecutionParentRef::sequence_child(format!(
                "{execution_id}-steps"
            ))),
            timestamp: 5.0,
        },
        WorkflowEvent::SessionAttached {
            execution_id: execution_id.to_string(),
            node_execution_id: format!("{execution_id}-implement"),
            session_id: duplicate_session_id,
            timestamp: 6.0,
        },
    ]
}

fn fanout_with_sequence_child_events(execution_id: &str, workspace: &str) -> Vec<WorkflowEvent> {
    vec![
        WorkflowEvent::ExecutionStarted {
            repository_root: None,
            execution_id: execution_id.to_string(),
            workflow_name: "multi-execution".to_string(),
            worktree_path: workspace.to_string(),
            created_from: ExecutionOrigin::DesktopUi,
            request: "review".to_string(),
            definition: fanout_with_sequence_child_definition(),
            timestamp: 1.0,
        },
        WorkflowEvent::NodeStarted {
            worktree: None,
            execution_id: execution_id.to_string(),
            node_execution_id: format!("{execution_id}-reviews"),
            node_name: "reviews".to_string(),
            kind: NodeKindName::Fanout,
            attempt: 1,
            parent: None,
            timestamp: 2.0,
        },
        WorkflowEvent::NodeStarted {
            worktree: None,
            execution_id: execution_id.to_string(),
            node_execution_id: format!("{execution_id}-review-sequence"),
            node_name: "review-sequence".to_string(),
            kind: NodeKindName::Sequence,
            attempt: 1,
            parent: Some(ExecutionParentRef::fanout_child(
                format!("{execution_id}-reviews"),
                Some(0),
                0,
            )),
            timestamp: 3.0,
        },
    ]
}

fn dynamic_fanout_with_sequence_child_definition() -> WorkflowDefinition {
    WorkflowDefinition {
        name: "dynamic-multi-execution".to_string(),
        description: String::new(),
        builtin: false,
        schemas: Default::default(),
        nodes: vec![
            NodeDefinition {
                name: "main".to_string(),
                kind: NodeKind::Sequence(SequenceSpec {
                    children: vec![
                        ChildEntry::reference("source"),
                        ChildEntry::reference("reviews"),
                    ],
                    ..SequenceSpec::default()
                }),
                ..NodeDefinition::default()
            },
            NodeDefinition {
                name: "source".to_string(),
                kind: NodeKind::Session(SessionSpec::default()),
                ..NodeDefinition::default()
            },
            NodeDefinition {
                name: "reviews".to_string(),
                kind: NodeKind::Fanout(FanoutSpec {
                    children: vec![ChildEntry::reference("review-sequence")],
                    items: Some(ItemsSource::ArtifactField {
                        node: "source".to_string(),
                        field_path: crate::domain::workflow::FieldPath::new(["items"]),
                    }),
                }),
                ..NodeDefinition::default()
            },
            NodeDefinition {
                name: "review-sequence".to_string(),
                kind: NodeKind::Sequence(SequenceSpec::default()),
                ..NodeDefinition::default()
            },
        ],
        entry: "main".to_string(),
    }
}

fn dynamic_fanout_with_sequence_child_events(
    execution_id: &str,
    workspace: &str,
) -> Vec<WorkflowEvent> {
    vec![
        WorkflowEvent::ExecutionStarted {
            repository_root: None,
            execution_id: execution_id.to_string(),
            workflow_name: "dynamic-multi-execution".to_string(),
            worktree_path: workspace.to_string(),
            created_from: ExecutionOrigin::DesktopUi,
            request: "review".to_string(),
            definition: dynamic_fanout_with_sequence_child_definition(),
            timestamp: 1.0,
        },
        WorkflowEvent::NodeStarted {
            worktree: None,
            execution_id: execution_id.to_string(),
            node_execution_id: format!("{execution_id}-main"),
            node_name: "main".to_string(),
            kind: NodeKindName::Sequence,
            attempt: 1,
            parent: None,
            timestamp: 2.0,
        },
        WorkflowEvent::NodeStarted {
            worktree: None,
            execution_id: execution_id.to_string(),
            node_execution_id: format!("{execution_id}-source"),
            node_name: "source".to_string(),
            kind: NodeKindName::Session,
            attempt: 1,
            parent: Some(ExecutionParentRef::sequence_child(format!(
                "{execution_id}-main"
            ))),
            timestamp: 3.0,
        },
        WorkflowEvent::ArtifactProduced {
            execution_id: execution_id.to_string(),
            node_execution_id: format!("{execution_id}-source"),
            node_name: "source".to_string(),
            contract: None,
            value: serde_json::json!({"items": ["only"]}),
            request_id: None,
            submitted_at: None,
            timestamp: 4.0,
        },
        WorkflowEvent::NodeCompleted {
            execution_id: execution_id.to_string(),
            node_execution_id: format!("{execution_id}-source"),
            node_name: "source".to_string(),
            attempt: 1,
            result_summary: None,
            token_usage: None,
            timestamp: 4.0,
        },
        WorkflowEvent::NodeStarted {
            worktree: None,
            execution_id: execution_id.to_string(),
            node_execution_id: format!("{execution_id}-reviews"),
            node_name: "reviews".to_string(),
            kind: NodeKindName::Fanout,
            attempt: 1,
            parent: Some(ExecutionParentRef::sequence_child(format!(
                "{execution_id}-main"
            ))),
            timestamp: 4.0,
        },
        WorkflowEvent::NodeStarted {
            worktree: None,
            execution_id: execution_id.to_string(),
            node_execution_id: format!("{execution_id}-review-sequence"),
            node_name: "review-sequence".to_string(),
            kind: NodeKindName::Sequence,
            attempt: 1,
            parent: Some(ExecutionParentRef::fanout_child(
                format!("{execution_id}-reviews"),
                Some(0),
                0,
            )),
            timestamp: 4.0,
        },
    ]
}

#[test]
fn test_workspace復元失敗_本番の変換でqueryとcodecの分類を保持する() {
    use crate::adaptor::gateway::local_event_store::{
        reader::storage_unavailable, test_helpers::ReadFailure,
    };
    use crate::adaptor::presenter::connect::classified_error;
    // Given
    for (failure, expected) in ReadFailure::cases() {
        let error = match failure {
            ReadFailure::Query(error) => error,
            ReadFailure::Sqlite(code) => storage_unavailable(&rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(code),
                None,
            )),
        };
        // When / Then
        assert_eq!(
            classified_error(fold_query_error(fact_log::FactReadError::Query(error))).code,
            expected
        );
    }
    assert_eq!(
        classified_error(fold_query_error(fact_log::FactReadError::Corrupt(
            "invalid stored value".into()
        )))
        .code,
        connectrpc::ErrorCode::DataLoss
    );
}

#[tokio::test]
async fn test_workspace_tree読み出し_報告実例の後方辺fanout既存fact列から介入対象を解決する() {
    // Given
    let directory = tempfile::TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let workspace = WorkspaceIdentity::new("/repo/.worktrees/feat-issues-1696");
    let execution_id = "97c31282-c12a-4163-b6f8-6735b78c73cf";
    let definition = reported_fanout_definition();
    let review = definition
        .node_by_name("review")
        .unwrap()
        .sequence()
        .unwrap();
    assert!(!matches!(
        review.effective_rules("fix_round"),
        EffectiveRules::Terminal
    ));
    let events = reported_fanout_events(execution_id, workspace.as_str());
    crate::adaptor::gateway::workflow::test_support::append_canonical_events(&store, &events)
        .await
        .unwrap();
    let rows_before = fact_log::read_tree_records(&store, execution_id)
        .await
        .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(Arc::clone(&store));

    // When
    let folded = repository
        .folded_workspace_trees(workspace.as_str())
        .await
        .unwrap();
    let tree = repository
        .workspace_tree_from_folded(workspace.as_str(), &folded)
        .unwrap()
        .unwrap();
    let waiting_execution_id = REPORTED_FANOUT_CHILD_2_ID;
    let waiting_node = tree
        .nodes()
        .iter()
        .find(|node| node.node_execution_id.as_deref() == Some(waiting_execution_id))
        .unwrap();
    let loaded_waiting = repository
        .load_node(&workspace, &waiting_node.id)
        .await
        .unwrap()
        .unwrap();
    let loaded_by_execution = repository
        .load_node_by_node_execution_id(waiting_execution_id)
        .await
        .unwrap()
        .unwrap();
    let session_id = REPORTED_SESSION_2_ID;
    let session_node_id = repository
        .node_id_for_session(&workspace, session_id)
        .await
        .unwrap()
        .unwrap();
    let loaded_session = repository
        .load_node(&workspace, &session_node_id)
        .await
        .unwrap()
        .unwrap();
    let rows_after = fact_log::read_tree_records(&store, execution_id)
        .await
        .unwrap();

    // Then
    let ids = tree
        .nodes()
        .iter()
        .map(|node| node.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(ids.len(), tree.nodes().len());
    let fanout_children = tree
        .nodes()
        .iter()
        .filter(|node| node.node_name.as_deref() == Some("review_acceptance_opus"))
        .collect::<Vec<_>>();
    assert_eq!(fanout_children.len(), 2);
    assert_ne!(fanout_children[0].id, fanout_children[1].id);
    assert_ne!(fanout_children[0].parent_id, fanout_children[1].parent_id);
    assert_eq!(
        loaded_waiting.node_execution_id,
        Some(waiting_execution_id.to_string())
    );
    assert_eq!(loaded_by_execution.id, waiting_node.id);
    assert_eq!(loaded_waiting.status, WorkspaceNodeStatus::Waiting);
    assert!(loaded_waiting.can_approve);
    assert_eq!(session_node_id, waiting_node.id);
    assert_eq!(
        loaded_session.node_execution_id,
        Some(waiting_execution_id.to_string())
    );
    assert_eq!(loaded_session.session_id, Some(session_id.to_string()));
    assert_eq!(rows_after, rows_before);
}

#[tokio::test]
async fn test_ツリー読み出し_command形のsession成果物を含む複数実行木と各nodeをfact変更なしで読める(
) {
    // Given
    let directory = tempfile::TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let workspace = WorkspaceIdentity::new("/repo/.worktrees/command-shaped-artifact");
    let execution_ids = [
        "00000000-0000-4000-8000-000000000743",
        "00000000-0000-4000-8000-000000000744",
    ];
    let definition = WorkflowDefinition {
        name: "session-artifact".to_string(),
        schemas: std::collections::BTreeMap::from([(
            "report-result".to_string(),
            SchemaDef::Object {
                properties: std::collections::BTreeMap::from([
                    ("exit_code".to_string(), SchemaDef::Integer),
                    ("duration".to_string(), SchemaDef::Integer),
                    ("stdout".to_string(), SchemaDef::String { r#enum: None }),
                    ("stderr".to_string(), SchemaDef::String { r#enum: None }),
                ]),
                required: ["exit_code", "duration", "stdout", "stderr"]
                    .map(str::to_string)
                    .into_iter()
                    .collect(),
            },
        )]),
        nodes: vec![NodeDefinition {
            name: "report".to_string(),
            kind: NodeKind::Session(SessionSpec {
                facets: FacetRefs {
                    instruction: Some("report".to_string()),
                    ..FacetRefs::default()
                },
                ..SessionSpec::default()
            }),
            artifact: Some("report-result".to_string()),
            ..NodeDefinition::default()
        }],
        entry: "report".to_string(),
        ..WorkflowDefinition::default()
    };
    crate::domain::workflow::services::validation::validate(&definition).unwrap();
    for execution_id in execution_ids {
        let node_execution_id = format!("{execution_id}-report");
        let mut events = vec![
            WorkflowEvent::ExecutionStarted {
                repository_root: None,
                execution_id: execution_id.to_string(),
                workflow_name: definition.name.clone(),
                worktree_path: workspace.as_str().to_string(),
                created_from: ExecutionOrigin::DesktopUi,
                request: "report".to_string(),
                definition: definition.clone(),
                timestamp: 1.0,
            },
            WorkflowEvent::NodeStarted {
                worktree: None,
                execution_id: execution_id.to_string(),
                node_execution_id: node_execution_id.clone(),
                node_name: "report".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                parent: None,
                timestamp: 2.0,
            },
            WorkflowEvent::SessionAttached {
                execution_id: execution_id.to_string(),
                node_execution_id: node_execution_id.clone(),
                session_id: format!("{execution_id}-session"),
                timestamp: 3.0,
            },
        ];
        if execution_id == execution_ids[0] {
            events.extend([
                WorkflowEvent::NodeSubmitReceived {
                    execution_id: execution_id.to_string(),
                    node_execution_id: node_execution_id.clone(),
                    timestamp: 4.0,
                },
                WorkflowEvent::ArtifactProduced {
                    execution_id: execution_id.to_string(),
                    node_execution_id: node_execution_id.clone(),
                    node_name: "report".to_string(),
                    contract: Some("report-result".to_string()),
                    value: serde_json::json!({
                        "exit_code": 0,
                        "duration": 123,
                        "stdout": "output",
                        "stderr": ""
                    }),
                    request_id: None,
                    submitted_at: None,
                    timestamp: 4.0,
                },
                WorkflowEvent::NodeStopReceived {
                    execution_id: execution_id.to_string(),
                    node_execution_id,
                    timestamp: 5.0,
                },
            ]);
        }
        crate::adaptor::gateway::workflow::test_support::append_canonical_events(&store, &events)
            .await
            .unwrap();
    }
    let rows_before = futures_util::future::join_all(execution_ids.map(|execution_id| async {
        fact_log::read_tree_records(&store, execution_id)
            .await
            .unwrap()
    }))
    .await;
    let repository = SqliteWorkspaceTreeRepository::new(Arc::clone(&store));

    // When
    let folded = repository
        .folded_workspace_trees(workspace.as_str())
        .await
        .unwrap();
    let tree = repository
        .workspace_tree_from_folded(workspace.as_str(), &folded)
        .unwrap()
        .unwrap();
    let mut loaded_nodes = Vec::new();
    for execution_id in execution_ids {
        let node_execution_id = format!("{execution_id}-report");
        let node = tree
            .nodes()
            .iter()
            .find(|node| node.node_execution_id.as_deref() == Some(&node_execution_id))
            .unwrap();
        loaded_nodes.push((
            node,
            repository
                .load_node(&workspace, &node.id)
                .await
                .unwrap()
                .unwrap(),
            repository
                .load_node_by_node_execution_id(&node_execution_id)
                .await
                .unwrap()
                .unwrap(),
        ));
    }
    let rows_after = futures_util::future::join_all(execution_ids.map(|execution_id| async {
        fact_log::read_tree_records(&store, execution_id)
            .await
            .unwrap()
    }))
    .await;

    // Then
    assert_eq!(folded.len(), 2);
    assert_eq!(tree.nodes().len(), 4);
    for execution_id in execution_ids {
        assert_eq!(
            tree.nodes()
                .iter()
                .filter(|node| node.execution_id.as_deref() == Some(execution_id))
                .count(),
            2
        );
    }
    for (node, loaded_by_id, loaded_by_execution) in &loaded_nodes {
        assert_eq!(*node, loaded_by_id);
        assert_eq!(*node, loaded_by_execution);
        assert_eq!(node.command_result, None);
    }
    assert!(loaded_nodes[0].0.has_artifact);
    assert_eq!(loaded_nodes[0].0.status, WorkspaceNodeStatus::Completed);
    assert!(!loaded_nodes[1].0.has_artifact);
    assert_eq!(loaded_nodes[1].0.status, WorkspaceNodeStatus::Running);
    assert_eq!(rows_after, rows_before);
}

#[tokio::test]
async fn test_workspace_tree読み出し_同一worktreeの複数executionでfanout子sequenceが衝突しない() {
    // Given
    let directory = tempfile::TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let workspace = WorkspaceIdentity::new("/repo/.worktrees/multiple-executions");
    let first_execution_id = "00000000-0000-4000-8000-000000000702";
    let second_execution_id = "00000000-0000-4000-8000-000000000703";
    crate::adaptor::gateway::workflow::test_support::append_canonical_events(
        &store,
        &fanout_with_sequence_child_events(first_execution_id, workspace.as_str()),
    )
    .await
    .unwrap();
    crate::adaptor::gateway::workflow::test_support::append_canonical_events(
        &store,
        &fanout_with_sequence_child_events(second_execution_id, workspace.as_str()),
    )
    .await
    .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(store);

    // When
    let folded = repository
        .folded_workspace_trees(workspace.as_str())
        .await
        .unwrap();
    let tree = repository
        .workspace_tree_from_folded(workspace.as_str(), &folded)
        .unwrap()
        .unwrap();

    // Then
    assert_eq!(folded.len(), 2);
    let ids = tree
        .nodes()
        .iter()
        .map(|node| node.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(ids.len(), tree.nodes().len());
    let sequence_ids = [first_execution_id, second_execution_id].map(|execution_id| {
        let node_execution_id = format!("{execution_id}-review-sequence");
        tree.nodes()
            .iter()
            .find(|node| node.node_execution_id.as_deref() == Some(node_execution_id.as_str()))
            .unwrap()
            .id
            .as_str()
    });
    assert_ne!(sequence_ids[0], sequence_ids[1]);
}

#[tokio::test]
async fn test_workspace_tree読み出し_同一worktreeの複数executionで動的fanout子sequenceが衝突しない()
{
    // Given
    let directory = tempfile::TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let workspace = WorkspaceIdentity::new("/repo/.worktrees/dynamic-multiple-executions");
    let first_execution_id = "00000000-0000-4000-8000-000000000705";
    let second_execution_id = "00000000-0000-4000-8000-000000000706";
    crate::adaptor::gateway::workflow::test_support::append_canonical_events(
        &store,
        &dynamic_fanout_with_sequence_child_events(first_execution_id, workspace.as_str()),
    )
    .await
    .unwrap();
    crate::adaptor::gateway::workflow::test_support::append_canonical_events(
        &store,
        &dynamic_fanout_with_sequence_child_events(second_execution_id, workspace.as_str()),
    )
    .await
    .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(store);

    // When
    let folded = repository
        .folded_workspace_trees(workspace.as_str())
        .await
        .unwrap();
    let tree = repository
        .workspace_tree_from_folded(workspace.as_str(), &folded)
        .unwrap()
        .unwrap();

    // Then
    assert_eq!(folded.len(), 2);
    let ids = tree
        .nodes()
        .iter()
        .map(|node| node.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(ids.len(), tree.nodes().len());
    let sequence_ids = [first_execution_id, second_execution_id].map(|execution_id| {
        let node_execution_id = format!("{execution_id}-review-sequence");
        tree.nodes()
            .iter()
            .find(|node| node.node_execution_id.as_deref() == Some(node_execution_id.as_str()))
            .unwrap()
            .id
            .as_str()
    });
    assert_ne!(sequence_ids[0], sequence_ids[1]);
}

#[tokio::test]
async fn test_workspace_tree読み出し_session二重束縛をcorruptとして拒否する() {
    // Given
    let directory = tempfile::TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let workspace = WorkspaceIdentity::new("/repo/.worktrees/corrupt-session-binding");
    let execution_id = "00000000-0000-4000-8000-000000000704";
    let events = duplicate_session_binding_events(execution_id, workspace.as_str());
    crate::adaptor::gateway::workflow::test_support::append_canonical_events(&store, &events)
        .await
        .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(store);

    // When
    let folded = repository
        .folded_workspace_trees(workspace.as_str())
        .await
        .unwrap();
    let error = repository
        .workspace_tree_from_folded(workspace.as_str(), &folded)
        .unwrap_err();

    // Then
    let LocalEventQueryError::Corrupt { correlation_id } = error else {
        panic!("duplicate Session binding must be classified as corrupt");
    };
    assert!(!correlation_id.is_empty());
}

#[tokio::test]
async fn public_session_root_id_loads_the_session_node_instead_of_the_internal_owner() {
    let directory = tempfile::TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let workspace = WorkspaceIdentity::new("/repo/.worktrees/feature");
    let session = AgentSession::create(
        "agent-session-1",
        workspace.clone(),
        workspace.as_str(),
        ProviderKind::Codex,
        AgentSessionTreeLocation::session_tree_root("agent-session-1").unwrap(),
    )
    .unwrap();
    LocalAgentSessionRepository::new(Arc::clone(&store))
        .create(session, "create-request-1")
        .await
        .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(store);

    let node_id = repository
        .node_id_for_session(&workspace, "agent-session-1")
        .await
        .unwrap()
        .expect("the standalone Session must have a public Node id");
    let node = repository
        .load_node(&workspace, &node_id)
        .await
        .unwrap()
        .expect("the public Node id must resolve");

    assert_eq!(node_id, "agent-session-1");
    assert_eq!(
        node.kind,
        crate::domain::workspace_tree::WorkspaceNodeKind::WorkflowSession
    );
    assert_eq!(node.node_execution_id.as_deref(), Some("agent-session-1"));
    assert_eq!(node.session_id.as_deref(), Some("agent-session-1"));
    assert!(!node.can_retry);
}

#[tokio::test]
async fn test_workspace_tree_repository_workspace同定子がworktreeと異なるsession木を解決する() {
    // Given: workspace identity と worktree path が異なる Session 起動由来の木
    let directory = tempfile::TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let workspace = WorkspaceIdentity::new("workspace-1");
    LocalAgentSessionRepository::new(Arc::clone(&store))
        .create(
            AgentSession::create(
                "agent-session-workspace-identity",
                workspace.clone(),
                "/repo/.worktrees/feature",
                ProviderKind::Codex,
                AgentSessionTreeLocation::session_tree_root("agent-session-workspace-identity")
                    .unwrap(),
            )
            .unwrap(),
            "create-workspace-identity-session",
        )
        .await
        .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(store);

    // When: workspace identity から木、Session の公開 node、詳細を引く
    let trees = repository
        .folded_workspace_trees(workspace.as_str())
        .await
        .unwrap();
    let node_id = repository
        .node_id_for_session(&workspace, "agent-session-workspace-identity")
        .await
        .unwrap()
        .unwrap();
    let node = repository
        .load_node(&workspace, &node_id)
        .await
        .unwrap()
        .unwrap();

    // Then: worktree path ではなく root の workspace identity で一貫して解決する
    assert_eq!(trees.len(), 1);
    assert_eq!(trees[0].0.root.workspace_identity, "workspace-1");
    assert_eq!(trees[0].0.root.worktree_path, "/repo/.worktrees/feature");
    assert_eq!(node_id, "agent-session-workspace-identity");
    assert_eq!(
        node.node_execution_id.as_deref(),
        Some("agent-session-workspace-identity")
    );
    assert_eq!(
        node.session_id.as_deref(),
        Some("agent-session-workspace-identity")
    );
}

#[tokio::test]
async fn a_session_owned_by_another_worktree_has_no_public_node_id() {
    let directory = tempfile::TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let owner = WorkspaceIdentity::new("/repo/.worktrees/feature");
    let other = WorkspaceIdentity::new("/repo/.worktrees/other");
    let session = AgentSession::create(
        "agent-session-1",
        owner.clone(),
        owner.as_str(),
        ProviderKind::Codex,
        AgentSessionTreeLocation::session_tree_root("agent-session-1").unwrap(),
    )
    .unwrap();
    LocalAgentSessionRepository::new(Arc::clone(&store))
        .create(session, "create-request-1")
        .await
        .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(store);

    assert!(
        repository
            .node_id_for_session(&other, "agent-session-1")
            .await
            .unwrap()
            .is_none(),
        "another Worktree must not resolve a public Node id for this Session"
    );
    assert!(
        repository
            .node_id_for_session(&owner, "unknown-session")
            .await
            .unwrap()
            .is_none(),
        "an unknown Session has no execution tree to publish"
    );
}

#[tokio::test]
async fn test_workspace_repository読取_実経路で失敗分類を保持する() {
    use crate::adaptor::gateway::local_event_store::test_helpers::ReadFailure;
    use crate::adaptor::presenter::connect::classified_error;
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store =
        LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into())).unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(store.clone());
    for (failure, expected) in ReadFailure::cases() {
        store.fail_next_read(failure);
        // When
        let error = repository
            .load_node_by_node_execution_id("node")
            .await
            .unwrap_err();
        // Then
        assert_eq!(classified_error(error).code, expected);
    }
}

#[tokio::test]
async fn test_workspace_tree読取_delegate親はworkingの子から青を導出する() {
    // Given
    let activity = crate::domain::workflow::AgentSessionActivity::Working;
    // When
    let status = delegate_parent_status_from_store(activity).await;
    // Then
    assert_eq!(
        status,
        crate::domain::workspace_tree::WorkspaceNodeStatusClassification::Active
    );
}

#[tokio::test]
async fn test_workspace_tree読取_delegate親は回答待ちの子から黄を導出する() {
    // Given
    let activity = crate::domain::workflow::AgentSessionActivity::AwaitingAnswer;
    // When
    let status = delegate_parent_status_from_store(activity).await;
    // Then
    assert_eq!(
        status,
        crate::domain::workspace_tree::WorkspaceNodeStatusClassification::Attention
    );
}

async fn delegate_parent_status_from_store(
    activity: crate::domain::workflow::AgentSessionActivity,
) -> crate::domain::workspace_tree::WorkspaceNodeStatusClassification {
    use crate::adaptor::gateway::workflow::fact_log::{
        append_single_fact, fold_tree_from, FactLogReadBackend,
    };
    use crate::domain::workflow::{
        AgentActivityObservedFact, ArtifactProducedFact, NodeFact, NodeFactMeta, Predicate,
        SessionAttachedFact, SessionDelegate, StartedFact, StopReceivedFact, SubmitReceivedFact,
        TreeRootFact,
    };

    const TREE: &str = "00000000-0000-4000-8000-000000001958";
    const CHILD: &str = "00000000-0000-4000-8000-000000001959";
    let definition = WorkflowDefinition {
        name: "delegate".into(),
        entry: "main".into(),
        nodes: vec![
            NodeDefinition {
                name: "main".into(),
                kind: NodeKind::Session(SessionSpec::default()),
                artifact: Some("result".into()),
                completion: NodeCompletion {
                    delegate: Some(SessionDelegate {
                        child: "verify".into(),
                        inputs: Vec::new(),
                        when: Predicate::Ref("child.done".into()),
                        max_iterations: 2,
                    }),
                    ..Default::default()
                },
                ..Default::default()
            },
            NodeDefinition {
                name: "verify".into(),
                kind: NodeKind::Session(SessionSpec::default()),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let parent = NodeFactMeta {
        tree_id: TREE.into(),
        node_execution_id: TREE.into(),
        parent_id: None,
        node_name: "main".into(),
        kind: NodeKindName::Session,
        attempt: 1,
    };
    let child = NodeFactMeta {
        tree_id: TREE.into(),
        node_execution_id: CHILD.into(),
        parent_id: Some(TREE.into()),
        node_name: "verify".into(),
        kind: NodeKindName::Session,
        attempt: 1,
    };
    let facts = [
        (
            parent.clone(),
            NodeFact::Started(StartedFact {
                worktree: None,
                parent: None,
                root: Some(Box::new(TreeRootFact {
                    repository_root: None,
                    workspace_identity: WorkspaceIdentity::new("/repo").as_str().into(),
                    worktree_path: "/repo".into(),
                    created_from: ExecutionOrigin::DesktopUi,
                    request: "test".into(),
                    workflow_name: "delegate".into(),
                    definition: Some(definition),
                    launched_as: crate::domain::workflow::ExecutionTreeLaunch::Workflow,
                })),
            }),
        ),
        (
            parent.clone(),
            NodeFact::SessionAttached(SessionAttachedFact {
                session_id: "parent-session".into(),
                provider_session_id: None,
                transcript_ref: None,
                initial_instruction_admitted: false,
            }),
        ),
        (
            parent.clone(),
            NodeFact::SubmitReceived(SubmitReceivedFact { request_id: None }),
        ),
        (
            parent.clone(),
            NodeFact::ArtifactProduced(ArtifactProducedFact {
                contract: Some("result".into()),
                value: serde_json::json!({"done": false}),
                request_id: None,
            }),
        ),
        (
            parent.clone(),
            NodeFact::StopReceived(StopReceivedFact {
                result_summary: None,
                token_usage: None,
            }),
        ),
        (
            child.clone(),
            NodeFact::Started(StartedFact {
                worktree: None,
                parent: Some(ExecutionParentRef::delegate_child(TREE)),
                root: None,
            }),
        ),
        (
            child.clone(),
            NodeFact::SessionAttached(SessionAttachedFact {
                session_id: "child-session".into(),
                provider_session_id: None,
                transcript_ref: None,
                initial_instruction_admitted: false,
            }),
        ),
        (
            child,
            NodeFact::AgentActivityObserved(AgentActivityObservedFact { activity }),
        ),
    ];
    let directory = tempfile::tempdir().unwrap();
    let store =
        LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into())).unwrap();
    for (index, (meta, fact)) in facts.iter().enumerate() {
        append_single_fact(&store, meta, fact, (index as i64 + 1) * 1000)
            .await
            .unwrap();
    }

    let restored = fold_tree_from(&FactLogReadBackend::Live(store.clone()), TREE)
        .await
        .unwrap()
        .unwrap();
    assert!(restored.aggregate.delegate_waits_for_child(TREE));
    assert_eq!(restored.session_activities[CHILD], activity);
    let repository = SqliteWorkspaceTreeRepository::new(store);
    repository
        .load_node_by_node_execution_id(TREE)
        .await
        .unwrap()
        .unwrap()
        .status_classification
}

async fn standalone_session_store(
    session_id: &str,
    workspace: &WorkspaceIdentity,
) -> (tempfile::TempDir, Arc<LocalEventStore>) {
    let directory = tempfile::TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let session = AgentSession::create(
        session_id,
        workspace.clone(),
        workspace.as_str(),
        ProviderKind::Codex,
        AgentSessionTreeLocation::session_tree_root(session_id).unwrap(),
    )
    .unwrap();
    LocalAgentSessionRepository::new(Arc::clone(&store))
        .create(session, "create-request-1")
        .await
        .unwrap();
    (directory, store)
}

fn standalone_session_meta(session_id: &str) -> crate::domain::workflow::NodeFactMeta {
    crate::domain::workflow::NodeFactMeta {
        tree_id: session_id.into(),
        node_execution_id: session_id.into(),
        parent_id: None,
        node_name: session_id.into(),
        kind: NodeKindName::Session,
        attempt: 1,
    }
}

#[tokio::test]
async fn test_workspace_tree読取_追記された事実だけを足し最初からfoldし直さない() {
    use crate::adaptor::gateway::workflow::fact_log::append_single_fact;
    use crate::domain::workflow::services::fact_replay::tree_fold_count;
    use crate::domain::workflow::{AgentActivityObservedFact, AgentSessionActivity, NodeFact};
    // Given: 一度読んだ実行木
    let workspace = WorkspaceIdentity::new("/repo");
    let (_directory, store) = standalone_session_store("agent-session-1", &workspace).await;
    let repository = SqliteWorkspaceTreeRepository::new(Arc::clone(&store));
    let before = repository
        .load_node(&workspace, "agent-session-1")
        .await
        .unwrap()
        .unwrap();
    let mut meta = standalone_session_meta("agent-session-1");
    meta.node_name = before.node_name.clone().unwrap();
    let folds = tree_fold_count();

    // When: 事実が 2 件追記され、その都度読む
    for activity in [
        AgentSessionActivity::Working,
        AgentSessionActivity::AwaitingAnswer,
    ] {
        append_single_fact(
            &store,
            &meta,
            &NodeFact::AgentActivityObserved(AgentActivityObservedFact { activity }),
            10_000,
        )
        .await
        .unwrap();
        let node = repository
            .load_node(&workspace, "agent-session-1")
            .await
            .unwrap()
            .unwrap();

        // Then: 追記が反映され、最初からの fold は起きていない
        assert_eq!(node.activity, Some(activity));
    }
    assert_eq!(tree_fold_count(), folds);
}

#[tokio::test]
async fn test_workspace_tree読取_終端の事実が追記された木は最初から読み直す() {
    use crate::adaptor::gateway::workflow::fact_log::{
        append_single_fact, fold_tree_from, FactLogReadBackend,
    };
    use crate::domain::workflow::services::fact_replay::tree_fold_count;
    use crate::domain::workflow::{AbortRequestedFact, NodeFact, RuntimeExecutionState};
    // Given: 一度読んだ実行木
    let workspace = WorkspaceIdentity::new("/repo");
    let (_directory, store) = standalone_session_store("agent-session-1", &workspace).await;
    let repository = SqliteWorkspaceTreeRepository::new(Arc::clone(&store));
    let node = repository
        .load_node(&workspace, "agent-session-1")
        .await
        .unwrap()
        .unwrap();
    let mut meta = standalone_session_meta("agent-session-1");
    meta.node_name = node.node_name.unwrap();

    // When: 終端の事実が追記される
    append_single_fact(
        &store,
        &meta,
        &NodeFact::AbortRequested(AbortRequestedFact::default()),
        10_000,
    )
    .await
    .unwrap();
    let folds = tree_fold_count();
    let held = repository
        .folded_tree("agent-session-1")
        .await
        .unwrap()
        .unwrap();

    // Then: 最初から読み直し、store から直接 fold した結果と一致する
    assert_eq!(tree_fold_count(), folds + 1);
    assert_eq!(*held.0.aggregate.state(), RuntimeExecutionState::Aborted);
    let direct = fold_tree_from(&FactLogReadBackend::Live(store), "agent-session-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(held.0, direct);
}

#[tokio::test]
async fn test_workspace_tree読取_複数workspaceをまとめて読み所属ごとに分ける() {
    // Given: 別々の workspace に属する 2 つの実行木
    let first = WorkspaceIdentity::new("/repo/first");
    let second = WorkspaceIdentity::new("/repo/second");
    let (_directory, store) = standalone_session_store("agent-session-1", &first).await;
    let session = AgentSession::create(
        "agent-session-2",
        second.clone(),
        second.as_str(),
        ProviderKind::Codex,
        AgentSessionTreeLocation::session_tree_root("agent-session-2").unwrap(),
    )
    .unwrap();
    LocalAgentSessionRepository::new(Arc::clone(&store))
        .create(session, "create-request-2")
        .await
        .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(store);

    // When
    let trees = repository
        .folded_workspaces(&[second.as_str(), "/repo/none", first.as_str()])
        .await;

    // Then: 指定した順に、その workspace の実行木だけを返す
    let ids = trees
        .into_iter()
        .map(|trees| {
            trees
                .unwrap()
                .iter()
                .map(|execution| execution.0.aggregate.id.clone())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec![
            vec!["agent-session-2".to_string()],
            vec![],
            vec!["agent-session-1".to_string()]
        ]
    );
}

#[tokio::test]
async fn test_workspace_tree読取_同じworkspaceのsessionだけを実行に載せる() {
    // Given
    let workspace = WorkspaceIdentity::new("/repo");
    let (_directory, store) = standalone_session_store("agent-session-2", &workspace).await;
    let sessions = LocalAgentSessionRepository::new(Arc::clone(&store));
    for (id, worktree) in [("agent-session-1", "/repo"), ("agent-session-3", "/other")] {
        let session = AgentSession::create(
            id,
            WorkspaceIdentity::new(worktree),
            worktree,
            ProviderKind::Codex,
            AgentSessionTreeLocation::session_tree_root(id).unwrap(),
        )
        .unwrap();
        sessions
            .create(session, &format!("create-{id}"))
            .await
            .unwrap();
    }
    let repository = SqliteWorkspaceTreeRepository::new(store);

    // When
    let tree = repository
        .load_trees(std::slice::from_ref(&workspace))
        .await
        .pop()
        .unwrap()
        .unwrap();

    // Then
    let mut session_ids = tree
        .executions()
        .iter()
        .map(|execution| execution.session.as_ref().unwrap().id().to_string())
        .collect::<Vec<_>>();
    session_ids.sort();
    assert_eq!(session_ids, vec!["agent-session-1", "agent-session-2"]);
    assert!(tree
        .executions()
        .iter()
        .all(|execution| execution.worktree_path == "/repo"));
}

#[tokio::test]
async fn test_workspace_tree読取_sessionのlifecycleと操作の可否を復元する() {
    use crate::domain::agent_session::aggregates::AgentSessionLifecycle;
    // Given
    let workspace = WorkspaceIdentity::new("/repo");
    let directory = tempfile::TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let sessions = LocalAgentSessionRepository::new(Arc::clone(&store));
    for id in ["open-session", "paused-session", "archived-session"] {
        let session = AgentSession::create(
            id,
            workspace.clone(),
            workspace.as_str(),
            ProviderKind::Claude,
            AgentSessionTreeLocation::session_tree_root(id).unwrap(),
        )
        .unwrap();
        let mut saved = sessions
            .create(session, &format!("create-{id}"))
            .await
            .unwrap();
        saved
            .session_mut()
            .associate_provider_session(format!("provider-{id}"), None)
            .unwrap();
        let mut saved = sessions
            .save(saved, &format!("associate-{id}"))
            .await
            .unwrap();
        match id {
            "paused-session" => {
                saved.session_mut().observe_provider_process_exit(Some(0));
                sessions.save(saved, "pause-session").await.unwrap();
            }
            "archived-session" => {
                saved.session_mut().archive().unwrap();
                sessions.save(saved, "archive-session").await.unwrap();
            }
            _ => {}
        }
    }
    let repository = SqliteWorkspaceTreeRepository::new(store);

    // When
    let tree = repository
        .load_trees(std::slice::from_ref(&workspace))
        .await
        .pop()
        .unwrap()
        .unwrap();

    // Then
    let session = |id: &str| {
        tree.executions()
            .iter()
            .filter_map(|execution| execution.session.as_ref())
            .find(|session| session.id() == id)
            .unwrap()
    };
    assert_eq!(tree.executions().len(), 3);
    for (id, lifecycle, can_archive, can_restore) in [
        ("open-session", AgentSessionLifecycle::Open, true, false),
        ("paused-session", AgentSessionLifecycle::Paused, true, false),
        (
            "archived-session",
            AgentSessionLifecycle::Archived,
            false,
            true,
        ),
    ] {
        let session = session(id);
        let operations = session.operations();
        assert_eq!(session.lifecycle(), lifecycle);
        assert_eq!(
            session.provider_session_id(),
            Some(format!("provider-{id}").as_str())
        );
        assert_eq!(operations.can_archive, can_archive);
        assert_eq!(operations.can_restore, can_restore);
        assert_eq!(operations.can_delete, can_restore);
    }
    assert_eq!(
        tree.archived_sessions()
            .iter()
            .map(|execution| execution.execution_id.as_str())
            .collect::<Vec<_>>(),
        vec!["archived-session"]
    );
}

#[tokio::test]
async fn test_workspace_tree読取_読まれなくなった木の保持を捨て次に読むとき読み直す() {
    use crate::domain::workflow::services::fact_replay::tree_fold_count;
    // Given: 読まれた直後に保持の期限が切れる repository
    let workspace = WorkspaceIdentity::new("/repo");
    let (_directory, store) = standalone_session_store("agent-session-1", &workspace).await;
    let mut repository = SqliteWorkspaceTreeRepository::new(store);
    Arc::get_mut(&mut repository).unwrap().fold_idle_limit = std::time::Duration::ZERO;
    repository
        .load_trees(std::slice::from_ref(&workspace))
        .await
        .pop()
        .unwrap()
        .unwrap();
    let folds = tree_fold_count();
    std::thread::sleep(std::time::Duration::from_millis(5));

    // When
    let tree = repository
        .load_trees(std::slice::from_ref(&workspace))
        .await
        .pop()
        .unwrap()
        .unwrap();

    // Then: 保持は捨てられていて、最初から読み直す
    assert_eq!(tree_fold_count(), folds + 1);
    assert_eq!(tree.executions().len(), 1);
}
