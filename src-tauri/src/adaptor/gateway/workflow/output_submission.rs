use std::collections::HashMap;

use crate::adaptor::gateway::workflow::domain_mapping::{
    workflow_definition_to_domain, workflow_schemas_to_domain,
};
use crate::adaptor::gateway::workflow::engine_error::WorkflowEngineError;
use crate::adaptor::gateway::workflow::event::WorkflowEvent;
use crate::adaptor::gateway::workflow::runtime_state::WorkflowExecution;
use crate::adaptor::gateway::workflow::schema::Workflow;
use crate::adaptor::gateway::workflow::state::{StepOutput, WorkflowExecutionState};
use crate::domain::workflow::services::contract_schema::SchemaViolation;
use crate::domain::workflow::services::{
    contract as workflow_contract, contract_schema, secret_masker, submission as domain_submission,
};
use crate::domain::workflow::{ContractType, ContractValidationResult, NodeName};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SubmissionParallelChildState {
    Running,
    Completed,
    Failed,
    Interrupted,
}

pub(crate) struct SubmissionParallelChild<'a> {
    pub(crate) step_name: &'a str,
    pub(crate) state: SubmissionParallelChildState,
}

pub(crate) struct SubmissionParallelRun<'a> {
    pub(crate) parent_step_name: &'a str,
    pub(crate) children: &'a [SubmissionParallelChild<'a>],
}

#[derive(Debug)]
pub(crate) struct SubmittedOutputMutation {
    pub(crate) workflow_name: String,
    prior_step_output: Option<StepOutput>,
}

#[derive(Debug)]
pub(crate) struct ValidatedSubmissionOutput {
    pub(crate) structured_output: serde_json::Value,
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

pub(crate) struct SubmissionTargetContext {
    pub(crate) workflow_name: String,
    pub(crate) worktree_path: String,
    pub(crate) session_id: Option<String>,
    pub(crate) run_index: u32,
}

pub(crate) fn validate_submit_output_request(
    run_id: &str,
    step_name: &str,
    contract: &str,
) -> Result<(), WorkflowEngineError> {
    uuid::Uuid::parse_str(run_id)
        .map_err(|_| WorkflowEngineError::ValidationError("run_id must be UUID".to_string()))?;
    NodeName::new(step_name).map_err(|_| {
        WorkflowEngineError::ValidationError("step_name must not be empty".to_string())
    })?;
    ContractType::new(contract).map_err(|_| {
        WorkflowEngineError::ValidationError("contract must not be empty".to_string())
    })?;
    Ok(())
}

pub(crate) fn validate_submission_output_with_secrets(
    workflow: &Workflow,
    contract: &str,
    structured_output: serde_json::Value,
    secrets: &[String],
) -> Result<ValidatedSubmissionOutput, SubmissionValidationError> {
    let redacted =
        secret_masker::mask_sensitive_structured_output(contract, structured_output, secrets);
    let schemas = workflow_schemas_to_domain(&workflow.schemas);
    match workflow_contract::validate_artifact_value(&schemas, contract, redacted.clone()) {
        ContractValidationResult::Valid {
            structured_output,
            result,
        } => Ok(ValidatedSubmissionOutput {
            structured_output,
            result,
        }),
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

pub(crate) fn is_accepting_submission_target(
    workflow: &Workflow,
    current_step_index: usize,
    parallel_run: Option<SubmissionParallelRun<'_>>,
    step_name: &str,
) -> bool {
    let workflow = workflow_definition_to_domain(workflow);
    let parallel_children = parallel_run.as_ref().map(|parallel| {
        parallel
            .children
            .iter()
            .map(|child| domain_submission::SubmissionParallelChild {
                step_name: child.step_name,
                state: match child.state {
                    SubmissionParallelChildState::Running => {
                        domain_submission::SubmissionParallelChildState::Running
                    }
                    SubmissionParallelChildState::Completed => {
                        domain_submission::SubmissionParallelChildState::Completed
                    }
                    SubmissionParallelChildState::Failed => {
                        domain_submission::SubmissionParallelChildState::Failed
                    }
                    SubmissionParallelChildState::Interrupted => {
                        domain_submission::SubmissionParallelChildState::Interrupted
                    }
                },
            })
            .collect::<Vec<_>>()
    });
    let parallel_run =
        parallel_run
            .as_ref()
            .map(|parallel| domain_submission::SubmissionParallelRun {
                parent_step_name: parallel.parent_step_name,
                children: parallel_children.as_deref().unwrap_or(&[]),
            });
    domain_submission::is_accepting_submission_target(
        &workflow,
        current_step_index,
        parallel_run,
        step_name,
    )
}

pub(crate) fn validate_submission_target_context(
    exec: &WorkflowExecution,
    run_id: &str,
    step_name: &str,
    contract: &str,
) -> Result<SubmissionTargetContext, WorkflowEngineError> {
    match exec.state {
        WorkflowExecutionState::Running | WorkflowExecutionState::WaitingApproval => {}
        _ => {
            return Err(WorkflowEngineError::InvalidState(format!(
                "run {run_id} is not accepting structured output (state: {})",
                exec.state.as_str()
            )));
        }
    }

    let workflow = workflow_definition_to_domain(&exec.workflow);
    let expected_contract = workflow_contract::lookup_node_artifact_contract(&workflow, step_name)
        .ok_or_else(|| {
            WorkflowEngineError::ValidationError(format!(
                "step '{step_name}' is not a valid submission target"
            ))
        })?;
    if expected_contract != contract {
        return Err(WorkflowEngineError::ValidationError(format!(
            "contract mismatch: step '{step_name}' expects '{expected_contract}', got '{contract}'"
        )));
    }

    let parallel_children = exec.parallel_run.as_ref().map(|parallel| {
        parallel
            .children
            .iter()
            .map(|child| SubmissionParallelChild {
                step_name: child.step_name.as_str(),
                state: (&child.state).into(),
            })
            .collect::<Vec<_>>()
    });
    let parallel_run = exec
        .parallel_run
        .as_ref()
        .map(|parallel| SubmissionParallelRun {
            parent_step_name: parallel.parent_step_name.as_str(),
            children: parallel_children.as_deref().unwrap_or(&[]),
        });
    if !is_accepting_submission_target(
        &exec.workflow,
        exec.current_step_index,
        parallel_run,
        step_name,
    ) {
        return Err(WorkflowEngineError::InvalidState(format!(
            "step '{step_name}' is not currently accepting structured output"
        )));
    }

    let session_id = exec
        .parallel_run
        .as_ref()
        .and_then(|parallel| {
            parallel
                .children
                .iter()
                .find(|child| child.step_name == step_name)
                .map(|child| child.session_id.clone())
        })
        .filter(|session_id| !session_id.is_empty())
        .or_else(|| {
            let current_node = exec.workflow.nodes.get(exec.current_step_index)?;
            (current_node.name == step_name)
                .then(|| exec.current_session_id.clone())
                .flatten()
        });

    Ok(SubmissionTargetContext {
        workflow_name: exec.workflow.name.clone(),
        worktree_path: exec.worktree_path.clone(),
        session_id,
        run_index: exec
            .step_execution_counts
            .get(step_name)
            .copied()
            .unwrap_or(0),
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_validated_submission(
    exec: &mut WorkflowExecution,
    run_id: &str,
    step_name: &str,
    contract: &str,
    validated_output: &serde_json::Value,
    validated_result: Option<String>,
    timestamp: f64,
) -> Result<SubmittedOutputMutation, WorkflowEngineError> {
    let target = validate_submission_target_context(exec, run_id, step_name, contract)?;
    let mutation = SubmittedOutputMutation {
        workflow_name: target.workflow_name,
        prior_step_output: exec.step_outputs.get(step_name).cloned(),
    };
    exec.step_outputs.insert(
        step_name.to_string(),
        StepOutput {
            step_name: step_name.to_string(),
            run_index: target.run_index,
            session_id: None,
            result: validated_result,
            structured_output: Some(validated_output.clone()),
            artifact_contract: Some(contract.to_string()),
            token_usage: None,
            completed_at: timestamp,
        },
    );
    Ok(mutation)
}

pub(crate) fn rollback_validated_submission(
    exec: &mut WorkflowExecution,
    step_name: &str,
    mutation: SubmittedOutputMutation,
) {
    match mutation.prior_step_output {
        Some(prior) => {
            exec.step_outputs.insert(step_name.to_string(), prior);
        }
        None => {
            exec.step_outputs.remove(step_name);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn artifact_produced_event(
    run_id: &str,
    workflow_name: &str,
    step_name: &str,
    contract: String,
    structured_output: serde_json::Value,
    request_id: Option<String>,
    submitted_at: Option<f64>,
    timestamp: f64,
) -> WorkflowEvent {
    WorkflowEvent::ArtifactProduced {
        run_id: run_id.to_string(),
        workflow_name: workflow_name.to_string(),
        node_name: step_name.to_string(),
        contract: Some(contract),
        value: structured_output,
        request_id,
        submitted_at,
        timestamp,
    }
}

pub(crate) fn submitted_step_output_for(
    step_outputs: &HashMap<String, StepOutput>,
    step_name: &str,
    run_index: u32,
    contract: &str,
) -> Option<StepOutput> {
    let output = step_outputs.get(step_name)?;
    if output.run_index == run_index
        && output.artifact_contract.as_deref() == Some(contract)
        && output.structured_output.is_some()
    {
        Some(output.clone())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::runtime_state::{
        ParallelChildRun, ParallelChildState, ParallelRunState, WorkflowExecution,
    };
    use crate::adaptor::gateway::workflow::schema::{
        FanoutSpec, InterimChild, NodeDefinition, NodeKind, SchemaDef,
    };
    use crate::adaptor::gateway::workflow::state::{TokenUsage, WorkflowExecutionState};
    use crate::adaptor::gateway::workflow::step_settings::WorkflowDefaults;

    fn workflow_with_parallel() -> Workflow {
        Workflow {
            name: "wf".to_string(),
            schemas: [(
                "review-verdict".to_string(),
                SchemaDef::Object {
                    properties: Default::default(),
                    required: Default::default(),
                    additional_properties: false,
                },
            )]
            .into_iter()
            .collect(),
            nodes: vec![NodeDefinition {
                name: "parallel-review".to_string(),
                kind: NodeKind::Fanout(FanoutSpec {
                    parallel_children: vec![
                        InterimChild {
                            name: "review-a".to_string(),
                            artifact: Some("review-verdict".to_string()),
                            ..Default::default()
                        },
                        InterimChild {
                            name: "review-b".to_string(),
                            artifact: Some("review-verdict".to_string()),
                            ..Default::default()
                        },
                    ],
                    aggregate: None,
                }),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn parallel_execution() -> WorkflowExecution {
        let mut exec = running_execution();
        exec.workflow = workflow_with_parallel();
        exec.current_session_id = None;
        exec.step_execution_counts =
            HashMap::from([("review-a".to_string(), 3), ("review-b".to_string(), 4)]);
        exec.parallel_run = Some(ParallelRunState {
            parent_step_name: "parallel-review".to_string(),
            aggregate: None,
            children: vec![
                ParallelChildRun {
                    step_name: "review-a".to_string(),
                    session_id: "session-a".to_string(),
                    state: ParallelChildState::Running,
                    result: None,
                    structured_output: None,
                    artifact_contract: Some("review-verdict".to_string()),
                    failure_kind: None,
                    failure_disposition: None,
                    token_usage: TokenUsage::default(),
                    run_index: 3,
                },
                ParallelChildRun {
                    step_name: "review-b".to_string(),
                    session_id: "session-b".to_string(),
                    state: ParallelChildState::Completed,
                    result: None,
                    structured_output: None,
                    artifact_contract: Some("review-verdict".to_string()),
                    failure_kind: None,
                    failure_disposition: None,
                    token_usage: TokenUsage::default(),
                    run_index: 4,
                },
            ],
        });
        exec
    }

    fn step_output(run_index: u32, contract: Option<&str>, structured: bool) -> StepOutput {
        StepOutput {
            step_name: "review-a".to_string(),
            run_index,
            session_id: None,
            result: Some("LGTM".to_string()),
            structured_output: structured.then(|| serde_json::json!({"verdict": "LGTM"})),
            artifact_contract: contract.map(ToOwned::to_owned),
            token_usage: Some(TokenUsage::default()),
            completed_at: 1000.0,
        }
    }

    fn running_execution() -> WorkflowExecution {
        WorkflowExecution {
            id: "run-1".to_string(),
            workflow: Workflow {
                name: "wf".to_string(),
                nodes: vec![NodeDefinition {
                    name: "review".to_string(),
                    artifact: Some("review-verdict".to_string()),
                    ..NodeDefinition::default()
                }],
                ..Workflow::default()
            },
            state: WorkflowExecutionState::Running,
            current_step_index: 0,
            step_execution_counts: HashMap::from([("review".to_string(), 2)]),
            step_history: Vec::new(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
            worktree_path: "/tmp/wt".to_string(),
            started_at: 1.0,
            updated_at: 2.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            current_stall_observations: Vec::new(),
        }
    }

    #[test]
    fn accepting_submission_target_accepts_only_running_parallel_child() {
        let children = [
            SubmissionParallelChild {
                step_name: "review-a",
                state: SubmissionParallelChildState::Running,
            },
            SubmissionParallelChild {
                step_name: "review-b",
                state: SubmissionParallelChildState::Completed,
            },
        ];
        let parallel_run = SubmissionParallelRun {
            parent_step_name: "parallel-review",
            children: &children,
        };

        assert!(is_accepting_submission_target(
            &workflow_with_parallel(),
            0,
            Some(parallel_run),
            "review-a",
        ));

        let parallel_run = SubmissionParallelRun {
            parent_step_name: "parallel-review",
            children: &children,
        };
        assert!(!is_accepting_submission_target(
            &workflow_with_parallel(),
            0,
            Some(parallel_run),
            "review-b",
        ));
    }

    #[test]
    fn submitted_step_output_requires_matching_run_index_contract_and_structured_output() {
        let outputs = HashMap::from([(
            "review-a".to_string(),
            step_output(2, Some("review-verdict"), true),
        )]);

        assert!(submitted_step_output_for(&outputs, "review-a", 2, "review-verdict").is_some());
        assert!(submitted_step_output_for(&outputs, "review-a", 1, "review-verdict").is_none());
        assert!(submitted_step_output_for(&outputs, "review-a", 2, "other-contract").is_none());

        let outputs = HashMap::from([(
            "review-a".to_string(),
            step_output(2, Some("review-verdict"), false),
        )]);
        assert!(submitted_step_output_for(&outputs, "review-a", 2, "review-verdict").is_none());
    }

    #[test]
    fn validate_submit_output_request_rejects_invalid_identity_fields() {
        assert!(matches!(
            validate_submit_output_request("not-a-uuid", "review", "review-verdict"),
            Err(WorkflowEngineError::ValidationError(message))
                if message == "run_id must be UUID"
        ));
        assert!(matches!(
            validate_submit_output_request(
                "00000000-0000-0000-0000-000000000001",
                " ",
                "review-verdict"
            ),
            Err(WorkflowEngineError::ValidationError(message))
                if message == "step_name must not be empty"
        ));
        assert!(matches!(
            validate_submit_output_request("00000000-0000-0000-0000-000000000001", "review", ""),
            Err(WorkflowEngineError::ValidationError(message))
                if message == "contract must not be empty"
        ));
    }

    #[test]
    fn validate_submission_output_with_secrets_accepts_schema_valid_artifact() {
        let workflow = Workflow {
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
                    additional_properties: false,
                },
            )]
            .into_iter()
            .collect(),
            variables: Default::default(),
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

        assert_eq!(
            validated.structured_output["spec_dir"],
            "docs/specs/feat-token"
        );
    }

    #[test]
    fn validate_submission_output_with_secrets_rejects_unsafe_spec_dir() {
        let workflow = Workflow {
            name: "wf".to_string(),
            description: String::new(),
            builtin: false,
            schemas: [(
                "spec-directory".to_string(),
                SchemaDef::Object {
                    properties: [("spec_dir".to_string(), SchemaDef::String { r#enum: None })]
                        .into_iter()
                        .collect(),
                    required: ["spec_dir".to_string()].into_iter().collect(),
                    additional_properties: false,
                },
            )]
            .into_iter()
            .collect(),
            variables: Default::default(),
            nodes: vec![],
        };

        for spec_dir in ["/tmp/spec", "../outside"] {
            let err = validate_submission_output_with_secrets(
                &workflow,
                "spec-directory",
                serde_json::json!({ "spec_dir": spec_dir }),
                &[],
            )
            .unwrap_err();
            assert!(matches!(
                err,
                SubmissionValidationError::SchemaViolation { ref violations, .. }
                    if violations.iter().any(|violation| violation.path == "$.spec_dir")
            ));
        }
    }

    #[test]
    fn artifact_produced_event_preserves_external_shape() {
        let event = artifact_produced_event(
            "run-1",
            "wf",
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
                run_id,
                workflow_name,
                node_name,
                contract,
                request_id: Some(request_id),
                submitted_at: Some(submitted_at),
                timestamp,
                ..
            } if run_id == "run-1"
                && workflow_name == "wf"
                && node_name == "review"
                && contract.as_deref() == Some("review-verdict")
                && request_id == "request-1"
                && (submitted_at - 10.0).abs() < f64::EPSILON
                && (timestamp - 20.0).abs() < f64::EPSILON
        ));
    }

    #[test]
    fn validate_submission_target_context_returns_current_step_target_details() {
        let mut exec = running_execution();
        exec.current_session_id = Some("session-current".to_string());

        let target =
            validate_submission_target_context(&exec, "run-1", "review", "review-verdict").unwrap();

        assert_eq!(target.workflow_name, "wf");
        assert_eq!(target.worktree_path, "/tmp/wt");
        assert_eq!(target.session_id.as_deref(), Some("session-current"));
        assert_eq!(target.run_index, 2);
    }

    #[test]
    fn validate_submission_target_context_returns_parallel_child_target_details() {
        let exec = parallel_execution();

        let target =
            validate_submission_target_context(&exec, "run-1", "review-a", "review-verdict")
                .unwrap();

        assert_eq!(target.workflow_name, "wf");
        assert_eq!(target.worktree_path, "/tmp/wt");
        assert_eq!(target.session_id.as_deref(), Some("session-a"));
        assert_eq!(target.run_index, 3);
    }

    #[test]
    fn apply_validated_submission_updates_step_output_without_workflow_variable_side_effects() {
        let mut exec = running_execution();
        let mutation = apply_validated_submission(
            &mut exec,
            "run-1",
            "review",
            "review-verdict",
            &serde_json::json!({"verdict": "LGTM"}),
            Some("LGTM".to_string()),
            42.0,
        )
        .unwrap();

        assert_eq!(mutation.workflow_name, "wf");
        let output = exec.step_outputs.get("review").unwrap();
        assert_eq!(output.run_index, 2);
        assert_eq!(output.artifact_contract.as_deref(), Some("review-verdict"));
        assert_eq!(output.result.as_deref(), Some("LGTM"));
        assert_eq!(output.completed_at, 42.0);
        assert!(exec.workflow_variables.is_empty());
    }

    #[test]
    fn apply_validated_submission_rejects_contract_mismatch_without_mutation() {
        let mut exec = running_execution();
        let err = apply_validated_submission(
            &mut exec,
            "run-1",
            "review",
            "other-contract",
            &serde_json::json!({"verdict": "LGTM"}),
            Some("LGTM".to_string()),
            42.0,
        )
        .unwrap_err();

        assert!(matches!(err, WorkflowEngineError::ValidationError(_)));
        assert!(exec.step_outputs.is_empty());
    }

    #[test]
    fn apply_validated_submission_rejects_non_accepting_parallel_child_without_mutation() {
        let mut exec = parallel_execution();
        let err = apply_validated_submission(
            &mut exec,
            "run-1",
            "review-b",
            "review-verdict",
            &serde_json::json!({"verdict": "LGTM"}),
            Some("LGTM".to_string()),
            42.0,
        )
        .unwrap_err();

        assert!(matches!(err, WorkflowEngineError::InvalidState(_)));
        assert!(exec.step_outputs.is_empty());
    }

    #[test]
    fn rollback_validated_submission_restores_previous_output_without_touching_variables() {
        let mut exec = running_execution();
        exec.step_outputs.insert(
            "review".to_string(),
            step_output(1, Some("review-verdict"), true),
        );
        exec.workflow_variables
            .insert("spec_dir".to_string(), "docs/specs/old".to_string());
        let mutation = apply_validated_submission(
            &mut exec,
            "run-1",
            "review",
            "review-verdict",
            &serde_json::json!({"verdict": "NEEDS_WORK"}),
            Some("NEEDS_WORK".to_string()),
            42.0,
        )
        .unwrap();

        rollback_validated_submission(&mut exec, "review", mutation);

        let output = exec.step_outputs.get("review").unwrap();
        assert_eq!(output.run_index, 1);
        assert_eq!(output.result.as_deref(), Some("LGTM"));
        assert_eq!(
            exec.workflow_variables.get("spec_dir").map(String::as_str),
            Some("docs/specs/old")
        );
    }
}
