use std::collections::{BTreeMap, HashMap, HashSet};

use crate::adaptor::protocol::workflow as workflow_wire;
use crate::domain::workflow;
use crate::domain::workflow::services::contract_schema;
use crate::domain::workflow::WorkflowStateSnapshot;

#[derive(Debug, Clone, Default)]
pub struct WorkflowStepRuntimeProjection {
    pub runtime_active: bool,
    pub tab_open: bool,
}

#[derive(Debug, Clone)]
pub struct WorkflowStateProjection {
    pub state: WorkflowStateSnapshot,
    pub runtime_states: HashMap<String, WorkflowStepRuntimeProjection>,
}

pub fn build_workflow_state_projection_from_sets(
    state: WorkflowStateSnapshot,
    active_sessions: &HashSet<String>,
    open_sessions: &HashSet<String>,
) -> WorkflowStateProjection {
    let runtime_states: HashMap<String, WorkflowStepRuntimeProjection> =
        workflow::services::session_projection::collect_step_session_ids(&state)
            .into_iter()
            .map(|session_id| {
                (
                    session_id.clone(),
                    WorkflowStepRuntimeProjection {
                        runtime_active: active_sessions.contains(&session_id),
                        tab_open: open_sessions.contains(&session_id),
                    },
                )
            })
            .collect();
    WorkflowStateProjection {
        state,
        runtime_states,
    }
}

pub fn workflow_state_to_view(
    state: WorkflowStateSnapshot,
) -> workflow_wire::WorkflowStateFieldsView {
    workflow_wire::WorkflowStateFieldsView {
        execution_id: state.execution_id,
        workflow_name: state.workflow_name,
        state: workflow_execution_state_to_view(state.state),
        current_step_index: state.current_step_index,
        current_step_name: state.current_step_name,
        current_session_id: state.current_session_id,
        total_steps: state.total_steps,
        step_history: state
            .step_history
            .into_iter()
            .map(step_history_entry_to_view)
            .collect(),
        step_execution_counts: state.step_execution_counts,
        workflow_definition: workflow_definition_to_view(state.workflow_definition),
        total_token_usage: token_usage_to_view(state.total_token_usage),
        step_states: state.step_states,
        step_outputs: state
            .step_outputs
            .into_iter()
            .map(|(key, output)| (key, step_output_to_view(output)))
            .collect(),
        node_executions: state
            .node_executions
            .into_iter()
            .map(node_execution_to_view)
            .collect(),
        approval_operations: state.approval_operations.map(|operations| {
            workflow_wire::ApprovalOperationsView {
                can_approve: operations.can_approve,
            }
        }),
        stall_observations: state
            .stall_observations
            .into_iter()
            .map(stall_observation_to_view)
            .collect(),
        started_at: state.started_at,
        updated_at: state.updated_at,
    }
}

fn stall_observation_to_view(
    observation: workflow::WorkflowStallObservation,
) -> workflow_wire::WorkflowStallObservationView {
    workflow_wire::WorkflowStallObservationView {
        chat_session_id: observation.session_id,
        step_name: observation.step_name,
        run_index: observation.run_index,
        turn_phase: observation.turn_phase,
        idle_secs: observation.idle_secs,
        signal_count: observation.signal_count,
        cap_reached: observation.cap_reached,
        observed_at: observation.observed_at,
    }
}

fn workflow_execution_state_to_view(
    state: workflow::WorkflowExecutionState,
) -> workflow_wire::WorkflowExecutionStateView {
    match state {
        workflow::WorkflowExecutionState::Running => {
            workflow_wire::WorkflowExecutionStateView::Running
        }
        workflow::WorkflowExecutionState::WaitingApproval => {
            workflow_wire::WorkflowExecutionStateView::WaitingApproval
        }
        workflow::WorkflowExecutionState::Completed => {
            workflow_wire::WorkflowExecutionStateView::Completed
        }
        workflow::WorkflowExecutionState::Failed {
            reason,
            kind,
            retry_count,
        } => workflow_wire::WorkflowExecutionStateView::Failed {
            reason,
            failure_kind: kind,
            retry_count,
        },
        workflow::WorkflowExecutionState::Aborted => {
            workflow_wire::WorkflowExecutionStateView::Aborted
        }
        workflow::WorkflowExecutionState::Interrupted => {
            workflow_wire::WorkflowExecutionStateView::Interrupted
        }
    }
}

fn token_usage_to_view(usage: workflow::TokenUsage) -> workflow_wire::TokenUsageView {
    workflow_wire::TokenUsageView {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
    }
}

fn workflow_definition_to_view(
    workflow: workflow::WorkflowDefinition,
) -> workflow_wire::WorkflowDefinitionView {
    workflow_wire::WorkflowDefinitionView {
        name: workflow.name,
        description: workflow.description,
        builtin: workflow.builtin,
        schemas: workflow
            .schemas
            .into_iter()
            .map(|(name, schema)| (name, contract_schema::schema_def_to_json_value(&schema)))
            .collect::<BTreeMap<_, _>>(),
        nodes: workflow
            .nodes
            .into_iter()
            .map(workflow_node_to_view)
            .collect(),
    }
}

fn workflow_node_to_view(
    node: workflow::NodeDefinition,
) -> workflow_wire::WorkflowNodeDefinitionView {
    let (kind, command, session, fanout) = match node.kind {
        workflow::NodeKind::Command(spec) => (
            workflow_wire::WorkflowNodeKindView::Command,
            Some(spec.command),
            None,
            None,
        ),
        workflow::NodeKind::Session(spec) => (
            workflow_wire::WorkflowNodeKindView::Session,
            None,
            Some(session_spec_to_view(spec)),
            None,
        ),
        workflow::NodeKind::Fanout(spec) => (
            workflow_wire::WorkflowNodeKindView::Fanout,
            None,
            None,
            Some(fanout_spec_to_view(spec)),
        ),
    };
    workflow_wire::WorkflowNodeDefinitionView {
        name: node.name,
        kind,
        command,
        session,
        fanout,
        artifact: node.artifact,
        input: node.input,
        inputs: node.inputs,
        collect: node.collect.map(collect_config_to_view),
        rules: node.rules.into_iter().map(rule_to_view).collect(),
    }
}

fn session_spec_to_view(spec: workflow::SessionSpec) -> workflow_wire::WorkflowSessionSpecView {
    workflow_wire::WorkflowSessionSpecView {
        model: spec.model,
        permission: spec.permission,
        gate: session_gate_to_view(spec.gate),
        facets: facet_refs_to_view(spec.facets),
    }
}

fn fanout_spec_to_view(spec: workflow::FanoutSpec) -> workflow_wire::WorkflowFanoutSpecView {
    workflow_wire::WorkflowFanoutSpecView {
        child: spec.child,
        items: spec.items.map(items_source_to_view),
        aggregate: spec.aggregate.map(aggregate_config_to_view),
    }
}

fn items_source_to_view(items: workflow::ItemsSource) -> workflow_wire::WorkflowItemsSourceView {
    match items {
        workflow::ItemsSource::Literal(values) => {
            workflow_wire::WorkflowItemsSourceView::Literal(values)
        }
        workflow::ItemsSource::ArtifactField { node, field } => {
            workflow_wire::WorkflowItemsSourceView::ArtifactField(format!("{node}.{field}"))
        }
    }
}

fn session_gate_to_view(gate: workflow::SessionGate) -> workflow_wire::WorkflowSessionGateView {
    match gate {
        workflow::SessionGate::Auto => workflow_wire::WorkflowSessionGateView::Auto,
        workflow::SessionGate::Approval => workflow_wire::WorkflowSessionGateView::Approval,
    }
}

fn facet_refs_to_view(facets: workflow::FacetRefs) -> workflow_wire::WorkflowFacetRefsView {
    workflow_wire::WorkflowFacetRefsView {
        policy: facets.policy,
        knowledge: facets.knowledge,
        instruction: facets.instruction,
    }
}

fn rule_to_view(rule: workflow::Rule) -> workflow_wire::WorkflowRuleView {
    match rule {
        workflow::Rule::When { on, then, next } => {
            workflow_wire::WorkflowRuleView::When { on, then, next }
        }
        workflow::Rule::Switch { on, cases, next } => {
            workflow_wire::WorkflowRuleView::Switch { on, cases, next }
        }
        workflow::Rule::LoopGuard {
            max_iterations,
            on_exhausted,
        } => workflow_wire::WorkflowRuleView::LoopGuard {
            max_iterations,
            on_exhausted,
        },
        workflow::Rule::Next(next) => workflow_wire::WorkflowRuleView::Next { next },
    }
}

fn collect_config_to_view(
    collect: workflow::CollectConfig,
) -> workflow_wire::WorkflowCollectConfigView {
    workflow_wire::WorkflowCollectConfigView {
        from: collect.from,
        reduce: reduce_strategy_to_view(collect.reduce),
    }
}

fn reduce_strategy_to_view(
    reduce: workflow::ReduceStrategy,
) -> workflow_wire::WorkflowReduceStrategyView {
    match reduce {
        workflow::ReduceStrategy::Last => workflow_wire::WorkflowReduceStrategyView::Last,
        workflow::ReduceStrategy::Concat => workflow_wire::WorkflowReduceStrategyView::Concat,
        workflow::ReduceStrategy::Grouped => workflow_wire::WorkflowReduceStrategyView::Grouped,
        workflow::ReduceStrategy::AnyNeedsFix => {
            workflow_wire::WorkflowReduceStrategyView::AnyNeedsFix
        }
        workflow::ReduceStrategy::AllPassed => workflow_wire::WorkflowReduceStrategyView::AllPassed,
    }
}

fn aggregate_config_to_view(
    aggregate: workflow::ParallelAggregate,
) -> workflow_wire::WorkflowAggregateConfigView {
    workflow_wire::WorkflowAggregateConfigView {
        all_match: aggregate.all_match,
        any_match: aggregate.any_match,
        then: aggregate.then,
        r#else: aggregate.r#else,
    }
}

fn step_history_entry_to_view(
    entry: workflow::StepHistoryEntry,
) -> workflow_wire::StepHistoryEntryView {
    workflow_wire::StepHistoryEntryView {
        step_name: entry.step_name,
        completed_at: entry.completed_at,
        result: entry.result,
        session_id: entry.session_id,
        token_usage: entry.token_usage.map(token_usage_to_view),
        structured_output: entry.structured_output,
        run_index: entry.run_index,
        child_outputs: entry
            .child_outputs
            .map(|children| children.into_iter().map(child_output_to_view).collect()),
        state: entry.state,
    }
}

fn child_output_to_view(
    output: workflow::ChildOutputSnapshot,
) -> workflow_wire::ChildOutputSnapshotView {
    workflow_wire::ChildOutputSnapshotView {
        step_name: output.step_name,
        session_id: output.session_id,
        result: output.result,
        run_index: output.run_index,
        completed_at: output.completed_at,
        structured_output: output.structured_output,
        artifact_contract: output.artifact_contract,
        state: output.state,
        failure_kind: output.failure_kind,
        failure_disposition: output.failure_disposition,
    }
}

fn node_execution_to_view(execution: workflow::NodeExecution) -> workflow_wire::NodeExecutionView {
    workflow_wire::NodeExecutionView {
        id: execution.id,
        execution_id: execution.execution_id,
        node_name: execution.node_name,
        kind: node_kind_name_to_view(execution.kind),
        attempt: execution.attempt,
        status: node_execution_status_to_view(execution.status),
        session_id: execution.session_id,
        artifact: execution.artifact,
        token_usage: execution.token_usage.map(token_usage_to_view),
        failure: execution
            .failure
            .map(|failure| workflow_wire::NodeExecutionFailureView {
                reason: failure.reason,
                kind: failure.kind,
            }),
        fanout_parent: execution
            .fanout_parent
            .map(|parent| workflow_wire::FanoutParentRefView {
                parent_node: parent.parent_node,
                parent_attempt: parent.parent_attempt,
                item_index: parent.item_index,
                child_index: parent.child_index,
            }),
        started_at: execution.started_at,
        completed_at: execution.completed_at,
    }
}

fn node_kind_name_to_view(kind: workflow::NodeKindName) -> workflow_wire::WorkflowNodeKindView {
    match kind {
        workflow::NodeKindName::Command => workflow_wire::WorkflowNodeKindView::Command,
        workflow::NodeKindName::Session => workflow_wire::WorkflowNodeKindView::Session,
        workflow::NodeKindName::Fanout => workflow_wire::WorkflowNodeKindView::Fanout,
    }
}

fn node_execution_status_to_view(
    status: workflow::NodeExecutionStatus,
) -> workflow_wire::NodeExecutionStatusView {
    match status {
        workflow::NodeExecutionStatus::Running => workflow_wire::NodeExecutionStatusView::Running,
        workflow::NodeExecutionStatus::WaitingApproval => {
            workflow_wire::NodeExecutionStatusView::WaitingApproval
        }
        workflow::NodeExecutionStatus::Succeeded => {
            workflow_wire::NodeExecutionStatusView::Succeeded
        }
        workflow::NodeExecutionStatus::Failed => workflow_wire::NodeExecutionStatusView::Failed,
        workflow::NodeExecutionStatus::Aborted => workflow_wire::NodeExecutionStatusView::Aborted,
    }
}

fn step_output_to_view(output: workflow::StepOutput) -> workflow_wire::StepOutputView {
    workflow_wire::StepOutputView {
        step_name: output.step_name,
        run_index: output.run_index,
        session_id: output.session_id,
        result: output.result,
        structured_output: output.structured_output,
        artifact_contract: output.artifact_contract,
        token_usage: output.token_usage.map(token_usage_to_view),
        completed_at: output.completed_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{
        ChildOutputSnapshot, FacetRefs, FanoutParentRef, FanoutSpec, ItemsSource, NodeDefinition,
        NodeExecution, NodeExecutionFailure, NodeExecutionStatus, NodeKind, NodeKindName,
        SessionGate, SessionSpec, StepHistoryEntry, TokenUsage, WorkflowDefinition,
        WorkflowExecutionState,
    };

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
                    artifact_contract: None,
                    state: "completed".to_string(),
                    failure_kind: None,
                    failure_disposition: None,
                }]),
                state: "completed".to_string(),
            }],
            step_execution_counts: HashMap::new(),
            workflow_definition: WorkflowDefinition {
                name: "wf".to_string(),
                description: String::new(),
                builtin: false,
                schemas: Default::default(),
                nodes: vec![],
            },
            total_token_usage: TokenUsage::default(),
            step_states: HashMap::new(),
            step_outputs: HashMap::new(),
            node_executions: vec![NodeExecution {
                id: "ne-running-child".to_string(),
                execution_id: "exec-1".to_string(),
                node_name: "running-child".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                status: NodeExecutionStatus::Running,
                session_id: Some("fanout-session".to_string()),
                artifact: None,
                token_usage: None,
                failure: None,
                fanout_parent: Some(FanoutParentRef {
                    parent_node: "current".to_string(),
                    parent_attempt: 1,
                    item_index: Some(0),
                    child_index: 0,
                }),
                started_at: 1.5,
                completed_at: None,
            }],
            approval_operations: None,
            stall_observations: Vec::new(),
            started_at: 0.0,
            updated_at: 2.0,
        }
    }

    #[test]
    fn presenter_adds_runtime_state_for_current_history_and_fanout_child_sessions() {
        let open_sessions =
            HashSet::from(["done-session".to_string(), "fanout-session".to_string()]);
        let active_sessions = HashSet::from([
            "current-session".to_string(),
            "child-session".to_string(),
            "fanout-session".to_string(),
        ]);

        let view =
            build_workflow_state_projection_from_sets(state(), &active_sessions, &open_sessions);

        assert!(view.runtime_states["current-session"].runtime_active);
        assert!(!view.runtime_states["current-session"].tab_open);
        assert!(view.runtime_states["child-session"].runtime_active);
        assert!(!view.runtime_states["child-session"].tab_open);
        assert!(view.runtime_states["done-session"].tab_open);
        assert!(!view.runtime_states["done-session"].runtime_active);
        assert!(view.runtime_states["fanout-session"].runtime_active);
        assert!(view.runtime_states["fanout-session"].tab_open);
    }

    #[test]
    fn workflow_state_to_view_preserves_failed_classification() {
        let mut state = state();
        state.state = WorkflowExecutionState::Failed {
            reason: "startup timed out".to_string(),
            kind: crate::domain::workflow::WorkflowStepFailureKind::StartupTimeout,
            retry_count: Some(2),
        };

        let view = workflow_state_to_view(state);

        assert_eq!(
            view.state,
            workflow_wire::WorkflowExecutionStateView::Failed {
                reason: "startup timed out".to_string(),
                failure_kind: crate::domain::workflow::WorkflowStepFailureKind::StartupTimeout,
                retry_count: Some(2),
            }
        );
        let json = serde_json::to_value(&view.state).unwrap();
        assert_eq!(json["failureKind"], "startup_timeout");
        assert_eq!(json["retryCount"], 2);
    }

    #[test]
    fn workflow_state_to_view_maps_node_execution_and_child_failure_metadata() {
        let mut state = state();
        state.node_executions[0].status = NodeExecutionStatus::Failed;
        state.node_executions[0].failure = Some(NodeExecutionFailure {
            reason: "model_refusal".to_string(),
            kind: crate::domain::workflow::WorkflowStepFailureKind::ModelRefusal,
        });
        state.step_history[0].child_outputs = Some(vec![ChildOutputSnapshot {
            step_name: "review-child".to_string(),
            session_id: Some("child-session".to_string()),
            result: Some("model_refusal".to_string()),
            run_index: 1,
            completed_at: 3.0,
            structured_output: None,
            artifact_contract: None,
            state: crate::domain::workflow::STEP_STATE_FAILED.to_string(),
            failure_kind: Some(crate::domain::workflow::WorkflowStepFailureKind::ModelRefusal),
            failure_disposition: Some(crate::domain::workflow::FailureDisposition::Partial),
        }]);

        let view = workflow_state_to_view(state);
        let child = view.step_history[0].child_outputs.as_ref().unwrap()[0].clone();
        let execution = view.node_executions[0].clone();

        assert_eq!(
            child.failure_kind,
            Some(crate::domain::workflow::WorkflowStepFailureKind::ModelRefusal)
        );
        assert_eq!(
            child.failure_disposition,
            Some(crate::domain::workflow::FailureDisposition::Partial)
        );
        assert_eq!(
            execution.failure.as_ref().map(|failure| failure.kind),
            Some(crate::domain::workflow::WorkflowStepFailureKind::ModelRefusal)
        );
        assert_eq!(execution.id, "ne-running-child");
        assert_eq!(execution.node_name, "running-child");
        assert_eq!(
            execution.status,
            workflow_wire::NodeExecutionStatusView::Failed
        );
        assert_eq!(
            execution.fanout_parent.as_ref().unwrap().item_index,
            Some(0)
        );

        let value = serde_json::to_value(view).unwrap();
        assert_eq!(
            value["stepHistory"][0]["childOutputs"][0]["failureKind"],
            "model_refusal"
        );
        assert_eq!(
            value["nodeExecutions"][0]["failure"]["kind"],
            "model_refusal"
        );
    }

    // ---- WorkflowState wire view serde ----

    fn make_session_test_node(name: &str, instruction: &str) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Session(SessionSpec {
                facets: FacetRefs {
                    instruction: Some(instruction.to_string()),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..NodeDefinition::default()
        }
    }

    fn make_approval_test_node(name: &str, instruction: &str) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Session(SessionSpec {
                gate: SessionGate::Approval,
                facets: FacetRefs {
                    instruction: Some(instruction.to_string()),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..NodeDefinition::default()
        }
    }

    fn make_test_workflow_for_session() -> WorkflowDefinition {
        WorkflowDefinition {
            name: "review-cycle".to_string(),
            description: "Test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                make_session_test_node("plan", "plan"),
                make_session_test_node("implement", "implement"),
                make_session_test_node("review", "review"),
                make_approval_test_node("report", "report"),
            ],
        }
    }

    #[test]
    fn workflow_state_view_serde_roundtrip() {
        let state = WorkflowStateSnapshot {
            execution_id: "exec-1".to_string(),
            workflow_name: "review-cycle".to_string(),
            state: WorkflowExecutionState::Running,
            current_step_index: 2,
            current_step_name: "review".to_string(),
            current_session_id: Some("sess-current".to_string()),
            total_steps: 4,
            step_history: vec![
                StepHistoryEntry {
                    step_name: "plan".to_string(),
                    completed_at: 1000.0,
                    result: None,
                    session_id: None,
                    token_usage: None,
                    structured_output: None,
                    run_index: 0,
                    child_outputs: None,
                    state: crate::domain::workflow::value_objects::default_step_entry_state(),
                },
                StepHistoryEntry {
                    step_name: "implement".to_string(),
                    completed_at: 1001.0,
                    result: Some("done".to_string()),
                    session_id: Some("sess-1".to_string()),
                    token_usage: Some(TokenUsage {
                        input_tokens: 100,
                        output_tokens: 50,
                    }),
                    structured_output: None,
                    run_index: 0,
                    child_outputs: None,
                    state: crate::domain::workflow::value_objects::default_step_entry_state(),
                },
            ],
            step_execution_counts: HashMap::new(),
            step_outputs: HashMap::new(),
            workflow_definition: make_test_workflow_for_session(),
            total_token_usage: TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
            },
            step_states: HashMap::new(),
            node_executions: vec![],
            approval_operations: None,
            stall_observations: Vec::new(),
            started_at: 999.0,
            updated_at: 1001.0,
        };
        let view = workflow_state_to_view(state);
        let json = serde_json::to_string(&view).unwrap();
        let back: workflow_wire::WorkflowStateFieldsView = serde_json::from_str(&json).unwrap();
        assert_eq!(back.execution_id, "exec-1");
        assert_eq!(back.workflow_name, "review-cycle");
        assert_eq!(
            back.state,
            workflow_wire::WorkflowExecutionStateView::Running
        );
        assert_eq!(back.current_step_index, 2);
        assert_eq!(back.current_step_name, "review");
        assert_eq!(back.total_steps, 4);
        assert_eq!(back.step_history.len(), 2);
        assert_eq!(back.step_history[0].step_name, "plan");
        assert_eq!(back.step_history[1].result, Some("done".to_string()));
    }

    #[test]
    fn workflow_definition_view_maps_fanout_child_and_items() {
        let definition = WorkflowDefinition {
            name: "fanout".to_string(),
            description: String::new(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                name: "dispatch".to_string(),
                kind: NodeKind::Fanout(FanoutSpec {
                    child: vec!["worker-a".to_string(), "worker-b".to_string()],
                    items: Some(ItemsSource::ArtifactField {
                        node: "scan".to_string(),
                        field: "items".to_string(),
                    }),
                    aggregate: None,
                }),
                ..NodeDefinition::default()
            }],
        };

        let view = workflow_definition_to_view(definition);
        let fanout = view.nodes[0].fanout.as_ref().unwrap();
        assert_eq!(fanout.child, ["worker-a", "worker-b"]);
        assert_eq!(
            fanout.items,
            Some(workflow_wire::WorkflowItemsSourceView::ArtifactField(
                "scan.items".to_string()
            ))
        );
    }
}
