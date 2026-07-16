use std::collections::HashMap;

use crate::adaptor::gateway::workflow::domain_mapping::{
    workflow_definition_to_domain, workflow_schemas_to_domain,
};
use crate::adaptor::gateway::workflow::engine_error::WorkflowEngineError;
use crate::adaptor::gateway::workflow::event::WorkflowEvent;
use crate::adaptor::gateway::workflow::runtime_state::WorkflowExecution;
use crate::adaptor::gateway::workflow::schema::WorkflowDefinitionYaml;
use crate::adaptor::gateway::workflow::state::{RuntimeArtifact, RuntimeExecutionState};
use crate::domain::workflow::services::contract_schema::SchemaViolation;
use crate::domain::workflow::services::{
    contract as workflow_contract, contract_schema, secret_masker,
};
use crate::domain::workflow::{ContractType, ContractValidationResult, NodeDefinitionName};

#[derive(Debug)]
pub(crate) struct SubmittedOutputMutation {
    pub(crate) workflow_name: String,
    prior_artifact_entry: Option<RuntimeArtifact>,
    pub(crate) node_execution_id: String,
    prior_node_execution_artifact: Option<serde_json::Value>,
    prior_fanout_child_output: Option<(Option<String>, Option<serde_json::Value>)>,
    fanout_child: bool,
}

#[derive(Debug)]
pub(crate) struct ValidatedSubmissionOutput {
    pub(crate) artifact: serde_json::Value,
    pub(crate) result: Option<String>,
}

#[derive(Debug)]
pub(crate) enum SubmissionValidationError {
    Engine(WorkflowEngineError),
    SchemaViolation {
        error: WorkflowEngineError,
        violations: Vec<SchemaViolation>,
    },
}

impl SubmissionValidationError {
    pub(crate) fn into_engine_error(self) -> WorkflowEngineError {
        match self {
            Self::Engine(error) | Self::SchemaViolation { error, .. } => error,
        }
    }

    pub(crate) fn schema_violations(&self) -> Option<&[SchemaViolation]> {
        match self {
            Self::SchemaViolation { violations, .. } => Some(violations),
            Self::Engine(_) => None,
        }
    }
}

#[derive(Debug)]
pub(crate) struct SubmissionTargetContext {
    pub(crate) workflow_name: String,
    pub(crate) worktree_path: String,
    pub(crate) session_id: Option<String>,
    pub(crate) attempt: u32,
    pub(crate) node_execution_id: String,
    pub(crate) fanout_child: bool,
}

pub(crate) fn validate_submit_output_request(
    execution_id: &str,
    node_name: &str,
    node_execution_id: Option<&str>,
    contract: &str,
) -> Result<(), WorkflowEngineError> {
    uuid::Uuid::parse_str(execution_id).map_err(|_| {
        WorkflowEngineError::ValidationError("execution_id must be UUID".to_string())
    })?;
    NodeDefinitionName::new(node_name).map_err(|_| {
        WorkflowEngineError::ValidationError("node_name must not be empty".to_string())
    })?;
    ContractType::new(contract).map_err(|_| {
        WorkflowEngineError::ValidationError("contract must not be empty".to_string())
    })?;
    if let Some(node_execution_id) = node_execution_id {
        uuid::Uuid::parse_str(node_execution_id).map_err(|_| {
            WorkflowEngineError::ValidationError(
                "node_execution_id must be UUID when provided".to_string(),
            )
        })?;
    }
    Ok(())
}

pub(crate) fn validate_submission_output_with_secrets(
    workflow: &WorkflowDefinitionYaml,
    contract: &str,
    artifact: serde_json::Value,
    secrets: &[String],
) -> Result<ValidatedSubmissionOutput, SubmissionValidationError> {
    let redacted = secret_masker::mask_sensitive_artifact(contract, artifact, secrets);
    let schemas = workflow_schemas_to_domain(&workflow.schemas);
    match workflow_contract::validate_artifact_value(&schemas, contract, redacted.clone()) {
        ContractValidationResult::Valid { artifact, result } => {
            Ok(ValidatedSubmissionOutput { artifact, result })
        }
        ContractValidationResult::Invalid(violation) => {
            let error = WorkflowEngineError::ValidationError(format!(
                "artifact schema validation failed ({}): {}",
                violation.reason, violation.details
            ));
            if violation.reason == "schema_violation" {
                let violations = schemas
                    .get(contract)
                    .and_then(|schema| contract_schema::validate(&redacted, schema, &schemas).err())
                    .unwrap_or_default();
                Err(SubmissionValidationError::SchemaViolation { error, violations })
            } else {
                Err(SubmissionValidationError::Engine(error))
            }
        }
    }
}

pub(crate) fn validate_submission_target_context(
    exec: &WorkflowExecution,
    execution_id: &str,
    node_name: &str,
    node_execution_id: Option<&str>,
    contract: &str,
) -> Result<SubmissionTargetContext, WorkflowEngineError> {
    match exec.state {
        RuntimeExecutionState::Running | RuntimeExecutionState::WaitingApproval => {}
        _ => {
            return Err(WorkflowEngineError::InvalidState(format!(
                "execution {execution_id} is not accepting structured output (state: {})",
                exec.state.as_str()
            )));
        }
    }

    let workflow = workflow_definition_to_domain(&exec.workflow);
    let expected_contract = workflow_contract::lookup_node_contract(&workflow, node_name)
        .ok_or_else(|| {
            WorkflowEngineError::ValidationError(format!(
                "node '{node_name}' is not a valid submission target"
            ))
        })?;
    if expected_contract != contract {
        return Err(WorkflowEngineError::ValidationError(format!(
            "contract mismatch: node '{node_name}' expects '{expected_contract}', got '{contract}'"
        )));
    }

    let current_node = exec
        .workflow
        .nodes
        .get(exec.current_node_index)
        .ok_or_else(|| WorkflowEngineError::InvalidState("current node is unavailable".into()))?;
    let parent_attempt = exec
        .node_execution_counts
        .get(&current_node.name)
        .copied()
        .unwrap_or(1);
    let candidates = exec
        .node_executions
        .iter()
        .filter(|execution| {
            execution.node_name == node_name
                && execution.status.is_active()
                && match execution.fanout_parent.as_ref() {
                    None => current_node.name == node_name && execution.attempt == parent_attempt,
                    Some(parent) => {
                        current_node.is_fanout()
                            && parent.parent_node == current_node.name
                            && parent.parent_attempt == parent_attempt
                    }
                }
        })
        .collect::<Vec<_>>();
    let execution = if let Some(node_execution_id) = node_execution_id {
        candidates
            .into_iter()
            .find(|execution| execution.id == node_execution_id)
            .ok_or_else(|| {
                WorkflowEngineError::InvalidState(format!(
                    "active node execution '{node_execution_id}' for node '{node_name}' was not found"
                ))
            })?
    } else {
        match candidates.as_slice() {
            [execution] => *execution,
            [] => {
                return Err(WorkflowEngineError::InvalidState(format!(
                    "node '{node_name}' is not currently accepting structured output"
                )))
            }
            candidates => {
                let candidate_ids = candidates
                    .iter()
                    .map(|execution| execution.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(WorkflowEngineError::InvalidState(format!(
                    "node '{node_name}' has {} active executions; node_execution_id is required; candidates: [{candidate_ids}]",
                    candidates.len(),
                )));
            }
        }
    };

    Ok(SubmissionTargetContext {
        workflow_name: exec.workflow.name.clone(),
        worktree_path: exec.worktree_path.clone(),
        session_id: execution.session_id.clone(),
        attempt: execution.attempt,
        node_execution_id: execution.id.clone(),
        fanout_child: execution.fanout_parent.is_some(),
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_validated_submission(
    exec: &mut WorkflowExecution,
    execution_id: &str,
    node_name: &str,
    node_execution_id: Option<&str>,
    contract: &str,
    validated_output: &serde_json::Value,
    validated_result: Option<String>,
    timestamp: f64,
) -> Result<SubmittedOutputMutation, WorkflowEngineError> {
    let target = validate_submission_target_context(
        exec,
        execution_id,
        node_name,
        node_execution_id,
        contract,
    )?;
    let execution_index = exec
        .node_executions
        .iter()
        .position(|execution| execution.id == target.node_execution_id)
        .ok_or_else(|| {
            WorkflowEngineError::InvalidState(format!(
                "node execution '{}' disappeared during output submission",
                target.node_execution_id
            ))
        })?;
    let mutation = SubmittedOutputMutation {
        workflow_name: target.workflow_name,
        prior_artifact_entry: (!target.fanout_child)
            .then(|| exec.artifacts.get(node_name).cloned())
            .flatten(),
        node_execution_id: target.node_execution_id.clone(),
        prior_node_execution_artifact: exec.node_executions[execution_index].artifact.clone(),
        prior_fanout_child_output: exec.fanout_runtime.as_ref().and_then(|fanout| {
            fanout
                .children
                .iter()
                .find(|child| child.node_execution_id == target.node_execution_id)
                .map(|child| (child.result.clone(), child.artifact.clone()))
        }),
        fanout_child: target.fanout_child,
    };
    exec.node_executions[execution_index].artifact = Some(validated_output.clone());
    if target.fanout_child {
        if let Some(child) = exec.fanout_runtime.as_mut().and_then(|fanout| {
            fanout
                .children
                .iter_mut()
                .find(|child| child.node_execution_id == target.node_execution_id)
        }) {
            child.artifact = Some(validated_output.clone());
            child.result = validated_result;
        }
    } else {
        exec.artifacts.insert(
            node_name.to_string(),
            RuntimeArtifact {
                node_name: node_name.to_string(),
                attempt: target.attempt,
                session_id: target.session_id,
                result: validated_result,
                artifact: Some(validated_output.clone()),
                contract: Some(contract.to_string()),
                token_usage: None,
                completed_at: timestamp,
            },
        );
    }
    Ok(mutation)
}

pub(crate) fn rollback_validated_submission(
    exec: &mut WorkflowExecution,
    node_name: &str,
    mutation: SubmittedOutputMutation,
) {
    if mutation.fanout_child {
        if let Some((result, artifact)) = mutation.prior_fanout_child_output.clone() {
            if let Some(child) = exec.fanout_runtime.as_mut().and_then(|fanout| {
                fanout
                    .children
                    .iter_mut()
                    .find(|child| child.node_execution_id == mutation.node_execution_id)
            }) {
                child.result = result;
                child.artifact = artifact;
            }
        }
    }
    if !mutation.fanout_child {
        match mutation.prior_artifact_entry {
            Some(prior) => {
                exec.artifacts.insert(node_name.to_string(), prior);
            }
            None => {
                exec.artifacts.remove(node_name);
            }
        }
    }
    if let Some(execution) = exec
        .node_executions
        .iter_mut()
        .find(|execution| execution.id == mutation.node_execution_id)
    {
        execution.artifact = mutation.prior_node_execution_artifact;
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn artifact_produced_event(
    execution_id: &str,
    _workflow_name: &str,
    node_execution_id: &str,
    node_name: &str,
    contract: String,
    artifact: serde_json::Value,
    request_id: Option<String>,
    submitted_at: Option<f64>,
    timestamp: f64,
) -> WorkflowEvent {
    WorkflowEvent::ArtifactProduced {
        execution_id: execution_id.to_string(),
        node_execution_id: node_execution_id.to_string(),
        node_name: node_name.to_string(),
        contract: Some(contract),
        value: artifact,
        request_id,
        submitted_at,
        timestamp,
    }
}

pub(crate) fn submitted_node_artifact_for(
    artifacts: &HashMap<String, RuntimeArtifact>,
    node_name: &str,
    attempt: u32,
    contract: &str,
) -> Option<RuntimeArtifact> {
    let output = artifacts.get(node_name)?;
    if output.attempt == attempt
        && output.contract.as_deref() == Some(contract)
        && output.artifact.is_some()
    {
        Some(output.clone())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::node_settings::WorkflowDefaults;
    use crate::adaptor::gateway::workflow::runtime_state::{
        FanoutChildRuntime, FanoutChildRuntimeState, FanoutRuntimeState, WorkflowExecution,
    };
    use crate::adaptor::gateway::workflow::schema::{
        FanoutSpec, NodeDefinition, NodeKind, NodeKindName, SchemaDef,
    };
    use crate::adaptor::gateway::workflow::state::{
        FanoutParentRef, NodeExecution, NodeExecutionStatus, RuntimeExecutionState, TokenUsage,
    };

    fn workflow_with_fanout() -> WorkflowDefinitionYaml {
        WorkflowDefinitionYaml {
            name: "wf".to_string(),
            schemas: [(
                "review-verdict".to_string(),
                SchemaDef::Object {
                    properties: Default::default(),
                    required: Default::default(),
                },
            )]
            .into_iter()
            .collect(),
            nodes: vec![
                NodeDefinition {
                    name: "fanout-review".to_string(),
                    kind: NodeKind::Fanout(FanoutSpec {
                        child: vec!["review-a".to_string(), "review-b".to_string()],
                        items: None,
                    }),
                    ..Default::default()
                },
                NodeDefinition {
                    name: "review-a".to_string(),
                    artifact: Some("review-verdict".to_string()),
                    ..Default::default()
                },
                NodeDefinition {
                    name: "review-b".to_string(),
                    artifact: Some("review-verdict".to_string()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    fn node_execution(
        id: &str,
        node_name: &str,
        attempt: u32,
        status: NodeExecutionStatus,
        session_id: Option<&str>,
        fanout_parent: Option<FanoutParentRef>,
    ) -> NodeExecution {
        NodeExecution {
            id: id.to_string(),
            execution_id: "execution-1".to_string(),
            node_name: node_name.to_string(),
            kind: NodeKindName::Session,
            attempt,
            status,
            session_id: session_id.map(str::to_string),
            display_command: None,
            artifact: None,
            token_usage: None,
            failure: None,
            fanout_parent,
            started_at: 1.0,
            completed_at: None,
        }
    }

    fn fanout_execution() -> WorkflowExecution {
        let mut exec = running_execution();
        exec.workflow = workflow_with_fanout();
        exec.current_session_id = None;
        exec.node_execution_counts = HashMap::from([
            ("fanout-review".to_string(), 1),
            ("review-a".to_string(), 3),
            ("review-b".to_string(), 4),
        ]);
        exec.fanout_runtime = Some(FanoutRuntimeState {
            parent_node_name: "fanout-review".to_string(),
            parent_node_execution_id: "00000000-0000-4000-8000-000000000200".to_string(),
            children: vec![
                FanoutChildRuntime {
                    node_execution_id: "00000000-0000-4000-8000-000000000201".to_string(),
                    node_name: "review-a".to_string(),
                    session_id: "session-a".to_string(),
                    state: FanoutChildRuntimeState::Running,
                    result: None,
                    artifact: None,
                    contract: Some("review-verdict".to_string()),
                    failure_kind: None,
                    failure_disposition: None,
                    token_usage: TokenUsage::default(),
                    attempt: 3,
                    completed_at: None,
                },
                FanoutChildRuntime {
                    node_execution_id: "00000000-0000-4000-8000-000000000202".to_string(),
                    node_name: "review-b".to_string(),
                    session_id: "session-b".to_string(),
                    state: FanoutChildRuntimeState::Completed,
                    result: None,
                    artifact: None,
                    contract: Some("review-verdict".to_string()),
                    failure_kind: None,
                    failure_disposition: None,
                    token_usage: TokenUsage::default(),
                    attempt: 4,
                    completed_at: Some(2.0),
                },
            ],
        });
        exec.node_executions = vec![
            node_execution(
                "00000000-0000-4000-8000-000000000200",
                "fanout-review",
                1,
                NodeExecutionStatus::Running,
                None,
                None,
            ),
            node_execution(
                "00000000-0000-4000-8000-000000000201",
                "review-a",
                3,
                NodeExecutionStatus::Running,
                Some("session-a"),
                Some(FanoutParentRef {
                    parent_node: "fanout-review".to_string(),
                    parent_attempt: 1,
                    item_index: None,
                    child_index: 0,
                }),
            ),
            node_execution(
                "00000000-0000-4000-8000-000000000202",
                "review-b",
                4,
                NodeExecutionStatus::Succeeded,
                Some("session-b"),
                Some(FanoutParentRef {
                    parent_node: "fanout-review".to_string(),
                    parent_attempt: 1,
                    item_index: None,
                    child_index: 1,
                }),
            ),
        ];
        exec
    }

    fn repeated_fanout_child_execution() -> WorkflowExecution {
        let mut exec = fanout_execution();
        let node_execution_id = "00000000-0000-4000-8000-000000000203";
        exec.node_executions.push(node_execution(
            node_execution_id,
            "review-a",
            4,
            NodeExecutionStatus::Running,
            Some("session-a-2"),
            Some(FanoutParentRef {
                parent_node: "fanout-review".to_string(),
                parent_attempt: 1,
                item_index: Some(1),
                child_index: 0,
            }),
        ));
        exec.fanout_runtime
            .as_mut()
            .unwrap()
            .children
            .push(FanoutChildRuntime {
                node_execution_id: node_execution_id.to_string(),
                node_name: "review-a".to_string(),
                session_id: "session-a-2".to_string(),
                state: FanoutChildRuntimeState::Running,
                result: None,
                artifact: None,
                contract: Some("review-verdict".to_string()),
                failure_kind: None,
                failure_disposition: None,
                token_usage: TokenUsage::default(),
                attempt: 4,
                completed_at: None,
            });
        exec
    }

    fn node_output(attempt: u32, contract: Option<&str>, structured: bool) -> RuntimeArtifact {
        RuntimeArtifact {
            node_name: "review-a".to_string(),
            attempt,
            session_id: None,
            result: Some("LGTM".to_string()),
            artifact: structured.then(|| serde_json::json!({"verdict": "LGTM"})),
            contract: contract.map(ToOwned::to_owned),
            token_usage: Some(TokenUsage::default()),
            completed_at: 1000.0,
        }
    }

    fn running_execution() -> WorkflowExecution {
        WorkflowExecution {
            id: "execution-1".to_string(),
            workflow: WorkflowDefinitionYaml {
                name: "wf".to_string(),
                nodes: vec![NodeDefinition {
                    name: "review".to_string(),
                    artifact: Some("review-verdict".to_string()),
                    ..NodeDefinition::default()
                }],
                ..WorkflowDefinitionYaml::default()
            },
            state: RuntimeExecutionState::Running,
            current_node_index: 0,
            node_execution_counts: HashMap::from([("review".to_string(), 2)]),
            node_history: Vec::new(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
            worktree_path: "/tmp/wt".to_string(),
            created_from: crate::domain::workflow::ExecutionOrigin::Cli,
            error_reason: None,
            started_at: 1.0,
            updated_at: 2.0,
            current_session_id: None,
            current_node_token_usage: TokenUsage::default(),
            artifacts: HashMap::new(),
            node_executions: vec![node_execution(
                "00000000-0000-4000-8000-000000000101",
                "review",
                2,
                NodeExecutionStatus::Running,
                Some("session-current"),
                None,
            )],
            request: None,
            fanout_runtime: None,
            current_stall_observations: Vec::new(),
        }
    }

    #[test]
    fn submitted_node_output_requires_matching_attempt_contract_and_artifact() {
        let outputs = HashMap::from([(
            "review-a".to_string(),
            node_output(2, Some("review-verdict"), true),
        )]);

        assert!(submitted_node_artifact_for(&outputs, "review-a", 2, "review-verdict").is_some());
        assert!(submitted_node_artifact_for(&outputs, "review-a", 1, "review-verdict").is_none());
        assert!(submitted_node_artifact_for(&outputs, "review-a", 2, "other-contract").is_none());

        let outputs = HashMap::from([(
            "review-a".to_string(),
            node_output(2, Some("review-verdict"), false),
        )]);
        assert!(submitted_node_artifact_for(&outputs, "review-a", 2, "review-verdict").is_none());
    }

    #[test]
    fn validate_submit_output_request_rejects_invalid_identity_fields() {
        assert!(matches!(
            validate_submit_output_request("not-a-uuid", "review", None, "review-verdict"),
            Err(WorkflowEngineError::ValidationError(message))
                if message == "execution_id must be UUID"
        ));
        assert!(matches!(
            validate_submit_output_request(
                "00000000-0000-0000-0000-000000000001",
                " ",
                None,
                "review-verdict"
            ),
            Err(WorkflowEngineError::ValidationError(message))
                if message == "node_name must not be empty"
        ));
        assert!(matches!(
            validate_submit_output_request(
                "00000000-0000-0000-0000-000000000001",
                "review",
                None,
                ""
            ),
            Err(WorkflowEngineError::ValidationError(message))
                if message == "contract must not be empty"
        ));
    }

    #[test]
    fn validate_submission_output_with_secrets_accepts_schema_valid_artifact() {
        let workflow = WorkflowDefinitionYaml {
            name: "wf".to_string(),
            description: String::new(),
            builtin: false,
            schemas: [(
                "spec-directory".to_string(),
                SchemaDef::Object {
                    properties: [
                        ("spec_dir".to_string(), SchemaDef::String { r#enum: None }),
                        ("design".to_string(), SchemaDef::String { r#enum: None }),
                    ]
                    .into_iter()
                    .collect(),
                    required: ["spec_dir".to_string(), "design".to_string()]
                        .into_iter()
                        .collect(),
                },
            )]
            .into_iter()
            .collect(),
            nodes: vec![],
        };
        let validated = validate_submission_output_with_secrets(
            &workflow,
            "spec-directory",
            serde_json::json!({
                "spec_dir": "docs/specs/feat-token",
                "design": "design.md"
            }),
            &["SECRET_TOKEN".to_string()],
        )
        .unwrap();

        assert_eq!(validated.artifact["spec_dir"], "docs/specs/feat-token");
    }

    #[test]
    fn validate_submission_output_accepts_authoring_builtin_spec_directory_contract() {
        for workflow_name in [
            "01_authoring_draft",
            "01_authoring_codex",
            "01_authoring_claude",
        ] {
            let workflow =
                crate::adaptor::gateway::workflow::builtin::load_builtin_workflow_resolved(
                    workflow_name,
                )
                .unwrap()
                .unwrap();
            let validated = validate_submission_output_with_secrets(
                &workflow,
                "spec-directory",
                serde_json::json!({
                    "spec_dir": "docs/specs/issues-1327",
                    "spec_id": "issues-1327",
                    "files": ["docs/specs/issues-1327/requirements.md"],
                    "related_issue": 1327,
                    "open_questions_resolved": true
                }),
                &[],
            )
            .unwrap();

            assert_eq!(
                validated.artifact["spec_dir"], "docs/specs/issues-1327",
                "{workflow_name} must accept spec-directory output"
            );
        }
    }

    #[test]
    fn artifact_produced_event_preserves_external_shape() {
        let event = artifact_produced_event(
            "execution-1",
            "wf",
            "node-execution-1",
            "review",
            "review-verdict".to_string(),
            serde_json::json!({"verdict": "LGTM"}),
            Some("request-1".to_string()),
            Some(10.0),
            20.0,
        );

        assert!(matches!(
            event,
            WorkflowEvent::ArtifactProduced {
                execution_id,
                node_name,
                contract,
                request_id: Some(request_id),
                submitted_at: Some(submitted_at),
                timestamp,
                ..
            } if execution_id == "execution-1"
                && node_name == "review"
                && contract.as_deref() == Some("review-verdict")
                && request_id == "request-1"
                && (submitted_at - 10.0).abs() < f64::EPSILON
                && (timestamp - 20.0).abs() < f64::EPSILON
        ));
    }

    #[test]
    fn validate_submission_target_context_returns_current_node_target_details() {
        let mut exec = running_execution();
        exec.current_session_id = Some("session-current".to_string());

        let target = validate_submission_target_context(
            &exec,
            "execution-1",
            "review",
            None,
            "review-verdict",
        )
        .unwrap();

        assert_eq!(target.workflow_name, "wf");
        assert_eq!(target.worktree_path, "/tmp/wt");
        assert_eq!(target.session_id.as_deref(), Some("session-current"));
        assert_eq!(target.attempt, 2);
    }

    #[test]
    fn validate_submission_target_context_returns_fanout_child_target_details() {
        let exec = fanout_execution();

        let target = validate_submission_target_context(
            &exec,
            "execution-1",
            "review-a",
            Some("00000000-0000-4000-8000-000000000201"),
            "review-verdict",
        )
        .unwrap();

        assert_eq!(target.workflow_name, "wf");
        assert_eq!(target.worktree_path, "/tmp/wt");
        assert_eq!(target.session_id.as_deref(), Some("session-a"));
        assert_eq!(target.attempt, 3);
    }

    #[test]
    fn validate_submission_target_context_requires_id_for_repeated_fanout_child() {
        let exec = repeated_fanout_child_execution();

        let error = validate_submission_target_context(
            &exec,
            "execution-1",
            "review-a",
            None,
            "review-verdict",
        )
        .unwrap_err();

        assert!(matches!(
            error,
            WorkflowEngineError::InvalidState(message)
                if message.contains("node_execution_id is required")
                    && message.contains("00000000-0000-4000-8000-000000000201")
                    && message.contains("00000000-0000-4000-8000-000000000203")
        ));
    }

    #[test]
    fn apply_and_rollback_fanout_submission_updates_only_addressed_execution() {
        let mut exec = repeated_fanout_child_execution();
        let addressed_id = "00000000-0000-4000-8000-000000000203";
        let value = serde_json::json!({"verdict": "LGTM"});

        let mutation = apply_validated_submission(
            &mut exec,
            "execution-1",
            "review-a",
            Some(addressed_id),
            "review-verdict",
            &value,
            Some("LGTM".to_string()),
            42.0,
        )
        .unwrap();

        assert!(!exec.artifacts.contains_key("review-a"));
        assert_eq!(
            exec.node_executions
                .iter()
                .find(|execution| execution.id == addressed_id)
                .and_then(|execution| execution.artifact.as_ref()),
            Some(&value)
        );
        assert!(exec
            .node_executions
            .iter()
            .find(|execution| execution.id.ends_with("201"))
            .unwrap()
            .artifact
            .is_none());
        assert_eq!(
            exec.fanout_runtime
                .as_ref()
                .unwrap()
                .children
                .iter()
                .find(|child| child.node_execution_id == addressed_id)
                .and_then(|child| child.result.as_deref()),
            Some("LGTM")
        );

        rollback_validated_submission(&mut exec, "review-a", mutation);

        assert!(exec
            .node_executions
            .iter()
            .find(|execution| execution.id == addressed_id)
            .unwrap()
            .artifact
            .is_none());
        assert!(exec
            .fanout_runtime
            .as_ref()
            .unwrap()
            .children
            .iter()
            .find(|child| child.node_execution_id == addressed_id)
            .unwrap()
            .artifact
            .is_none());
    }

    #[test]
    fn apply_validated_submission_updates_node_output_without_legacy_variable_side_effects() {
        let mut exec = running_execution();
        let mutation = apply_validated_submission(
            &mut exec,
            "execution-1",
            "review",
            None,
            "review-verdict",
            &serde_json::json!({"verdict": "LGTM"}),
            Some("LGTM".to_string()),
            42.0,
        )
        .unwrap();

        assert_eq!(mutation.workflow_name, "wf");
        let output = exec.artifacts.get("review").unwrap();
        assert_eq!(output.attempt, 2);
        assert_eq!(output.contract.as_deref(), Some("review-verdict"));
        assert_eq!(output.result.as_deref(), Some("LGTM"));
        assert_eq!(output.completed_at, 42.0);
    }

    #[test]
    fn apply_validated_submission_rejects_contract_mismatch_without_mutation() {
        let mut exec = running_execution();
        let err = apply_validated_submission(
            &mut exec,
            "execution-1",
            "review",
            None,
            "other-contract",
            &serde_json::json!({"verdict": "LGTM"}),
            Some("LGTM".to_string()),
            42.0,
        )
        .unwrap_err();

        assert!(matches!(err, WorkflowEngineError::ValidationError(_)));
        assert!(exec.artifacts.is_empty());
    }

    #[test]
    fn apply_validated_submission_rejects_non_accepting_fanout_child_without_mutation() {
        let mut exec = fanout_execution();
        let err = apply_validated_submission(
            &mut exec,
            "execution-1",
            "review-b",
            Some("00000000-0000-4000-8000-000000000202"),
            "review-verdict",
            &serde_json::json!({"verdict": "LGTM"}),
            Some("LGTM".to_string()),
            42.0,
        )
        .unwrap_err();

        assert!(matches!(err, WorkflowEngineError::InvalidState(_)));
        assert!(exec.artifacts.is_empty());
    }

    #[test]
    fn rollback_validated_submission_restores_previous_output() {
        let mut exec = running_execution();
        exec.artifacts.insert(
            "review".to_string(),
            node_output(1, Some("review-verdict"), true),
        );
        let mutation = apply_validated_submission(
            &mut exec,
            "execution-1",
            "review",
            None,
            "review-verdict",
            &serde_json::json!({"verdict": "NEEDS_WORK"}),
            Some("NEEDS_WORK".to_string()),
            42.0,
        )
        .unwrap();

        rollback_validated_submission(&mut exec, "review", mutation);

        let output = exec.artifacts.get("review").unwrap();
        assert_eq!(output.attempt, 1);
        assert_eq!(output.result.as_deref(), Some("LGTM"));
    }
}
