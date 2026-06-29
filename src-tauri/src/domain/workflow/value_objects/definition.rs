use std::collections::HashMap;

pub const MAX_NODES_PER_WORKFLOW: usize = 256;
pub const MAX_PARALLEL_CHILDREN: usize = 64;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct WorkflowDefinition {
    pub name: String,
    pub description: String,
    pub builtin: bool,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum NodeType {
    #[default]
    Agent,
    Bash,
    Approval,
    Parallel,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct NodeDefinition {
    pub name: String,
    pub node_type: NodeType,
    pub policy: Option<String>,
    pub knowledge: Option<String>,
    pub instruction: Option<String>,
    pub output_contract: Option<String>,
    pub input_contracts: Option<Vec<String>>,
    pub pass_previous_response: Option<bool>,
    pub pass_output_from: Option<Vec<String>>,
    pub inline_prompt: Option<String>,
    pub collect: Option<CollectConfig>,
    pub command: Option<String>,
    pub parallel_children: Option<Vec<ChildNodeDefinition>>,
    pub aggregate: Option<ParallelAggregate>,
    pub transition_rules: Vec<TransitionRule>,
    pub cycle_guard: Option<CycleGuard>,
    pub resets_cycle_for: Option<Vec<String>>,
    pub model: Option<String>,
    pub permission: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChildNodeDefinition {
    pub name: String,
    pub node_type: NodeType,
    pub policy: Option<String>,
    pub knowledge: Option<String>,
    pub instruction: Option<String>,
    pub output_contract: Option<String>,
    pub input_contracts: Option<Vec<String>>,
    pub pass_previous_response: Option<bool>,
    pub pass_output_from: Option<Vec<String>>,
    pub model: Option<String>,
    pub permission: Option<String>,
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

#[derive(Debug, Clone, PartialEq)]
pub struct ParallelAggregate {
    pub all_match: Option<String>,
    pub any_match: Option<String>,
    pub then: String,
    pub r#else: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TransitionRule {
    pub r#match: String,
    pub next: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CycleGuard {
    pub max_iterations: u32,
    pub on_exhausted: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CollectConfig {
    pub from: Vec<String>,
    pub reduce: ReduceStrategy,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReduceStrategy {
    Last,
    Concat,
    Grouped,
    AnyNeedsFix,
    AllPassed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowSummary {
    pub name: String,
    pub description: String,
    pub builtin: bool,
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
}
