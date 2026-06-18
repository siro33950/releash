use std::collections::HashMap;

use serde::{Deserialize, Serialize};

pub const MAX_NODES_PER_WORKFLOW: usize = 256;
pub const MAX_PARALLEL_CHILDREN: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct WorkflowDefinition {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub builtin: bool,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub variables: HashMap<String, String>,
    pub nodes: Vec<NodeDefinition>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResolvedFacets {
    pub policy: Option<String>,
    pub knowledge: Option<String>,
    pub instruction: Option<String>,
    pub output_contract: Option<String>,
    pub input_contracts: Vec<String>,
}

impl ResolvedFacets {
    pub fn is_empty(&self) -> bool {
        self.policy.is_none()
            && self.knowledge.is_none()
            && self.instruction.is_none()
            && self.output_contract.is_none()
            && self.input_contracts.is_empty()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub enum NodeType {
    #[default]
    Agent,
    Bash,
    Approval,
    Parallel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct NodeDefinition {
    pub name: String,
    #[serde(rename = "type")]
    pub node_type: NodeType,
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
    pub collect: Option<CollectConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_children: Option<Vec<ChildNodeDefinition>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggregate: Option<ParallelAggregate>,
    #[serde(default, rename = "rules", skip_serializing_if = "Vec::is_empty")]
    pub transition_rules: Vec<TransitionRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle_guard: Option<CycleGuard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_cycle_for: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,
    #[serde(skip)]
    pub resolved_facets: ResolvedFacets,
}

impl NodeDefinition {
    pub fn has_facet_refs(&self) -> bool {
        self.policy.is_some()
            || self.knowledge.is_some()
            || self.instruction.is_some()
            || self.output_contract.is_some()
            || self.input_contracts.as_ref().is_some_and(|v| !v.is_empty())
    }

    pub fn is_parallel(&self) -> bool {
        matches!(self.node_type, NodeType::Parallel)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct ChildNodeDefinition {
    pub name: String,
    #[serde(rename = "type")]
    pub node_type: NodeType,
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
    pub resolved_facets: ResolvedFacets,
}

impl ChildNodeDefinition {
    pub fn has_facet_refs(&self) -> bool {
        self.policy.is_some()
            || self.knowledge.is_some()
            || self.instruction.is_some()
            || self.output_contract.is_some()
            || self.input_contracts.as_ref().is_some_and(|v| !v.is_empty())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ParallelAggregate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all_match: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub any_match: Option<String>,
    pub then: String,
    pub r#else: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TransitionRule {
    pub r#match: String,
    pub next: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CycleGuard {
    pub max_iterations: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_exhausted: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CollectConfig {
    pub from: Vec<String>,
    pub reduce: ReduceStrategy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReduceStrategy {
    Last,
    Concat,
    Grouped,
    AnyNeedsFix,
    AllPassed,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WorkflowSummary {
    pub name: String,
    pub description: String,
    pub builtin: bool,
    #[serde(default)]
    pub is_running: bool,
}

#[cfg(test)]
mod definition_tests {
    use super::*;

    #[test]
    fn test_node_definition_facet参照を検出する() {
        let mut node = NodeDefinition::default();
        assert!(!node.has_facet_refs());
        node.output_contract = Some("spec-directory".to_string());
        assert!(node.has_facet_refs());
    }

    #[test]
    fn test_reduce_strategy_snake_case互換を保つ() {
        let value = serde_json::to_value(ReduceStrategy::AnyNeedsFix).unwrap();
        assert_eq!(value, serde_json::json!("any_needs_fix"));
    }
}
