use std::collections::{BTreeMap, BTreeSet};

pub const MAX_NODES_PER_WORKFLOW: usize = 256;
pub const MAX_PARALLEL_CHILDREN: usize = 64;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct WorkflowDefinition {
    pub name: String,
    pub description: String,
    pub builtin: bool,
    pub schemas: BTreeMap<String, SchemaDef>,
    pub nodes: Vec<NodeDefinition>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SchemaDef {
    Object {
        properties: BTreeMap<String, SchemaDef>,
        required: BTreeSet<String>,
        additional_properties: bool,
    },
    Array {
        items: String,
    },
    String {
        r#enum: Option<Vec<String>>,
    },
    Boolean,
    Integer,
    Number,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum NodeKindName {
    Command,
    #[default]
    Session,
    Fanout,
}

impl NodeKindName {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Session => "session",
            Self::Fanout => "fanout",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    Command(CommandSpec),
    Session(SessionSpec),
    Fanout(FanoutSpec),
}

impl Default for NodeKind {
    fn default() -> Self {
        Self::Session(SessionSpec::default())
    }
}

impl NodeKind {
    pub fn name(&self) -> NodeKindName {
        match self {
            Self::Command(_) => NodeKindName::Command,
            Self::Session(_) => NodeKindName::Session,
            Self::Fanout(_) => NodeKindName::Fanout,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CommandSpec {
    pub command: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SessionGate {
    #[default]
    Auto,
    Approval,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct FacetRefs {
    pub policy: Option<String>,
    pub knowledge: Option<String>,
    pub instruction: Option<String>,
}

impl FacetRefs {
    pub fn is_empty(&self) -> bool {
        self.policy.is_none() && self.knowledge.is_none() && self.instruction.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SessionSpec {
    pub model: Option<String>,
    pub permission: Option<String>,
    pub gate: SessionGate,
    pub facets: FacetRefs,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct FanoutSpec {
    pub parallel_children: Vec<InterimChild>,
    pub aggregate: Option<ParallelAggregate>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct NodeDefinition {
    pub name: String,
    pub kind: NodeKind,
    pub artifact: Option<String>,
    pub input: Option<String>,
    pub inputs: Vec<String>,
    pub collect: Option<CollectConfig>,
    pub transition_rules: Vec<TransitionRule>,
    pub cycle_guard: Option<CycleGuard>,
    pub resets_cycle_for: Option<Vec<String>>,
}

impl NodeDefinition {
    pub fn has_facet_refs(&self) -> bool {
        self.session()
            .is_some_and(|session| !session.facets.is_empty())
    }

    pub fn kind_name(&self) -> NodeKindName {
        self.kind.name()
    }

    pub fn is_command(&self) -> bool {
        matches!(self.kind, NodeKind::Command(_))
    }

    pub fn is_session(&self) -> bool {
        matches!(self.kind, NodeKind::Session(_))
    }

    pub fn is_approval_session(&self) -> bool {
        self.session()
            .is_some_and(|session| session.gate == SessionGate::Approval)
    }

    pub fn is_fanout(&self) -> bool {
        matches!(self.kind, NodeKind::Fanout(_))
    }

    pub fn command(&self) -> Option<&str> {
        match &self.kind {
            NodeKind::Command(spec) => Some(spec.command.as_str()),
            _ => None,
        }
    }

    pub fn session(&self) -> Option<&SessionSpec> {
        match &self.kind {
            NodeKind::Session(spec) => Some(spec),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn session_mut(&mut self) -> Option<&mut SessionSpec> {
        match &mut self.kind {
            NodeKind::Session(spec) => Some(spec),
            _ => None,
        }
    }

    pub fn fanout(&self) -> Option<&FanoutSpec> {
        match &self.kind {
            NodeKind::Fanout(spec) => Some(spec),
            _ => None,
        }
    }

    pub fn model(&self) -> Option<&str> {
        self.session().and_then(|session| session.model.as_deref())
    }

    pub fn permission(&self) -> Option<&str> {
        self.session()
            .and_then(|session| session.permission.as_deref())
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct InterimChild {
    pub name: String,
    pub model: Option<String>,
    pub permission: Option<String>,
    pub facets: FacetRefs,
    pub artifact: Option<String>,
    pub input: Option<String>,
}

impl InterimChild {
    pub fn has_facet_refs(&self) -> bool {
        !self.facets.is_empty()
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
        if let Some(session) = node.session_mut() {
            session.facets.instruction = Some("spec-authoring".to_string());
        }
        assert!(node.has_facet_refs());
    }
}
