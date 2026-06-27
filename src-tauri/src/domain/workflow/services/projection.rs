//! Workflow state projection rules.
//!
//! This module keeps presentation-independent workflow state derivation in the
//! domain layer. Infrastructure can map runtime storage types into these value
//! objects, but the rules for derived fields live here.

use crate::domain::workflow::services::approval_rules;
use crate::domain::workflow::value_objects::{
    ApprovalOperations, NodeDefinition, ParallelStepState, StepHistoryEntry, TokenUsage,
    WorkflowExecutionState, STEP_STATE_COMPLETED,
};
use crate::domain::workflow::ParallelRunState;

pub fn total_token_usage(step_history: &[StepHistoryEntry]) -> TokenUsage {
    let mut usage = TokenUsage::default();
    for entry in step_history {
        if let Some(entry_usage) = &entry.token_usage {
            usage.add(entry_usage);
        }
    }
    usage
}

pub fn approval_operations(
    state: &WorkflowExecutionState,
    current_step: Option<&NodeDefinition>,
) -> Option<ApprovalOperations> {
    if !matches!(state, WorkflowExecutionState::WaitingApproval) {
        return None;
    }
    let step = current_step?;
    Some(ApprovalOperations {
        can_reject: approval_rules::can_reject(&step.transition_rules),
    })
}

pub fn active_parallel_steps(parallel_run: Option<&ParallelRunState>) -> Vec<ParallelStepState> {
    let Some(parallel_run) = parallel_run else {
        return Vec::new();
    };
    parallel_run
        .children
        .iter()
        .map(|child| ParallelStepState {
            step_name: child.step_name.clone(),
            state: child.state.as_str().to_string(),
            session_id: if child.session_id.is_empty() {
                None
            } else {
                Some(child.session_id.clone())
            },
            result: child.result.clone(),
            run_index: child.run_index,
            completed_at: None,
            structured_output: child.structured_output.clone(),
            output_contract: child.output_contract.clone(),
            failure_kind: child.failure_kind,
            failure_disposition: child.failure_disposition,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{
        value_objects::{FailureDisposition, NodeType, TransitionRule, WorkflowStepFailureKind},
        ParallelChildRun, ParallelChildState, ParallelRunState,
    };

    #[test]
    fn total_token_usage_sums_history_entries_and_skips_missing_usage() {
        let usage = total_token_usage(&[
            StepHistoryEntry {
                step_name: "plan".to_string(),
                completed_at: 1.0,
                result: None,
                session_id: None,
                token_usage: Some(TokenUsage {
                    input_tokens: 3,
                    output_tokens: 5,
                }),
                structured_output: None,
                run_index: 1,
                child_outputs: None,
                state: STEP_STATE_COMPLETED.to_string(),
            },
            StepHistoryEntry {
                step_name: "review".to_string(),
                completed_at: 2.0,
                result: None,
                session_id: None,
                token_usage: None,
                structured_output: None,
                run_index: 1,
                child_outputs: None,
                state: STEP_STATE_COMPLETED.to_string(),
            },
            StepHistoryEntry {
                step_name: "fix".to_string(),
                completed_at: 3.0,
                result: None,
                session_id: None,
                token_usage: Some(TokenUsage {
                    input_tokens: 7,
                    output_tokens: 11,
                }),
                structured_output: None,
                run_index: 1,
                child_outputs: None,
                state: STEP_STATE_COMPLETED.to_string(),
            },
        ]);
        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.output_tokens, 16);
    }

    #[test]
    fn approval_operations_only_exposes_reject_when_waiting_approval() {
        let step = NodeDefinition {
            name: "approve".to_string(),
            node_type: NodeType::Approval,
            transition_rules: vec![TransitionRule {
                r#match: "reject".to_string(),
                next: "fix".to_string(),
            }],
            ..Default::default()
        };

        assert_eq!(
            approval_operations(&WorkflowExecutionState::WaitingApproval, Some(&step)),
            Some(ApprovalOperations { can_reject: true })
        );
        assert_eq!(
            approval_operations(&WorkflowExecutionState::Running, Some(&step)),
            None
        );
    }

    #[test]
    fn active_parallel_steps_projects_children_and_omits_empty_session_id() {
        let steps = active_parallel_steps(Some(&ParallelRunState {
            parent_step_name: "parallel-review".to_string(),
            aggregate: None,
            children: vec![
                ParallelChildRun {
                    step_name: "review-structure".to_string(),
                    session_id: "session-a".to_string(),
                    state: ParallelChildState::Running,
                    result: None,
                    structured_output: None,
                    output_contract: None,
                    failure_kind: None,
                    failure_disposition: None,
                    token_usage: TokenUsage::default(),
                    run_index: 1,
                },
                ParallelChildRun {
                    step_name: "review-test".to_string(),
                    session_id: String::new(),
                    state: ParallelChildState::Completed,
                    result: Some("ok".to_string()),
                    structured_output: Some(serde_json::json!({ "status": "ok" })),
                    output_contract: Some("contract".to_string()),
                    failure_kind: None,
                    failure_disposition: None,
                    token_usage: TokenUsage::default(),
                    run_index: 2,
                },
            ],
        }));

        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].state, "running");
        assert_eq!(steps[0].session_id.as_deref(), Some("session-a"));
        assert_eq!(steps[1].state, "completed");
        assert_eq!(steps[1].session_id, None);
        assert_eq!(steps[1].result.as_deref(), Some("ok"));
        assert_eq!(steps[1].output_contract.as_deref(), Some("contract"));
    }

    #[test]
    fn active_parallel_steps_preserves_partial_failure_metadata() {
        let steps = active_parallel_steps(Some(&ParallelRunState {
            parent_step_name: "parallel-review".to_string(),
            aggregate: None,
            children: vec![ParallelChildRun {
                step_name: "review-policy".to_string(),
                session_id: "session-a".to_string(),
                state: ParallelChildState::Failed,
                result: Some("model_refusal".to_string()),
                structured_output: Some(serde_json::json!({
                    "failureKind": "model_refusal",
                    "disposition": "partial",
                })),
                output_contract: None,
                failure_kind: Some(WorkflowStepFailureKind::ModelRefusal),
                failure_disposition: Some(FailureDisposition::Partial),
                token_usage: TokenUsage::default(),
                run_index: 1,
            }],
        }));

        assert_eq!(steps.len(), 1);
        assert_eq!(
            steps[0].failure_kind,
            Some(WorkflowStepFailureKind::ModelRefusal)
        );
        assert_eq!(
            steps[0].failure_disposition,
            Some(FailureDisposition::Partial)
        );
    }
}
