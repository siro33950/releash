//! Workflow state projection rules.
//!
//! This module keeps presentation-independent workflow state derivation in the
//! domain layer. Infrastructure can map runtime storage types into these value
//! objects, but the rules for derived fields live here.

use crate::domain::workflow::value_objects::{
    ApprovalOperations, NodeDefinition, StepHistoryEntry, TokenUsage, WorkflowExecutionState,
};
#[cfg(test)]
use crate::domain::workflow::STEP_STATE_COMPLETED;

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
    let current_step = current_step?;
    if !current_step.is_approval_session() {
        return None;
    }
    Some(ApprovalOperations { can_approve: true })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::value_objects::{
        FacetRefs, NodeDefinition, NodeKind, SessionGate, SessionSpec,
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
    fn approval_operations_only_exists_when_waiting_for_approval_session() {
        let approval_step = NodeDefinition {
            name: "approve".to_string(),
            kind: NodeKind::Session(SessionSpec {
                gate: SessionGate::Approval,
                facets: FacetRefs {
                    instruction: Some("implement".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        let auto_step = NodeDefinition {
            name: "auto".to_string(),
            kind: NodeKind::Session(SessionSpec {
                gate: SessionGate::Auto,
                facets: FacetRefs {
                    instruction: Some("implement".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };

        assert_eq!(
            approval_operations(
                &WorkflowExecutionState::WaitingApproval,
                Some(&approval_step)
            ),
            Some(ApprovalOperations { can_approve: true })
        );
        assert_eq!(
            approval_operations(&WorkflowExecutionState::WaitingApproval, Some(&auto_step)),
            None
        );
        assert_eq!(
            approval_operations(&WorkflowExecutionState::Running, Some(&approval_step)),
            None
        );
    }
}
