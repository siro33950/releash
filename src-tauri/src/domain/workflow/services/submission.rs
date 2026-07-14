//! Node output reset rules.

use crate::domain::workflow::value_objects::WorkflowDefinition;

pub fn step_output_keys_to_clear_for_new_execution(
    workflow: &WorkflowDefinition,
    step_index: usize,
) -> Vec<String> {
    let Some(step) = workflow.nodes.get(step_index) else {
        return Vec::new();
    };
    // Fanout child artifacts are retained only in the parent artifact array. They are not
    // addressable through the workflow-wide node-name output map, so only the parent key can
    // be stale when a new execution starts.
    vec![step.name.clone()]
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
                aggregate: None,
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
        let parallel = fanout_node("parallel-review", vec!["quality", "security"]);
        let workflow = workflow(vec![session_node("draft"), parallel]);

        assert_eq!(
            step_output_keys_to_clear_for_new_execution(&workflow, 1),
            vec!["parallel-review"]
        );
        assert_eq!(
            step_output_keys_to_clear_for_new_execution(&workflow, 0),
            vec!["draft"]
        );
        assert!(step_output_keys_to_clear_for_new_execution(&workflow, 99).is_empty());
    }
}
