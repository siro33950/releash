use std::collections::BTreeMap;

use super::WorkflowDefinition;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DefinitionResolution {
    pub dynamic_fanout_names: std::collections::BTreeSet<String>,
    pub definition_error: Option<String>,
    pub node_errors: BTreeMap<String, String>,
    pub schema_errors: BTreeMap<String, String>,
}

impl DefinitionResolution {
    pub fn node_error(&self, definition: &WorkflowDefinition, name: &str) -> Option<String> {
        if let Some(reason) = &self.definition_error {
            return Some(reason.clone());
        }
        if let Some(reason) = self.node_errors.get(name) {
            return Some(format!("Node definition '{name}' is unavailable: {reason}"));
        }
        let Some(node) = definition.node_by_name(name) else {
            return Some(format!("Node definition '{name}' is unavailable"));
        };
        node.artifact
            .iter()
            .chain(
                node.input
                    .iter()
                    .filter_map(|input| input.contract.as_ref()),
            )
            .find_map(|contract| {
                self.schema_errors
                    .get(contract)
                    .map(|reason| format!("Contract '{contract}' is unavailable: {reason}"))
            })
    }
}

#[cfg(test)]
#[path = "definition_resolution_test.rs"]
mod definition_resolution_tests;
