use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::domain::workflow as domain;
use crate::domain::workflow::services::contract_schema;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkflowDto {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub builtin: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub schemas: BTreeMap<String, serde_json::Value>,
    pub nodes: Vec<NodeDefinitionDto>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub(crate) enum NodeKindDto {
    #[default]
    Session,
    Command,
    Fanout,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub(crate) struct FacetRefsDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub(crate) struct SessionSpecDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,
    pub gate: SessionGateDto,
    pub facets: FacetRefsDto,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SessionGateDto {
    #[default]
    Auto,
    Approval,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub(crate) struct FanoutSpecDto {
    pub parallel_children: Vec<InterimChildDto>,
    pub aggregate: Option<ParallelAggregateDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct NodeDefinitionDto {
    pub name: String,
    pub kind: NodeKindDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionSpecDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fanout: Option<FanoutSpecDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collect: Option<CollectConfigDto>,
    #[serde(default, rename = "rules", skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<RuleDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct InterimChildDto {
    pub name: String,
    pub facets: FacetRefsDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParallelAggregateDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all_match: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub any_match: Option<String>,
    pub then: String,
    pub r#else: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "type")]
pub(crate) enum RuleDto {
    When {
        on: String,
        then: String,
        next: String,
    },
    Switch {
        on: String,
        cases: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next: Option<String>,
    },
    LoopGuard {
        max_iterations: u32,
        on_exhausted: String,
    },
    Next {
        next: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct CollectConfigDto {
    pub from: Vec<String>,
    pub reduce: ReduceStrategyDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReduceStrategyDto {
    Last,
    Concat,
    Grouped,
    AnyNeedsFix,
    AllPassed,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct WorkflowSummaryDto {
    pub name: String,
    pub description: String,
    pub builtin: bool,
    #[serde(default)]
    pub is_running: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct FacetSummaryDto {
    pub key: String,
    pub kind: String,
    pub description: String,
    pub builtin: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RunStatusDto {
    Running,
    WaitingApproval,
    Completed,
    Failed,
    Aborted,
    Interrupted,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TriggerSourceDto {
    DesktopUi,
    Remote,
    Cli,
    Agent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkflowRunSummaryDto {
    pub run_id: String,
    pub workflow_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    pub status: RunStatusDto,
    pub worktree_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_node_name: Option<String>,
    pub trigger_source: TriggerSourceDto,
    pub started_at: f64,
    pub updated_at: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_reason: Option<String>,
}

pub(crate) fn workflow_to_dto(definition: &domain::WorkflowDefinition) -> WorkflowDto {
    WorkflowDto {
        name: definition.name.clone(),
        description: definition.description.clone(),
        builtin: definition.builtin,
        schemas: definition
            .schemas
            .iter()
            .map(|(name, schema)| {
                (
                    name.clone(),
                    contract_schema::schema_def_to_json_value(schema),
                )
            })
            .collect(),
        nodes: definition.nodes.iter().map(node_to_dto).collect(),
    }
}

pub(crate) fn workflow_summary_to_dto(summary: domain::WorkflowSummary) -> WorkflowSummaryDto {
    WorkflowSummaryDto {
        name: summary.name,
        description: summary.description,
        builtin: summary.builtin,
        is_running: summary.is_running,
    }
}

pub(crate) fn facet_summary_to_dto(summary: domain::FacetSummary) -> FacetSummaryDto {
    FacetSummaryDto {
        key: summary.key,
        kind: summary.kind,
        description: summary.description,
        builtin: summary.builtin,
    }
}

pub(crate) fn run_summary_to_dto(summary: domain::WorkflowRunSummary) -> WorkflowRunSummaryDto {
    WorkflowRunSummaryDto {
        run_id: summary.run_id,
        workflow_name: summary.workflow_name,
        task: summary.task,
        status: run_status_to_dto(summary.status),
        worktree_path: summary.worktree_path,
        current_node_name: summary.current_node_name,
        trigger_source: trigger_source_to_dto(summary.trigger_source),
        started_at: summary.started_at,
        updated_at: summary.updated_at,
        completed_at: summary.completed_at,
        error_reason: summary.error_reason,
    }
}

fn node_to_dto(node: &domain::NodeDefinition) -> NodeDefinitionDto {
    NodeDefinitionDto {
        name: node.name.clone(),
        kind: node_kind_to_dto(node.kind_name()),
        command: node.command().map(str::to_string),
        session: node.session().map(session_to_dto),
        fanout: node.fanout().map(fanout_to_dto),
        artifact: node.artifact.clone(),
        input: node.input.clone(),
        inputs: node.inputs.clone(),
        collect: node.collect.as_ref().map(collect_to_dto),
        rules: node.rules.iter().map(rule_to_dto).collect(),
    }
}

fn session_to_dto(session: &domain::SessionSpec) -> SessionSpecDto {
    SessionSpecDto {
        model: session.model.clone(),
        permission: session.permission.clone(),
        gate: gate_to_dto(session.gate),
        facets: facet_refs_to_dto(&session.facets),
    }
}

fn fanout_to_dto(fanout: &domain::FanoutSpec) -> FanoutSpecDto {
    FanoutSpecDto {
        parallel_children: fanout
            .parallel_children
            .iter()
            .map(child_node_to_dto)
            .collect(),
        aggregate: fanout.aggregate.as_ref().map(aggregate_to_dto),
    }
}

fn child_node_to_dto(child: &domain::InterimChild) -> InterimChildDto {
    InterimChildDto {
        name: child.name.clone(),
        facets: facet_refs_to_dto(&child.facets),
        artifact: child.artifact.clone(),
        input: child.input.clone(),
        model: child.model.clone(),
        permission: child.permission.clone(),
    }
}

fn node_kind_to_dto(kind: domain::NodeKindName) -> NodeKindDto {
    match kind {
        domain::NodeKindName::Command => NodeKindDto::Command,
        domain::NodeKindName::Session => NodeKindDto::Session,
        domain::NodeKindName::Fanout => NodeKindDto::Fanout,
    }
}

fn gate_to_dto(gate: domain::SessionGate) -> SessionGateDto {
    match gate {
        domain::SessionGate::Auto => SessionGateDto::Auto,
        domain::SessionGate::Approval => SessionGateDto::Approval,
    }
}

fn facet_refs_to_dto(facets: &domain::FacetRefs) -> FacetRefsDto {
    FacetRefsDto {
        policy: facets.policy.clone(),
        knowledge: facets.knowledge.clone(),
        instruction: facets.instruction.clone(),
    }
}

fn collect_to_dto(collect: &domain::CollectConfig) -> CollectConfigDto {
    CollectConfigDto {
        from: collect.from.clone(),
        reduce: reduce_strategy_to_dto(&collect.reduce),
    }
}

fn reduce_strategy_to_dto(reduce: &domain::ReduceStrategy) -> ReduceStrategyDto {
    match reduce {
        domain::ReduceStrategy::Last => ReduceStrategyDto::Last,
        domain::ReduceStrategy::Concat => ReduceStrategyDto::Concat,
        domain::ReduceStrategy::Grouped => ReduceStrategyDto::Grouped,
        domain::ReduceStrategy::AnyNeedsFix => ReduceStrategyDto::AnyNeedsFix,
        domain::ReduceStrategy::AllPassed => ReduceStrategyDto::AllPassed,
    }
}

fn aggregate_to_dto(aggregate: &domain::ParallelAggregate) -> ParallelAggregateDto {
    ParallelAggregateDto {
        all_match: aggregate.all_match.clone(),
        any_match: aggregate.any_match.clone(),
        then: aggregate.then.clone(),
        r#else: aggregate.r#else.clone(),
    }
}

fn rule_to_dto(rule: &domain::Rule) -> RuleDto {
    match rule {
        domain::Rule::When { on, then, next } => RuleDto::When {
            on: on.clone(),
            then: then.clone(),
            next: next.clone(),
        },
        domain::Rule::Switch { on, cases, next } => RuleDto::Switch {
            on: on.clone(),
            cases: cases.clone(),
            next: next.clone(),
        },
        domain::Rule::LoopGuard {
            max_iterations,
            on_exhausted,
        } => RuleDto::LoopGuard {
            max_iterations: *max_iterations,
            on_exhausted: on_exhausted.clone(),
        },
        domain::Rule::Next(next) => RuleDto::Next { next: next.clone() },
    }
}

fn run_status_to_dto(status: domain::RunStatus) -> RunStatusDto {
    match status {
        domain::RunStatus::Running => RunStatusDto::Running,
        domain::RunStatus::WaitingApproval => RunStatusDto::WaitingApproval,
        domain::RunStatus::Completed => RunStatusDto::Completed,
        domain::RunStatus::Failed => RunStatusDto::Failed,
        domain::RunStatus::Aborted => RunStatusDto::Aborted,
        domain::RunStatus::Interrupted => RunStatusDto::Interrupted,
    }
}

fn trigger_source_to_dto(source: domain::TriggerSource) -> TriggerSourceDto {
    match source {
        domain::TriggerSource::DesktopUi => TriggerSourceDto::DesktopUi,
        domain::TriggerSource::Remote => TriggerSourceDto::Remote,
        domain::TriggerSource::Cli => TriggerSourceDto::Cli,
        domain::TriggerSource::Agent => TriggerSourceDto::Agent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_dto_serializes_like_existing_wire_shape() {
        let workflow = WorkflowDto {
            name: "wf".to_string(),
            description: "desc".to_string(),
            builtin: false,
            schemas: [(
                "plan".to_string(),
                serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": [],
                    "additionalProperties": false
                }),
            )]
            .into_iter()
            .collect(),
            nodes: vec![NodeDefinitionDto {
                name: "step".to_string(),
                kind: NodeKindDto::Session,
                session: Some(SessionSpecDto {
                    gate: SessionGateDto::Auto,
                    facets: FacetRefsDto {
                        instruction: Some("inst".to_string()),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                artifact: Some("plan".to_string()),
                input: Some("plan".to_string()),
                rules: vec![RuleDto::Next {
                    next: "done".to_string(),
                }],
                ..Default::default()
            }],
        };

        assert_eq!(
            serde_json::to_value(workflow).unwrap(),
            serde_json::json!({
                "name": "wf",
                "description": "desc",
                "builtin": false,
                "schemas": {
                    "plan": {
                        "type": "object",
                        "properties": {},
                        "required": [],
                        "additionalProperties": false
                    }
                },
                "nodes": [{
                    "name": "step",
                    "kind": "session",
                    "session": {
                        "gate": "auto",
                        "facets": {
                            "instruction": "inst"
                        }
                    },
                    "artifact": "plan",
                    "input": "plan",
                    "rules": [{"type": "next", "next": "done"}]
                }]
            })
        );
    }

    #[test]
    fn run_summary_dto_serializes_like_existing_wire_shape() {
        let summary = WorkflowRunSummaryDto {
            run_id: "00000000-0000-4000-8000-000000000001".to_string(),
            workflow_name: "wf".to_string(),
            task: None,
            status: RunStatusDto::Running,
            worktree_path: "/repo".to_string(),
            current_node_name: None,
            trigger_source: TriggerSourceDto::DesktopUi,
            started_at: 1.0,
            updated_at: 2.0,
            completed_at: None,
            error_reason: None,
        };

        assert_eq!(
            serde_json::to_value(summary).unwrap(),
            serde_json::json!({
                "runId": "00000000-0000-4000-8000-000000000001",
                "workflowName": "wf",
                "status": "running",
                "worktreePath": "/repo",
                "triggerSource": "desktop_ui",
                "startedAt": 1.0,
                "updatedAt": 2.0
            })
        );
    }
}
