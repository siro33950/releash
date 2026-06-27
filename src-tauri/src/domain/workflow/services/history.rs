//! Workflow history entry construction rules.

use std::collections::HashMap;

use crate::domain::workflow::value_objects::{
    ChildOutputSnapshot, StepHistoryEntry, StepOutput, TokenUsage, STEP_STATE_ABORTED,
    STEP_STATE_COMPLETED,
};
use crate::domain::workflow::ParallelRunState;

#[derive(Debug, Clone, PartialEq)]
pub struct CompletedStepHistoryInput {
    pub step_name: String,
    pub completed_at: f64,
    pub result: Option<String>,
    pub session_id: Option<String>,
    pub token_usage: Option<TokenUsage>,
    pub structured_output: Option<serde_json::Value>,
    pub run_index: u32,
}

pub fn completed_step_history_entry(input: CompletedStepHistoryInput) -> StepHistoryEntry {
    StepHistoryEntry {
        step_name: input.step_name,
        completed_at: input.completed_at,
        result: input.result,
        session_id: input.session_id,
        token_usage: input.token_usage,
        structured_output: input.structured_output,
        run_index: input.run_index,
        child_outputs: None,
        state: crate::domain::workflow::value_objects::default_step_entry_state(),
    }
}

pub fn step_output_from_completed_history_entry(
    entry: &StepHistoryEntry,
    output_contract: Option<String>,
) -> Option<StepOutput> {
    entry.structured_output.as_ref()?;
    Some(StepOutput {
        step_name: entry.step_name.clone(),
        run_index: entry.run_index,
        session_id: entry.session_id.clone(),
        result: entry.result.clone(),
        structured_output: entry.structured_output.clone(),
        output_contract,
        token_usage: entry.token_usage.clone(),
        completed_at: entry.completed_at,
    })
}

pub fn session_start_failed_result(reason: impl AsRef<str>) -> String {
    format!("session_start_failed: {}", reason.as_ref())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeStartFailureKind {
    StepSession,
    ParallelChildren,
}

pub fn runtime_start_failure_reason(
    kind: RuntimeStartFailureKind,
    error: impl AsRef<str>,
) -> String {
    match kind {
        RuntimeStartFailureKind::StepSession => {
            format!("Failed to start step session: {}", error.as_ref())
        }
        RuntimeStartFailureKind::ParallelChildren => {
            format!("Failed to start parallel children: {}", error.as_ref())
        }
    }
}

pub fn aborted_step_history_entry(
    step_name: String,
    run_index: u32,
    session_id: Option<String>,
    token_usage: TokenUsage,
    timestamp: f64,
) -> StepHistoryEntry {
    StepHistoryEntry {
        step_name,
        completed_at: timestamp,
        result: None,
        session_id,
        token_usage: Some(token_usage),
        structured_output: None,
        run_index,
        child_outputs: None,
        state: STEP_STATE_ABORTED.to_string(),
    }
}

pub fn aborted_parallel_history_entry(
    parallel_run: &ParallelRunState,
    step_outputs: &HashMap<String, StepOutput>,
    parent_run_index: u32,
    timestamp: f64,
) -> StepHistoryEntry {
    let child_outputs = parallel_run
        .children
        .iter()
        .map(|child| {
            let output = step_outputs.get(&child.step_name);
            ChildOutputSnapshot {
                step_name: child.step_name.clone(),
                session_id: output
                    .and_then(|value| value.session_id.clone())
                    .or_else(|| non_empty_session_id(&child.session_id)),
                result: output
                    .and_then(|value| value.result.clone())
                    .or_else(|| child.result.clone()),
                run_index: child.run_index,
                completed_at: output.map(|value| value.completed_at).unwrap_or(timestamp),
                structured_output: output.and_then(|value| value.structured_output.clone()),
                output_contract: output.and_then(|value| value.output_contract.clone()),
                state: if child.state.is_completed() {
                    STEP_STATE_COMPLETED.to_string()
                } else {
                    STEP_STATE_ABORTED.to_string()
                },
                failure_kind: child.failure_kind,
                failure_disposition: child.failure_disposition,
            }
        })
        .collect();
    StepHistoryEntry {
        step_name: parallel_run.parent_step_name.clone(),
        completed_at: timestamp,
        result: None,
        session_id: None,
        token_usage: None,
        structured_output: None,
        run_index: parent_run_index,
        child_outputs: Some(child_outputs),
        state: STEP_STATE_ABORTED.to_string(),
    }
}

fn non_empty_session_id(session_id: &str) -> Option<String> {
    if session_id.is_empty() {
        None
    } else {
        Some(session_id.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{ParallelChildRun, ParallelChildState};

    #[test]
    fn completed_step_history_entry_and_output_share_completion_fields() {
        let entry = completed_step_history_entry(CompletedStepHistoryInput {
            step_name: "review".to_string(),
            completed_at: 10.0,
            result: Some("LGTM".to_string()),
            session_id: Some("session-review".to_string()),
            token_usage: Some(TokenUsage {
                input_tokens: 2,
                output_tokens: 3,
            }),
            structured_output: Some(serde_json::json!({"ok": true})),
            run_index: 4,
        });

        assert_eq!(entry.state, "completed");
        let output =
            step_output_from_completed_history_entry(&entry, Some("review-contract".to_string()))
                .unwrap();
        assert_eq!(output.step_name, "review");
        assert_eq!(output.session_id.as_deref(), Some("session-review"));
        assert_eq!(output.result.as_deref(), Some("LGTM"));
        assert_eq!(output.output_contract.as_deref(), Some("review-contract"));
        assert_eq!(output.token_usage.unwrap().input_tokens, 2);
    }

    #[test]
    fn completed_step_without_structured_output_has_no_step_output() {
        let entry = completed_step_history_entry(CompletedStepHistoryInput {
            step_name: "review".to_string(),
            completed_at: 10.0,
            result: None,
            session_id: None,
            token_usage: None,
            structured_output: None,
            run_index: 1,
        });

        assert!(step_output_from_completed_history_entry(&entry, None).is_none());
    }

    #[test]
    fn session_start_failed_result_preserves_persisted_prefix() {
        assert_eq!(
            session_start_failed_result("backend unavailable"),
            "session_start_failed: backend unavailable"
        );
    }

    #[test]
    fn runtime_start_failure_reason_preserves_failed_state_messages() {
        assert_eq!(
            runtime_start_failure_reason(RuntimeStartFailureKind::StepSession, "backend down"),
            "Failed to start step session: backend down"
        );
        assert_eq!(
            runtime_start_failure_reason(RuntimeStartFailureKind::ParallelChildren, "backend down"),
            "Failed to start parallel children: backend down"
        );
    }

    #[test]
    fn aborted_step_history_entry_keeps_session_and_token_usage() {
        let entry = aborted_step_history_entry(
            "review".to_string(),
            3,
            Some("session-review".to_string()),
            TokenUsage {
                input_tokens: 5,
                output_tokens: 8,
            },
            12.0,
        );

        assert_eq!(entry.step_name, "review");
        assert_eq!(entry.run_index, 3);
        assert_eq!(entry.session_id.as_deref(), Some("session-review"));
        assert_eq!(entry.token_usage.unwrap().input_tokens, 5);
        assert_eq!(entry.state, "aborted");
    }

    #[test]
    fn aborted_parallel_history_entry_snapshots_children_with_output_fallback() {
        let mut step_outputs = HashMap::new();
        step_outputs.insert(
            "child-a".to_string(),
            StepOutput {
                step_name: "child-a".to_string(),
                run_index: 1,
                session_id: Some("output-session-a".to_string()),
                result: Some("from output".to_string()),
                structured_output: Some(serde_json::json!({ "ok": true })),
                output_contract: Some("review".to_string()),
                token_usage: None,
                completed_at: 10.0,
            },
        );

        let entry = aborted_parallel_history_entry(
            &ParallelRunState {
                parent_step_name: "parallel-review".to_string(),
                aggregate: None,
                children: vec![
                    ParallelChildRun {
                        step_name: "child-a".to_string(),
                        session_id: "child-session-a".to_string(),
                        state: ParallelChildState::Completed,
                        result: Some("from child".to_string()),
                        structured_output: None,
                        output_contract: None,
                        failure_kind: None,
                        failure_disposition: None,
                        token_usage: TokenUsage::default(),
                        run_index: 1,
                    },
                    ParallelChildRun {
                        step_name: "child-b".to_string(),
                        session_id: "child-session-b".to_string(),
                        state: ParallelChildState::Running,
                        result: Some("partial".to_string()),
                        structured_output: None,
                        output_contract: None,
                        failure_kind: None,
                        failure_disposition: None,
                        token_usage: TokenUsage::default(),
                        run_index: 2,
                    },
                ],
            },
            &step_outputs,
            4,
            20.0,
        );

        assert_eq!(entry.step_name, "parallel-review");
        assert_eq!(entry.run_index, 4);
        let children = entry.child_outputs.unwrap();
        assert_eq!(children[0].session_id.as_deref(), Some("output-session-a"));
        assert_eq!(children[0].result.as_deref(), Some("from output"));
        assert_eq!(children[0].completed_at, 10.0);
        assert_eq!(children[0].state, "completed");
        assert_eq!(children[1].session_id.as_deref(), Some("child-session-b"));
        assert_eq!(children[1].result.as_deref(), Some("partial"));
        assert_eq!(children[1].completed_at, 20.0);
        assert_eq!(children[1].state, "aborted");
    }
}
