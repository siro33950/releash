//! Pure workflow transition decisions.

use std::collections::HashMap;

use regex::RegexBuilder;

use crate::domain::workflow::value_objects::{
    ApprovalDecision, NodeType, TransitionRule, WorkflowDefinition, WorkflowExecutionState,
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
    },
    AutoEvaluate {
        rules: Vec<TransitionRule>,
        node_name: String,
    },
    WaitApproval,
    UnexpectedNodeType {
        node_name: String,
        node_type: NodeType,
    },
    NotRunning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalTransitionDecision {
    Advance,
    TransitionTo(String),
}

pub fn decide_next_node(workflow: &WorkflowDefinition, current_index: usize) -> NextNodeDecision {
    if current_index + 1 >= workflow.nodes.len() {
        NextNodeDecision::Completed
    } else {
        NextNodeDecision::TransitionTo(workflow.nodes[current_index + 1].name.clone())
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

pub fn decide_turn_complete_action(
    workflow: &WorkflowDefinition,
    current_index: usize,
    state: &WorkflowExecutionState,
    exit_code: i64,
) -> Result<TurnCompleteDecision, WorkflowError> {
    if !matches!(state, WorkflowExecutionState::Running) {
        return Ok(TurnCompleteDecision::NotRunning);
    }

    let node = workflow.nodes.get(current_index).ok_or_else(|| {
        WorkflowError::validation(format!("node index out of range: {current_index}"))
    })?;

    if exit_code != 0 {
        return Ok(TurnCompleteDecision::SessionError {
            node_name: node.name.clone(),
            exit_code,
        });
    }

    match node.node_type {
        NodeType::Agent => Ok(TurnCompleteDecision::AutoEvaluate {
            rules: node.transition_rules.clone(),
            node_name: node.name.clone(),
        }),
        NodeType::Approval => Ok(TurnCompleteDecision::WaitApproval),
        NodeType::Bash | NodeType::Parallel => Ok(TurnCompleteDecision::UnexpectedNodeType {
            node_name: node.name.clone(),
            node_type: node.node_type,
        }),
    }
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
    use crate::domain::workflow::value_objects::{CycleGuard, NodeDefinition};

    fn node(name: &str, node_type: NodeType) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            node_type,
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
            node("plan", NodeType::Agent),
            node("done", NodeType::Agent),
        ]);

        assert_eq!(
            decide_next_node(&workflow, 0),
            NextNodeDecision::TransitionTo("done".to_string())
        );
        assert_eq!(decide_next_node(&workflow, 1), NextNodeDecision::Completed);
    }

    #[test]
    fn check_cycle_guard_reports_boundary_and_fallback() {
        let mut guarded = node("review", NodeType::Agent);
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
            node("agent", NodeType::Agent),
            node("approval", NodeType::Approval),
            node("script", NodeType::Bash),
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
            TurnCompleteDecision::UnexpectedNodeType { .. }
        ));
    }

    #[test]
    fn decide_approval_action_uses_reject_rule() {
        let mut approval = node("approve", NodeType::Approval);
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
