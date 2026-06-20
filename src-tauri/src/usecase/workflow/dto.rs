use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::domain::workflow as domain;
use crate::domain::workflow::value_objects::ResolvedFacets;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkflowDto {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub builtin: bool,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub variables: HashMap<String, String>,
    pub nodes: Vec<NodeDefinitionDto>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct ResolvedFacetsDto {
    pub policy: Option<String>,
    pub knowledge: Option<String>,
    pub instruction: Option<String>,
    pub output_contract: Option<String>,
    pub input_contracts: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub(crate) enum NodeTypeDto {
    #[default]
    Agent,
    Bash,
    Approval,
    Parallel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct NodeDefinitionDto {
    pub name: String,
    #[serde(rename = "type")]
    pub node_type: NodeTypeDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_contract: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_contracts: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass_previous_response: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass_output_from: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collect: Option<CollectConfigDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_children: Option<Vec<ChildNodeDefinitionDto>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggregate: Option<ParallelAggregateDto>,
    #[serde(default, rename = "rules", skip_serializing_if = "Vec::is_empty")]
    pub transition_rules: Vec<TransitionRuleDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle_guard: Option<CycleGuardDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_cycle_for: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,
    #[serde(skip)]
    pub resolved_facets: ResolvedFacetsDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChildNodeDefinitionDto {
    pub name: String,
    #[serde(rename = "type")]
    pub node_type: NodeTypeDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_contract: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_contracts: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass_previous_response: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass_output_from: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,
    #[serde(skip)]
    pub resolved_facets: ResolvedFacetsDto,
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
#[serde(deny_unknown_fields)]
pub(crate) struct TransitionRuleDto {
    pub r#match: String,
    pub next: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct CycleGuardDto {
    pub max_iterations: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_exhausted: Option<String>,
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
        variables: definition.variables.clone(),
        nodes: definition.nodes.iter().map(node_to_dto).collect(),
    }
}

pub(crate) fn workflow_from_dto(workflow: WorkflowDto) -> domain::WorkflowDefinition {
    domain::WorkflowDefinition {
        name: workflow.name,
        description: workflow.description,
        builtin: workflow.builtin,
        variables: workflow.variables,
        nodes: workflow.nodes.into_iter().map(node_from_dto).collect(),
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
        node_type: node_type_to_dto(node.node_type),
        policy: node.policy.clone(),
        knowledge: node.knowledge.clone(),
        instruction: node.instruction.clone(),
        output_contract: node.output_contract.clone(),
        input_contracts: node.input_contracts.clone(),
        pass_previous_response: node.pass_previous_response,
        pass_output_from: node.pass_output_from.clone(),
        inline_prompt: node.inline_prompt.clone(),
        collect: node.collect.as_ref().map(collect_to_dto),
        command: node.command.clone(),
        parallel_children: node
            .parallel_children
            .as_ref()
            .map(|children| children.iter().map(child_node_to_dto).collect()),
        aggregate: node.aggregate.as_ref().map(aggregate_to_dto),
        transition_rules: node
            .transition_rules
            .iter()
            .map(transition_rule_to_dto)
            .collect(),
        cycle_guard: node.cycle_guard.as_ref().map(cycle_guard_to_dto),
        resets_cycle_for: node.resets_cycle_for.clone(),
        model: node.model.clone(),
        permission: node.permission.clone(),
        resolved_facets: resolved_facets_to_dto(&node.resolved_facets),
    }
}

fn node_from_dto(node: NodeDefinitionDto) -> domain::NodeDefinition {
    domain::NodeDefinition {
        name: node.name,
        node_type: node_type_from_dto(node.node_type),
        policy: node.policy,
        knowledge: node.knowledge,
        instruction: node.instruction,
        output_contract: node.output_contract,
        input_contracts: node.input_contracts,
        pass_previous_response: node.pass_previous_response,
        pass_output_from: node.pass_output_from,
        inline_prompt: node.inline_prompt,
        collect: node.collect.map(collect_from_dto),
        command: node.command,
        parallel_children: node
            .parallel_children
            .map(|children| children.into_iter().map(child_node_from_dto).collect()),
        aggregate: node.aggregate.map(aggregate_from_dto),
        transition_rules: node
            .transition_rules
            .into_iter()
            .map(transition_rule_from_dto)
            .collect(),
        cycle_guard: node.cycle_guard.map(cycle_guard_from_dto),
        resets_cycle_for: node.resets_cycle_for,
        model: node.model,
        permission: node.permission,
        resolved_facets: resolved_facets_from_dto(node.resolved_facets),
    }
}

fn child_node_to_dto(child: &domain::ChildNodeDefinition) -> ChildNodeDefinitionDto {
    ChildNodeDefinitionDto {
        name: child.name.clone(),
        node_type: node_type_to_dto(child.node_type),
        policy: child.policy.clone(),
        knowledge: child.knowledge.clone(),
        instruction: child.instruction.clone(),
        output_contract: child.output_contract.clone(),
        input_contracts: child.input_contracts.clone(),
        pass_previous_response: child.pass_previous_response,
        pass_output_from: child.pass_output_from.clone(),
        model: child.model.clone(),
        permission: child.permission.clone(),
        resolved_facets: resolved_facets_to_dto(&child.resolved_facets),
    }
}

fn child_node_from_dto(child: ChildNodeDefinitionDto) -> domain::ChildNodeDefinition {
    domain::ChildNodeDefinition {
        name: child.name,
        node_type: node_type_from_dto(child.node_type),
        policy: child.policy,
        knowledge: child.knowledge,
        instruction: child.instruction,
        output_contract: child.output_contract,
        input_contracts: child.input_contracts,
        pass_previous_response: child.pass_previous_response,
        pass_output_from: child.pass_output_from,
        model: child.model,
        permission: child.permission,
        resolved_facets: resolved_facets_from_dto(child.resolved_facets),
    }
}

fn node_type_to_dto(node_type: domain::NodeType) -> NodeTypeDto {
    match node_type {
        domain::NodeType::Agent => NodeTypeDto::Agent,
        domain::NodeType::Bash => NodeTypeDto::Bash,
        domain::NodeType::Approval => NodeTypeDto::Approval,
        domain::NodeType::Parallel => NodeTypeDto::Parallel,
    }
}

fn node_type_from_dto(node_type: NodeTypeDto) -> domain::NodeType {
    match node_type {
        NodeTypeDto::Agent => domain::NodeType::Agent,
        NodeTypeDto::Bash => domain::NodeType::Bash,
        NodeTypeDto::Approval => domain::NodeType::Approval,
        NodeTypeDto::Parallel => domain::NodeType::Parallel,
    }
}

fn collect_to_dto(collect: &domain::CollectConfig) -> CollectConfigDto {
    CollectConfigDto {
        from: collect.from.clone(),
        reduce: reduce_strategy_to_dto(&collect.reduce),
    }
}

fn collect_from_dto(collect: CollectConfigDto) -> domain::CollectConfig {
    domain::CollectConfig {
        from: collect.from,
        reduce: reduce_strategy_from_dto(collect.reduce),
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

fn reduce_strategy_from_dto(reduce: ReduceStrategyDto) -> domain::ReduceStrategy {
    match reduce {
        ReduceStrategyDto::Last => domain::ReduceStrategy::Last,
        ReduceStrategyDto::Concat => domain::ReduceStrategy::Concat,
        ReduceStrategyDto::Grouped => domain::ReduceStrategy::Grouped,
        ReduceStrategyDto::AnyNeedsFix => domain::ReduceStrategy::AnyNeedsFix,
        ReduceStrategyDto::AllPassed => domain::ReduceStrategy::AllPassed,
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

fn aggregate_from_dto(aggregate: ParallelAggregateDto) -> domain::ParallelAggregate {
    domain::ParallelAggregate {
        all_match: aggregate.all_match,
        any_match: aggregate.any_match,
        then: aggregate.then,
        r#else: aggregate.r#else,
    }
}

fn transition_rule_to_dto(rule: &domain::TransitionRule) -> TransitionRuleDto {
    TransitionRuleDto {
        r#match: rule.r#match.clone(),
        next: rule.next.clone(),
    }
}

fn transition_rule_from_dto(rule: TransitionRuleDto) -> domain::TransitionRule {
    domain::TransitionRule {
        r#match: rule.r#match,
        next: rule.next,
    }
}

fn cycle_guard_to_dto(guard: &domain::CycleGuard) -> CycleGuardDto {
    CycleGuardDto {
        max_iterations: guard.max_iterations,
        on_exhausted: guard.on_exhausted.clone(),
    }
}

fn cycle_guard_from_dto(guard: CycleGuardDto) -> domain::CycleGuard {
    domain::CycleGuard {
        max_iterations: guard.max_iterations,
        on_exhausted: guard.on_exhausted,
    }
}

fn resolved_facets_to_dto(resolved: &ResolvedFacets) -> ResolvedFacetsDto {
    ResolvedFacetsDto {
        policy: resolved.policy.clone(),
        knowledge: resolved.knowledge.clone(),
        instruction: resolved.instruction.clone(),
        output_contract: resolved.output_contract.clone(),
        input_contracts: resolved.input_contracts.clone(),
    }
}

fn resolved_facets_from_dto(resolved: ResolvedFacetsDto) -> ResolvedFacets {
    ResolvedFacets {
        policy: resolved.policy,
        knowledge: resolved.knowledge,
        instruction: resolved.instruction,
        output_contract: resolved.output_contract,
        input_contracts: resolved.input_contracts,
    }
}

fn run_status_to_dto(status: domain::RunStatus) -> RunStatusDto {
    match status {
        domain::RunStatus::Running => RunStatusDto::Running,
        domain::RunStatus::WaitingApproval => RunStatusDto::WaitingApproval,
        domain::RunStatus::Completed => RunStatusDto::Completed,
        domain::RunStatus::Failed => RunStatusDto::Failed,
        domain::RunStatus::Aborted => RunStatusDto::Aborted,
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
            variables: Default::default(),
            nodes: vec![NodeDefinitionDto {
                name: "step".to_string(),
                node_type: NodeTypeDto::Agent,
                input_contracts: Some(vec!["input".to_string()]),
                output_contract: Some("output".to_string()),
                transition_rules: vec![TransitionRuleDto {
                    r#match: "ok".to_string(),
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
                "nodes": [{
                    "name": "step",
                    "type": "agent",
                    "input_contracts": ["input"],
                    "output_contract": "output",
                    "rules": [{"match": "ok", "next": "done"}]
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
