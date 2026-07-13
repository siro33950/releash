//! Pure workflow transition decisions.

use crate::domain::workflow::value_objects::{
    NodeKindName, SessionGate, WorkflowDefinition, WorkflowExecutionState, WorkflowStepFailureKind,
};
use crate::domain::workflow::WorkflowError;

#[derive(Debug, Clone, PartialEq)]
pub enum TurnCompleteDecision {
    SessionError {
        node_name: String,
        exit_code: i64,
        kind: WorkflowStepFailureKind,
    },
    AutoEvaluate {
        node_name: String,
    },
    WaitApproval,
    UnexpectedNodeKind {
        node_name: String,
        kind: NodeKindName,
    },
    NotRunning,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TurnCompleteMutationPlan {
    SessionError {
        node_name: String,
        exit_code: i64,
        kind: WorkflowStepFailureKind,
        history_result: String,
        failure_reason: String,
    },
    AutoEvaluate {
        node_name: String,
    },
    RequestApproval {
        node_name: String,
    },
    UnexpectedNodeKind {
        node_name: String,
        kind: NodeKindName,
        failure_reason: String,
    },
    NotRunning,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalApplication {
    pub effective_result: String,
    pub structured_output: Option<serde_json::Value>,
    pub artifact_contract: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalCompletion {
    pub result: String,
    pub structured_output: Option<serde_json::Value>,
    pub artifact_contract: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalApplicationPlan {
    pub completion: ApprovalCompletion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionFailureSignal {
    #[cfg(test)]
    Timeout,
    #[cfg(test)]
    Crash,
    ModelRefusal,
}

pub fn classify_session_error(
    exit_code: i64,
    signal: Option<SessionFailureSignal>,
) -> WorkflowStepFailureKind {
    match signal {
        #[cfg(test)]
        Some(SessionFailureSignal::Timeout) => WorkflowStepFailureKind::StaleRuntimeTimeout,
        #[cfg(test)]
        Some(SessionFailureSignal::Crash) => WorkflowStepFailureKind::InfrastructureCrash,
        Some(SessionFailureSignal::ModelRefusal) => WorkflowStepFailureKind::ModelRefusal,
        None if exit_code == 124 => WorkflowStepFailureKind::StaleRuntimeTimeout,
        None => WorkflowStepFailureKind::InfrastructureCrash,
    }
}

#[cfg(test)]
pub fn decide_turn_complete_action(
    workflow: &WorkflowDefinition,
    current_index: usize,
    state: &WorkflowExecutionState,
    exit_code: i64,
) -> Result<TurnCompleteDecision, WorkflowError> {
    decide_turn_complete_action_with_signal(workflow, current_index, state, exit_code, None)
}

pub fn decide_turn_complete_action_with_signal(
    workflow: &WorkflowDefinition,
    current_index: usize,
    state: &WorkflowExecutionState,
    exit_code: i64,
    signal: Option<SessionFailureSignal>,
) -> Result<TurnCompleteDecision, WorkflowError> {
    if !matches!(state, WorkflowExecutionState::Running) {
        return Ok(TurnCompleteDecision::NotRunning);
    }

    let node = workflow.nodes.get(current_index).ok_or_else(|| {
        WorkflowError::validation(format!("node index out of range: {current_index}"))
    })?;

    if exit_code != 0 || signal.is_some() {
        let kind = classify_session_error(exit_code, signal);
        return Ok(TurnCompleteDecision::SessionError {
            node_name: node.name.clone(),
            exit_code,
            kind,
        });
    }

    if let Some(session) = node.session() {
        return match session.gate {
            SessionGate::Auto => Ok(TurnCompleteDecision::AutoEvaluate {
                node_name: node.name.clone(),
            }),
            SessionGate::Approval => Ok(TurnCompleteDecision::WaitApproval),
        };
    }

    Ok(TurnCompleteDecision::UnexpectedNodeKind {
        node_name: node.name.clone(),
        kind: node.kind_name(),
    })
}

#[cfg(test)]
pub fn plan_turn_complete_mutation(
    workflow: &WorkflowDefinition,
    current_index: usize,
    state: &WorkflowExecutionState,
    exit_code: i64,
) -> Result<TurnCompleteMutationPlan, WorkflowError> {
    plan_turn_complete_mutation_with_signal(workflow, current_index, state, exit_code, None)
}

pub fn plan_turn_complete_mutation_with_signal(
    workflow: &WorkflowDefinition,
    current_index: usize,
    state: &WorkflowExecutionState,
    exit_code: i64,
    signal: Option<SessionFailureSignal>,
) -> Result<TurnCompleteMutationPlan, WorkflowError> {
    let decision =
        decide_turn_complete_action_with_signal(workflow, current_index, state, exit_code, signal)?;
    let plan = match decision {
        TurnCompleteDecision::SessionError {
            node_name,
            exit_code,
            kind,
        } => TurnCompleteMutationPlan::SessionError {
            history_result: format!("error (exit_code: {exit_code})"),
            failure_reason: format!(
                "AgentSession error at step '{node_name}' (exit_code: {exit_code})"
            ),
            node_name,
            exit_code,
            kind,
        },
        TurnCompleteDecision::AutoEvaluate { node_name } => {
            TurnCompleteMutationPlan::AutoEvaluate { node_name }
        }
        TurnCompleteDecision::WaitApproval => {
            let node_name = workflow
                .nodes
                .get(current_index)
                .ok_or_else(|| {
                    WorkflowError::validation(format!(
                        "node index out of range: {current_index}"
                    ))
                })?
                .name
                .clone();
            TurnCompleteMutationPlan::RequestApproval { node_name }
        }
        TurnCompleteDecision::UnexpectedNodeKind {
            node_name,
            kind,
        } => TurnCompleteMutationPlan::UnexpectedNodeKind {
            failure_reason: format!(
                "Workflow engine reached turn_complete for unexpected node kind {kind:?} at step '{node_name}' (this should have been rejected upstream)"
            ),
            node_name,
            kind,
        },
        TurnCompleteDecision::NotRunning => TurnCompleteMutationPlan::NotRunning,
    };
    Ok(plan)
}

pub fn decide_approve_action(
    workflow: &WorkflowDefinition,
    current_index: usize,
    state: &WorkflowExecutionState,
) -> Result<(), WorkflowError> {
    if !matches!(state, WorkflowExecutionState::WaitingApproval) {
        return Err(WorkflowError::invalid_state(
            "Workflow is not waiting for approval",
        ));
    }

    let node = workflow.nodes.get(current_index).ok_or_else(|| {
        WorkflowError::validation(format!("node index out of range: {current_index}"))
    })?;
    if !node.is_approval_session() {
        return Err(WorkflowError::UnauthorizedApprovalTarget(
            "current step is not an approval-gated session".to_string(),
        ));
    }
    Ok(())
}

pub fn plan_approval_application(
    workflow: &WorkflowDefinition,
    current_index: usize,
    state: &WorkflowExecutionState,
    application: ApprovalApplication,
) -> Result<ApprovalApplicationPlan, WorkflowError> {
    decide_approve_action(workflow, current_index, state)?;
    Ok(ApprovalApplicationPlan {
        completion: ApprovalCompletion {
            result: application.effective_result,
            structured_output: application.structured_output,
            artifact_contract: application.artifact_contract,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::value_objects::{
        CommandSpec, FacetRefs, NodeDefinition, NodeKind, NodeKindName, SessionGate, SessionSpec,
    };

    enum TestKind {
        Session,
        ApprovalSession,
        Command,
    }

    fn node(name: &str, kind: TestKind) -> NodeDefinition {
        let kind = match kind {
            TestKind::Session => NodeKind::Session(SessionSpec {
                facets: FacetRefs {
                    instruction: Some("implement".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            }),
            TestKind::ApprovalSession => NodeKind::Session(SessionSpec {
                gate: SessionGate::Approval,
                facets: FacetRefs {
                    instruction: Some("implement".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            }),
            TestKind::Command => NodeKind::Command(CommandSpec {
                command: "cargo test".to_string(),
            }),
        };
        NodeDefinition {
            name: name.to_string(),
            kind,
            ..Default::default()
        }
    }

    fn workflow(nodes: Vec<NodeDefinition>) -> WorkflowDefinition {
        WorkflowDefinition {
            name: "wf".to_string(),
            description: String::new(),
            builtin: false,
            schemas: Default::default(),
            nodes,
        }
    }

    #[test]
    fn decide_turn_complete_action_distinguishes_session_gates_and_unexpected_node() {
        let workflow = workflow(vec![
            node("agent", TestKind::Session),
            node("approval", TestKind::ApprovalSession),
            node("script", TestKind::Command),
        ]);

        assert!(matches!(
            decide_turn_complete_action(&workflow, 0, &WorkflowExecutionState::Running, 0).unwrap(),
            TurnCompleteDecision::AutoEvaluate { .. }
        ));
        assert_eq!(
            decide_turn_complete_action(&workflow, 1, &WorkflowExecutionState::Running, 0).unwrap(),
            TurnCompleteDecision::WaitApproval
        );
        assert!(matches!(
            decide_turn_complete_action(&workflow, 2, &WorkflowExecutionState::Running, 0).unwrap(),
            TurnCompleteDecision::UnexpectedNodeKind {
                kind: NodeKindName::Command,
                ..
            }
        ));
    }

    #[test]
    fn plan_turn_complete_mutation_builds_domain_failure_details() {
        let workflow = workflow(vec![node("agent", TestKind::Session)]);

        let plan = plan_turn_complete_mutation(&workflow, 0, &WorkflowExecutionState::Running, 42)
            .unwrap();

        assert_eq!(
            plan,
            TurnCompleteMutationPlan::SessionError {
                node_name: "agent".to_string(),
                exit_code: 42,
                kind: WorkflowStepFailureKind::InfrastructureCrash,
                history_result: "error (exit_code: 42)".to_string(),
                failure_reason: "AgentSession error at step 'agent' (exit_code: 42)".to_string()
            }
        );
    }

    #[test]
    fn plan_turn_complete_mutation_uses_explicit_model_refusal_signal() {
        let workflow = workflow(vec![node("agent", TestKind::Session)]);

        let plan = plan_turn_complete_mutation_with_signal(
            &workflow,
            0,
            &WorkflowExecutionState::Running,
            0,
            Some(SessionFailureSignal::ModelRefusal),
        )
        .unwrap();

        match plan {
            TurnCompleteMutationPlan::SessionError { kind, .. } => {
                assert_eq!(kind, WorkflowStepFailureKind::ModelRefusal);
            }
            other => panic!("unexpected plan: {other:?}"),
        }
    }

    #[test]
    fn classify_session_error_maps_runtime_failure_sources() {
        assert_eq!(
            classify_session_error(124, Some(SessionFailureSignal::Timeout)),
            WorkflowStepFailureKind::StaleRuntimeTimeout
        );
        assert_eq!(
            classify_session_error(-1, Some(SessionFailureSignal::Crash)),
            WorkflowStepFailureKind::InfrastructureCrash
        );
        assert_eq!(
            classify_session_error(1, Some(SessionFailureSignal::ModelRefusal)),
            WorkflowStepFailureKind::ModelRefusal
        );
        assert_eq!(
            classify_session_error(42, None),
            WorkflowStepFailureKind::InfrastructureCrash
        );
    }

    #[test]
    fn plan_approval_application_keeps_completion_data_on_approve() {
        let approval = node("approve", TestKind::ApprovalSession);
        let workflow = workflow(vec![approval, node("fix", TestKind::Session)]);

        let plan = plan_approval_application(
            &workflow,
            0,
            &WorkflowExecutionState::WaitingApproval,
            ApprovalApplication {
                effective_result: "approve".to_string(),
                structured_output: Some(serde_json::json!({ "decision": "approve" })),
                artifact_contract: Some("approval-contract".to_string()),
            },
        )
        .unwrap();

        assert_eq!(plan.completion.result, "approve");
        assert_eq!(
            plan.completion.artifact_contract.as_deref(),
            Some("approval-contract")
        );
    }

    #[test]
    fn approve_rejects_waiting_state_on_non_approval_gate_session() {
        let workflow = workflow(vec![node("implement", TestKind::Session)]);

        assert!(matches!(
            decide_approve_action(&workflow, 0, &WorkflowExecutionState::WaitingApproval),
            Err(WorkflowError::UnauthorizedApprovalTarget(_))
        ));
    }
}
