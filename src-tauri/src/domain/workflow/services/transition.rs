//! Pure workflow transition decisions.

use std::collections::HashMap;

use regex::RegexBuilder;

use crate::domain::workflow::value_objects::{
    ApprovalDecision, NodeKindName, SessionGate, TransitionRule, WorkflowDefinition,
    WorkflowExecutionState, WorkflowStepFailureKind,
};
use crate::domain::workflow::WorkflowError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NextNodeDecision {
    Completed,
    TransitionTo(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CycleGuardDecision {
    Allowed,
    Exceeded {
        max_iterations: u32,
        count: u32,
        on_exhausted: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum TurnCompleteDecision {
    SessionError {
        node_name: String,
        exit_code: i64,
        kind: WorkflowStepFailureKind,
    },
    AutoEvaluate {
        rules: Vec<TransitionRule>,
        node_name: String,
    },
    WaitApproval,
    UnexpectedNodeKind {
        node_name: String,
        kind: NodeKindName,
    },
    NotRunning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalTransitionDecision {
    Advance,
    TransitionTo(String),
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
        rules: Vec<TransitionRule>,
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
    pub output_contract: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalCompletion {
    pub result: String,
    pub structured_output: Option<serde_json::Value>,
    pub output_contract: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalApplicationTransition {
    Advance,
    TransitionTo(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalApplicationPlan {
    pub completion: ApprovalCompletion,
    pub transition: ApprovalApplicationTransition,
}

pub fn decide_next_node(workflow: &WorkflowDefinition, current_index: usize) -> NextNodeDecision {
    if current_index + 1 >= workflow.nodes.len() {
        NextNodeDecision::Completed
    } else {
        NextNodeDecision::TransitionTo(workflow.nodes[current_index + 1].name.clone())
    }
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

pub fn check_cycle_guard(
    workflow: &WorkflowDefinition,
    step_execution_counts: &HashMap<String, u32>,
    target_node_name: &str,
) -> Result<CycleGuardDecision, WorkflowError> {
    let node = workflow
        .nodes
        .iter()
        .find(|node| node.name == target_node_name)
        .ok_or_else(|| WorkflowError::validation(format!("node not found: {target_node_name}")))?;

    let Some(guard) = &node.cycle_guard else {
        return Ok(CycleGuardDecision::Allowed);
    };

    let count = step_execution_counts
        .get(target_node_name)
        .copied()
        .unwrap_or(0);
    if count >= guard.max_iterations {
        Ok(CycleGuardDecision::Exceeded {
            max_iterations: guard.max_iterations,
            count,
            on_exhausted: guard.on_exhausted.clone(),
        })
    } else {
        Ok(CycleGuardDecision::Allowed)
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
                rules: node.transition_rules.clone(),
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
        TurnCompleteDecision::AutoEvaluate { rules, node_name } => {
            TurnCompleteMutationPlan::AutoEvaluate { rules, node_name }
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

pub fn decide_approval_action(
    workflow: &WorkflowDefinition,
    current_index: usize,
    state: &WorkflowExecutionState,
    decision: &ApprovalDecision,
) -> Result<ApprovalTransitionDecision, WorkflowError> {
    if !matches!(state, WorkflowExecutionState::WaitingApproval) {
        return Err(WorkflowError::invalid_state(
            "Workflow is not waiting for approval",
        ));
    }

    let node = workflow.nodes.get(current_index).ok_or_else(|| {
        WorkflowError::validation(format!("node index out of range: {current_index}"))
    })?;

    match decision {
        ApprovalDecision::Approve { .. } => Ok(ApprovalTransitionDecision::Advance),
        ApprovalDecision::Reject { .. } => match node
            .transition_rules
            .iter()
            .find(|rule| rule.r#match == "reject")
        {
            Some(rule) => Ok(ApprovalTransitionDecision::TransitionTo(rule.next.clone())),
            None => Err(WorkflowError::invalid_state(format!(
                "Step '{}' does not allow reject",
                node.name
            ))),
        },
        ApprovalDecision::Abort => Err(WorkflowError::invalid_state(
            "Abort is not an approval transition",
        )),
    }
}

pub fn plan_approval_application(
    workflow: &WorkflowDefinition,
    current_index: usize,
    state: &WorkflowExecutionState,
    decision: &ApprovalDecision,
    application: ApprovalApplication,
) -> Result<ApprovalApplicationPlan, WorkflowError> {
    let transition = match decide_approval_action(workflow, current_index, state, decision)? {
        ApprovalTransitionDecision::Advance => ApprovalApplicationTransition::Advance,
        ApprovalTransitionDecision::TransitionTo(target) => {
            ApprovalApplicationTransition::TransitionTo(target)
        }
    };
    Ok(ApprovalApplicationPlan {
        completion: ApprovalCompletion {
            result: application.effective_result,
            structured_output: application.structured_output,
            output_contract: application.output_contract,
        },
        transition,
    })
}

pub fn evaluate_auto_rules(text: &str, rules: &[TransitionRule]) -> Option<(String, String)> {
    for rule in rules {
        let Ok(regex) = RegexBuilder::new(&rule.r#match).size_limit(1 << 20).build() else {
            continue;
        };
        if regex.is_match(text) {
            return Some((rule.next.clone(), rule.r#match.clone()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::value_objects::{
        CommandSpec, CycleGuard, FacetRefs, NodeDefinition, NodeKind, NodeKindName, SessionGate,
        SessionSpec,
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
            variables: Default::default(),
            nodes,
        }
    }

    #[test]
    fn decide_next_node_returns_following_node_or_completed() {
        let workflow = workflow(vec![
            node("plan", TestKind::Session),
            node("done", TestKind::Session),
        ]);

        assert_eq!(
            decide_next_node(&workflow, 0),
            NextNodeDecision::TransitionTo("done".to_string())
        );
        assert_eq!(decide_next_node(&workflow, 1), NextNodeDecision::Completed);
    }

    #[test]
    fn check_cycle_guard_reports_boundary_and_fallback() {
        let mut guarded = node("review", TestKind::Session);
        guarded.cycle_guard = Some(CycleGuard {
            max_iterations: 2,
            on_exhausted: Some("fallback".to_string()),
        });
        let workflow = workflow(vec![guarded]);
        let counts = HashMap::from([("review".to_string(), 2)]);

        assert_eq!(
            check_cycle_guard(&workflow, &counts, "review").unwrap(),
            CycleGuardDecision::Exceeded {
                max_iterations: 2,
                count: 2,
                on_exhausted: Some("fallback".to_string())
            }
        );
    }

    #[test]
    fn decide_turn_complete_action_distinguishes_agent_approval_and_unexpected_node() {
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
    fn decide_approval_action_uses_reject_rule() {
        let mut approval = node("approve", TestKind::ApprovalSession);
        approval.transition_rules = vec![TransitionRule {
            r#match: "reject".to_string(),
            next: "fix".to_string(),
        }];
        let workflow = workflow(vec![approval]);

        assert_eq!(
            decide_approval_action(
                &workflow,
                0,
                &WorkflowExecutionState::WaitingApproval,
                &ApprovalDecision::Reject {
                    reason: "needs fix".to_string()
                },
            )
            .unwrap(),
            ApprovalTransitionDecision::TransitionTo("fix".to_string())
        );
    }

    #[test]
    fn plan_approval_application_keeps_completion_data_and_reject_target() {
        let mut approval = node("approve", TestKind::ApprovalSession);
        approval.transition_rules = vec![TransitionRule {
            r#match: "reject".to_string(),
            next: "fix".to_string(),
        }];
        let workflow = workflow(vec![approval, node("fix", TestKind::Session)]);

        let plan = plan_approval_application(
            &workflow,
            0,
            &WorkflowExecutionState::WaitingApproval,
            &ApprovalDecision::Reject {
                reason: "needs work".to_string(),
            },
            ApprovalApplication {
                effective_result: "reject".to_string(),
                structured_output: Some(serde_json::json!({ "decision": "reject" })),
                output_contract: Some("approval-contract".to_string()),
            },
        )
        .unwrap();

        assert_eq!(
            plan.transition,
            ApprovalApplicationTransition::TransitionTo("fix".to_string())
        );
        assert_eq!(plan.completion.result, "reject");
        assert_eq!(
            plan.completion.output_contract.as_deref(),
            Some("approval-contract")
        );
    }

    #[test]
    fn evaluate_auto_rules_returns_first_matching_regex_rule() {
        let rules = vec![
            TransitionRule {
                r#match: "FIX".to_string(),
                next: "implement".to_string(),
            },
            TransitionRule {
                r#match: "NEEDS_FIX".to_string(),
                next: "review".to_string(),
            },
        ];

        assert_eq!(
            evaluate_auto_rules("<decision>NEEDS_FIX</decision>", &rules),
            Some(("implement".to_string(), "FIX".to_string()))
        );
    }

    #[test]
    fn evaluate_auto_rules_skips_invalid_regex_rules() {
        let rules = vec![
            TransitionRule {
                r#match: "[invalid".to_string(),
                next: "broken".to_string(),
            },
            TransitionRule {
                r#match: "LGTM".to_string(),
                next: "report".to_string(),
            },
        ];

        assert_eq!(
            evaluate_auto_rules("<decision>LGTM</decision>", &rules),
            Some(("report".to_string(), "LGTM".to_string()))
        );
    }
}
