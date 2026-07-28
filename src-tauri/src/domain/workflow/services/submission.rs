//! Node output reset rules.

use crate::domain::workflow::value_objects::WorkflowDefinition;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmissionViolation {
    MissingSubmitOutput,
    InvalidSubmitOutput,
}

pub fn submission_violation_reason(violation: SubmissionViolation) -> &'static str {
    match violation {
        SubmissionViolation::MissingSubmitOutput => "missing_submit_output",
        SubmissionViolation::InvalidSubmitOutput => "invalid_submit_output",
    }
}

pub fn artifact_keys_to_clear_for_new_node_execution(
    workflow: &WorkflowDefinition,
    node_index: usize,
) -> Vec<String> {
    let Some(node) = workflow.nodes.get(node_index) else {
        return Vec::new();
    };
    // Fanout child artifacts are retained only in the parent artifact array. They are not
    // addressable through the workflow-wide node-name output map, so only the parent key can
    // be stale when a new execution starts.
    vec![node.name.clone()]
}

#[cfg(test)]
mod submission_tests {
    use super::*;
    use crate::domain::workflow::value_objects::{
        FanoutSpec, NodeDefinition, NodeKind, SessionSpec, WorkflowDefinition,
    };

    fn session_node(name: &str) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Session(SessionSpec::default()),
            ..Default::default()
        }
    }

    fn fanout_node(name: &str, children: Vec<&str>) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Fanout(FanoutSpec {
                child: children.into_iter().map(str::to_string).collect(),
                items: None,
            }),
            ..Default::default()
        }
    }

    fn workflow(nodes: Vec<NodeDefinition>) -> WorkflowDefinition {
        WorkflowDefinition {
            name: "wf".to_string(),
            description: String::new(),
            builtin: false,
            schemas: Default::default(),
            nodes,
        }
    }

    #[test]
    fn output_keys_to_clear_include_only_fanout_parent() {
        let fanout = fanout_node("fanout-review", vec!["quality", "security"]);
        let workflow = workflow(vec![session_node("draft"), fanout]);

        assert_eq!(
            artifact_keys_to_clear_for_new_node_execution(&workflow, 1),
            vec!["fanout-review"]
        );
        assert_eq!(
            artifact_keys_to_clear_for_new_node_execution(&workflow, 0),
            vec!["draft"]
        );
        assert!(artifact_keys_to_clear_for_new_node_execution(&workflow, 99).is_empty());
    }
}
