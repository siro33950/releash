use std::collections::HashSet;

use crate::domain::workflow::status_aggregation::{RepresentativeStatus, StepProgress};
use crate::domain::workflow::WorkflowStateSnapshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepSessionProjection {
    pub session_id: Option<String>,
    pub step_name: String,
    pub run_index: Option<u32>,
    pub group_step_name: String,
    pub group_run_index: Option<u32>,
    pub progress: StepProgress,
    pub representative: RepresentativeStatus,
    pub order: usize,
}

pub fn current_run_index(state: &WorkflowStateSnapshot) -> Option<u32> {
    state
        .step_execution_counts
        .get(&state.current_step_name)
        .copied()
        .or(Some(1))
}

fn projection(
    session_id: Option<String>,
    step_name: String,
    run_index: Option<u32>,
    group_step_name: String,
    group_run_index: Option<u32>,
    status: &str,
    order: usize,
) -> StepSessionProjection {
    StepSessionProjection {
        session_id,
        step_name,
        run_index,
        group_step_name,
        group_run_index,
        progress: StepProgress::from_status_str(status),
        representative: RepresentativeStatus::from_status_str(status),
        order,
    }
}

pub fn collect_step_session_projections(
    state: &WorkflowStateSnapshot,
) -> Vec<StepSessionProjection> {
    let mut projections = Vec::new();
    let mut next_order = 0usize;

    for entry in &state.step_history {
        let order = next_order;
        next_order += 1;
        projections.push(projection(
            entry.session_id.clone(),
            entry.step_name.clone(),
            Some(entry.run_index),
            entry.step_name.clone(),
            Some(entry.run_index),
            &entry.state,
            order,
        ));
        if let Some(children) = entry.child_outputs.as_ref() {
            for child in children {
                projections.push(projection(
                    child.session_id.clone(),
                    child.step_name.clone(),
                    Some(child.run_index),
                    entry.step_name.clone(),
                    Some(entry.run_index),
                    &child.state,
                    order,
                ));
            }
        }
    }

    if state.current_session_id.is_some() || state.state.is_active() {
        let order = next_order;
        let run_index = current_run_index(state);
        projections.push(projection(
            state.current_session_id.clone(),
            state.current_step_name.clone(),
            run_index,
            state.current_step_name.clone(),
            run_index,
            state.state.as_str(),
            order,
        ));
        for step in &state.active_parallel_steps {
            projections.push(projection(
                step.session_id.clone(),
                step.step_name.clone(),
                Some(step.run_index),
                state.current_step_name.clone(),
                run_index,
                &step.state,
                order,
            ));
        }
    }

    retain_unique_step_session_projections(&mut projections);
    projections
}

fn retain_unique_step_session_projections(projections: &mut Vec<StepSessionProjection>) {
    let mut seen = HashSet::new();
    projections.retain(|projection| {
        seen.insert((
            projection.session_id.clone(),
            projection.step_name.clone(),
            projection.run_index,
            projection.group_step_name.clone(),
            projection.group_run_index,
        ))
    });
}

pub fn collect_step_session_ids(state: &WorkflowStateSnapshot) -> HashSet<String> {
    let mut ids = collect_step_session_projections(state)
        .into_iter()
        .filter_map(|projection| projection.session_id)
        .collect::<HashSet<_>>();
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
                    failure_kind: None,
                    failure_disposition: None,
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
                failure_kind: None,
                failure_disposition: None,
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
    fn projects_group_key_run_index_progress_and_order_from_snapshot() {
        let projections = collect_step_session_projections(&state());

        let done = projections
            .iter()
            .find(|projection| projection.session_id.as_deref() == Some("done-session"))
            .expect("done session projection");
        assert_eq!(done.step_name, "done");
        assert_eq!(done.run_index, Some(1));
        assert_eq!(done.group_step_name, "done");
        assert_eq!(done.group_run_index, Some(1));
        assert_eq!(done.progress, StepProgress::Completed);
        assert_eq!(done.representative, RepresentativeStatus::Completed);
        assert_eq!(done.order, 0);

        let child = projections
            .iter()
            .find(|projection| projection.session_id.as_deref() == Some("child-session"))
            .expect("child session projection");
        assert_eq!(child.step_name, "child");
        assert_eq!(child.run_index, Some(1));
        assert_eq!(child.group_step_name, "done");
        assert_eq!(child.group_run_index, Some(1));
        assert_eq!(child.progress, StepProgress::Completed);
        assert_eq!(child.representative, RepresentativeStatus::Completed);
        assert_eq!(child.order, 0);

        let current = projections
            .iter()
            .find(|projection| projection.session_id.as_deref() == Some("current-session"))
            .expect("current session projection");
        assert_eq!(current.step_name, "current");
        assert_eq!(current.group_step_name, "current");
        assert_eq!(current.run_index, Some(1));
        assert_eq!(current.group_run_index, Some(1));
        assert_eq!(current.progress, StepProgress::Running);
        assert_eq!(current.representative, RepresentativeStatus::Running);
        assert_eq!(current.order, 1);

        let parallel = projections
            .iter()
            .find(|projection| projection.session_id.as_deref() == Some("parallel-session"))
            .expect("parallel session projection");
        assert_eq!(parallel.step_name, "running-child");
        assert_eq!(parallel.run_index, Some(1));
        assert_eq!(parallel.group_step_name, "current");
        assert_eq!(parallel.group_run_index, Some(1));
        assert_eq!(parallel.progress, StepProgress::Running);
        assert_eq!(parallel.representative, RepresentativeStatus::Running);
        assert_eq!(parallel.order, 1);
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
                            failure_kind: None,
                            failure_disposition: None,
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
                            failure_kind: None,
                            failure_disposition: None,
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
