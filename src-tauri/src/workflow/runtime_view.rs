use std::collections::HashSet;

use crate::workflow::state::WorkflowState;

pub fn collect_step_session_ids(state: &WorkflowState) -> HashSet<String> {
    let mut ids = HashSet::new();
    if let Some(session_id) = state.current_session_id.as_ref() {
        ids.insert(session_id.clone());
    }
    for entry in &state.step_history {
        if let Some(session_id) = entry.session_id.as_ref() {
            ids.insert(session_id.clone());
        }
        if let Some(children) = entry.child_outputs.as_ref() {
            ids.extend(children.iter().filter_map(|child| child.session_id.clone()));
        }
    }
    ids.extend(
        state
            .active_parallel_steps
            .iter()
            .filter_map(|step| step.session_id.clone()),
    );
    ids
}

#[cfg(test)]
pub fn contains_step_session(state: &WorkflowState, session_id: &str) -> bool {
    collect_step_session_ids(state).contains(session_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::schema::Workflow;
    use crate::workflow::state::{
        ChildOutputSnapshot, ParallelStepState, StepHistoryEntry, TokenUsage,
        WorkflowExecutionState,
    };
    use std::collections::HashMap;

    fn state() -> WorkflowState {
        WorkflowState {
            execution_id: "exec-1".to_string(),
            workflow_name: "wf".to_string(),
            state: WorkflowExecutionState::Running,
            current_step_index: 1,
            current_step_name: "current".to_string(),
            current_session_id: Some("current-session".to_string()),
            total_steps: 2,
            step_history: vec![StepHistoryEntry {
                step_name: "done".to_string(),
                completed_at: 1.0,
                result: Some("ok".to_string()),
                session_id: Some("done-session".to_string()),
                token_usage: None,
                structured_output: None,
                run_index: 1,
                child_outputs: Some(vec![ChildOutputSnapshot {
                    step_name: "child".to_string(),
                    session_id: Some("child-session".to_string()),
                    result: Some("ok".to_string()),
                    run_index: 1,
                    completed_at: 2.0,
                    structured_output: None,
                    output_contract: None,
                    state: crate::workflow::state::default_step_entry_state(),
                }]),
                state: crate::workflow::state::default_step_entry_state(),
            }],
            step_execution_counts: HashMap::new(),
            workflow_definition: Workflow {
                variables: Default::default(),
                name: "wf".to_string(),
                description: String::new(),
                builtin: false,
                nodes: vec![],
            },
            total_token_usage: TokenUsage::default(),
            step_states: HashMap::new(),
            step_outputs: HashMap::new(),
            active_parallel_steps: vec![ParallelStepState {
                step_name: "running-child".to_string(),
                state: "running".to_string(),
                session_id: Some("parallel-session".to_string()),
                result: None,
                run_index: 1,
                completed_at: None,
                structured_output: None,
                output_contract: None,
            }],
            workflow_variables: HashMap::new(),
            approval_operations: None,
            started_at: 0.0,
            updated_at: 2.0,
        }
    }

    #[test]
    fn contains_step_session_checks_current_history_child_and_parallel_sessions() {
        let state = state();

        assert!(contains_step_session(&state, "current-session"));
        assert!(contains_step_session(&state, "done-session"));
        assert!(contains_step_session(&state, "child-session"));
        assert!(contains_step_session(&state, "parallel-session"));
        assert!(!contains_step_session(&state, "regular-chat-session"));
    }

    /// spec issues-1023: 中断された run でも、step_history に積まれた aborted entry の
    /// session_id（および parallel child snapshot の session_id）が
    /// `collect_step_session_ids` で回収されることを担保する。これにより UI から
    /// 中断された step の session log にアクセスできる経路が維持される。
    #[test]
    fn collect_includes_session_ids_from_aborted_history_entries_and_child_snapshots() {
        let aborted_state = WorkflowState {
            execution_id: "exec-aborted".to_string(),
            workflow_name: "wf".to_string(),
            state: WorkflowExecutionState::Aborted,
            current_step_index: 0,
            current_step_name: "plan".to_string(),
            current_session_id: None,
            total_steps: 1,
            step_history: vec![
                StepHistoryEntry {
                    step_name: "plan".to_string(),
                    completed_at: 1.0,
                    result: None,
                    session_id: Some("aborted-step-session".to_string()),
                    token_usage: None,
                    structured_output: None,
                    run_index: 1,
                    child_outputs: None,
                    state: "aborted".to_string(),
                },
                StepHistoryEntry {
                    step_name: "parallel-review".to_string(),
                    completed_at: 2.0,
                    result: None,
                    session_id: None,
                    token_usage: None,
                    structured_output: None,
                    run_index: 1,
                    child_outputs: Some(vec![
                        ChildOutputSnapshot {
                            step_name: "child-a".to_string(),
                            session_id: Some("session-a".to_string()),
                            result: Some("LGTM".to_string()),
                            run_index: 1,
                            completed_at: 1.5,
                            structured_output: None,
                            output_contract: None,
                            state: "completed".to_string(),
                        },
                        ChildOutputSnapshot {
                            step_name: "child-b".to_string(),
                            session_id: Some("session-b".to_string()),
                            result: None,
                            run_index: 1,
                            completed_at: 2.0,
                            structured_output: None,
                            output_contract: None,
                            state: "aborted".to_string(),
                        },
                    ]),
                    state: "aborted".to_string(),
                },
            ],
            step_execution_counts: HashMap::new(),
            workflow_definition: Workflow {
                variables: Default::default(),
                name: "wf".to_string(),
                description: String::new(),
                builtin: false,
                nodes: vec![],
            },
            total_token_usage: TokenUsage::default(),
            step_states: HashMap::new(),
            step_outputs: HashMap::new(),
            active_parallel_steps: vec![],
            workflow_variables: HashMap::new(),
            approval_operations: None,
            started_at: 0.0,
            updated_at: 2.0,
        };

        assert!(contains_step_session(
            &aborted_state,
            "aborted-step-session"
        ));
        assert!(contains_step_session(&aborted_state, "session-a"));
        assert!(contains_step_session(&aborted_state, "session-b"));
    }
}
