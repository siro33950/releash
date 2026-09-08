//! YAML workflow schema boundary.
//!
//! Workflow semantics and serde shape are owned by domain value objects.  The
//! adaptor keeps only names used by file/network boundaries and presenter DTOs.

use serde::Serialize;

pub use crate::domain::workflow::{
    CommandSpec, FacetRefs, FanoutSpec, ItemsSource, NodeDefinition, NodeKind, Rule, SchemaDef,
    SequenceSpec, SessionSpec, WorkflowDefinition as WorkflowDefinitionYaml,
};
#[cfg(test)]
pub use crate::domain::workflow::{NodeCompletion, NodeKindName};

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Summary {
    pub name: String,
    pub description: String,
    pub builtin: bool,
    #[serde(default)]
    pub is_running: bool,
    pub source_format: crate::domain::workflow::WorkflowSourceFormat,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FacetSummary {
    pub key: String,
    pub kind: String,
    pub description: String,
    pub builtin: bool,
}
