use releashd::test_support::integration::platform::FailureRecordStore;
use releashd::test_support::integration::workflow::AgentActivityObservedFact;
use releashd::test_support::integration::workspace::session_detail;

use releashd::test_support::integration::workflow::append_single_fact;
use releashd::test_support::integration::workflow::fold_tree_from;
use releashd::test_support::integration::workflow::tree_fold_count;
use releashd::test_support::integration::workflow::NodeFact;

use releashd::test_support::integration::persistence::LocalEventStore;
use releashd::test_support::integration::persistence::LocalEventStoreConfig;
use releashd::test_support::integration::platform::LocalEventQueryError;
use releashd::test_support::integration::sessions::AgentSession;
use releashd::test_support::integration::sessions::LocalAgentSessionRepository;
use releashd::test_support::integration::workflow::FactLogReadBackend;
use releashd::test_support::integration::workspace::SqliteWorkspaceTreeRepository;
use releashd::test_support::integration::workspace::WorkspaceIdentity;
use releashd::test_support::integration::workspace::WorkspaceTreeRepository;
use std::sync::Arc;

use releashd::test_support::integration::providers::ProviderKind;
use releashd::test_support::integration::sessions::AgentSessionRepository;
use releashd::test_support::integration::sessions::AgentSessionTreeLocation;
use releashd::test_support::integration::workflow::definition_FacetRefs as FacetRefs;
use releashd::test_support::integration::workflow::ChildEntry;
use releashd::test_support::integration::workflow::EffectiveRules;
use releashd::test_support::integration::workflow::ExecutionParentRef;
use releashd::test_support::integration::workflow::FanoutSpec;
use releashd::test_support::integration::workflow::ItemsSource;
use releashd::test_support::integration::workflow::NodeCompletion;
use releashd::test_support::integration::workflow::NodeDefinition;
use releashd::test_support::integration::workflow::NodeKind;
use releashd::test_support::integration::workflow::NodeKindName;
use releashd::test_support::integration::workflow::Rule;
use releashd::test_support::integration::workflow::SchemaDef;
use releashd::test_support::integration::workflow::SequenceSpec;
use releashd::test_support::integration::workflow::SessionSpec;
use releashd::test_support::integration::workflow::WorkflowDefinitionContent;
use releashd::test_support::integration::workflow::WorkflowEvent;
use releashd::test_support::integration::workspace::WorkspaceNodeStatus;

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

fn reported_fanout_definition() -> WorkflowDefinitionContent {
    WorkflowDefinitionContent {
        name: "dev-cycle".to_string(),
        description: String::new(),
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
fn fanout_with_sequence_child_definition() -> WorkflowDefinitionContent {
    WorkflowDefinitionContent {
        name: "multi-execution".to_string(),
        description: String::new(),
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
    let definition = WorkflowDefinitionContent {
        name: "duplicate-session-binding".to_string(),
        description: String::new(),
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

fn dynamic_fanout_with_sequence_child_definition() -> WorkflowDefinitionContent {
    WorkflowDefinitionContent {
        name: "dynamic-multi-execution".to_string(),
        description: String::new(),
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
                        field_path: releashd::test_support::integration::workflow::FieldPath::new(
                            ["items"],
                        ),
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

#[tokio::test]
pub async fn test_workspace_tree読み出し_報告実例の後方辺fanout既存fact列から介入対象を解決する() {
    // Given
    let directory = tempfile::TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
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
    releashd::test_support::integration::workflow::append_canonical_events(&store, &events)
        .await
        .unwrap();
    let rows_before =
        releashd::test_support::integration::workflow::read_tree_records(&store, execution_id)
            .await
            .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(
        Arc::clone(&store),
        std::sync::Arc::new(FailureRecordStore::default()),
    );

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
    let loaded_by_execution = releashd::test_support::integration::workspace::node_for_execution(
        &*repository,
        &workspace,
        waiting_execution_id,
    )
    .await
    .unwrap()
    .unwrap();
    let session_id = REPORTED_SESSION_2_ID;
    let session_node_id = repository
        .load_node_by_session_id(&workspace, session_id)
        .await
        .unwrap()
        .map(|selection| selection.id)
        .unwrap();
    let loaded_session = repository
        .load_node(&workspace, &session_node_id)
        .await
        .unwrap()
        .unwrap();
    let rows_after =
        releashd::test_support::integration::workflow::read_tree_records(&store, execution_id)
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
pub async fn test_ツリー読み出し_command形のsession成果物を含む複数実行木と各nodeをfact変更なしで読める(
) {
    // Given
    let directory = tempfile::TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let workspace = WorkspaceIdentity::new("/repo/.worktrees/command-shaped-artifact");
    let execution_ids = [
        "00000000-0000-4000-8000-000000000743",
        "00000000-0000-4000-8000-000000000744",
    ];
    let definition = WorkflowDefinitionContent {
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
        ..WorkflowDefinitionContent::default()
    };
    releashd::test_support::integration::workflow::validate(&definition).unwrap();
    for execution_id in execution_ids {
        let node_execution_id = format!("{execution_id}-report");
        let mut events = vec![
            WorkflowEvent::ExecutionStarted {
                repository_root: None,
                execution_id: execution_id.to_string(),
                workflow_name: definition.name.clone(),
                worktree_path: workspace.as_str().to_string(),
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
        releashd::test_support::integration::workflow::append_canonical_events(&store, &events)
            .await
            .unwrap();
    }
    let rows_before = futures_util::future::join_all(execution_ids.map(|execution_id| async {
        releashd::test_support::integration::workflow::read_tree_records(&store, execution_id)
            .await
            .unwrap()
    }))
    .await;
    let repository = SqliteWorkspaceTreeRepository::new(
        Arc::clone(&store),
        std::sync::Arc::new(FailureRecordStore::default()),
    );

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
            releashd::test_support::integration::workspace::node_for_execution(
                &*repository,
                &workspace,
                &node_execution_id,
            )
            .await
            .unwrap()
            .unwrap(),
        ));
    }
    let rows_after = futures_util::future::join_all(execution_ids.map(|execution_id| async {
        releashd::test_support::integration::workflow::read_tree_records(&store, execution_id)
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
pub async fn test_workspace_tree読み出し_同一worktreeの複数executionでfanout子sequenceが衝突しない()
{
    // Given
    let directory = tempfile::TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let workspace = WorkspaceIdentity::new("/repo/.worktrees/multiple-executions");
    let first_execution_id = "00000000-0000-4000-8000-000000000702";
    let second_execution_id = "00000000-0000-4000-8000-000000000703";
    releashd::test_support::integration::workflow::append_canonical_events(
        &store,
        &fanout_with_sequence_child_events(first_execution_id, workspace.as_str()),
    )
    .await
    .unwrap();
    releashd::test_support::integration::workflow::append_canonical_events(
        &store,
        &fanout_with_sequence_child_events(second_execution_id, workspace.as_str()),
    )
    .await
    .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(
        store,
        std::sync::Arc::new(FailureRecordStore::default()),
    );

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
pub async fn test_workspace_tree読み出し_同一worktreeの複数executionで動的fanout子sequenceが衝突しない(
) {
    // Given
    let directory = tempfile::TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let workspace = WorkspaceIdentity::new("/repo/.worktrees/dynamic-multiple-executions");
    let first_execution_id = "00000000-0000-4000-8000-000000000705";
    let second_execution_id = "00000000-0000-4000-8000-000000000706";
    releashd::test_support::integration::workflow::append_canonical_events(
        &store,
        &dynamic_fanout_with_sequence_child_events(first_execution_id, workspace.as_str()),
    )
    .await
    .unwrap();
    releashd::test_support::integration::workflow::append_canonical_events(
        &store,
        &dynamic_fanout_with_sequence_child_events(second_execution_id, workspace.as_str()),
    )
    .await
    .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(
        store,
        std::sync::Arc::new(FailureRecordStore::default()),
    );

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
pub async fn test_workspace_tree読み出し_session二重束縛をcorruptとして拒否する() {
    // Given
    let directory = tempfile::TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let workspace = WorkspaceIdentity::new("/repo/.worktrees/corrupt-session-binding");
    let execution_id = "00000000-0000-4000-8000-000000000704";
    let events = duplicate_session_binding_events(execution_id, workspace.as_str());
    releashd::test_support::integration::workflow::append_canonical_events(&store, &events)
        .await
        .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(
        store,
        std::sync::Arc::new(FailureRecordStore::default()),
    );

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
pub async fn public_session_root_id_loads_the_session_node_instead_of_the_internal_owner() {
    let directory = tempfile::TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
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
    let repository = SqliteWorkspaceTreeRepository::new(
        store,
        std::sync::Arc::new(FailureRecordStore::default()),
    );

    let node_id = repository
        .load_node_by_session_id(&workspace, "agent-session-1")
        .await
        .unwrap()
        .map(|selection| selection.id)
        .expect("the standalone Session must have a public Node id");
    let node = repository
        .load_node(&workspace, &node_id)
        .await
        .unwrap()
        .expect("the public Node id must resolve");

    assert_eq!(node_id, "agent-session-1");
    assert_eq!(
        node.kind,
        releashd::test_support::integration::workspace::WorkspaceNodeKind::WorkflowSession
    );
    assert_eq!(node.node_execution_id.as_deref(), Some("agent-session-1"));
    assert_eq!(node.session_id.as_deref(), Some("agent-session-1"));
    assert!(!node.can_retry);
}

#[tokio::test]
pub async fn test_workspace_tree_repository_workspace同定子がworktreeと異なるsession木を解決する() {
    // Given: workspace identity と worktree path が異なる Session 起動由来の木
    let directory = tempfile::TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
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
    let repository = SqliteWorkspaceTreeRepository::new(
        store,
        std::sync::Arc::new(FailureRecordStore::default()),
    );

    // When: workspace identity から木、Session の公開 node、詳細を引く
    let trees = repository
        .folded_workspace_trees(workspace.as_str())
        .await
        .unwrap();
    let node_id = repository
        .load_node_by_session_id(&workspace, "agent-session-workspace-identity")
        .await
        .unwrap()
        .map(|selection| selection.id)
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
pub async fn a_session_owned_by_another_worktree_has_no_public_node_id() {
    let directory = tempfile::TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
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
    let repository = SqliteWorkspaceTreeRepository::new(
        store,
        std::sync::Arc::new(FailureRecordStore::default()),
    );

    assert!(
        repository
            .load_node_by_session_id(&other, "agent-session-1")
            .await
            .unwrap()
            .map(|selection| selection.id)
            .is_none(),
        "another Worktree must not resolve a public Node id for this Session"
    );
    assert!(
        repository
            .load_node_by_session_id(&owner, "unknown-session")
            .await
            .unwrap()
            .map(|selection| selection.id)
            .is_none(),
        "an unknown Session has no execution tree to publish"
    );
}

#[tokio::test]
pub async fn test_workspace_repository読取_実経路で失敗分類を保持する() {
    use releashd::test_support::integration::persistence::ReadFailure;
    use releashd::test_support::integration::transport::classified_error;
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().into(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(
        store.clone(),
        std::sync::Arc::new(FailureRecordStore::default()),
    );
    for (failure, expected) in ReadFailure::cases() {
        store.fail_next_read(failure);
        // When
        let error = repository
            .load_node(
                &releashd::test_support::integration::workspace::WorkspaceIdentity::new("/repo"),
                "node",
            )
            .await
            .unwrap_err();
        // Then
        assert_eq!(classified_error(error).code, expected);
    }
}

#[tokio::test]
pub async fn test_workspace_tree読取_delegate親はworkingの子から青を導出する() {
    // Given
    let activity = releashd::test_support::integration::workflow::AgentSessionActivity::Working;
    // When
    let status = delegate_parent_status_from_store(activity).await;
    // Then
    assert_eq!(
        status,
        releashd::test_support::integration::workspace::WorkspaceNodeStatusClassification::Active
    );
}

#[tokio::test]
pub async fn test_workspace_tree読取_delegate親は回答待ちの子から黄を導出する() {
    // Given
    let activity =
        releashd::test_support::integration::workflow::AgentSessionActivity::AwaitingAnswer;
    // When
    let status = delegate_parent_status_from_store(activity).await;
    // Then
    assert_eq!(
        status,
        releashd::test_support::integration::workspace::WorkspaceNodeStatusClassification::Attention
    );
}

pub(super) async fn delegate_fixture(
    workspace: &str,
    activity: releashd::test_support::integration::workflow::AgentSessionActivity,
) -> (
    tempfile::TempDir,
    Arc<LocalEventStore>,
    releashd::test_support::integration::workflow::NodeFactMeta,
    releashd::test_support::integration::workflow::NodeFactMeta,
) {
    use releashd::test_support::integration::workflow::append_single_fact;

    use releashd::test_support::integration::workflow::AgentActivityObservedFact;
    use releashd::test_support::integration::workflow::ArtifactProducedFact;
    use releashd::test_support::integration::workflow::NodeFact;
    use releashd::test_support::integration::workflow::NodeFactMeta;
    use releashd::test_support::integration::workflow::Predicate;
    use releashd::test_support::integration::workflow::SessionAttachedFact;
    use releashd::test_support::integration::workflow::SessionDelegate;
    use releashd::test_support::integration::workflow::StartedFact;
    use releashd::test_support::integration::workflow::StopReceivedFact;
    use releashd::test_support::integration::workflow::SubmitReceivedFact;
    use releashd::test_support::integration::workflow::TreeRootFact;

    const TREE: &str = "00000000-0000-4000-8000-000000001958";
    const ROOT: &str = "00000000-0000-4000-8000-000000001957";
    const CHILD: &str = "00000000-0000-4000-8000-000000001959";
    let definition = WorkflowDefinitionContent {
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
        node_execution_id: ROOT.into(),
        parent_id: None,
        node_name: "main".into(),
        kind: NodeKindName::Session,
        attempt: 1,
    };
    let child = NodeFactMeta {
        tree_id: TREE.into(),
        node_execution_id: CHILD.into(),
        parent_id: Some(ROOT.into()),
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
                    workspace_identity: WorkspaceIdentity::new(workspace).as_str().into(),
                    worktree_path: workspace.into(),
                    request: "test".into(),
                    workflow_name: "delegate".into(),
                    definition: Some(definition),
                    launched_as:
                        releashd::test_support::integration::workflow::ExecutionTreeLaunch::Workflow,
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
                parent: Some(ExecutionParentRef::delegate_child(ROOT)),
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
            child.clone(),
            NodeFact::AgentActivityObserved(AgentActivityObservedFact { activity }),
        ),
    ];
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().into(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    for (index, (meta, fact)) in facts.iter().enumerate() {
        append_single_fact(&store, meta, fact, (index as i64 + 1) * 1000)
            .await
            .unwrap();
    }

    (directory, store, parent, child)
}

async fn delegate_parent_status_from_store(
    activity: releashd::test_support::integration::workflow::AgentSessionActivity,
) -> releashd::test_support::integration::workspace::WorkspaceNodeStatusClassification {
    let (_directory, store, parent, child) = delegate_fixture("/repo", activity).await;
    let restored = fold_tree_from(&FactLogReadBackend::Live(store.clone()), &parent.tree_id)
        .await
        .unwrap()
        .unwrap();
    assert!(restored
        .aggregate
        .delegate_waits_for_child(&parent.node_execution_id));
    assert_eq!(
        restored.session_activities[&child.node_execution_id],
        activity
    );
    let repository = SqliteWorkspaceTreeRepository::new(
        store,
        std::sync::Arc::new(FailureRecordStore::default()),
    );
    let node = releashd::test_support::integration::workspace::node_for_execution(
        &*repository,
        &releashd::test_support::integration::workspace::WorkspaceIdentity::new("/repo"),
        &parent.node_execution_id,
    )
    .await
    .unwrap()
    .unwrap();
    // When
    let detail = session_detail(
        &*repository,
        &WorkspaceIdentity::new("/repo"),
        &parent.tree_id,
        &parent.node_execution_id,
    )
    .await
    .unwrap()
    .unwrap();
    // Then
    assert_eq!(
        detail.classification,
        node.status_classification.as_public_str()
    );
    assert_eq!(detail.id, node.id);
    assert_eq!(detail.title, node.title);
    assert_eq!(detail.capabilities.can_rename, node.can_rename);
    node.status_classification
}

async fn standalone_session_store(
    session_id: &str,
    workspace: &WorkspaceIdentity,
) -> (tempfile::TempDir, Arc<LocalEventStore>) {
    let directory = tempfile::TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
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

fn recreate_session_tree(connection: &rusqlite::Connection, tree_id: &str, workspace: &str) {
    use releashd::test_support::integration::persistence::{
        append_node_event, delete_tree, read_tree, NewNodeEventRow,
    };
    let rows = read_tree(connection, tree_id).unwrap();
    let head = rows.last().unwrap().seq;

    delete_tree(connection, tree_id).unwrap();
    for row in rows {
        let mut detail: serde_json::Value = serde_json::from_str(&row.detail).unwrap();
        if row.event_type == "started" {
            detail["root"]["workspaceIdentity"] = workspace.into();
        }
        append_node_event(
            connection,
            &NewNodeEventRow {
                tree_id: row.tree_id,
                node_execution_id: row.node_execution_id,
                parent_id: row.parent_id,
                node_name: row.node_name,
                kind: row.kind,
                attempt: row.attempt,
                event_type: row.event_type,
                session_id: row.session_id,
                detail: detail.to_string(),
            },
            row.timestamp_ms + 1000,
        )
        .unwrap();
    }
    assert_eq!(
        read_tree(connection, tree_id).unwrap().last().unwrap().seq,
        head
    );
}

#[tokio::test]
pub async fn test_実行木のid読取_一覧の復元を共有し別の木の破損から独立する() {
    // Given
    let workspace = WorkspaceIdentity::new("/repo");
    let (directory, store) = standalone_session_store("session", &workspace).await;
    let repository = SqliteWorkspaceTreeRepository::new(
        store.clone(),
        std::sync::Arc::new(FailureRecordStore::default()),
    );
    let listed = repository
        .load_trees(std::slice::from_ref(&workspace))
        .await
        .remove(0)
        .unwrap();
    let folds = tree_fold_count();
    let unrelated = AgentSession::create(
        "unrelated",
        WorkspaceIdentity::new("/other"),
        "/other",
        ProviderKind::Codex,
        AgentSessionTreeLocation::session_tree_root("unrelated").unwrap(),
    )
    .unwrap();
    LocalAgentSessionRepository::new(store)
        .create(unrelated, "create-unrelated")
        .await
        .unwrap();
    let connection =
        rusqlite::Connection::open(directory.path().join("local-event-store.sqlite3")).unwrap();
    connection.execute("UPDATE node_events SET detail = 'broken' WHERE tree_id = 'unrelated' AND event_type = 'started'", []).unwrap();

    // When
    let actual = repository
        .load_execution_tree(&workspace, "session")
        .await
        .unwrap()
        .unwrap();
    // Then
    assert_eq!(
        actual.session_node("session"),
        listed.session_node("session")
    );
    assert_eq!(tree_fold_count(), folds);
    assert!(repository
        .load_execution_tree(&WorkspaceIdentity::new("/other"), "session")
        .await
        .unwrap()
        .is_none());
    assert!(repository
        .load_execution_tree(&workspace, "missing")
        .await
        .unwrap()
        .is_none());

    // When
    recreate_session_tree(&connection, "session", "/relocated");
    // Then
    assert!(repository
        .load_execution_tree(&workspace, "session")
        .await
        .unwrap()
        .is_none());
    let relocated = repository
        .load_execution_tree(&WorkspaceIdentity::new("/relocated"), "session")
        .await
        .unwrap()
        .unwrap();
    assert!(relocated.session_node("session").is_some());
}

fn standalone_session_meta(
    session_id: &str,
) -> releashd::test_support::integration::workflow::NodeFactMeta {
    releashd::test_support::integration::workflow::NodeFactMeta {
        tree_id: session_id.into(),
        node_execution_id: session_id.into(),
        parent_id: None,
        node_name: session_id.into(),
        kind: NodeKindName::Session,
        attempt: 1,
    }
}

#[tokio::test]
pub async fn test_workspace_tree読取_追記された事実だけを足し最初からfoldし直さない() {
    use releashd::test_support::integration::workflow::tree_fold_count;

    use releashd::test_support::integration::workflow::AgentSessionActivity;

    // Given: 一度読んだ実行木
    let workspace = WorkspaceIdentity::new("/repo");
    let (_directory, store) = standalone_session_store("agent-session-1", &workspace).await;
    let repository = SqliteWorkspaceTreeRepository::new(
        Arc::clone(&store),
        std::sync::Arc::new(FailureRecordStore::default()),
    );
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
pub async fn test_workspace_tree読取_終端の事実が追記された木は最初から読み直す() {
    use releashd::test_support::integration::workflow::AbortRequestedFact;

    use releashd::test_support::integration::workflow::RuntimeExecutionState;
    // Given: 一度読んだ実行木
    let workspace = WorkspaceIdentity::new("/repo");
    let (_directory, store) = standalone_session_store("agent-session-1", &workspace).await;
    let repository = SqliteWorkspaceTreeRepository::new(
        Arc::clone(&store),
        std::sync::Arc::new(FailureRecordStore::default()),
    );
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
pub async fn test_workspace_tree読取_複数workspaceをまとめて読み所属ごとに分ける() {
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
    let repository = SqliteWorkspaceTreeRepository::new(
        store,
        std::sync::Arc::new(FailureRecordStore::default()),
    );

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
pub async fn test_workspace_tree読取_同じworkspaceのsessionだけを実行に載せる() {
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
    let repository = SqliteWorkspaceTreeRepository::new(
        store,
        std::sync::Arc::new(FailureRecordStore::default()),
    );

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
pub async fn test_workspace_tree読取_sessionのlifecycleと操作の可否を復元する() {
    use releashd::test_support::integration::sessions::AgentSessionLifecycle;
    // Given
    let workspace = WorkspaceIdentity::new("/repo");
    let directory = tempfile::TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
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
                let records =
                    releashd::test_support::integration::workflow::read_tree_records(&store, id)
                        .await
                        .unwrap();
                append_single_fact(
                    &store,
                    &records[0].meta,
                    &NodeFact::ArchiveRequested(
                        releashd::test_support::integration::workflow::ArchiveRequestedFact {
                            archived_at: 1.0,
                        },
                    ),
                    1_000,
                )
                .await
                .unwrap();
            }
            _ => {}
        }
    }
    let repository = SqliteWorkspaceTreeRepository::new(
        store,
        std::sync::Arc::new(FailureRecordStore::default()),
    );

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
    assert_eq!(repository.load_all_trees().await.unwrap().0, vec![tree]);
}

#[tokio::test]
pub async fn test_workspace_tree読取_読まれなくなった木の保持を捨て次に読むとき読み直す() {
    // Given: 読まれた直後に保持の期限が切れる repository
    let workspace = WorkspaceIdentity::new("/repo");
    let (_directory, store) = standalone_session_store("agent-session-1", &workspace).await;
    let mut repository = SqliteWorkspaceTreeRepository::new(
        store,
        std::sync::Arc::new(FailureRecordStore::default()),
    );
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

#[tokio::test]
pub async fn test_sessionタブ分類_delegateの次周回は前の子を保持し明示的なretryだけ履歴にする() {
    use releashd::test_support::integration::workflow::{
        ArtifactProducedFact, StartedFact, StopReceivedFact, SubmitReceivedFact,
    };
    // Given
    for retry in [false, true] {
        let (_directory, store, parent, child) = delegate_fixture(
            "/repo",
            releashd::test_support::integration::workflow::AgentSessionActivity::AwaitingAnswer,
        )
        .await;
        let mut facts = if retry {
            vec![(child.clone(), NodeFact::RetryRequested)]
        } else {
            vec![
                (
                    child.clone(),
                    NodeFact::SubmitReceived(SubmitReceivedFact { request_id: None }),
                ),
                (
                    child.clone(),
                    NodeFact::ArtifactProduced(ArtifactProducedFact {
                        contract: None,
                        value: serde_json::json!({"done": false}),
                        request_id: None,
                    }),
                ),
                (
                    child.clone(),
                    NodeFact::StopReceived(StopReceivedFact {
                        result_summary: None,
                        token_usage: None,
                    }),
                ),
                (child.clone(), NodeFact::AgentActivityObserved(AgentActivityObservedFact {
                    activity: releashd::test_support::integration::workflow::AgentSessionActivity::AwaitingAnswer,
                })),
                (
                    parent.clone(),
                    NodeFact::DelegateResultInjected(child.node_execution_id.clone()),
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
            ]
        };
        let mut next = child.clone();
        next.node_execution_id = "00000000-0000-4000-8000-000000001960".into();
        next.attempt = 2;
        facts.extend([
            (next.clone(), NodeFact::Started(StartedFact { worktree: None, parent: Some(ExecutionParentRef::delegate_child(&parent.node_execution_id)), root: None })),
            (next.clone(), releashd::test_support::integration::fixtures::session_attached("next-session")),
            (next, NodeFact::AgentActivityObserved(AgentActivityObservedFact { activity: releashd::test_support::integration::workflow::AgentSessionActivity::Working })),
        ]);
        // When
        for (index, (meta, fact)) in facts.iter().enumerate() {
            append_single_fact(&store, meta, fact, 10000 + index as i64 * 1000)
                .await
                .unwrap();
        }
        let repository = SqliteWorkspaceTreeRepository::new(
            store,
            std::sync::Arc::new(FailureRecordStore::default()),
        );
        let listed = releashd::test_support::integration::workspace::node_for_execution(
            &*repository,
            &WorkspaceIdentity::new("/repo"),
            &parent.node_execution_id,
        )
        .await
        .unwrap()
        .unwrap();
        let detail = session_detail(
            &*repository,
            &WorkspaceIdentity::new("/repo"),
            &parent.tree_id,
            &parent.node_execution_id,
        )
        .await
        .unwrap()
        .unwrap();
        // Then
        assert_eq!(
            detail.classification,
            if retry { "active" } else { "attention" }
        );
        assert_eq!(
            detail.classification,
            listed.status_classification.as_public_str()
        );
    }
}

#[tokio::test]
pub async fn test_会話詳細の合成子_保存事実から部分完了と承認待ちの一覧分類に一致する() {
    use releashd::test_support::integration::workflow::{
        ArtifactProducedFact, ExecutionTreeLaunch, IsolatedWorktree, NodeFactMeta, Predicate,
        SessionAttachedFact, SessionDelegate, StartedFact, StopReceivedFact, SubmitReceivedFact,
        TreeRootFact,
    };
    // Given
    for (fanout, count) in [(false, 1), (true, 0), (true, 1), (true, 2)] {
        let directory = tempfile::tempdir().unwrap();
        let store = LocalEventStore::open(LocalEventStoreConfig::production(
            directory.path().into(),
            Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
        ))
        .unwrap();
        let tree = "00000000-0000-4000-8000-000000002063";
        let parent = "00000000-0000-4000-8000-000000002064";
        let scope = "00000000-0000-4000-8000-000000002065";
        let definition = WorkflowDefinitionContent {
            name: "composite".into(),
            entry: "main".into(),
            nodes: vec![
                NodeDefinition {
                    name: "main".into(),
                    completion: NodeCompletion {
                        delegate: Some(SessionDelegate {
                            child: "scope".into(),
                            inputs: Vec::new(),
                            when: Predicate::Ref(
                                if count == 2 {
                                    "child.1.passed"
                                } else {
                                    "child.judge.passed"
                                }
                                .into(),
                            ),
                            max_iterations: 2,
                        }),
                        ..NodeCompletion::require_approval()
                    },
                    ..Default::default()
                },
                NodeDefinition {
                    name: "scope".into(),
                    kind: if fanout {
                        NodeKind::Fanout(FanoutSpec {
                            children: vec![ChildEntry::reference("judge")],
                            items: if count == 1 {
                                None
                            } else {
                                Some(ItemsSource::Literal(
                                    (0..count).map(|index| serde_json::json!(index)).collect(),
                                ))
                            },
                        })
                    } else {
                        NodeKind::Sequence(SequenceSpec {
                            entry: None,
                            children: vec![ChildEntry::reference("judge")],
                        })
                    },
                    worktree: (count == 0).then_some(
                        releashd::test_support::integration::workflow::WorktreeMode::Isolated,
                    ),
                    completion: if count == 0 {
                        NodeCompletion::require_approval()
                    } else {
                        NodeCompletion::default()
                    },
                    ..Default::default()
                },
                NodeDefinition {
                    name: "judge".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let meta = NodeFactMeta {
            tree_id: tree.into(),
            node_execution_id: parent.into(),
            node_name: "main".into(),
            kind: NodeKindName::Session,
            attempt: 1,
            parent_id: None,
        };
        let mut facts = vec![
            (
                meta.clone(),
                NodeFact::Started(StartedFact {
                    root: Some(Box::new(TreeRootFact {
                        repository_root: Some("/repo".into()),
                        workspace_identity: "/repo".into(),
                        worktree_path: "/repo".into(),
                        request: String::new(),
                        workflow_name: "composite".into(),
                        definition: Some(definition),
                        launched_as: ExecutionTreeLaunch::Workflow,
                    })),
                    parent: None,
                    worktree: None,
                }),
            ),
            (
                meta.clone(),
                NodeFact::SessionAttached(SessionAttachedFact {
                    session_id: "parent-session".into(),
                    provider_session_id: None,
                    transcript_ref: None,
                    initial_instruction_admitted: false,
                }),
            ),
            (
                meta.clone(),
                NodeFact::SubmitReceived(SubmitReceivedFact { request_id: None }),
            ),
            (
                meta.clone(),
                NodeFact::ArtifactProduced(ArtifactProducedFact {
                    contract: None,
                    value: serde_json::json!({"body":"private"}),
                    request_id: None,
                }),
            ),
            (
                meta.clone(),
                NodeFact::StopReceived(StopReceivedFact {
                    result_summary: None,
                    token_usage: None,
                }),
            ),
        ];
        let scope_parent = ExecutionParentRef::delegate_child(parent);
        facts.push((
            NodeFactMeta {
                node_execution_id: scope.into(),
                node_name: "scope".into(),
                kind: if fanout {
                    NodeKindName::Fanout
                } else {
                    NodeKindName::Sequence
                },
                parent_id: Some(parent.into()),
                ..meta.clone()
            },
            NodeFact::Started(StartedFact {
                root: None,
                parent: Some(scope_parent),
                worktree: (count == 0).then(|| IsolatedWorktree {
                    branch: "isolated".into(),
                    path: "/isolated".into(),
                }),
            }),
        ));
        for (seq, (meta, fact)) in facts.iter().enumerate() {
            append_single_fact(&store, meta, fact, seq as i64 * 1000)
                .await
                .unwrap();
        }
        let repository = SqliteWorkspaceTreeRepository::new(
            store.clone(),
            std::sync::Arc::new(FailureRecordStore::default()),
        );

        for index in 0..count.max(1) {
            if count > 0 {
                let child_parent = if fanout {
                    ExecutionParentRef::fanout_child(scope, (count == 2).then_some(index), 0)
                } else {
                    ExecutionParentRef::sequence_child(scope)
                };
                let child = NodeFactMeta {
                    node_execution_id: format!("00000000-0000-4000-8000-{:012}", 2070 + index),
                    node_name: "judge".into(),
                    parent_id: Some(scope.into()),
                    ..meta.clone()
                };
                // When
                for fact in [
                    NodeFact::Started(StartedFact {
                        root: None,
                        parent: Some(child_parent),
                        worktree: None,
                    }),
                    NodeFact::SubmitReceived(SubmitReceivedFact { request_id: None }),
                    NodeFact::ArtifactProduced(ArtifactProducedFact {
                        contract: None,
                        value: serde_json::json!({"passed":true,"body":"private"}),
                        request_id: None,
                    }),
                    NodeFact::StopReceived(StopReceivedFact {
                        result_summary: None,
                        token_usage: None,
                    }),
                ] {
                    append_single_fact(&store, &child, &fact, 10000 + index as i64 * 1000)
                        .await
                        .unwrap();
                }
            }
            // When
            let detail =
                session_detail(&*repository, &WorkspaceIdentity::new("/repo"), tree, parent)
                    .await
                    .unwrap()
                    .unwrap();
            let list = releashd::test_support::integration::workspace::node_for_execution(
                &*repository,
                &WorkspaceIdentity::new("/repo"),
                parent,
            )
            .await
            .unwrap()
            .unwrap();
            // Then
            assert_eq!(
                detail.classification,
                list.status_classification.as_public_str(),
                "fanout={fanout}, count={count}, index={index}"
            );
            assert_eq!(
                detail.classification,
                if count == 2 && index == 0 {
                    "active"
                } else {
                    "attention"
                }
            );
            assert_eq!(detail.capabilities.can_approve, list.can_approve);
        }
    }
}

#[tokio::test]
pub async fn test_全実行木読取_全worktreeを返し保持済みのfoldへ追記だけを反映する() {
    use releashd::test_support::integration::workflow::{AbortRequestedFact, AgentSessionActivity};
    // Given
    let first = WorkspaceIdentity::new("/repo/first");
    let second = WorkspaceIdentity::new("/repo/second");
    let (_directory, store) = standalone_session_store("first-session", &first).await;
    let sessions = LocalAgentSessionRepository::new(store.clone());
    sessions
        .create(
            AgentSession::create(
                "second-session",
                second.clone(),
                second.as_str(),
                ProviderKind::Claude,
                AgentSessionTreeLocation::session_tree_root("second-session").unwrap(),
            )
            .unwrap(),
            "second",
        )
        .await
        .unwrap();
    let repository = SqliteWorkspaceTreeRepository::new(
        store.clone(),
        std::sync::Arc::new(FailureRecordStore::default()),
    );

    // When / Then
    repository
        .folded_tree("first-session")
        .await
        .unwrap()
        .unwrap();
    repository
        .folded_workspace_trees(second.as_str())
        .await
        .unwrap();
    let before = repository.load_all_trees().await.unwrap().0;
    assert_eq!(
        before
            .iter()
            .map(|tree| tree.workspace_identity().as_str())
            .collect::<Vec<_>>(),
        vec![first.as_str(), second.as_str()]
    );
    let folds = tree_fold_count();
    assert_eq!(repository.load_all_trees().await.unwrap().0, before);
    assert_eq!(tree_fold_count(), folds);
    let mut meta = standalone_session_meta("first-session");
    meta.node_name = before[0]
        .session_node("first-session")
        .unwrap()
        .node_name
        .clone()
        .unwrap();
    append_single_fact(
        &store,
        &meta,
        &NodeFact::AgentActivityObserved(AgentActivityObservedFact {
            activity: AgentSessionActivity::AwaitingAnswer,
        }),
        10_000,
    )
    .await
    .unwrap();
    let changed = repository.load_all_trees().await.unwrap().0;
    assert_eq!(
        changed[0].session_node("first-session").unwrap().activity,
        Some(AgentSessionActivity::AwaitingAnswer)
    );
    assert_eq!(changed[1], before[1]);
    assert_eq!(tree_fold_count(), folds);
    append_single_fact(
        &store,
        &meta,
        &NodeFact::AbortRequested(AbortRequestedFact::default()),
        11_000,
    )
    .await
    .unwrap();
    let completed = repository.load_all_trees().await.unwrap().0;
    let direct = SqliteWorkspaceTreeRepository::new(
        store.clone(),
        std::sync::Arc::new(FailureRecordStore::default()),
    )
    .load_all_trees()
    .await
    .unwrap()
    .0;
    assert_eq!(completed, direct);
}

#[tokio::test]
pub async fn test_repository読取_全件と指定worktreeと単木で背景失敗を反映し解消する() {
    use releashd::test_support::integration::platform::{
        Failure, FailureKey, FailureRecordStore, TechnicalFailureNature, WorkFailure,
    };
    // Given
    let workspace = WorkspaceIdentity::new("/repo");
    let (_directory, store) = standalone_session_store("session", &workspace).await;
    let failures = Arc::new(FailureRecordStore::default());
    let repository = SqliteWorkspaceTreeRepository::new(store, failures.clone());

    let before = repository.load_all_trees().await.unwrap().0.remove(0);
    let key = FailureKey::new("workflow_recovery", "session");
    // When / Then
    for attention in [true, false] {
        if attention {
            failures.observe(
                &key,
                WorkFailure {
                    kind: Failure::Technical(TechnicalFailureNature::Other),
                    message: "repair required".into(),
                },
            );
        } else {
            failures.resolve(&key);
        }
        let all = repository.load_all_trees().await.unwrap().0.remove(0);
        let listed = repository
            .load_trees(std::slice::from_ref(&workspace))
            .await
            .remove(0)
            .unwrap();
        let single = repository
            .load_execution_tree(&workspace, "session")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(all, listed);
        assert_eq!(single, listed);
        assert_eq!(
            all.session_node("session").unwrap().error_reason.as_deref(),
            attention.then_some("repair required")
        );
        if !attention {
            assert_eq!(all, before);
        }
    }
}

#[tokio::test]
pub async fn test_全実行木読取_空のstoreと読取失敗と保持期限切れを扱う() {
    use releashd::test_support::integration::persistence::ReadFailure;
    // Given
    let workspace = WorkspaceIdentity::new("/repo");
    let (_directory, store) = standalone_session_store("session", &workspace).await;
    let mut repository = SqliteWorkspaceTreeRepository::new(
        store.clone(),
        std::sync::Arc::new(FailureRecordStore::default()),
    );
    Arc::get_mut(&mut repository).unwrap().fold_idle_limit = std::time::Duration::ZERO;
    let before = repository.load_all_trees().await.unwrap().0;
    let folds = tree_fold_count();
    // When / Then
    assert_eq!(repository.load_all_trees().await.unwrap().0, before);
    assert_eq!(tree_fold_count(), folds + 1);
    for (failure, expected) in ReadFailure::cases() {
        store.fail_next_read(failure);
        let error = repository.load_all_trees().await.unwrap_err();
        assert_eq!(
            releashd::test_support::integration::transport::classified_error(error).code,
            expected
        );
        assert_eq!(repository.load_all_trees().await.unwrap().0, before);
    }
    let directory = tempfile::tempdir().unwrap();
    let empty = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().into(),
        Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    assert!(SqliteWorkspaceTreeRepository::new(
        empty,
        std::sync::Arc::new(FailureRecordStore::default())
    )
    .load_all_trees()
    .await
    .unwrap()
    .0
    .is_empty());
}

#[tokio::test]
pub async fn test_全実行木読取_削除して同じidとheadで作り直すと所属とsessionを復元し直す() {
    // Given
    let workspace = WorkspaceIdentity::new("/repo");
    let (directory, store) = standalone_session_store("session", &workspace).await;
    let repository =
        SqliteWorkspaceTreeRepository::new(store.clone(), Arc::new(FailureRecordStore::default()));
    let single_repository =
        SqliteWorkspaceTreeRepository::new(store, Arc::new(FailureRecordStore::default()));
    let before = repository.load_all_trees().await.unwrap().0;
    assert_eq!(before[0].workspace_identity(), &workspace);
    single_repository
        .load_execution_tree(&workspace, "session")
        .await
        .unwrap()
        .unwrap();
    let connection =
        rusqlite::Connection::open(directory.path().join("local-event-store.sqlite3")).unwrap();
    // When
    recreate_session_tree(&connection, "session", "/relocated");
    let actual = repository.load_all_trees().await.unwrap().0;

    // Then
    let relocated = WorkspaceIdentity::new("/relocated");
    assert_eq!(actual.len(), 1);
    assert_eq!(actual[0].workspace_identity(), &relocated);
    assert_eq!(
        actual[0].executions()[0]
            .session
            .as_ref()
            .unwrap()
            .workspace(),
        &relocated
    );
    assert!(actual[0].session_node("session").is_some());
    let folds = tree_fold_count();
    assert_eq!(repository.load_all_trees().await.unwrap().0, actual);
    assert_eq!(tree_fold_count(), folds);
    assert_eq!(
        repository
            .load_trees(std::slice::from_ref(&relocated))
            .await
            .remove(0)
            .unwrap(),
        actual[0]
    );
    assert_eq!(
        single_repository
            .load_execution_tree(&relocated, "session")
            .await
            .unwrap()
            .unwrap(),
        actual[0]
    );
}

#[tokio::test]
pub async fn test_限定実行木読取_同じidとheadで再作成された木の所属を更新する() {
    // Given
    let workspace = WorkspaceIdentity::new("/repo");
    let (directory, store) = standalone_session_store("session", &workspace).await;
    let repository =
        SqliteWorkspaceTreeRepository::new(store, Arc::new(FailureRecordStore::default()));
    let before = repository
        .load_trees(std::slice::from_ref(&workspace))
        .await
        .remove(0)
        .unwrap();
    assert!(before.session_node("session").is_some());
    let connection =
        rusqlite::Connection::open(directory.path().join("local-event-store.sqlite3")).unwrap();
    // When
    recreate_session_tree(&connection, "session", "/relocated");
    let relocated = WorkspaceIdentity::new("/relocated");
    let actual = repository.load_trees(&[workspace, relocated.clone()]).await;
    // Then
    assert!(actual[0].as_ref().unwrap().nodes().is_empty());
    let tree = actual[1].as_ref().unwrap();
    assert_eq!(tree.workspace_identity(), &relocated);
    assert!(tree.session_node("session").is_some());
    assert_eq!(
        tree.executions()[0].session.as_ref().unwrap().workspace(),
        &relocated
    );
}

#[tokio::test]
pub async fn test_限定実行木読取_変更のないrootを読み直さず識別値の変更時に検証する() {
    use releashd::test_support::integration::workspace::{
        ExecutionReads, SqliteWorkspaceQueryService, WorkspaceQueryService,
    };
    // Given
    let workspace = WorkspaceIdentity::new("/repo");
    let (directory, store) = standalone_session_store("session", &workspace).await;
    LocalAgentSessionRepository::new(store.clone())
        .create(
            AgentSession::create(
                "other-session",
                WorkspaceIdentity::new("/other"),
                "/other",
                ProviderKind::Codex,
                AgentSessionTreeLocation::session_tree_root("other-session").unwrap(),
            )
            .unwrap(),
            "create-other-session",
        )
        .await
        .unwrap();
    let reads = ExecutionReads::new(store);
    let repository = SqliteWorkspaceTreeRepository::with_reads(
        reads.clone(),
        std::sync::Arc::new(FailureRecordStore::default()),
    );
    let query =
        SqliteWorkspaceQueryService::with_reads(reads, Arc::new(FailureRecordStore::default()));
    let before = repository.load_all_trees().await.unwrap().0;
    let expected = before
        .iter()
        .find(|tree| tree.workspace_identity() == &workspace)
        .unwrap();
    let folds = tree_fold_count();
    let connection =
        rusqlite::Connection::open(directory.path().join("local-event-store.sqlite3")).unwrap();
    connection.execute("UPDATE node_events SET detail = 'already consumed' WHERE tree_id = 'other-session' AND event_type = 'started'", []).unwrap();
    // When / Then
    for _ in 0..2 {
        assert_eq!(
            repository
                .load_trees(std::slice::from_ref(&workspace))
                .await
                .remove(0)
                .unwrap(),
            *expected
        );
        query.node_detail(&workspace, "session").await.unwrap();
        assert_eq!(tree_fold_count(), folds);
    }
    assert_eq!(repository.load_all_trees().await.unwrap().0, before);
    connection.execute("UPDATE node_events SET timestamp = timestamp + 1000 WHERE tree_id = 'other-session' AND event_type = 'started'", []).unwrap();
    assert!(repository.load_all_trees().await.is_err());
}

#[tokio::test]
pub async fn test_repository読取_実行木と子nodeの背景失敗が併存しても反映順が一定である() {
    use releashd::test_support::integration::platform::{
        Failure, FailureKey, FailureRecordStore, TechnicalFailureNature, WorkFailure,
    };
    use releashd::test_support::integration::workflow::AgentSessionActivity;
    // Given
    let workspace = WorkspaceIdentity::new("/repo");
    let (_directory, store, parent, child) =
        delegate_fixture(workspace.as_str(), AgentSessionActivity::Working).await;
    let failures = Arc::new(FailureRecordStore::default());
    for (target, message) in [
        (parent.tree_id.as_str(), "tree repair required"),
        (child.node_execution_id.as_str(), "child repair required"),
    ] {
        failures.observe(
            &FailureKey::new("workflow_recovery", target),
            WorkFailure {
                kind: Failure::Technical(TechnicalFailureNature::Other),
                message: message.into(),
            },
        );
    }
    use releashd::test_support::integration::workspace::{
        ExecutionReads, SqliteWorkspaceQueryService, WorkspaceQueryService,
    };
    let query = SqliteWorkspaceQueryService::with_reads(
        ExecutionReads::new(store.clone()),
        failures.clone(),
    );
    let repository = SqliteWorkspaceTreeRepository::new(store, failures);
    let expected = repository.load_all_trees().await.unwrap().0.remove(0);
    assert_eq!(
        expected
            .nodes()
            .iter()
            .find(|node| node.node_execution_id.as_deref() == Some(&child.node_execution_id))
            .unwrap()
            .error_reason
            .as_deref(),
        Some("tree repair required")
    );
    // When / Then
    for node in expected.nodes() {
        let detail = query
            .node_detail(&workspace, &node.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            detail.classification,
            node.status_classification.as_public_str()
        );
        assert_eq!(detail.error_reason, node.failure_reason());
    }
    for _ in 0..32 {
        assert_eq!(
            repository.load_all_trees().await.unwrap().0,
            vec![expected.clone()]
        );
        assert_eq!(
            repository
                .load_trees(std::slice::from_ref(&workspace))
                .await
                .remove(0)
                .unwrap(),
            expected
        );
        assert_eq!(
            repository
                .load_execution_tree(&workspace, &parent.tree_id)
                .await
                .unwrap(),
            Some(expected.clone())
        );
    }
}

#[tokio::test]
pub async fn test_repository読取_全件と指定workspaceと単木の失敗変換が一致する() {
    use releashd::test_support::integration::persistence::ReadFailure;
    use releashd::test_support::integration::platform::FailureRecordStore;
    use releashd::test_support::integration::transport::classified_error;
    // Given
    let workspace = WorkspaceIdentity::new("/repo");
    let (_directory, store) = standalone_session_store("session", &workspace).await;
    let repository =
        SqliteWorkspaceTreeRepository::new(store.clone(), Arc::new(FailureRecordStore::default()));
    // When / Then
    for (failure, code) in ReadFailure::cases() {
        store.fail_next_read(failure.clone());
        let all = repository.load_all_trees().await.unwrap_err();
        store.fail_next_read(failure.clone());
        let listed = repository
            .load_trees(std::slice::from_ref(&workspace))
            .await
            .remove(0)
            .unwrap_err();
        store.fail_next_read(failure);
        let single = repository
            .load_execution_tree(&workspace, "session")
            .await
            .unwrap_err();
        assert_eq!(
            std::mem::discriminant(&all),
            std::mem::discriminant(&listed)
        );
        assert_eq!(
            std::mem::discriminant(&all),
            std::mem::discriminant(&single)
        );
        for error in [all, listed, single] {
            assert_eq!(classified_error(error).code, code);
        }
    }
}

#[tokio::test]
pub async fn test_session文脈復元_全件と単木でproviderと隔離先が一致する() {
    use releashd::test_support::integration::platform::FailureRecordStore;
    use releashd::test_support::integration::workflow::IsolatedWorktree;
    // Given
    for (provider_name, provider) in [
        ("claude", ProviderKind::Claude),
        ("codex", ProviderKind::Codex),
    ] {
        for isolated in [false, true] {
            let workspace = WorkspaceIdentity::new("/repo");
            let (directory, store) = standalone_session_store("session", &workspace).await;
            let connection =
                rusqlite::Connection::open(directory.path().join("local-event-store.sqlite3"))
                    .unwrap();
            let (node_name, detail): (String, String) = connection
                .query_row(
                    "SELECT node_name, detail FROM node_events WHERE event_type = 'started'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            let mut detail: serde_json::Value = serde_json::from_str(&detail).unwrap();
            detail["root"]["repositoryRoot"] = serde_json::json!("/repo");
            detail["root"]["definition"]["nodes"][&node_name]["session"]["provider"] =
                serde_json::json!(provider_name);
            if isolated {
                detail["root"]["definition"]["nodes"][&node_name]["worktree"] =
                    serde_json::json!("isolated");
            }
            connection
                .execute(
                    "UPDATE node_events SET detail = ?1 WHERE event_type = 'started'",
                    [detail.to_string()],
                )
                .unwrap();
            let all_repository = SqliteWorkspaceTreeRepository::new(
                store.clone(),
                Arc::new(FailureRecordStore::default()),
            );
            let single_repository =
                SqliteWorkspaceTreeRepository::new(store, Arc::new(FailureRecordStore::default()));
            // When
            let all = all_repository.load_all_trees().await.unwrap().0.remove(0);
            let single = single_repository
                .load_execution_tree(&workspace, "session")
                .await
                .unwrap()
                .unwrap();
            // Then
            assert_eq!(all, single);
            let session = all.executions()[0].session.as_ref().unwrap();
            assert_eq!(session.provider(), provider);
            let expected = if isolated {
                IsolatedWorktree::for_attempt("/repo", "session", 1).path
            } else {
                "/repo".into()
            };
            assert_eq!(session.worktree_path(), expected);
        }
    }
}

#[tokio::test]
pub async fn test_全実行木読取_同じworkspaceの破損木と正常木を分離して修復後に回復する() {
    use releashd::test_support::integration::workflow::{
        AgentSessionActivity, NodeProcessPresence, NodeProcessReader, WorkflowError,
    };
    struct Processes;
    impl NodeProcessReader for Processes {
        fn presence(
            &self,
            _: &str,
            id: &str,
            _: NodeKindName,
            _: Option<&str>,
        ) -> Result<NodeProcessPresence, WorkflowError> {
            if id == "broken" {
                Err(WorkflowError::External("process read failed".into()))
            } else {
                Ok(NodeProcessPresence::Unknown)
            }
        }
    }
    for warm in [false, true] {
        // Given
        let workspace = WorkspaceIdentity::new("/repo");
        let (directory, store) = standalone_session_store("broken", &workspace).await;
        LocalAgentSessionRepository::new(store.clone())
            .create(
                AgentSession::create(
                    "healthy",
                    workspace.clone(),
                    "/repo",
                    ProviderKind::Codex,
                    AgentSessionTreeLocation::session_tree_root("healthy").unwrap(),
                )
                .unwrap(),
                "create-healthy",
            )
            .await
            .unwrap();
        let mut repository = SqliteWorkspaceTreeRepository::new(
            store.clone(),
            Arc::new(FailureRecordStore::default()),
        );
        if warm {
            let (trees, failures) = repository.load_all_trees().await.unwrap();
            assert_eq!(trees[0].executions().len(), 2);
            assert!(failures.is_empty());
        }
        let connection =
            rusqlite::Connection::open(directory.path().join("local-event-store.sqlite3")).unwrap();
        let mut meta = standalone_session_meta("broken");
        meta.node_name = connection.query_row("SELECT node_name FROM node_events WHERE tree_id = 'broken' AND event_type = 'started'", [], |row| row.get(0)).unwrap();
        append_single_fact(
            &store,
            &meta,
            &NodeFact::AgentActivityObserved(AgentActivityObservedFact {
                activity: AgentSessionActivity::Working,
            }),
            10000,
        )
        .await
        .unwrap();
        let (seq, detail): (i64, String) = connection.query_row("SELECT seq, detail FROM node_events WHERE tree_id = 'broken' ORDER BY seq DESC LIMIT 1", [], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
        connection
            .execute(
                "UPDATE node_events SET detail = 'broken' WHERE tree_id = 'broken' AND seq = ?1",
                [seq],
            )
            .unwrap();
        // When / Then
        for _ in 0..2 {
            let (trees, failures) = repository.load_all_trees().await.unwrap();
            assert_eq!(trees.len(), 1);
            assert_eq!(trees[0].executions().len(), 1);
            assert_eq!(trees[0].executions()[0].execution_id, "healthy");
            assert_eq!(failures.len(), 1);
            assert_eq!(failures[0].0, "broken");
            assert!(matches!(
                failures[0].1,
                WorkflowError::CorruptStoredState(_)
            ));
        }
        // When / Then
        connection
            .execute(
                "UPDATE node_events SET detail = ?1 WHERE tree_id = 'broken' AND seq = ?2",
                rusqlite::params![detail, seq],
            )
            .unwrap();
        let (trees, failures) = repository.load_all_trees().await.unwrap();
        assert_eq!(trees[0].executions().len(), 2);
        assert!(failures.is_empty());
        Arc::get_mut(&mut repository).unwrap().processes = Some(Arc::new(Processes));
        let (trees, failures) = repository.load_all_trees().await.unwrap();
        assert_eq!(trees[0].executions().len(), 1);
        assert_eq!(trees[0].executions()[0].execution_id, "healthy");
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].0, "broken");
        assert_eq!(
            releashd::test_support::integration::transport::classified_error(failures[0].1.clone())
                .code,
            connectrpc::ErrorCode::Internal
        );
    }
}
