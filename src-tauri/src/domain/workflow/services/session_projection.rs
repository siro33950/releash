use std::collections::HashSet;

use crate::domain::workflow::WorkflowStateSnapshot;

pub fn collect_step_session_ids(state: &WorkflowStateSnapshot) -> HashSet<String> {
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

pub fn collect_completed_step_session_ids(state: &WorkflowStateSnapshot) -> Vec<String> {
    let mut ids = Vec::new();
    let Some(entry) = state.step_history.last() else {
        return ids;
    };
    if let Some(session_id) = entry.session_id.as_ref() {
        ids.push(session_id.clone());
    }
    if let Some(children) = entry.child_outputs.as_ref() {
        ids.extend(children.iter().filter_map(|child| child.session_id.clone()));
    }
    ids.sort();
    ids.dedup();
    ids
}

pub fn collect_terminal_step_session_ids(state: &WorkflowStateSnapshot) -> Vec<String> {
    let mut ids = Vec::new();
    ids.extend(state.current_session_id.iter().cloned());
    ids.extend(
        state
            .active_parallel_steps
            .iter()
            .filter_map(|step| step.session_id.clone()),
    );
    ids.sort();
    ids.dedup();
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{
        ChildOutputSnapshot, ParallelStepState, StepHistoryEntry, WorkflowDefinition,
        WorkflowExecutionState, STEP_STATE_ABORTED, STEP_STATE_COMPLETED, STEP_STATE_RUNNING,
    };
    use std::collections::HashMap;

    fn state() -> WorkflowStateSnapshot {
        WorkflowStateSnapshot {
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
                    state: STEP_STATE_COMPLETED.to_string(),
                }]),
                state: STEP_STATE_COMPLETED.to_string(),
            }],
            step_execution_counts: HashMap::new(),
            workflow_definition: WorkflowDefinition {
                name: "wf".to_string(),
                description: String::new(),
                builtin: false,
                variables: HashMap::new(),
                nodes: Vec::new(),
            },
            total_token_usage: Default::default(),
            step_states: HashMap::new(),
            step_outputs: HashMap::new(),
            active_parallel_steps: vec![ParallelStepState {
                step_name: "running-child".to_string(),
                state: STEP_STATE_RUNNING.to_string(),
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
    fn collects_current_history_child_and_parallel_session_ids() {
        let ids = collect_step_session_ids(&state());

        assert!(ids.contains("current-session"));
        assert!(ids.contains("done-session"));
        assert!(ids.contains("child-session"));
        assert!(ids.contains("parallel-session"));
        assert!(!ids.contains("regular-chat-session"));
    }

    #[test]
    fn completed_step_session_ids_use_last_history_entry_only() {
        let ids = collect_completed_step_session_ids(&state());

        assert_eq!(ids, vec!["child-session", "done-session"]);
    }

    #[test]
    fn terminal_step_session_ids_use_current_and_active_parallel_sessions() {
        let ids = collect_terminal_step_session_ids(&state());

        assert_eq!(ids, vec!["current-session", "parallel-session"]);
    }

    #[test]
    fn collects_session_ids_from_aborted_history_entries_and_child_snapshots() {
        let aborted_state = WorkflowStateSnapshot {
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
                    state: STEP_STATE_ABORTED.to_string(),
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
                            state: STEP_STATE_COMPLETED.to_string(),
                        },
                        ChildOutputSnapshot {
                            step_name: "child-b".to_string(),
                            session_id: Some("session-b".to_string()),
                            result: None,
                            run_index: 1,
                            completed_at: 2.0,
                            structured_output: None,
                            output_contract: None,
                            state: STEP_STATE_ABORTED.to_string(),
                        },
                    ]),
                    state: STEP_STATE_ABORTED.to_string(),
                },
            ],
            step_execution_counts: HashMap::new(),
            workflow_definition: WorkflowDefinition {
                name: "wf".to_string(),
                description: String::new(),
                builtin: false,
                variables: HashMap::new(),
                nodes: Vec::new(),
            },
            total_token_usage: Default::default(),
            step_states: HashMap::new(),
            step_outputs: HashMap::new(),
            active_parallel_steps: vec![],
            workflow_variables: HashMap::new(),
            approval_operations: None,
            started_at: 0.0,
            updated_at: 2.0,
        };

        let ids = collect_step_session_ids(&aborted_state);

        assert!(ids.contains("aborted-step-session"));
        assert!(ids.contains("session-a"));
        assert!(ids.contains("session-b"));
    }
}
